//! Host-JVM integration. Explicitly opt in after `bash android/scripts/test-jni.sh`.
use std::fs;
use std::path::PathBuf;

use mirelay::config::{Config, InitOverrides, ServerConfig};
use mirelay::http_source::HttpSource;
use mirelay::server::{ApiState, ServerStore, router};
use mirelay::state::StateStore;
use mirelay::sync::sync_once;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires locally built JNI library, JDK, and compiled NativeSmoke class"]
async fn java_rust_tus_server_linux_receiver_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 1024 * 1024).unwrap();
    store.initialize().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let token = "jni-local-integration-token";
    let app = router(ApiState::new(store.clone(), "linux".into(), token).unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for (name, bytes) in [
        ("分享.txt", "JNI → Rust → tus → Linux\n".as_bytes().to_vec()),
        ("unknown.bin", vec![0, 255, 128, 7, 6, 5, 4, 3, 2, 1]),
    ] {
        let input = root.path().join(name);
        fs::write(&input, &bytes).unwrap();
        let request = serde_json::json!({
            "file": input, "state_file": root.path().join(format!("{name}.resume")),
            "server_url": url, "token": token, "name": name, "chunk_size_bytes": 3,
            "max_chunks": null, "allow_insecure_http": true, "request_timeout_seconds": 5,
            "keep_completed_state": true
        });
        let request_file = root.path().join(format!("{name}.request.json"));
        fs::write(&request_file, serde_json::to_vec(&request).unwrap()).unwrap();
        tokio::task::spawn_blocking(move || {
            let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let output = std::process::Command::new("java")
                .arg("--enable-native-access=ALL-UNNAMED")
                .arg(format!(
                    "-Djava.library.path={}/android/native/target/debug",
                    project.display()
                ))
                .arg("-cp")
                .arg(project.join("android/.local/jni-tests"))
                .arg("NativeSmoke")
                .arg(request_file)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
        })
        .await
        .unwrap();
    }
    assert_eq!(store.stats("linux").unwrap().pending, 2);
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(root.path().join("linux")),
        ..Default::default()
    })
    .unwrap();
    config.server = ServerConfig::Http {
        base_url: url.clone(),
        token_env: "UNUSED".into(),
        request_timeout_seconds: 5,
        page_size: 50,
        retry_max_attempts: 1,
        retry_base_delay_milliseconds: 1,
        retry_max_delay_milliseconds: 5,
        allow_insecure_http: true,
    };
    let state_file = config.storage.state_file.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let source = HttpSource::new(&url, token, 5, 50, true).unwrap();
        sync_once(&config, &source).unwrap()
    })
    .await
    .unwrap();
    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 2);
    assert_eq!(summary.acknowledged, 2);
    for record in StateStore::new(state_file)
        .load()
        .unwrap()
        .deliveries
        .values()
    {
        assert_eq!(
            fs::read(&record.stored_path).unwrap(),
            fs::read(root.path().join(&record.original_name)).unwrap()
        );
    }
    assert_eq!(store.stats("linux").unwrap().pending, 0);
    server.abort();
}
