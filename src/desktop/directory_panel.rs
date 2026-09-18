//! Directory-mode orchestration and presentation. The receiver ledger remains
//! the sole persisted truth; DeliveryRecord is only an adapter for shared lists.
use super::*;
use crate::config::directory_state_dir;
use crate::directory::receiver::{Applied, ReceiveEvent, Receiver, Root, validate_history};
use crate::directory::{Inventory, InventoryUpdate, client::DirectoryClient};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(super) struct DirectorySnapshot {
    pub files: BTreeMap<String, Applied>,
}

pub(super) struct Prepared {
    registration: BridgeRegistration,
    config: Config,
    inventory: Inventory,
    identity: (u64, u64),
}

fn http(config: &Config) -> Result<(&str, &str, bool)> {
    match &config.server {
        ServerConfig::Http {
            base_url,
            token_env,
            allow_insecure_http,
            ..
        } => Ok((base_url, token_env, *allow_insecure_http)),
        _ => bail!("Directory sync requires a paired HTTP Folder."),
    }
}

fn unused_folder(client: &DirectoryClient) -> Result<()> {
    let remote = client.state()?;
    anyhow::ensure!(
        remote.receiver.is_none() && remote.entries.is_empty(),
        "This server Folder already has directory state. Restore its original Linux configuration and state, or create a new paired Folder. Nothing was reset."
    );
    Ok(())
}

fn empty_delivery_queue(config: &Config, token: &str) -> Result<()> {
    use crate::source::DeliverySource;
    let (url, _, insecure) = http(config)?;
    let source = crate::http_source::HttpSource::new(url, token, 10, 100, insecure)?;
    let scan = source.scan_pending(crate::directory::receiver::MAX_FILE)?;
    anyhow::ensure!(
        scan.deliveries.is_empty() && scan.issues.is_empty(),
        "This Folder still has delivery-only files waiting on the server. Receive them with its original delivery setup, or create a new paired Folder."
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    paths: &DesktopPaths,
    name: String,
    directory: PathBuf,
    url: &str,
    insecure: bool,
    automatic: bool,
    token: &str,
) -> Result<Prepared> {
    let id = Uuid::new_v4().to_string();
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(paths.bridge_data_dir.join(&id)),
        library_dir: Some(directory),
        ..Default::default()
    })?;
    config.directory_sync = true;
    set_http_server(&mut config, url.trim_end_matches('/'), insecure);
    config.validate()?;
    let registration = BridgeRegistration {
        kind: Default::default(),
        id: id.clone(),
        name,
        config_path: paths.bridge_config_dir.join(format!("{id}.toml")),
        auto_receive: automatic,
    };
    let registry = paths.registry.load()?;
    ensure_resources_unique(&registry, &BridgeResources::from_config(&config), None)?;
    let mut preview = registry.clone();
    preview.add(registration.clone())?;
    let root = Root::open(&config.storage.library_dir)?;
    let identity = root.identity()?;
    let inventory = root.inventory()?;
    unused_folder(&DirectoryClient::new(
        http(&config)?.0,
        token,
        insecure,
        "receiver",
    )?)?;
    empty_delivery_queue(&config, token)?;
    Ok(Prepared {
        registration,
        config,
        inventory,
        identity,
    })
}

/// Confirmation rechecks the preview before creating local state. Once the
/// registration is durable, a lost publish response is retryable with Receive.
pub(super) fn initialize(
    paths: &DesktopPaths,
    prepared: Prepared,
    token: &str,
) -> Result<(String, Option<String>)> {
    let Prepared {
        registration,
        config,
        inventory,
        identity,
    } = prepared;
    let root = Root::open(&config.storage.library_dir)?;
    anyhow::ensure!(
        root.identity()? == identity && root.inventory()? == inventory,
        "Directory changed since review. Review it again; nothing was initialized."
    );
    let (url, _, insecure) = http(&config)?;
    let client = DirectoryClient::new(url, token, insecure, "receiver")?;
    unused_folder(&client)?;
    empty_delivery_queue(&config, token)?;
    let state = directory_state_dir(&config);
    let id = registration.id.clone();
    let mut receiver = None;
    paths.registry.update(|registry| {
        ensure_resources_unique(registry, &BridgeResources::from_config(&config), None)?;
        let mut preview = registry.clone();
        preview.add(registration.clone())?;
        anyhow::ensure!(
            !registration.config_path.try_exists()?,
            "Folder configuration already exists."
        );
        config.ensure_directories()?;
        receiver = Some(Receiver::open(&config.storage.library_dir, &state, url)?);
        anyhow::ensure!(
            receiver.as_ref().context("Missing receiver")?.identity()? == identity,
            "Directory was replaced during initialization. No Folder was registered."
        );
        config.save(&registration.config_path, false)?;
        registry.add(registration)
    })?;
    // Keep the receiver lock until publication finishes. No incoming file is
    // downloaded by this setup operation; Android must confirm its own preview.
    let result = client.publish(&InventoryUpdate {
        previous_id: None,
        inventory,
    });
    drop(receiver);
    Ok((
        id,
        result.err().map(|error| {
            format!("Folder saved; publishing was not confirmed: {error}. Use Receive to retry.")
        }),
    ))
}

