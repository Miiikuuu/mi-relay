use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use mirelay::{
    protocol::FolderCreated,
    server::{ApiState, ServerStore, router},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN: &str = "administrator-test-credential-only-00000001";
const LEGACY: &str = "legacy-test-credential";
const SENDER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[tokio::test]
async fn protocol_duplicate_auth_and_oversized_setup_requests_are_rejected() {
    let (_root, _store, app) = setup();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/folders")
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Folder"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 426);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/folders")
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("mirelay-protocol-version", "1")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Folder"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/folders",
            ADMIN,
            json!({"name":"x".repeat(17000)})
        )
        .await
        .0,
        413
    );
    for name in ["", "\u{001b}Folder", &"x".repeat(129)] {
        assert_eq!(
            request(&app, "POST", "/api/v1/folders", ADMIN, json!({"name":name}))
                .await
                .0,
            400
        );
    }
}

#[tokio::test]
async fn missing_admin_does_not_promote_legacy_and_reserved_queues_cannot_be_legacy() {
    let (_root, store, _app) = setup();
    let app = router(ApiState::new(store.clone(), "linux".into(), LEGACY).unwrap());
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/folders",
            LEGACY,
            json!({"name":"Folder"})
        )
        .await
        .0,
        401
    );
    assert!(ApiState::new(store, "folder_reserved".into(), LEGACY).is_err());
}

#[test]
fn version_one_upgrade_preserves_existing_delivery_and_supports_new_folders() {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
    store.initialize().unwrap();
    let file = root.path().join("old.txt");
    std::fs::write(&file, b"existing user content").unwrap();
    let delivery = store.enqueue("linux", &file, "old.txt".into()).unwrap();
    {
        let db =
            rusqlite::Connection::open(root.path().join("server/mirelay-server.sqlite3")).unwrap();
        db.execute_batch("DROP TABLE directory_versions; DROP TABLE directory_indexes; DROP TABLE folders; PRAGMA user_version=1;")
            .unwrap();
    }
    store.initialize().unwrap();
    assert_eq!(store.stats("linux").unwrap().pending, 1);
    assert_eq!(
        store.get_delivery("linux", &delivery.id).unwrap().unwrap(),
        delivery
    );
    let db = rusqlite::Connection::open(root.path().join("server/mirelay-server.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM folders", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn setup() -> (tempfile::TempDir, ServerStore, Router) {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
    store.initialize().unwrap();
    let api = router(
        ApiState::new(store.clone(), "linux".into(), LEGACY)
            .unwrap()
            .with_admin_token(ADMIN)
            .unwrap(),
    );
    (root, store, api)
}

async fn request(app: &Router, method: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("mirelay-protocol-version", "1")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 32768).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn create(app: &Router) -> FolderCreated {
    let (status, json) = request(
        app,
        "POST",
        "/api/v1/folders",
        ADMIN,
        json!({"name":"Pixiv"}),
    )
    .await;
    assert_eq!(status, 200, "{json}");
    serde_json::from_value(json).unwrap()
}

fn url(folder: &FolderCreated, path: &str) -> String {
    format!("/f/{}/api/v1/{path}", folder.folder_id)
}

async fn claim(app: &Router, folder: &FolderCreated, token: &str) -> (u16, Value) {
    request(
        app,
        "POST",
        "/api/v1/pairings/claim",
        token,
        json!({"pairing_code":folder.pairing_code,"sender_token":token}),
    )
    .await
}

async fn ready(app: &Router, folder: &FolderCreated) {
    let (status, info) = claim(app, folder, SENDER).await;
    assert_eq!(status, 200);
    let (status, info) = request(
        app,
        "POST",
        &url(folder, "pairing/confirm"),
        &folder.receiver_token,
        json!({"verification":info["verification"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(info["state"], "ready");
}

#[tokio::test]
async fn legacy_and_sender_credentials_cannot_create_folders() {
    let (_root, _store, app) = setup();
    for token in [LEGACY, SENDER, "wrong"] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/v1/folders",
                token,
                json!({"name":"Private"})
            )
            .await
            .0,
            401
        );
    }
}

#[tokio::test]
async fn no_transfer_before_both_parties_confirm() {
    let (_root, _store, app) = setup();
    let folder = create(&app).await;
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&folder, "deliveries?status=pending"),
            &folder.receiver_token,
            json!(null)
        )
        .await
        .0,
        409
    );
    let (_, info) = claim(&app, &folder, SENDER).await;
    assert_eq!(info["state"], "awaiting_confirmation");
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&folder, "deliveries?status=pending"),
            &folder.receiver_token,
            json!(null)
        )
        .await
        .0,
        409
    );
    assert_eq!(
        request(&app, "POST", &url(&folder, "uploads"), SENDER, json!(null))
            .await
            .0,
        409
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &url(&folder, "pairing/confirm"),
            SENDER,
            json!({"verification":info["verification"]})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &url(&folder, "pairing/confirm"),
            &folder.receiver_token,
            json!({"verification":"000000000000"})
        )
        .await
        .0,
        409
    );
}

