use super::*;
use crate::{
    config::ConnectionState, pairing::PairingClient, protocol::FolderExitStatus, state::StateLock,
};

/// A queued desktop receive reloads its config only AFTER taking this lock.
/// Keep lock inodes: unlinking a lock could let a stale waiter bypass its owner.
pub(super) fn operation_lock(config: &Path) -> Result<StateLock> {
    StateStore::new(config.with_extension("operation")).lock_exclusive()
}

fn remove_owned_tree(path: &Path, depth: usize) -> Result<()> {
    anyhow::ensure!(depth <= 8, "Unexpected nesting in app-owned state");
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        !meta.file_type().is_symlink(),
        "Refusing symlink in cleanup: {}",
        path.display()
    );
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            remove_owned_tree(&entry?.path(), depth + 1)?;
        }
        // State lock files and retirement markers intentionally survive.
        if std::fs::read_dir(path)?.next().is_none() {
            std::fs::remove_dir(path)?;
        }
    } else {
        anyhow::ensure!(meta.is_file(), "Unexpected file type in app-owned state");
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .context("Invalid state filename")?;
        if name.ends_with(".lock") || name == "retired.json" || name.ends_with(".retired") {
            return Ok(());
        }
        std::fs::remove_file(path)?;
    }
    if let Some(parent) = path.parent() {
        crate::fsutil::sync_directory(parent)?;
    }
    Ok(())
}

fn clean_local(paths: &DesktopPaths, id: &str, config_path: &Path, config: &Config) -> Result<()> {
    anyhow::ensure!(
        Uuid::parse_str(id)?.to_string() == id,
        "Invalid local Folder identity"
    );
    let owned = paths.bridge_data_dir.join(id);
    anyhow::ensure!(
        config_path == paths.bridge_config_dir.join(format!("{id}.toml"))
            && config.storage.state_file == owned.join("state.json"),
        "This imported configuration has no verified app-owned cleanup scope. Keep it stopped and review its paths manually."
    );
    let owned = std::fs::canonicalize(&owned)?;
    anyhow::ensure!(
        owned == paths.bridge_data_dir.join(id),
        "Refusing redirected cleanup root"
    );
    let library = std::fs::canonicalize(&config.storage.library_dir)?;
    anyhow::ensure!(
        !library.starts_with(&owned) && !owned.starts_with(&library),
        "App state and original files overlap; manual review required"
    );
    let _state = StateStore::new(config.storage.state_file.clone()).lock_exclusive()?;
    let directory = crate::config::directory_state_dir(config);
    let mut locks = Vec::new();
    if directory.exists() {
        anyhow::ensure!(
            std::fs::canonicalize(&directory)? == directory,
            "Refusing redirected directory state"
        );
        locks.push(StateStore::new(directory.join("receiver.json")).lock_exclusive()?);
        locks.push(StateStore::new(directory.join("sender.json")).lock_exclusive()?);
        crate::fsutil::atomic_write(&directory.join("retired.json"), b"{\"retired\":true}")?;
    }
    let retired = config.storage.state_file.with_extension("json.retired");
    crate::fsutil::atomic_write(&retired, b"{\"retired\":true}")?;
    remove_owned_tree(&directory, 0)?;
    remove_owned_tree(&config.storage.state_file, 0)?;
    Ok(())
}