pub(super) fn snapshot(config: Config) -> Result<BridgeSnapshot> {
    let (url, _, _) = http(&config)?;
    let files = Receiver::inspect(
        &config.storage.library_dir,
        &directory_state_dir(&config),
        url,
    )?;
    let deliveries: Vec<_> = files
        .iter()
        .map(|(path, file)| record(&config.storage.library_dir, path, file))
        .collect();
    let counts = BridgeCounts {
        total: files.len(),
        acknowledged: files.values().filter(|v| v.acknowledged).count(),
        ack_pending: files.values().filter(|v| !v.acknowledged).count(),
        ..Default::default()
    };
    Ok(BridgeSnapshot {
        connection_state: config.connection_state,
        name: bridge_name_for_path(&config.storage.library_dir),
        device_id: config.device_id.clone(),
        source_label: format!("Directory sync · {url}"),
        library_dir: config.storage.library_dir,
        max_file_size_bytes: config.limits.max_file_size_bytes,
        wallpaper_label: "Disabled in directory sync".into(),
        counts,
        deliveries: Arc::new(deliveries.into()),
        directory: Some(Arc::new(DirectorySnapshot { files })),
    })
}

pub(super) fn record(root: &Path, path: &str, file: &Applied) -> DeliveryRecord {
    DeliveryRecord {
        id: format!("directory:{}:{path}", file.version),
        original_name: path.into(),
        media_type: file.media_type.clone(),
        sha256: file.sha256.clone(),
        size: file.size,
        stored_path: root.join(path),
        delivery_status: if file.acknowledged {
            DeliveryStatus::Acknowledged
        } else {
            DeliveryStatus::AckPending
        },
        wallpaper_status: WallpaperStatus::NotApplicable,
        wallpaper_attempts: 0,
        // A UI-only attention marker; never written into delivery-only state.
        delivery_error: file
            .conflict
            .then(|| "Conflict copy kept. Review the retained local copy.".into()),
        wallpaper_error: None,
        source_created_at_unix: None,
        received_at_unix: file.received_at_unix,
        acknowledged_at_unix: None,
        imported_at_unix: None,
    }
}

pub(super) fn receive(
    config: Config,
    token: Option<&str>,
    emit: &dyn Fn(SyncEvent),
) -> Result<SyncResult> {
    let (url, environment, insecure) = http(&config)?;
    let secret = token.map(str::to_owned).or_else(|| std::env::var(environment).ok()).context("Set the receiver session token in Folder Settings, or its credential environment variable.")?;
    // Refuse missing/tampered state before any network request or re-binding.
    let before = Receiver::inspect(
        &config.storage.library_dir,
        &directory_state_dir(&config),
        url,
    )?;
    let client = DirectoryClient::new(url, &secret, insecure, "receiver")?;
    let source = source_for(&config, Some(&secret))?;
    let mut receiver = Receiver::open(
        &config.storage.library_dir,
        &directory_state_dir(&config),
        url,
    )?;
    let result = receiver.sync_with_events(&client, source.as_ref(), &|event| match event {
        ReceiveEvent::Active(entry, phase) => emit(SyncEvent::FileActive {
            id: format!("directory:{}:{}", entry.version, entry.path),
            original_name: entry.path.clone(),
            media_type: entry.media_type.clone(),
            size: entry.size,
            phase,
        }),
        ReceiveEvent::Settled(entry, file) => emit(SyncEvent::FileSettled {
            id: format!("directory:{}:{}", file.version, entry.path),
            record: Some(Box::new(record(
                &config.storage.library_dir,
                &entry.path,
                file,
            ))),
            error: None,
        }),
    });
    let mut summary = SyncSummary {
        received: receiver
            .files()
            .iter()
            .filter(|(path, next)| {
                before
                    .get(*path)
                    .is_none_or(|old| old.version != next.version)
            })
            .count(),
        acknowledged: receiver
            .files()
            .iter()
            .filter(|(path, next)| {
                next.acknowledged
                    && before
                        .get(*path)
                        .is_none_or(|old| !old.acknowledged || old.version != next.version)
            })
            .count(),
        ..Default::default()
    };
    if let Err(error) = result {
        summary.failures.push(crate::sync::SyncFailure {
            item: "Directory sync".into(),
            message: format!("{error:#}"),
        });
    }
    drop(receiver); // snapshot uses a shared lock, never re-enter the writer.
    Ok(SyncResult {
        summary,
        snapshot: snapshot(config)?,
    })
}

