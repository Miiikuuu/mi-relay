use super::*;
use crate::{config::ConnectionState, pairing::PairingClient};

pub(super) fn status(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Connected => "Ready",
        ConnectionState::DisconnectPending => "Disconnect pending",
        ConnectionState::Disconnected => "Disconnected",
        ConnectionState::ExitPending => "Clean exit pending",
        ConnectionState::ExitLocalCleaned => "Waiting for peer cleanup",
        ConnectionState::Exited => "Clean exit complete",
    }
}

pub(super) fn empty_message(state: ConnectionState, directory: bool) -> &'static str {
    match state {
        ConnectionState::Connected if directory => {
            "Initialize the source on Android, then select Receive. Existing Linux-only files stay unchanged."
        }
        ConnectionState::Connected => "Select Receive to check for new files.",
        ConnectionState::DisconnectPending => {
            "Transfers are stopped. Retry disconnect in Folder settings. Original files are kept."
        }
        ConnectionState::Disconnected => {
            "Transfers are stopped. Original files are kept. Create a new Folder to reconnect."
        }
        ConnectionState::ExitPending => {
            "Transfers are stopped. Retry clean exit in Folder settings. Keep this receipt until cleanup completes."
        }
        ConnectionState::ExitLocalCleaned => {
            "Local cleanup is complete. Check clean exit in Folder settings after the other device finishes. Original files are kept."
        }
        ConnectionState::Exited => {
            "Both devices and the relay confirmed cleanup. Original files are kept. You can remove this receipt in Folder settings."
        }
    }
}

pub(super) fn removal_message(state: ConnectionState) -> &'static str {
    if state == ConnectionState::Exited {
        "Remove this completed exit receipt and its MiRelay-owned configuration. Original and received files stay in place. Safety lock files and retirement markers remain."
    } else {
        "This only removes the Folder from MiRelay. Its configuration, state, and files remain. For legacy connections, shared server credentials are NOT revoked."
    }
}

fn scoped(config: &Config) -> bool {
    matches!(&config.server, ServerConfig::Http { base_url, .. }
        if reqwest::Url::parse(base_url).is_ok_and(|url| url.path().contains("/f/")))
}

pub(super) fn can_remove(config: &Config) -> bool {
    !scoped(config)
        || matches!(
            config.connection_state,
            ConnectionState::Disconnected | ConnectionState::Exited
        )
}

fn disconnect(paths: &DesktopPaths, id: &str, supplied_token: &str) -> Result<()> {
    // Persist the fail-closed state before sending any request. A crash between
    // these two files still leaves the config blocking all receive operations.
    let mut saved = None;
    paths.registry.update(|registry| {
        let entry = registry
            .bridges
            .iter()
            .find(|b| b.id == id)
            .context("Folder no longer exists")?;
        let path = entry.config_path.clone();
        let mut config = load_editable_config(&path)?;
        anyhow::ensure!(
            !matches!(
                config.connection_state,
                ConnectionState::ExitPending
                    | ConnectionState::ExitLocalCleaned
                    | ConnectionState::Exited
            ),
            "Use Clean exit to preserve cleanup receipts."
        );
        anyhow::ensure!(
            scoped(&config),
            "Legacy credentials can only be removed locally."
        );
        if config.connection_state != ConnectionState::Disconnected {
            config.connection_state = ConnectionState::DisconnectPending;
            config.save(&path, true)?;
        }
        saved = Some((path, config));
        registry.set_auto_receive(id, false)
    })?;
    let (path, mut config) = saved.context("Missing Folder configuration")?;
    if config.connection_state == ConnectionState::Disconnected {
        return Ok(());
    }
    let ServerConfig::Http {
        base_url,
        allow_insecure_http,
        token_env,
        ..
    } = &config.server
    else {
        unreachable!()
    };
    let token = if supplied_token.trim().is_empty() {
        std::env::var(token_env)
            .context("Enter the Folder session token to retry disconnection.")?
    } else {
        supplied_token.trim().to_owned()
    };
    PairingClient::new(base_url, &token, *allow_insecure_http)?.disconnect()?;
    paths.registry.update(|registry| {
        let current = registry
            .bridges
            .iter()
            .find(|b| b.id == id)
            .context("Folder no longer exists")?;
        anyhow::ensure!(
            current.config_path == path,
            "Folder configuration changed; retry disconnection."
        );
        let latest = load_editable_config(&path)?;
        anyhow::ensure!(
            latest == config,
            "Folder configuration changed; retry disconnection."
        );
        config.connection_state = ConnectionState::Disconnected;
        config.save(&path, true)?;
        registry.set_auto_receive(id, false)
    })
}