#[tokio::test]
async fn invitation_is_single_sender_and_retry_idempotent() {
    let (_root, _store, app) = setup();
    let folder = create(&app).await;
    assert_eq!(claim(&app, &folder, SENDER).await.0, 200);
    assert_eq!(claim(&app, &folder, SENDER).await.0, 200);
    assert_eq!(claim(&app, &folder, OTHER).await.0, 401);
    ready(&app, &folder).await;
    assert_eq!(claim(&app, &folder, SENDER).await.1["state"], "ready");
}

#[tokio::test]
async fn concurrent_claims_cannot_bind_two_senders() {
    let (_root, _store, app) = setup();
    let folder = create(&app).await;
    let (a, b) = tokio::join!(claim(&app, &folder, SENDER), claim(&app, &folder, OTHER));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 401]);
}

#[tokio::test]
async fn expired_and_tampered_invitations_are_rejected() {
    let (root, _store, app) = setup();
    let folder = create(&app).await;
    let db = rusqlite::Connection::open(root.path().join("server/mirelay-server.sqlite3")).unwrap();
    db.execute(
        "UPDATE folders SET expires=0 WHERE id=?",
        [&folder.folder_id],
    )
    .unwrap();
    assert_eq!(claim(&app, &folder, SENDER).await.0, 401);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/pairings/claim",
            SENDER,
            json!({"pairing_code":"bad","sender_token":SENDER})
        )
        .await
        .0,
        401
    );
}

#[tokio::test]
async fn credentials_and_invitation_are_stored_only_as_hashes() {
    let (root, _store, app) = setup();
    let folder = create(&app).await;
    ready(&app, &folder).await;
    let db = rusqlite::Connection::open(root.path().join("server/mirelay-server.sqlite3")).unwrap();
    let sizes: (i64, i64, i64) = db
        .query_row(
            "SELECT length(receiver_hash),length(sender_hash),length(invite_hash) FROM folders",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(sizes, (32, 32, 32));
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let bytes = std::fs::read(root.path().join("server/mirelay-server.sqlite3")).unwrap();
    for secret in [
        &folder.receiver_token,
        &folder.pairing_code,
        &SENDER.to_owned(),
    ] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}

#[tokio::test]
async fn folders_and_roles_are_isolated_including_guessed_delivery_ids() {
    let (root, store, app) = setup();
    let a = create(&app).await;
    let b = create(&app).await;
    ready(&app, &a).await;
    ready(&app, &b).await;
    let file = root.path().join("test.txt");
    std::fs::write(&file, b"private content").unwrap();
    let delivery = store
        .enqueue(&format!("folder_{}", a.folder_id), &file, "test.txt".into())
        .unwrap();
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&a, "deliveries?status=pending"),
            &b.receiver_token,
            json!(null)
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&a, "deliveries?status=pending"),
            SENDER,
            json!(null)
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&a, "deliveries?status=pending"),
            LEGACY,
            json!(null)
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/v1/deliveries?status=pending",
            &a.receiver_token,
            json!(null)
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &url(&a, "uploads"),
            &a.receiver_token,
            json!(null)
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&a, "deliveries?status=pending"),
            &a.receiver_token,
            json!(null)
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &url(&b, "deliveries?status=pending"),
            &b.receiver_token,
            json!(null)
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &url(&b, &format!("deliveries/{}/ack", delivery.id)),
            &b.receiver_token,
            json!({"sha256":delivery.sha256})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &url(&a, &format!("deliveries/{}/ack", delivery.id)),
            SENDER,
            json!({"sha256":delivery.sha256})
        )
        .await
        .0,
        403
    );
}

