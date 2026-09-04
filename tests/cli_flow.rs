use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(b"MiRelay CLI integration test");
    bytes
}

#[test]
fn init_enqueue_sync_and_status_form_a_complete_local_flow() {
    let root = tempdir().unwrap();
    let config = root.path().join("config.toml");
    let data = root.path().join("data");
    let image = root.path().join("wallpaper.png");
    fs::write(&image, png_bytes()).unwrap();

    Command::cargo_bin("mirelay")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "init",
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized MiRelay"));

    Command::cargo_bin("mirelay")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "mock",
            "enqueue",
            image.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Published mock delivery"));

    Command::cargo_bin("mirelay")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("received:                 1"))
        .stdout(predicate::str::contains("wallpaper not configured: 1"));

    Command::cargo_bin("mirelay")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source pending:          0"))
        .stdout(predicate::str::contains("stored total:            1"))
        .stdout(predicate::str::contains("delivery acknowledged:   1"));

    assert_eq!(
        fs::read_dir(data.join("library").join(".mirelay-staging-v1"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(data.join("mock-inbox").join(".acks"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn checksum_failure_never_creates_an_ack() {
    let root = tempdir().unwrap();
    let config = root.path().join("config.toml");
    let data = root.path().join("data");
    let image = root.path().join("wallpaper.png");
    fs::write(&image, png_bytes()).unwrap();

    Command::cargo_bin("mirelay")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "init",
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .assert()
        .success();
    Command::cargo_bin("mirelay")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "mock",
            "enqueue",
            image.to_str().unwrap(),
        ])
        .assert()
        .success();

    let payload = fs::read_dir(data.join("mock-inbox"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "payload")
        })
        .unwrap();
    fs::write(payload, b"tampered").unwrap();

    Command::cargo_bin("mirelay")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sync completed with 1 error"));

    assert!(!data.join("mock-inbox").join(".acks").exists());
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["deliveries"].as_object().unwrap().len(), 0);
}

#[test]
fn bare_relative_config_path_initializes_cleanly() {
    let root = tempdir().unwrap();
    Command::cargo_bin("mirelay")
        .unwrap()
        .current_dir(root.path())
        .args(["--config", "config.toml", "init", "--data-dir", "data"])
        .assert()
        .success();

    assert!(root.path().join("config.toml").exists());
    assert!(root.path().join("data/state.json").exists());
}

#[test]
fn malformed_existing_config_is_not_reported_as_uninitialized() {
    let root = tempdir().unwrap();
    let config = root.path().join("config.toml");
    fs::write(&config, "this is not valid toml = [").unwrap();

    Command::cargo_bin("mirelay")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse config"))
        .stderr(predicate::str::contains("not initialized").not());
}
