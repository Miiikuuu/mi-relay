use super::*;
use crate::config::directory_state_dir;
use crate::directory::{client::DirectoryClient, receiver::Receiver, sender::Sender};
use crate::pairing::PairingClient;
use crate::server::{ApiState, ServerStore, router};
use std::fs;

const ADMIN: &str = "desktop-directory-admin-fixture-only";
const SENDER: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, bytes).unwrap();
}

pub(super) async fn scenario(test: impl FnOnce(&Path, DesktopPaths, &str, &str) + Send + 'static) {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 100 * 1024 * 1024).unwrap();
    store.initialize().unwrap();
    let app = router(
        ApiState::new(store, "legacy".into(), "legacy-test-token")
            .unwrap()
            .with_admin_token(ADMIN)
            .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let result = tokio::task::spawn_blocking(move || {
        let admin = PairingClient::new(&base, ADMIN, true).unwrap();
        let folder = admin.create_folder("Desktop directory QA").unwrap();
        let url = format!("{base}/f/{}", folder.folder_id);
        let claimed = admin.claim(&folder.pairing_code, SENDER).unwrap();
        PairingClient::new(&url, &folder.receiver_token, true)
            .unwrap()
            .confirm(claimed.verification.as_deref().unwrap())
            .unwrap();
        let paths =
            resolve_desktop_paths(None, Some(root.path().join("desktop/folders.toml"))).unwrap();
        test(root.path(), paths, &url, &folder.receiver_token);
    })
    .await;
    task.abort();
    result.unwrap();
}

pub(super) fn setup(
    root: &Path,
    paths: &DesktopPaths,
    url: &str,
    token: &str,
) -> (String, Config, PathBuf) {
    let destination = root.join("destination");
    fs::create_dir_all(&destination).unwrap();
    let preview = directory_panel::prepare(
        paths,
        "Directory QA".into(),
        destination,
        url,
        true,
        true,
        token,
    )
    .unwrap();
    let (id, warning) = directory_panel::initialize(paths, preview, token).unwrap();
    assert!(warning.is_none());
    let path = paths.registry.load().unwrap().bridges[0]
        .config_path
        .clone();
    let config = Config::load(&path).unwrap();
    (id, config, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_is_read_only_and_confirmation_detects_changed_source_and_remote() {
    scenario(|root, paths, url, token| {
        let destination = root.join("destination");
        write(&destination, "existing.txt", b"one");
        let preview = directory_panel::prepare(
            &paths,
            "Directory QA".into(),
            destination.clone(),
            url,
            true,
            false,
            token,
        )
        .unwrap();
        assert!(paths.registry.load().unwrap().bridges.is_empty());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        assert!(
            DirectoryClient::new(url, token, true, "receiver")
                .unwrap()
                .state()
                .unwrap()
                .receiver
                .is_none()
        );
        write(&destination, "existing.txt", b"two");
        assert!(directory_panel::initialize(&paths, preview, token).is_err());
        assert!(paths.registry.load().unwrap().bridges.is_empty());
        assert!(!destination.join(".mirelay-receiver.lock").exists());
        let preview = directory_panel::prepare(
            &paths,
            "Directory QA".into(),
            destination.clone(),
            url,
            true,
            false,
            token,
        )
        .unwrap();
        let client = DirectoryClient::new(url, token, true, "receiver").unwrap();
        client
            .publish(&crate::directory::InventoryUpdate {
                previous_id: None,
                inventory: crate::directory::receiver::Root::open(&destination)
                    .unwrap()
                    .inventory()
                    .unwrap(),
            })
            .unwrap();
        assert!(directory_panel::initialize(&paths, preview, token).is_err());
        assert!(paths.registry.load().unwrap().bridges.is_empty());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identical_content_in_replaced_root_does_not_reuse_review_identity() {
    scenario(|root, paths, url, token| {
        let destination = root.join("destination");
        write(&destination, "file.txt", b"same");
        let preview = directory_panel::prepare(
            &paths,
            "Directory QA".into(),
            destination.clone(),
            url,
            true,
            false,
            token,
        )
        .unwrap();
        fs::rename(&destination, root.join("original-directory")).unwrap();
        write(&destination, "file.txt", b"same");
        assert!(directory_panel::initialize(&paths, preview, token).is_err());
        assert!(paths.registry.load().unwrap().bridges.is_empty());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desktop_receive_preserves_paths_updates_conflicts_and_real_receipts() {
    scenario(|root, paths, url, token| {
        write(&root.join("destination"), "edited.txt", b"Linux original");
        let (id, config, path) = setup(root, &paths, url, token);
        let secret_free = fs::read_to_string(&path).unwrap(); assert!(!secret_free.contains(token)); assert!(secret_free.contains("directory_sync = true"));
        let source = root.join("source"); write(&source, "edited.txt", b"Android original");
        write(&source, "Pixiv/画师/original.bin", &[0, 255, 7, 0, 1]); write(&source, "duplicate.bin", &[0, 255, 7, 0, 1]);
        let client = DirectoryClient::new(url, SENDER, true, "sender").unwrap();
        let mut sender = Sender::open(&source, &root.join("sender-state"), url).unwrap();
        sender.preview(&client).unwrap(); sender.initialize(&client).unwrap(); assert_eq!(sender.send(&client, SENDER, true).unwrap(), 3);
        let events = RefCell::new(Vec::new());
        let first = sync_registered_bridge_with_events(&paths.registry, &id, &path, Some(token), &|event| events.borrow_mut().push(event)).unwrap();
        assert_eq!(first.summary.received, 3); assert_eq!(first.summary.acknowledged, 3); assert!(first.summary.failures.is_empty());
        assert_eq!(fs::read(config.storage.library_dir.join("Pixiv/画师/original.bin")).unwrap(), [0, 255, 7, 0, 1]);
        assert_eq!(first.snapshot.deliveries.iter().filter(|file| file.delivery_error.is_some()).count(), 1);
        let copied = &first.snapshot.directory.as_ref().unwrap().files["edited.txt"];
        assert!(copied.conflict); assert_eq!(fs::read(config.storage.library_dir.join(copied.history.as_ref().unwrap())).unwrap(), b"Linux original");
        assert_eq!(events.borrow().iter().filter(|event| matches!(event, SyncEvent::FileActive { phase: SyncPhase::Downloading, .. })).count(), 3);
        assert_eq!(events.borrow().iter().filter(|event| matches!(event, SyncEvent::FileSettled { record: Some(record), .. } if record.delivery_status == DeliveryStatus::Acknowledged)).count(), 3);
        assert!(client.state().unwrap().entries.iter().all(|entry| entry.acknowledged));
        assert!(!config.storage.state_file.exists(), "No duplicate delivery ledger");
        let raw = fs::read(directory_state_dir(&config).join("receiver.json")).unwrap();
        let reloaded = snapshot_from_config(config.clone()).unwrap(); assert_eq!(reloaded.counts.total, 3);
        assert_eq!(fs::read(directory_state_dir(&config).join("receiver.json")).unwrap(), raw, "Refresh is read-only");
        assert_eq!(reloaded.deliveries.page(&[], FileSort::NameAsc, FileFilter::Attention, FileKind::All, "", 100).total, 1);
        assert_eq!(reloaded.deliveries.page(&[], FileSort::NameAsc, FileFilter::Completed, FileKind::All, "", 100).total, 2);
        write(&source, "edited.txt", b"Android update"); assert_eq!(sender.send(&client, SENDER, true).unwrap(), 1);
        let updated = sync_registered_bridge(&paths.registry, &id, &path, Some(token)).unwrap();
        assert_eq!(updated.summary.received, 1); assert_eq!(updated.snapshot.deliveries.len(), 3);
        assert!(!updated.snapshot.directory.as_ref().unwrap().files["edited.txt"].conflict);
        let view = BridgeView { registration: paths.registry.load().unwrap().bridges[0].clone(), snapshot: Some(updated.snapshot), error: None };
        assert_eq!(automatic_sync_requests(std::slice::from_ref(&view), &HashMap::new()).len(), 1);
        paths.registry.update(|registry| registry.set_auto_receive(&id, false)).unwrap();
        let mut paused = view; paused.registration.auto_receive = false;
        assert!(automatic_sync_requests(&[paused], &HashMap::new()).is_empty());
        fs::remove_file(source.join("duplicate.bin")).unwrap();
        assert_eq!(sender.send(&client, SENDER, true).unwrap(), 0);
        assert!(sync_registered_bridge(&paths.registry, &id, &path, Some(token)).unwrap().summary.failures.is_empty());
        assert!(config.storage.library_dir.join("duplicate.bin").exists());
    }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_loss_server_remap_and_legacy_pipeline_are_rejected_without_reset() {
    scenario(|root, paths, url, token| {
        let (id, config, path) = setup(root, &paths, url, token);
        let before = fs::read(&path).unwrap();
        assert!(
            update_bridge(
                &paths,
                &id,
                "Renamed".into(),
                &format!("http://127.0.0.1:1/f/{}", Uuid::new_v4()),
                true,
                false
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            crate::sync::sync_once(
                &config,
                &crate::source::FilesystemSource::new(root.join("unused"))
            )
            .is_err()
        );
        assert!(!config.storage.state_file.exists());
        assert!(crate::sync::retry_wallpapers(&config, false).is_err());
        let ledger = directory_state_dir(&config).join("receiver.json");
        fs::rename(&ledger, ledger.with_extension("backup")).unwrap();
        assert!(snapshot_from_config(config.clone()).is_err());
        assert!(directory_panel::receive(config, Some(token), &|_| {}).is_err());
        assert!(
            !ledger.exists(),
            "Missing state must never be silently recreated"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_ack_remains_visible_and_recovers_without_redownload() {
    scenario(|root, paths, url, token| {
        let (id, config, path) = setup(root, &paths, url, token);
        let source = root.join("source");
        write(&source, "data.txt", b"durable bytes");
        let client = DirectoryClient::new(url, SENDER, true, "sender").unwrap();
        let mut sender = Sender::open(&source, &root.join("sender-state"), url).unwrap();
        sender.preview(&client).unwrap();
        sender.initialize(&client).unwrap();
        sender.send(&client, SENDER, true).unwrap();
        let replacement = RefCell::new(None);
        let result = sync_registered_bridge_with_events(
            &paths.registry,
            &id,
            &path,
            Some(token),
            &|event| {
                if matches!(
                    event,
                    SyncEvent::FileActive {
                        phase: SyncPhase::Confirming,
                        ..
                    }
                ) {
                    *replacement.borrow_mut() = Some(
                        PairingClient::new(url, token, true)
                            .unwrap()
                            .renew()
                            .unwrap(),
                    );
                }
            },
        )
        .unwrap();
        assert!(!result.summary.failures.is_empty());
        assert_eq!(result.snapshot.counts.ack_pending, 1);
        assert_eq!(
            fs::read(config.storage.library_dir.join("data.txt")).unwrap(),
            b"durable bytes"
        );
        let invite = replacement.into_inner().unwrap();
        let claimed = PairingClient::new(url.split("/f/").next().unwrap(), ADMIN, true)
            .unwrap()
            .claim(&invite.pairing_code, SENDER)
            .unwrap();
        PairingClient::new(url, token, true)
            .unwrap()
            .confirm(claimed.verification.as_deref().unwrap())
            .unwrap();
        let events = RefCell::new(Vec::new());
        let resumed = sync_registered_bridge_with_events(
            &paths.registry,
            &id,
            &path,
            Some(token),
            &|event| events.borrow_mut().push(event),
        )
        .unwrap();
        assert!(resumed.summary.failures.is_empty());
        assert_eq!(resumed.summary.received, 0);
        assert_eq!(resumed.summary.acknowledged, 1);
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            SyncEvent::FileActive {
                phase: SyncPhase::Downloading,
                ..
            }
        )));
    })
    .await;
}

#[test]
fn directory_configuration_rejects_unsupported_modes_and_old_default_stays_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(root.path().into()),
        ..Default::default()
    })
    .unwrap();
    assert!(!toml::to_string(&config).unwrap().contains("directory_sync"));
    config.directory_sync = true;
    assert!(config.validate().is_err());
    set_http_server(&mut config, "http://127.0.0.1:1", true);
    assert!(config.validate().is_err());
    set_http_server(
        &mut config,
        &format!("http://127.0.0.1:1/f/{}", Uuid::new_v4()),
        true,
    );
    assert!(config.validate().is_ok());
    config.wallpaper.command = vec!["/bin/true".into()];
    assert!(config.validate().is_err());
}

#[test]
fn directory_preview_rejects_symlinks_bad_names_and_overlapping_state_before_network() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("source");
    fs::create_dir(&directory).unwrap();
    let paths =
        resolve_desktop_paths(None, Some(root.path().join("desktop/folders.toml"))).unwrap();
    let url = format!("http://127.0.0.1:1/f/{}", Uuid::new_v4());
    std::os::unix::fs::symlink(&directory, root.path().join("link")).unwrap();
    for (name, target) in [
        ("QA", root.path().join("link")),
        ("bad\nname", directory.clone()),
        ("QA", root.path().to_path_buf()),
    ] {
        assert!(
            directory_panel::prepare(&paths, name.into(), target, &url, true, true, SENDER)
                .is_err()
        );
    }
    assert!(paths.registry.load().unwrap().bridges.is_empty());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
}

#[test]
fn directory_inspection_validates_history_and_remains_compatible_with_old_ledgers() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("destination");
    fs::create_dir(&directory).unwrap();
    let state = root.path().join("state");
    drop(Receiver::open(&directory, &state, "scope").unwrap());
    let path = state.join("receiver.json");
    let mut ledger: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    ledger["files"]["data.txt"] = serde_json::json!({"version":1,"sha256":"a".repeat(64),"conflict":true,"history":format!(".mirelay-history-{}", Uuid::new_v4()),"acknowledged":true});
    fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    let old = Receiver::inspect(&directory, &state, "scope").unwrap();
    assert_eq!(old["data.txt"].received_at_unix, 0);
    ledger["files"]["data.txt"]["history"] = "../outside".into();
    fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(Receiver::inspect(&directory, &state, "scope").is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_delivery_only_files_block_directory_initialization() {
    scenario(|root, paths, url, _token| {
        let file = root.join("pending.txt");
        fs::write(&file, b"delivery-only").unwrap();
        crate::tus_client::upload_with_events(
            crate::tus_client::UploadRequest {
                file,
                state_file: root.join("resume.json"),
                server_url: url.into(),
                token: SENDER.into(),
                name: None,
                directory: None,
                chunk_size_bytes: 1024,
                max_chunks: None,
                allow_insecure_http: true,
                request_timeout_seconds: 5,
                keep_completed_state: true,
            },
            |_| true,
        )
        .unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        assert!(
            directory_panel::prepare(
                &paths,
                "QA".into(),
                destination.clone(),
                url,
                true,
                false,
                _token
            )
            .is_err()
        );
        assert!(paths.registry.load().unwrap().bridges.is_empty());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    })
    .await;
}

#[test]
fn publish_failure_keeps_durable_registration_and_reports_retry_without_reset() {
    use httpmock::prelude::*;
    use serde_json::json;
    let server = MockServer::start();
    let id = Uuid::new_v4().to_string();
    let url = server.url(format!("/f/{id}"));
    server.mock(|when,then| { when.method(GET).path(format!("/f/{id}/api/v1/handshake")); then.status(200).json_body(json!({"schema_version":1,"folder_id":id,"name":"QA","role":"receiver","state":"ready","verification":"012345abcdef","max_file_size_bytes":104857600})); });
    server.mock(|when, then| {
        when.method(GET).path(format!("/f/{id}/api/v1/directory"));
        then.status(200)
            .json_body(json!({"schema_version":1,"receiver":null,"entries":[]}));
    });
    server.mock(|when, then| {
        when.method(GET).path(format!("/f/{id}/api/v1/deliveries"));
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({"schema_version":1,"items":[],"next_cursor":null}));
    });
    let mut publish = server.mock(|when, then| {
        when.method(PUT)
            .path(format!("/f/{id}/api/v1/directory/index"));
        then.status(503).body("untrusted error body");
    });
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let paths =
        resolve_desktop_paths(None, Some(root.path().join("desktop/folders.toml"))).unwrap();
    let prepared =
        directory_panel::prepare(&paths, "QA".into(), directory, &url, true, false, SENDER)
            .unwrap();
    let (registered, warning) = directory_panel::initialize(&paths, prepared, SENDER).unwrap();
    assert!(
        warning
            .as_ref()
            .is_some_and(|warning| warning.contains("Use Receive to retry")
                && !warning.contains("untrusted error body"))
    );
    publish.assert_calls(1);
    let config_path = paths.registry.load().unwrap().bridges[0]
        .config_path
        .clone();
    assert!(snapshot_from_config(Config::load(&config_path).unwrap()).is_ok());
    publish.delete();
    let config = Config::load(&config_path).unwrap();
    let inventory = crate::directory::receiver::Root::open(&config.storage.library_dir)
        .unwrap()
        .inventory()
        .unwrap();
    server.mock(|when, then| {
        when.method(PUT)
            .path(format!("/f/{id}/api/v1/directory/index"));
        then.status(200).json_body_obj(&inventory);
    });
    assert!(
        sync_registered_bridge(&paths.registry, &registered, &config_path, Some(SENDER))
            .unwrap()
            .summary
            .failures
            .is_empty()
    );
    assert_eq!(paths.registry.load().unwrap().bridges.len(), 1);
}