pub(super) fn panel(
    ui: &Rc<DesktopUi>,
    editor: &adw::Window,
    task: &pairing_panel::SetupTask,
    registration: &BridgeRegistration,
    config: &Config,
    token: &gtk::PasswordEntry,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Disconnect Folder")
        .build();
    let description = match config.connection_state {
        ConnectionState::Connected if !scoped(config) => {
            "Legacy connection: Remove only stops local management. Shared server credentials remain valid."
        }
        ConnectionState::Connected => "Revoke both devices' access to this Folder. Files are kept.",
        ConnectionState::DisconnectPending => {
            "Stopped locally. Server confirmation is pending; enter your session token and retry. Do not remove this Folder yet."
        }
        ConnectionState::Disconnected => {
            "Disconnected on the server. Files are kept. Create a new Folder to reconnect, or Remove this entry."
        }
        ConnectionState::ExitPending => {
            "Clean exit pending. Transfers are stopped. Retry to finish cleanup."
        }
        ConnectionState::ExitLocalCleaned => {
            "Linux cleanup confirmed. Waiting for Android; use Check clean exit after the phone reconnects."
        }
        ConnectionState::Exited => {
            "Both devices and the relay confirmed cleanup. Original and received files are kept. You can remove this receipt."
        }
    };
    group.set_description(Some(description));
    if scoped(config)
        && !matches!(
            config.connection_state,
            ConnectionState::Disconnected | ConnectionState::Exited
        )
    {
        super::clean_exit::add_button(&group, ui, editor, task, registration, config, token);
    }
    if matches!(
        config.connection_state,
        ConnectionState::ExitPending | ConnectionState::ExitLocalCleaned | ConnectionState::Exited
    ) {
        return group;
    }
    if !scoped(config) || config.connection_state == ConnectionState::Disconnected {
        return group;
    }
    let button = gtk::Button::with_label(
        if config.connection_state == ConnectionState::DisconnectPending {
            "Retry disconnect"
        } else {
            "Disconnect"
        },
    );
    button.add_css_class("destructive-action");
    button.set_widget_name("folder-disconnect");
    group.add(&button);
    let ui = ui.clone();
    let editor = editor.clone();
    let task = task.clone();
    let id = registration.id.clone();
    let token = token.clone();
    button.connect_clicked(move |_| {
        let confirm = gtk::MessageDialog::builder().transient_for(&editor).modal(true)
            .message_type(gtk::MessageType::Warning).buttons(gtk::ButtonsType::Cancel)
            .text("Disconnect this Folder?")
            .secondary_text("Both devices will lose access. Original and received files stay unchanged. Already-authorized requests may finish. Reconnecting requires a new Folder. Network failure leaves this Folder stopped locally until you retry.").build();
        confirm.add_button("Disconnect", gtk::ResponseType::Accept);
        let ui = ui.clone(); let editor = editor.clone(); let task = task.clone();
        let id = id.clone(); let token = token.clone();
        confirm.connect_response(move |dialog, response| {
            dialog.close();
            if response != gtk::ResponseType::Accept { return; }
            let paths = ui.paths.clone(); let work_id = id.clone(); let supplied = token.text().to_string();
            if !supplied.trim().is_empty() { ui.tokens.borrow_mut().insert(id.clone(), supplied.trim().to_owned()); }
            let ui = ui.clone(); let editor = editor.clone(); let id = id.clone();
            task.run(move || disconnect(&paths, &work_id, &supplied), move |result| {
                editor.close();
                match result {
                    Ok(()) => { ui.tokens.borrow_mut().remove(&id); ui.widgets.toast_overlay.add_toast(adw::Toast::new("Folder disconnected. Files remain in place.")); }
                    Err(error) => ui.show_error(&format!("Disconnection not confirmed. Reopen Folder Settings to retry. The relay may need an update. {error:#}")),
                }
                ui.start_load_all();
            });
        });
        confirm.present();
    });
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_states_never_invite_receiving_and_removal_copy_matches_scope() {
        for state in [
            ConnectionState::DisconnectPending,
            ConnectionState::Disconnected,
            ConnectionState::ExitPending,
            ConnectionState::ExitLocalCleaned,
            ConnectionState::Exited,
        ] {
            assert_ne!(status(state), "Ready");
            for directory in [false, true] {
                assert!(!empty_message(state, directory).contains("select Receive"));
                assert!(!empty_message(state, directory).contains("Select Receive"));
            }
        }
        assert_eq!(status(ConnectionState::Exited), "Clean exit complete");
        assert!(removal_message(ConnectionState::Exited).contains("MiRelay-owned configuration"));
        assert!(
            !removal_message(ConnectionState::Exited)
                .contains("configuration, state, and files remain")
        );
        assert!(
            removal_message(ConnectionState::Disconnected)
                .contains("configuration, state, and files remain")
        );
        assert!(removal_message(ConnectionState::Connected).contains("NOT revoked"));
        assert_eq!(status(ConnectionState::Connected), "Ready");
        assert!(empty_message(ConnectionState::Connected, false).contains("Select Receive"));
        assert!(empty_message(ConnectionState::Connected, true).contains("Initialize the source"));
    }

    #[test]
    fn snapshots_preserve_every_connection_state() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            library_dir: Some(root.path().join("originals")),
            ..Default::default()
        })
        .unwrap();
        for state in [
            ConnectionState::Connected,
            ConnectionState::DisconnectPending,
            ConnectionState::Disconnected,
            ConnectionState::ExitPending,
            ConnectionState::ExitLocalCleaned,
            ConnectionState::Exited,
        ] {
            config.connection_state = state;
            assert_eq!(
                snapshot_from_config(config.clone())
                    .unwrap()
                    .connection_state,
                state
            );
        }
    }

    #[test]
    fn stopped_snapshots_ignore_stale_auto_flag_but_pending_exits_still_retry() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            library_dir: Some(root.path().join("originals")),
            ..Default::default()
        })
        .unwrap();
        let path = root.path().join("folder.toml");
        let tokens = HashMap::from([("fixture".into(), "test-only".into())]);
        for state in [
            ConnectionState::Connected,
            ConnectionState::DisconnectPending,
            ConnectionState::Disconnected,
            ConnectionState::ExitPending,
            ConnectionState::ExitLocalCleaned,
            ConnectionState::Exited,
        ] {
            config.connection_state = state;
            config.save(&path, true).unwrap();
            for auto_receive in [false, true] {
                let views = [BridgeView {
                    registration: BridgeRegistration {
                        kind: Default::default(),
                        id: "fixture".into(),
                        name: "Fixture".into(),
                        config_path: path.clone(),
                        auto_receive,
                    },
                    snapshot: Some(snapshot_from_config(config.clone()).unwrap()),
                    error: None,
                }];
                let expected = (state.is_connected() && auto_receive)
                    || matches!(
                        state,
                        ConnectionState::ExitPending | ConnectionState::ExitLocalCleaned
                    );
                assert_eq!(
                    !automatic_sync_requests(&views, &tokens).is_empty(),
                    expected
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_disconnect_persists_barrier_then_retry_preserves_local_files() {
        directory_tests::scenario(|root, paths, url, token| {
            let (id, config, path) = directory_tests::setup(root, &paths, url, token);
            let original = config.storage.library_dir.join("keep.txt");
            std::fs::write(&original, b"original stays").unwrap();
            assert!(!can_remove(&config));
            assert!(disconnect(&paths, &id, "invalid-token").is_err());
            let pending = Config::load(&path).unwrap();
            assert_eq!(pending.connection_state, ConnectionState::DisconnectPending);
            assert!(!paths.registry.load().unwrap().bridges[0].auto_receive);
            assert!(source_for(&pending, Some(token)).is_err());
            assert!(sync_registered_bridge(&paths.registry, &id, &path, Some(token)).is_err());
            assert!(update_bridge(&paths, &id, "Changed".into(), url, true, true).is_err());
            assert!(!can_remove(&pending));
            disconnect(&paths, &id, token).unwrap();
            disconnect(&paths, &id, "").unwrap();
            let closed = Config::load(&path).unwrap();
            assert_eq!(closed.connection_state, ConnectionState::Disconnected);
            assert!(can_remove(&closed));
            assert!(sync_registered_bridge(&paths.registry, &id, &path, Some(token)).is_err());
            paths
                .registry
                .update(|registry| registry.remove(&id).map(drop))
                .unwrap();
            assert_eq!(std::fs::read(original).unwrap(), b"original stays");
            assert!(path.exists());
            assert!(crate::config::directory_state_dir(&config).exists());
            assert_eq!(
                PairingClient::new(url, token, true)
                    .unwrap()
                    .handshake()
                    .unwrap()
                    .state,
                "disconnected"
            );
        })
        .await;
    }
}
