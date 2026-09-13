use std::fs;

use mirelay::config::{Config, LimitsConfig, ServerConfig, StorageConfig, WallpaperConfig};
use mirelay::http_source::HttpSource;
use mirelay::server::{ApiState, ServerStore, router};
use mirelay::state::StateStore;
use mirelay::sync::sync_once;

const TOKEN: &str = "full-stack-test-token";

fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(b"MiRelay full server-client test");
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_server_and_linux_client_complete_the_delivery_protocol() {
    let root = tempfile::tempdir().unwrap();
    let server_store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
    server_store.initialize().unwrap();
    let image = root.path().join("source.png");
    let bytes = png_bytes();
    fs::write(&image, &bytes).unwrap();
    let first_queued = server_store
        .enqueue("linux", &image, "source.png".into())
        .unwrap();
    let second_queued = server_store
        .enqueue("linux", &image, "same-content.png".into())
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(ApiState::new(server_store.clone(), "linux".into(), TOKEN).unwrap());
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let config = Config {
        schema_version: 1,
        connection_state: Default::default(),
        directory_sync: false,
        device_id: "linux-test-client".into(),
        server: ServerConfig::Http {
            base_url: format!("http://{address}"),
            token_env: "UNUSED_IN_DIRECT_TEST".into(),
            request_timeout_seconds: 5,
            page_size: 1,
            retry_max_attempts: 3,
            retry_base_delay_milliseconds: 1,
            retry_max_delay_milliseconds: 5,
            allow_insecure_http: true,
        },
        storage: StorageConfig {
            library_dir: root.path().join("client-library"),
            state_file: root.path().join("client-state.json"),
        },
        wallpaper: WallpaperConfig {
            command: vec![],
            timeout_seconds: 2,
        },
        limits: LimitsConfig {
            max_file_size_bytes: 1024 * 1024,
        },
    };
    let sync_config = config.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let source = HttpSource::new(&format!("http://{address}"), TOKEN, 5, 1, true)?;
        sync_once(&sync_config, &source)
    })
    .await
    .unwrap()
    .unwrap();

    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 2);
    assert_eq!(summary.acknowledged, 2);
    let state = StateStore::new(config.storage.state_file.clone())
        .load()
        .unwrap();
    for queued in [&first_queued, &second_queued] {
        let record = &state.deliveries[&queued.id];
        assert_eq!(fs::read(&record.stored_path).unwrap(), bytes);
    }
    let stats = server_store.stats("linux").unwrap();
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.acknowledged, 2);
    assert!(server_store.open_content(&first_queued).is_err());

    shutdown_tx.send(()).unwrap();
    server_task.await.unwrap();
}