pub(super) fn review(
    ui: &Rc<DesktopUi>,
    parent: &adw::Window,
    task: &pairing_panel::SetupTask,
    prepared: Prepared,
    token: String,
    error: &gtk::Label,
) {
    let dialog = gtk::MessageDialog::builder().transient_for(parent).modal(true).message_type(gtk::MessageType::Question)
        .buttons(gtk::ButtonsType::Cancel).text("Initialize directory sync?")
        .secondary_text(format!("{}\n{} existing files · {}\n\nFile paths, sizes and hashes will be published to your relay. Future received versions keep their original paths. Replaced files are retained; deletions do not propagate. Nothing is uploaded from Linux.\n\nAndroid must review its own directory before sending. This Folder's path, mode and server cannot be changed after initialization.", prepared.config.storage.library_dir.display(), prepared.inventory.entries.len(), format_bytes(prepared.inventory.entries.iter().map(|v| v.size).sum()))).build();
    dialog.add_button("Initialize", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let prepared = Rc::new(RefCell::new(Some(prepared)));
    let (ui, parent, task, error) = (ui.clone(), parent.clone(), task.clone(), error.clone());
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept
            && let Some(prepared) = prepared.borrow_mut().take()
        {
            let (paths, secret) = (ui.paths.clone(), token.clone());
            let (ui, parent, error, token) =
                (ui.clone(), parent.clone(), error.clone(), token.clone());
            task.run(
                move || initialize(&paths, prepared, &secret),
                move |result| match result {
                    Ok((id, warning)) => {
                        if !token.is_empty() {
                            ui.tokens.borrow_mut().insert(id, token);
                        }
                        parent.close();
                        ui.start_load_all();
                        ui.widgets
                            .toast_overlay
                            .add_toast(adw::Toast::new(warning.as_deref().unwrap_or(
                            "Directory initialized. Review and initialize the source on Android.",
                        )));
                    }
                    Err(failure) => {
                        error.set_label(&safe_ui_message(&format!("{failure:#}"), 1800));
                        error.set_visible(true);
                    }
                },
            );
        }
        dialog.close();
    });
    dialog.present();
}

pub(super) fn history_button(root: &Path, path: &str, file: &Applied) -> Option<gtk::Button> {
    let history = file.history.as_ref()?;
    if validate_history(path, history).is_err() {
        return None;
    }
    let absolute = root.join(history);
    let button = gtk::Button::builder()
        .icon_name("document-open-recent-symbolic")
        .tooltip_text("Retained copy for this version")
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    let (path, file) = (path.to_owned(), file.clone());
    button.connect_clicked(move |button| {
        let dialog = gtk::MessageDialog::builder().modal(true).buttons(gtk::ButtonsType::Close).text(if file.conflict { "Conflict copy kept" } else { "Previous version kept" })
            .secondary_text(format!("{path}\nSource version {}\n\nRetained copy:\n{}\n\nNo file was deleted. This shows the copy retained for the current version; older copies remain on disk.", file.version, absolute.display())).build();
        if let Some(parent) = button.root().and_then(|root| root.downcast::<gtk::Window>().ok()) { dialog.set_transient_for(Some(&parent)); }
        // Selectable path, no implicit execution of an arbitrary received file.
        if let Some(label) = dialog.message_area().last_child().and_then(|widget| widget.downcast::<gtk::Label>().ok()) { label.set_selectable(true); }
        dialog.connect_response(|dialog, _| dialog.close()); dialog.present();
    });
    Some(button)
}