#[tokio::test]
async fn renewal_revokes_old_sender_and_prevents_stale_confirmation() {
    let (_root, _store, app) = setup();
    let folder = create(&app).await;
    ready(&app, &folder).await;
    let (_, old) = request(&app, "GET", &url(&folder, "handshake"), SENDER, json!(null)).await;
    let (status, new) = request(
        &app,
        "POST",
        &url(&folder, "pairing/renew"),
        &folder.receiver_token,
        json!(null),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        request(&app, "GET", &url(&folder, "handshake"), SENDER, json!(null))
            .await
            .0,
        401
    );
    assert_eq!(claim(&app, &folder, SENDER).await.0, 401);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/pairings/claim",
            OTHER,
            json!({"pairing_code":new["pairing_code"],"sender_token":OTHER})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &url(&folder, "pairing/confirm"),
            &folder.receiver_token,
            json!({"verification":old["verification"]})
        )
        .await
        .0,
        409
    );
}

#[tokio::test]
async fn restart_preserves_pairing_and_legacy_queue() {
    let (_root, store, app) = setup();
    let folder = create(&app).await;
    ready(&app, &folder).await;
    store.initialize().unwrap();
    let restarted = router(
        ApiState::new(store, "linux".into(), LEGACY)
            .unwrap()
            .with_admin_token(ADMIN)
            .unwrap(),
    );
    assert_eq!(
        request(
            &restarted,
            "GET",
            &url(&folder, "handshake"),
            SENDER,
            json!(null)
        )
        .await
        .1["state"],
        "ready"
    );
    assert_eq!(
        request(
            &restarted,
            "GET",
            "/api/v1/deliveries?status=pending",
            LEGACY,
            json!(null)
        )
        .await
        .0,
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_url_tus_upload_and_linux_receive_use_shared_core() {
    use mirelay::{
        pairing::PairingClient,
        tus_client::{UploadRequest, upload_with_events},
    };
    let (root, store, app) = setup();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let path = root.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let admin = PairingClient::new(&base, ADMIN, true).unwrap();
        let folder = admin.create_folder("Existing directory").unwrap();
        let scoped = format!("{base}/f/{}", folder.folder_id);
        let receiver = PairingClient::new(&scoped, &folder.receiver_token, true).unwrap();
        assert_eq!(receiver.handshake().unwrap().state, "awaiting_peer");
        let info = admin.claim(&folder.pairing_code, SENDER).unwrap();
        receiver
            .confirm(info.verification.as_deref().unwrap())
            .unwrap();
        let file = path.join("sample.bin");
        let payload: Vec<u8> = (0..262144).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &payload).unwrap();
        let paused = upload_with_events(
            UploadRequest {
                directory: None,
                file: file.clone(),
                state_file: path.join("upload.json"),
                server_url: scoped.clone(),
                token: SENDER.into(),
                name: None,
                chunk_size_bytes: 65536,
                max_chunks: Some(1),
                allow_insecure_http: true,
                request_timeout_seconds: 5,
                keep_completed_state: false,
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(paused.uploaded_bytes, 65536);
        assert!(paused.delivery_id.is_none());
        let request = UploadRequest {
            directory: None,
            file,
            state_file: path.join("upload.json"),
            server_url: scoped.clone(),
            token: SENDER.into(),
            name: None,
            chunk_size_bytes: 65536,
            max_chunks: None,
            allow_insecure_http: true,
            request_timeout_seconds: 5,
            keep_completed_state: false,
        };
        let result = upload_with_events(request, |_| true).unwrap();
        assert!(serde_json::to_value(result).unwrap().is_object());
        let mut config = mirelay::config::Config::defaults(mirelay::config::InitOverrides {
            data_dir: Some(path.join("receiver")),
            ..Default::default()
        })
        .unwrap();
        config.wallpaper.command.clear();
        let source =
            mirelay::http_source::HttpSource::new(&scoped, &folder.receiver_token, 5, 10, true)
                .unwrap();
        let summary = mirelay::sync::sync_once(&config, &source).unwrap();
        assert_eq!(summary.received, 1);
        assert_eq!(summary.acknowledged, 1);
        assert!(summary.failures.is_empty());
        let state = mirelay::state::StateStore::new(config.storage.state_file.clone())
            .load()
            .unwrap();
        assert_eq!(
            std::fs::read(&state.deliveries.values().next().unwrap().stored_path).unwrap(),
            payload
        );
        assert_eq!(
            store
                .stats(&format!("folder_{}", folder.folder_id))
                .unwrap()
                .pending,
            0
        );
    })
    .await
    .unwrap();
    task.abort();
}