pub(super) fn run(paths: &DesktopPaths, id: &str, supplied: &str) -> Result<FolderExitStatus> {
    let mut saved = None;
    paths.registry.update(|registry| {
        let entry = registry.bridges.iter().find(|b| b.id == id).context("Folder no longer exists")?;
        let path = entry.config_path.clone();
        let mut config = load_editable_config(&path)?;
        anyhow::ensure!(matches!(&config.server, ServerConfig::Http { base_url, .. } if reqwest::Url::parse(base_url).is_ok_and(|u| u.path().contains("/f/"))), "Clean exit requires independent Folder credentials");
        anyhow::ensure!(config.connection_state != ConnectionState::Disconnected, "This older disconnected Folder no longer has a guaranteed receipt credential");
        if !matches!(config.connection_state, ConnectionState::ExitLocalCleaned | ConnectionState::Exited) {
            config.connection_state = ConnectionState::ExitPending;
            config.save(&path, true)?;
        }
        saved = Some((path, config));
        registry.set_auto_receive(id, false)
    })?;
    let (path, config) = saved.context("Missing Folder configuration")?;
    let ServerConfig::Http {
        base_url,
        token_env,
        allow_insecure_http,
        ..
    } = &config.server
    else {
        unreachable!()
    };
    let token = if supplied.trim().is_empty() {
        std::env::var(token_env)
            .context("Enter this Folder's receiver token to resume clean exit")?
    } else {
        supplied.trim().into()
    };
    let client = PairingClient::new(base_url, &token, *allow_insecure_http)?;
    anyhow::ensure!(
        client.exit_status()?.role == "receiver",
        "Use a receiver credential on Linux"
    );
    let status = client.clean_exit()?;
    anyhow::ensure!(
        status.role == "receiver",
        "Use a receiver credential on Linux"
    );
    let _operation = operation_lock(&path)?;
    let mut current = load_registered_config(&paths.registry, id, &path)?;
    anyhow::ensure!(
        current == config,
        "Folder configuration changed during exit; retry"
    );
    if !matches!(
        current.connection_state,
        ConnectionState::ExitLocalCleaned | ConnectionState::Exited
    ) {
        clean_local(paths, id, &path, &current)?;
        current.connection_state = ConnectionState::ExitLocalCleaned;
        current.save(&path, true)?;
    }
    let receipt = client.acknowledge_exit()?;
    anyhow::ensure!(
        receipt.receiver_cleaned && receipt.role == "receiver",
        "Local exit acknowledgement was not confirmed"
    );
    if receipt.complete() {
        current.connection_state = ConnectionState::Exited;
        current.save(&path, true)?;
    }
    Ok(receipt)
}

/// Called from the existing receive/check cycle, never a polling service.
/// Once requested by the peer, local cleanup is retried instead of receiving.
pub(super) fn resume_if_requested(paths: &DesktopPaths, request: &SyncRequest) -> Result<bool> {
    let config = load_registered_config(&paths.registry, &request.bridge_id, &request.config_path)?;
    let ServerConfig::Http {
        base_url,
        token_env,
        allow_insecure_http,
        ..
    } = &config.server
    else {
        return Ok(false);
    };
    if !reqwest::Url::parse(base_url).is_ok_and(|u| u.path().contains("/f/")) {
        return Ok(false);
    }
    if matches!(
        config.connection_state,
        ConnectionState::Disconnected | ConnectionState::Exited
    ) {
        return Ok(false);
    }
    let token = request
        .token
        .clone()
        .or_else(|| std::env::var(token_env).ok())
        .context("Enter the Folder receiver token")?;
    let pending = matches!(
        config.connection_state,
        ConnectionState::ExitPending | ConnectionState::ExitLocalCleaned
    );
    if !pending {
        let client = PairingClient::new(base_url, &token, *allow_insecure_http)?;
        // An old relay may not implement retirement. Ordinary transfer behavior
        // remains unchanged; a known pending exit must never fall back to it.
        let Ok(status) = client.exit_status() else {
            return Ok(false);
        };
        if !status.requested {
            return Ok(false);
        }
    }
    run(paths, &request.bridge_id, &token)?;
    Ok(true)
}

pub(super) fn forget_receipt(
    paths: &DesktopPaths,
    registration: &BridgeRegistration,
) -> Result<()> {
    let config = load_editable_config(&registration.config_path)?;
    if config.connection_state != ConnectionState::Exited {
        return Ok(());
    }
    anyhow::ensure!(
        registration.config_path
            == paths
                .bridge_config_dir
                .join(format!("{}.toml", registration.id))
            && std::fs::canonicalize(&registration.config_path)? == registration.config_path,
        "Refusing unowned exit receipt path"
    );
    std::fs::remove_file(&registration.config_path)?;
    crate::fsutil::sync_directory(&paths.bridge_config_dir)?;
    Ok(())
}

