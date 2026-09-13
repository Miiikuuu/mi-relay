use super::*;
use crate::{config::ConnectionState, pairing::PairingClient};

fn scoped(config: &Config) -> bool {
    matches!(&config.server, ServerConfig::Http { base_url, .. }
        if reqwest::Url::parse(base_url).is_ok_and(|url| url.path().contains("/f/")))
}

pub(super) fn can_remove(config: &Config) -> bool {
    !scoped(config) || config.connection_state == ConnectionState::Disconnected
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
    };
    group.set_description(Some(description));
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
