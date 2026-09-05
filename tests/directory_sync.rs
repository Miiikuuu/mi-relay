#![cfg(all(target_os = "linux", target_env = "gnu"))]
use mirelay::{
    directory::{
        client::DirectoryClient,
        receiver::{Receiver, Root},
        sender::Sender,
        *,
    },
    pairing::PairingClient,
    server::{ApiState, ServerStore, router},
    source::DeliverySource,
    tus_client::{UploadRequest, upload_with_events},
};
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::Path};
const ADMIN: &str = "directory-administrator-test-credential-only";
const TOKEN: &str = "1111111111111111111111111111111111111111111111111111111111111111";

async fn scenario(test: impl FnOnce(&Path, &ServerStore, &str, &str, &str) + Send + 'static) {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 100 * 1024 * 1024).unwrap();
    store.initialize().unwrap();
    let app = router(
        ApiState::new(store.clone(), "legacy".into(), "legacy-test-token")
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
        let folder = admin.create_folder("Directory QA").unwrap();
        let scoped = format!("{base}/f/{}", folder.folder_id);
        let info = admin.claim(&folder.pairing_code, TOKEN).unwrap();
        PairingClient::new(&scoped, &folder.receiver_token, true)
            .unwrap()
            .confirm(info.verification.as_deref().unwrap())
            .unwrap();
        test(
            root.path(),
            &store,
            &scoped,
            &folder.receiver_token,
            &format!("folder_{}", folder.folder_id),
        );
    })
    .await;
    task.abort();
    result.unwrap();
}
fn write(root: &Path, path: &str, bytes: &[u8]) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, bytes).unwrap();
}
fn source(url: &str, token: &str) -> mirelay::http_source::HttpSource {
    mirelay::http_source::HttpSource::new(url, token, 5, 100, true).unwrap()
}
fn request(root: &Path, url: &str, path: &str, version: u64) -> UploadRequest {
    UploadRequest {
        file: root.join("payload"),
        state_file: root.join("resume.json"),
        server_url: url.into(),
        token: TOKEN.into(),
        name: path.rsplit('/').next().map(str::to_owned),
        directory: Some(DirectoryVersion {
            path: path.into(),
            version,
        }),
        chunk_size_bytes: 3,
        max_chunks: None,
        allow_insecure_http: true,
        request_timeout_seconds: 5,
        keep_completed_state: true,
    }
}
fn entry(path: &str, version: u64, bytes: &[u8]) -> DirectoryEntry {
    DirectoryEntry {
        path: path.into(),
        version,
        delivery_id: uuid::Uuid::new_v4().to_string(),
        sha256: hex::encode(Sha256::digest(bytes)),
        size: bytes.len() as u64,
        media_type: "text/plain".into(),
        acknowledged: false,
        conflict: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_directory_initialization_updates_conflicts_deletion_and_restart() {
    scenario(|root, store, url, token, queue| {
        let src = root.join("phone-like");
        let dst = root.join("linux");
        write(&src, "same.txt", b"same");
        write(&dst, "same.txt", b"same");
        write(&src, "Pixiv/画师/original.txt", b"new art");
        write(&src, "Another/original.txt", b"new art");
        let binary: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
        write(&src, "Documents/unknown.bin", &binary);
        write(&src, "different.txt", b"phone copy");
        write(&dst, "different.txt", b"existing Linux copy");
        write(&dst, "keep-me.txt", b"destination-only");
        let send = DirectoryClient::new(url, TOKEN, true, "sender").unwrap();
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let transport = source(url, token);
        let mut receiver = Receiver::open(&dst, &root.join("receiver-state"), url).unwrap();
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 0);
        let mut sender = Sender::open(&src, &root.join("sender-state"), url).unwrap();
        assert!(sender.send(&send, TOKEN, true).is_err());
        let preview = sender.preview(&send).unwrap();
        assert_eq!(
            (
                preview.comparison.identical,
                preview.comparison.missing.len(),
                preview.comparison.different.len(),
                preview.comparison.destination_only
            ),
            (1, 3, 1, 1)
        );
        assert_eq!(store.stats(queue).unwrap().pending, 0);
        sender.initialize(&send).unwrap();
        assert_eq!(store.stats(queue).unwrap().pending, 0);
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 4);
        assert_eq!(
            transport
                .scan_pending(100 * 1024 * 1024)
                .unwrap()
                .deliveries
                .len(),
            0,
            "Legacy receivers must never flatten directory transfers"
        );
        assert!(
            send.state()
                .unwrap()
                .entries
                .iter()
                .all(|e| !e.acknowledged)
        );
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 4);
        assert_eq!(fs::read(dst.join("Documents/unknown.bin")).unwrap(), binary);
        assert_eq!(
            fs::read(dst.join("Pixiv/画师/original.txt")).unwrap(),
            b"new art"
        );
        assert_eq!(
            fs::read(dst.join("Another/original.txt")).unwrap(),
            b"new art"
        );
        let conflict = &receiver.files()["different.txt"];
        assert!(conflict.conflict);
        assert_eq!(
            fs::read(dst.join(conflict.history.as_ref().unwrap())).unwrap(),
            b"existing Linux copy"
        );
        assert!(send.state().unwrap().entries.iter().all(|e| e.acknowledged));
        assert_eq!(store.stats(queue).unwrap().pending, 0);
        write(&src, "Pixiv/画师/original.txt", b"version two");
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 1);
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 1);
        assert!(!receiver.files()["Pixiv/画师/original.txt"].conflict);
        write(&dst, "Pixiv/画师/original.txt", b"Linux edit");
        write(&src, "Pixiv/画师/original.txt", b"version three");
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 1);
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 1);
        let conflict = &receiver.files()["Pixiv/画师/original.txt"];
        assert!(conflict.conflict);
        assert_eq!(
            fs::read(dst.join(conflict.history.as_ref().unwrap())).unwrap(),
            b"Linux edit"
        );
        fs::remove_file(src.join("Another/original.txt")).unwrap();
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 0);
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 0);
        assert!(dst.join("Another/original.txt").exists());
        assert_eq!(
            fs::read(dst.join("keep-me.txt")).unwrap(),
            b"destination-only"
        );
        drop(sender);
        drop(receiver);
        write(&src, "offline/new.txt", b"created while stopped");
        let mut sender = Sender::open(&src, &root.join("sender-state"), url).unwrap();
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 1);
        assert!(Sender::open(&src, &root.join("another-state"), "another-scope").is_err());
        let mut receiver = Receiver::open(&dst, &root.join("receiver-state"), url).unwrap();
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 1);
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previews_expire_when_either_directory_changes_and_publish_is_cas() {
    scenario(|root, store, url, token, queue| {
        let src = root.join("source");
        let dst = root.join("destination");
        write(&src, "a.txt", b"source");
        write(&dst, "b.txt", b"receiver");
        let send = DirectoryClient::new(url, TOKEN, true, "sender").unwrap();
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let mut receiver = Receiver::open(&dst, &root.join("rs"), url).unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        let mut sender = Sender::open(&src, &root.join("ss"), url).unwrap();
        sender.preview(&send).unwrap();
        write(&src, "a.txt", b"changed");
        assert!(sender.initialize(&send).is_err());
        sender.preview(&send).unwrap();
        write(&dst, "b.txt", b"changed receiver");
        let old = recv.state().unwrap().receiver.unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        assert!(sender.initialize(&send).is_err());
        assert!(
            recv.publish(&InventoryUpdate {
                previous_id: None,
                inventory: old
            })
            .is_err()
        );
        assert_eq!(store.stats(queue).unwrap().pending, 0);
        sender.preview(&send).unwrap();
        sender.initialize(&send).unwrap();
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tus_directory_resume_cannot_change_path_or_version_and_legacy_ack_is_rejected() {
    scenario(|root, store, url, token, queue| {
        write(root, "payload", b"one resumable file");
        assert!(upload_with_events(request(root, url, "sub/a.txt", 1), |_| true).is_err());
        let dst = root.join("dst");
        fs::create_dir(&dst).unwrap();
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let mut receiver = Receiver::open(&dst, &root.join("rs"), url).unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        let mut paused = request(root, url, "sub/a.txt", 1);
        paused.max_chunks = Some(1);
        assert_eq!(
            upload_with_events(paused, |_| true).unwrap().uploaded_bytes,
            3
        );
        assert!(upload_with_events(request(root, url, "sub/b.txt", 1), |_| true).is_err());
        assert!(upload_with_events(request(root, url, "sub/a.txt", 2), |_| true).is_err());
        let complete = upload_with_events(request(root, url, "sub/a.txt", 1), |_| true).unwrap();
        assert!(
            source(url, token)
                .acknowledge(complete.delivery_id.as_ref().unwrap(), &complete.sha256)
                .is_err()
        );
        assert_eq!(store.stats(queue).unwrap().pending, 1);
        receiver.sync(&recv, &source(url, token)).unwrap();
        assert_eq!(
            fs::read(dst.join("sub/a.txt")).unwrap(),
            b"one resumable file"
        );
        assert_eq!(
            upload_with_events(request(root, url, "sub/a.txt", 1), |_| true)
                .unwrap()
                .delivery_id,
            complete.delivery_id
        );
        assert_eq!(store.stats(queue).unwrap().pending, 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_order_completion_never_resurrects_acked_versions() {
    scenario(|root, store, url, token, queue| {
        let dst = root.join("dst");
        fs::create_dir(&dst).unwrap();
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let mut receiver = Receiver::open(&dst, &root.join("rs"), url).unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        let old = root.join("old");
        let new = root.join("new");
        write(&old, "payload", b"old version");
        write(&new, "payload", b"latest version");
        let mut paused = request(&old, url, "version.txt", 1);
        paused.max_chunks = Some(1);
        upload_with_events(paused, |_| true).unwrap();
        upload_with_events(request(&new, url, "version.txt", 2), |_| true).unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        upload_with_events(request(&old, url, "version.txt", 1), |_| true).unwrap();
        assert_eq!(receiver.sync(&recv, &source(url, token)).unwrap(), 0);
        assert_eq!(
            fs::read(dst.join("version.txt")).unwrap(),
            b"latest version"
        );
        let state = recv.state().unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].version, 2);
        assert_eq!(store.stats(queue).unwrap().pending, 0);
    })
    .await;
}

#[test]
fn receiver_rejects_bad_hash_paths_symlinks_special_files_and_state_rebinding() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dst = root.join("dst");
    fs::create_dir(&dst).unwrap();
    let state = root.join("state");
    let mut receiver = Receiver::open(&dst, &state, "scope").unwrap();
    for path in [
        "../escape.txt",
        "/absolute.txt",
        "a/../../escape.txt",
        ".mirelay-state",
        "bad\\path",
    ] {
        assert!(
            receiver
                .apply(
                    &entry(path, 1, b"content"),
                    Box::new(Cursor::new(b"content"))
                )
                .is_err()
        );
    }
    assert!(
        receiver
            .apply(
                &entry("a.txt", 1, b"expected"),
                Box::new(Cursor::new(b"wrong"))
            )
            .is_err()
    );
    assert!(!dst.join("a.txt").exists());
    write(root, "outside/private.txt", b"untouched");
    std::os::unix::fs::symlink(root.join("outside"), dst.join("link")).unwrap();
    assert!(
        receiver
            .apply(
                &entry("link/private.txt", 1, b"new"),
                Box::new(Cursor::new(b"new"))
            )
            .is_err()
    );
    std::os::unix::fs::symlink(root.join("outside/private.txt"), dst.join("file.txt")).unwrap();
    assert!(
        receiver
            .apply(&entry("file.txt", 1, b"new"), Box::new(Cursor::new(b"new")))
            .is_err()
    );
    assert_eq!(
        fs::read(root.join("outside/private.txt")).unwrap(),
        b"untouched"
    );
    assert!(Root::open(&dst.join("link")).is_err());
    assert!(receiver.inventory().is_err());
    assert!(Receiver::open(&dst, &root.join("other-state"), "scope").is_err());
    drop(receiver);
    assert!(Receiver::open(&dst, &state, "other-scope").is_err());
    let other = root.join("other");
    fs::create_dir(&other).unwrap();
    assert!(Receiver::open(&other, &state, "scope").is_err());
}

#[test]
fn replayed_or_older_versions_do_not_duplicate_or_overwrite_local_edits() {
    let root = tempfile::tempdir().unwrap();
    let dst = root.path().join("dst");
    fs::create_dir(&dst).unwrap();
    let mut receiver = Receiver::open(&dst, &root.path().join("state"), "scope").unwrap();
    let version = entry("nested/file.txt", 2, b"version two");
    receiver
        .apply(&version, Box::new(Cursor::new(b"version two")))
        .unwrap();
    write(&dst, "nested/file.txt", b"local edit after delivery");
    receiver
        .apply(&version, Box::new(Cursor::new(b"version two")))
        .unwrap();
    receiver
        .apply(
            &entry("nested/file.txt", 1, b"old"),
            Box::new(Cursor::new(b"old")),
        )
        .unwrap();
    assert_eq!(
        fs::read(dst.join("nested/file.txt")).unwrap(),
        b"local edit after delivery"
    );
    assert!(
        receiver
            .apply(
                &entry("nested/file.txt", 2, b"different"),
                Box::new(Cursor::new(b"different"))
            )
            .is_err()
    );
}

#[test]
fn comparison_rejects_file_parent_collisions_and_keeps_equal_bytes_at_distinct_paths() {
    let inv = |paths: Vec<&str>| Inventory {
        id: uuid::Uuid::new_v4().to_string(),
        entries: paths
            .into_iter()
            .map(|p| InventoryEntry {
                path: p.into(),
                sha256: "a".repeat(64),
                size: 1,
            })
            .collect(),
    };
    assert!(compare(&inv(vec!["a"]), &inv(vec!["a/b"])).is_err());
    assert!(inv(vec!["a", "a/b"]).validate().is_err());
    assert!(inv(vec!["a", "a"]).validate().is_err());
    let result = compare(
        &inv(vec!["a/one.txt", "b/one.txt"]),
        &inv(vec!["a/one.txt"]),
    )
    .unwrap();
    assert_eq!(result.identical, 1);
    assert_eq!(result.missing, vec!["b/one.txt"]);
}

#[test]
fn version_two_database_migrates_without_changing_legacy_content() {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
    store.initialize().unwrap();
    write(root.path(), "old.txt", b"legacy");
    let old = store
        .enqueue("legacy", &root.path().join("old.txt"), "old.txt".into())
        .unwrap();
    let db = rusqlite::Connection::open(root.path().join("server/mirelay-server.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TABLE directory_versions; DROP TABLE directory_indexes; PRAGMA user_version=2;",
    )
    .unwrap();
    store.initialize().unwrap();
    store.initialize().unwrap();
    assert_eq!(store.get_delivery("legacy", &old.id).unwrap().unwrap(), old);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .list_pending("legacy", 0, None, 100)
            .unwrap()
            .deliveries
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directory_api_enforces_roles_scope_and_inventory_limits() {
    scenario(|_root,_store,url,token,_queue| {
        let http=reqwest::blocking::Client::new();
        let call=|method:&str,path:&str,credential:&str,body:serde_json::Value| {
            http.request(method.parse().unwrap(),format!("{url}/api/v1/{path}"))
                .bearer_auth(credential).header("mirelay-protocol-version","1").json(&body).send().unwrap()
        };
        let inventory=Inventory {id:uuid::Uuid::new_v4().to_string(),entries:(0..600).map(|i| InventoryEntry {path:format!("existing/file-{i}.txt"),sha256:"a".repeat(64),size:1}).collect()};
        let body=serde_json::to_value(InventoryUpdate {previous_id:None,inventory:inventory.clone()}).unwrap();
        assert!(serde_json::to_vec(&body).unwrap().len()>16*1024);
        assert_eq!(call("PUT","directory/index",TOKEN,body.clone()).status(),403);
        assert_eq!(call("PUT","directory/index",token,body).status(),200);
        let client=DirectoryClient::new(url,TOKEN,true,"sender").unwrap();
        assert_eq!(client.state().unwrap().receiver.unwrap(),inventory);
        let ack=serde_json::json!({"path":"file.txt","version":1,"sha256":"b".repeat(64),"conflict":false});
        assert_eq!(call("POST","directory/ack",TOKEN,ack.clone()).status(),403);
        assert_eq!(call("POST","directory/ack",token,ack).status(),409);
        assert_eq!(call("GET","directory","wrong-token",serde_json::Value::Null).status(),401);
        let other=PairingClient::new(url.split("/f/").next().unwrap(),ADMIN,true).unwrap().create_folder("Other").unwrap();
        assert_eq!(call("GET","directory",&other.receiver_token,serde_json::Value::Null).status(),401);
        let mut bad=inventory.clone();bad.entries[0].path="../escape".into();
        assert_eq!(call("PUT","directory/index",token,serde_json::json!({"previous_id":inventory.id,"inventory":bad})).status(),400);
        assert_eq!(call("PUT","directory/index",token,serde_json::json!({"previous_id":"x".repeat(MAX_BODY+1),"inventory":inventory})).status(),413);
        assert_eq!(client.state().unwrap().receiver.unwrap().entries.len(),600);
    }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_comparison_accounts_for_in_flight_versions() {
    scenario(|root, _store, url, token, _queue| {
        let dst = root.join("dst");
        let src = root.join("src");
        write(&dst, "file.txt", b"desired");
        write(&src, "file.txt", b"desired");
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let send = DirectoryClient::new(url, TOKEN, true, "sender").unwrap();
        let mut receiver = Receiver::open(&dst, &root.join("rs"), url).unwrap();
        receiver.sync(&recv, &source(url, token)).unwrap();
        write(root, "payload", b"older pending upload");
        upload_with_events(request(root, url, "file.txt", 1), |_| true).unwrap();
        let mut sender = Sender::open(&src, &root.join("ss"), url).unwrap();
        let preview = sender.preview(&send).unwrap();
        assert_eq!(preview.comparison.different, vec!["file.txt"]);
        sender.initialize(&send).unwrap();
        assert_eq!(sender.send(&send, TOKEN, true).unwrap(), 1);
        receiver.sync(&recv, &source(url, token)).unwrap();
        assert_eq!(fs::read(dst.join("file.txt")).unwrap(), b"desired");
        assert_eq!(recv.state().unwrap().entries[0].version, 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receipt_retry_checks_the_file_and_reconciles_a_lost_ack_response() {
    scenario(|root, store, url, token, queue| {
        let dst = root.join("dst");
        fs::create_dir(&dst).unwrap();
        let state = root.join("rs");
        let recv = DirectoryClient::new(url, token, true, "receiver").unwrap();
        let transport = source(url, token);
        let mut receiver = Receiver::open(&dst, &state, url).unwrap();
        receiver.sync(&recv, &transport).unwrap();
        write(root, "payload", b"durable incoming");
        upload_with_events(request(root, url, "file.txt", 1), |_| true).unwrap();
        let remote = recv.state().unwrap().entries.remove(0);
        receiver
            .apply(&remote, transport.open_payload(&remote.delivery()).unwrap())
            .unwrap();
        drop(receiver);
        write(&dst, "file.txt", b"changed before receipt");
        let mut receiver = Receiver::open(&dst, &state, url).unwrap();
        assert!(receiver.sync(&recv, &transport).is_err());
        assert_eq!(store.stats(queue).unwrap().pending, 1);
        write(&dst, "file.txt", b"durable incoming");
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 0);
        assert_eq!(store.stats(queue).unwrap().pending, 0);
        drop(receiver);
        let file = state.join("receiver.json");
        let mut ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        ledger["files"]["file.txt"]["acknowledged"] = serde_json::json!(false);
        fs::write(&file, serde_json::to_vec(&ledger).unwrap()).unwrap();
        let mut receiver = Receiver::open(&dst, &state, url).unwrap();
        assert_eq!(receiver.sync(&recv, &transport).unwrap(), 0);
        assert!(receiver.files()["file.txt"].acknowledged);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experimental_cli_requires_confirmation_and_runs_a_real_round_trip() {
    scenario(|root, _store, url, token, _queue| {
        let src = root.join("src");
        let dst = root.join("dst");
        fs::create_dir(&dst).unwrap();
        write(&src, "album/note.txt", b"CLI round trip");
        let invoke = |mode: &str, confirm: bool| {
            let receive = mode == "receive";
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_mirelay-directory"));
            command
                .arg("--directory")
                .arg(if receive { &dst } else { &src })
                .arg("--state-dir")
                .arg(root.join(if receive { "rs" } else { "ss" }))
                .args(["--server-url", url, "--allow-insecure-http", mode])
                .env("MIRELAY_TOKEN", if receive { token } else { TOKEN });
            if confirm {
                command.arg("--confirm");
            }
            command.output().unwrap()
        };
        assert!(invoke("receive", false).status.success());
        assert!(!invoke("send", false).status.success());
        assert!(invoke("preview", false).status.success());
        assert!(!invoke("initialize", false).status.success());
        assert!(invoke("initialize", true).status.success());
        let sent = invoke("send", false);
        assert!(
            sent.status.success(),
            "{}",
            String::from_utf8_lossy(&sent.stderr)
        );
        assert!(
            String::from_utf8_lossy(&invoke("status", false).stdout)
                .contains("\"acknowledged\": false")
        );
        let received = invoke("receive", false);
        assert!(
            received.status.success(),
            "{}",
            String::from_utf8_lossy(&received.stderr)
        );
        assert_eq!(
            fs::read(dst.join("album/note.txt")).unwrap(),
            b"CLI round trip"
        );
        assert!(
            String::from_utf8_lossy(&invoke("status", false).stdout)
                .contains("\"acknowledged\": true")
        );
    })
    .await;
}

#[test]
fn inventory_5000_files_is_bounded_stable_and_rejects_overflow() {
    let root = tempfile::tempdir().unwrap();
    let bytes = vec![b'x'; 1024];
    for i in 0..5000 {
        write(
            root.path(),
            &format!("group-{}/file-{i}.txt", i / 100),
            &bytes,
        );
    }
    let source = Root::open(root.path()).unwrap();
    let started = std::time::Instant::now();
    let first = source.inventory().unwrap();
    let duration = started.elapsed();
    assert_eq!(first.entries.len(), 5000);
    assert!(duration.as_secs() < 15, "5000-file scan took {duration:?}");
    assert_eq!(source.inventory().unwrap(), first);
    println!(
        "5000 files / 4.88 MiB, SHA-256 inventory: {} ms",
        duration.as_millis()
    );
    write(root.path(), "overflow.txt", b"one more");
    assert!(source.inventory().is_err());
}