pub(super) fn add_button(
    group: &adw::PreferencesGroup,
    ui: &Rc<DesktopUi>,
    editor: &adw::Window,
    task: &pairing_panel::SetupTask,
    registration: &BridgeRegistration,
    config: &Config,
    token: &gtk::PasswordEntry,
) {
    let retry = matches!(
        config.connection_state,
        ConnectionState::ExitPending | ConnectionState::ExitLocalCleaned
    );
    let button = gtk::Button::with_label(if retry {
        "Retry / check clean exit"
    } else {
        "Clean exit"
    });
    button.set_widget_name("folder-clean-exit");
    button.add_css_class("destructive-action");
    group.add(&button);
    let ui = ui.clone();
    let editor = editor.clone();
    let task = task.clone();
    let token = token.clone();
    let id = registration.id.clone();
    button.connect_clicked(move |_| {
        let dialog = gtk::MessageDialog::builder().transient_for(&editor).modal(true)
            .message_type(gtk::MessageType::Warning).buttons(gtk::ButtonsType::Cancel)
            .text("Clean exit from this Folder?")
            .secondary_text("Permanently revoke both devices and remove relay files, unfinished uploads and MiRelay-managed local state. Pending files will not be delivered. Phone originals and Linux received files are kept. An offline peer remains pending until it opens MiRelay and finishes cleanup. A minimal exit receipt is retained.").build();
        dialog.add_button("Clean exit", gtk::ResponseType::Accept);
        let ui = ui.clone(); let editor = editor.clone(); let task = task.clone(); let token = token.clone(); let id = id.clone();
        dialog.connect_response(move |dialog, response| {
            dialog.close(); if response != gtk::ResponseType::Accept { return; }
            let paths = ui.paths.clone(); let id_work = id.clone(); let supplied = token.text().to_string();
            if !supplied.trim().is_empty() { ui.tokens.borrow_mut().insert(id.clone(), supplied.trim().into()); }
            let ui = ui.clone(); let editor = editor.clone(); let id = id.clone();
            task.run(move || run(&paths, &id_work, &supplied), move |result| {
                editor.close();
                match result {
                    Ok(status) => {
                        if status.complete() { ui.tokens.borrow_mut().remove(&id); }
                        ui.widgets.toast_overlay.add_toast(adw::Toast::new(if status.complete() { "Clean exit complete. Original files kept." } else { "Relay and Linux cleaned. Waiting for Android; keep this exit receipt." }));
                    }
                    Err(e) => ui.show_error(&format!("Clean exit is pending; transfers stay stopped. Reopen Folder settings to retry. {e:#}")),
                }
                ui.start_load_all();
            });
        }); dialog.present();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    const SENDER: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exit_drains_receiver_cleans_owned_state_preserves_originals_and_waits_for_peer() {
        directory_tests::scenario(|root, paths, url, token| {
            let (id, config, path) = directory_tests::setup(root, &paths, url, token);
            let original = config.storage.library_dir.join("keep.txt");
            std::fs::write(&original, b"user original").unwrap();
            assert!(run(&paths, &id, "bad").is_err());
            assert_eq!(
                Config::load(&path).unwrap().connection_state,
                ConnectionState::ExitPending
            );
            assert!(!paths.registry.load().unwrap().bridges[0].auto_receive);
            let pending = run(&paths, &id, token).unwrap();
            assert!(pending.server_cleaned && pending.receiver_cleaned && !pending.sender_cleaned);
            let local = Config::load(&path).unwrap();
            assert_eq!(local.connection_state, ConnectionState::ExitLocalCleaned);
            assert!(!connection::can_remove(&local));
            assert!(
                !crate::config::directory_state_dir(&config)
                    .join("receiver.json")
                    .exists()
            );
            assert!(
                crate::directory::receiver::Receiver::open(
                    &config.storage.library_dir,
                    &crate::config::directory_state_dir(&config),
                    url
                )
                .is_err()
            );
            assert!(snapshot_from_config(local).is_ok());
            assert!(!run(&paths, &id, token).unwrap().complete());
            let peer = PairingClient::new(url, SENDER, true).unwrap();
            assert!(peer.acknowledge_exit().unwrap().complete());
            assert!(run(&paths, &id, token).unwrap().complete());
            assert!(connection::can_remove(&Config::load(&path).unwrap()));
            assert_eq!(std::fs::read(&original).unwrap(), b"user original");
            let entry = paths.registry.load().unwrap().bridges[0].clone();
            paths.registry.update(|r| r.remove(&id).map(drop)).unwrap();
            forget_receipt(&paths, &entry).unwrap();
            assert!(!path.exists());
            assert!(original.exists());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cleanup_rejects_symlink_and_resumes_without_touching_its_target() {
        directory_tests::scenario(|root, paths, url, token| {
            let (id, config, path) = directory_tests::setup(root, &paths, url, token);
            let original = config.storage.library_dir.join("keep.txt");
            std::fs::write(&original, b"keep").unwrap();
            let injected = crate::config::directory_state_dir(&config).join("unexpected-link");
            std::os::unix::fs::symlink(&original, &injected).unwrap();
            assert!(run(&paths, &id, token).is_err());
            assert_eq!(std::fs::read(&original).unwrap(), b"keep");
            assert_eq!(
                Config::load(&path).unwrap().connection_state,
                ConnectionState::ExitPending
            );
            std::fs::remove_file(&injected).unwrap();
            assert!(run(&paths, &id, token).unwrap().receiver_cleaned);
        })
        .await;
    }
}
