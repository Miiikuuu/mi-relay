use std::fs;

use assert_cmd::Command;
use mirelay::config::{Config, LimitsConfig, ServerConfig, StorageConfig, WallpaperConfig};
use mirelay::http_source::HttpSource;
use mirelay::model::WallpaperStatus;
use mirelay::server::{ApiState, ServerStore, router};
use mirelay::state::StateStore;
use mirelay::sync::sync_once;
use predicates::prelude::*;

const TOKEN: &str = "linux-sender-to-linux-receiver-token";

fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    for index in 0..4096_u32 {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linux_tus_sender_resumes_then_linux_receiver_downloads_and_acks() {
    let root = tempfile::tempdir().unwrap();
    let server_store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
    server_store.initialize().unwrap();
    let image = root.path().join("source.png");
    let bytes = png_bytes();
    fs::write(&image, &bytes).unwrap();
    let upload_state = root.path().join("upload-state.json");

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

    let first_image = image.clone();
    let first_state = upload_state.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("mirelay-upload")
            .unwrap()
            .env("MIRELAY_TOKEN", TOKEN)
            .arg(&first_image)
            .arg("--server-url")
            .arg(format!("http://{address}"))
            .arg("--state-file")
            .arg(&first_state)
            .arg("--chunk-size-bytes")
            .arg("1024")
            .arg("--max-chunks")
            .arg("1")
            .arg("--allow-insecure-http")
            .assert()
            .success()
            .stdout(predicate::str::contains("Paused after 1 chunk"));
    })
    .await
    .unwrap();
    assert!(upload_state.exists());
    assert_eq!(server_store.stats("linux").unwrap().pending, 0);

    let second_image = image.clone();
    let second_state = upload_state.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("mirelay-upload")
            .unwrap()
            .env("MIRELAY_TOKEN", TOKEN)
            .arg(&second_image)
            .arg("--server-url")
            .arg(format!("http://{address}"))
            .arg("--state-file")
            .arg(&second_state)
            .arg("--chunk-size-bytes")
            .arg("1024")
            .arg("--allow-insecure-http")
            .assert()
            .success()
            .stdout(predicate::str::contains("Resuming tus upload"))
            .stdout(predicate::str::contains("Upload complete"));
    })
    .await
    .unwrap();
    assert!(!upload_state.exists());
    assert_eq!(server_store.stats("linux").unwrap().pending, 1);

    let config = Config {
        schema_version: 1,
        connection_state: Default::default(),
        directory_sync: false,
        device_id: "linux-receiver".into(),
        server: ServerConfig::Http {
            base_url: format!("http://{address}"),
            token_env: "UNUSED_IN_DIRECT_TEST".into(),
            request_timeout_seconds: 5,
            page_size: 50,
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
        let source = HttpSource::new(&format!("http://{address}"), TOKEN, 5, 50, true)?;
        sync_once(&sync_config, &source)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 1);
    assert_eq!(summary.acknowledged, 1);

    let state = StateStore::new(config.storage.state_file).load().unwrap();
    let record = state.deliveries.values().next().unwrap();
    assert_eq!(fs::read(&record.stored_path).unwrap(), bytes);
    assert_eq!(server_store.stats("linux").unwrap().pending, 0);
    assert_eq!(server_store.stats("linux").unwrap().acknowledged, 1);

    shutdown_tx.send(()).unwrap();
    server_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_and_unknown_binary_are_delivered_without_running_the_wallpaper_hook() {
    let root = tempfile::tempdir().unwrap();
    let server_store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
    server_store.initialize().unwrap();

    let text = root.path().join("notes.md");
    let text_bytes = "MiRelay 可以传递普通 UTF-8 文本。\n".as_bytes().to_vec();
    fs::write(&text, &text_bytes).unwrap();
    let binary = root.path().join("payload.custom");
    let binary_bytes = vec![0x00, 0xff, 0x10, 0x80, 0x01, 0x02, 0x03, 0x7f];
    fs::write(&binary, &binary_bytes).unwrap();

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

    for (source, state_name) in [(&text, "text-upload.json"), (&binary, "binary-upload.json")] {
        let source = source.clone();
        let state_file = root.path().join(state_name);
        tokio::task::spawn_blocking(move || {
            Command::cargo_bin("mirelay-upload")
                .unwrap()
                .env("MIRELAY_TOKEN", TOKEN)
                .arg(source)
                .arg("--server-url")
                .arg(format!("http://{address}"))
                .arg("--state-file")
                .arg(state_file)
                .arg("--chunk-size-bytes")
                .arg("3")
                .arg("--allow-insecure-http")
                .assert()
                .success()
                .stdout(predicate::str::contains("Upload complete"));
        })
        .await
        .unwrap();
    }
    assert_eq!(server_store.stats("linux").unwrap().pending, 2);

    let config = Config {
        schema_version: 1,
        connection_state: Default::default(),
        directory_sync: false,
        device_id: "linux-receiver".into(),
        server: ServerConfig::Http {
            base_url: format!("http://{address}"),
            token_env: "UNUSED_IN_DIRECT_TEST".into(),
            request_timeout_seconds: 5,
            page_size: 50,
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
            command: vec![
                "/definitely/not/a/real/mirelay-wallpaper-hook".into(),
                "{path}".into(),
            ],
            timeout_seconds: 2,
        },
        limits: LimitsConfig {
            max_file_size_bytes: 1024 * 1024,
        },
    };
    let sync_config = config.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let source = HttpSource::new(&format!("http://{address}"), TOKEN, 5, 50, true)?;
        sync_once(&sync_config, &source)
    })
    .await
    .unwrap()
    .unwrap();

    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 2);
    assert_eq!(summary.acknowledged, 2);
    assert_eq!(summary.wallpaper_not_applicable, 2);
    assert_eq!(summary.wallpaper_applied, 0);
    assert_eq!(summary.wallpaper_not_configured, 0);

    let state = StateStore::new(config.storage.state_file).load().unwrap();
    assert_eq!(state.deliveries.len(), 2);
    for record in state.deliveries.values() {
        assert_eq!(record.wallpaper_status, WallpaperStatus::NotApplicable);
        assert_eq!(record.wallpaper_attempts, 0);
        assert!(record.wallpaper_error.is_none());
        match record.original_name.as_str() {
            "notes.md" => {
                assert_eq!(record.media_type, "text/plain");
                assert_eq!(record.stored_path.extension().unwrap(), "txt");
                assert_eq!(fs::read(&record.stored_path).unwrap(), text_bytes);
            }
            "payload.custom" => {
                assert_eq!(record.media_type, "application/octet-stream");
                assert_eq!(record.stored_path.extension().unwrap(), "bin");
                assert_eq!(fs::read(&record.stored_path).unwrap(), binary_bytes);
            }
            unexpected => panic!("unexpected delivered file: {unexpected}"),
        }
    }
    assert_eq!(server_store.stats("linux").unwrap().pending, 0);
    assert_eq!(server_store.stats("linux").unwrap().acknowledged, 2);

    shutdown_tx.send(()).unwrap();
    server_task.await.unwrap();
}
