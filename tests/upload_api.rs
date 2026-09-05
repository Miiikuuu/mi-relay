use std::fs;

use mirelay::server::{ApiState, ServerStore, router};
use mirelay::tus_client::{
    UploadEvent, UploadRequest, is_retryable_upload_error, upload_with_events,
};

const TOKEN: &str = "isolated-mobile-api-token";

struct Server {
    root: tempfile::TempDir,
    store: ServerStore,
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
        store.initialize().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(ApiState::new(store.clone(), "linux".into(), TOKEN).unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            root,
            store,
            url,
            task,
        }
    }

    fn request(&self) -> UploadRequest {
        UploadRequest {
            directory: None,
            file: self.root.path().join("payload"),
            state_file: self.root.path().join("resume.json"),
            server_url: self.url.clone(),
            token: TOKEN.into(),
            name: Some("Shared 中文.bin".into()),
            chunk_size_bytes: 3,
            max_chunks: None,
            allow_insecure_http: true,
            request_timeout_seconds: 5,
            keep_completed_state: true,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mobile_pause_resume_and_completed_receipt_replay_keep_one_delivery() {
    let server = Server::start().await;
    fs::write(
        server.root.path().join("payload"),
        [0, 255, 128, 1, 2, 3, 4, 5, 6, 7],
    )
    .unwrap();
    let request = server.request();
    let paused = tokio::task::spawn_blocking(move || {
        upload_with_events(
            request,
            |event| !matches!(event, UploadEvent::Progress { uploaded, .. } if *uploaded >= 3),
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(paused.uploaded_bytes, 3);
    assert!(paused.delivery_id.is_none());
    assert_eq!(server.store.stats("linux").unwrap().pending, 0);

    let request = server.request();
    let completed = tokio::task::spawn_blocking(move || upload_with_events(request, |_| true))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.uploaded_bytes, 10);
    assert!(completed.delivery_id.is_some());
    assert!(server.root.path().join("resume.json").exists());
    let request = server.request();
    let replay = tokio::task::spawn_blocking(move || upload_with_events(request, |_| true))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        replay, completed,
        "crash before mobile receipt persistence must not enqueue a duplicate"
    );
    assert_eq!(server.store.stats("linux").unwrap().pending, 1);
    let state = fs::read_to_string(server.root.path().join("resume.json")).unwrap();
    assert!(!state.contains(TOKEN));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_staged_payload_is_not_resumed_or_acknowledged() {
    let server = Server::start().await;
    let payload = server.root.path().join("payload");
    fs::write(&payload, "original payload").unwrap();
    let mut request = server.request();
    request.max_chunks = Some(1);
    tokio::task::spawn_blocking(move || upload_with_events(request, |_| true))
        .await
        .unwrap()
        .unwrap();
    let before = fs::read(server.root.path().join("resume.json")).unwrap();
    fs::write(&payload, "modified payload").unwrap();
    let request = server.request();
    let error = tokio::task::spawn_blocking(move || upload_with_events(request, |_| true))
        .await
        .unwrap()
        .unwrap_err();
    assert!(!is_retryable_upload_error(&error));
    assert_eq!(
        before,
        fs::read(server.root.path().join("resume.json")).unwrap()
    );
    assert_eq!(server.store.stats("linux").unwrap().pending, 0);
}

#[test]
fn library_input_guards_reject_invalid_options_without_touching_files() {
    let root = tempfile::tempdir().unwrap();
    for (chunk, timeout, name) in [
        (0, 30, "ok.txt"),
        (17 * 1024 * 1024, 30, "ok.txt"),
        (1, 0, "ok.txt"),
        (1, 3601, "ok.txt"),
        (1, 30, "../bad"),
        (1, 30, "a\\b"),
        (1, 30, "line\nname"),
    ] {
        let request = UploadRequest {
            directory: None,
            file: root.path().join("source"),
            state_file: root.path().join("state"),
            server_url: "https://example.invalid".into(),
            token: TOKEN.into(),
            name: Some(name.into()),
            chunk_size_bytes: chunk,
            max_chunks: None,
            allow_insecure_http: false,
            request_timeout_seconds: timeout,
            keep_completed_state: true,
        };
        assert!(upload_with_events(request, |_| true).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthorized_token_does_not_receive_an_automatic_retry() {
    let server = Server::start().await;
    fs::write(server.root.path().join("payload"), "safe sample").unwrap();
    let mut request = server.request();
    request.token = "incorrect-token".into();
    let error = tokio::task::spawn_blocking(move || upload_with_events(request, |_| true))
        .await
        .unwrap()
        .unwrap_err();
    assert!(!is_retryable_upload_error(&error));
    assert_eq!(server.store.stats("linux").unwrap().pending, 0);
}
