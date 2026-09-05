use std::time::Duration;

use assert_cmd::Command;
use httpmock::prelude::*;
use mirelay::config::{Config, LimitsConfig, ServerConfig, StorageConfig, WallpaperConfig};
use mirelay::http_source::{HttpRetryPolicy, HttpSource};
use mirelay::model::{DeliveryStatus, WallpaperStatus};
use mirelay::source::DeliverySource;
use mirelay::state::StateStore;
use mirelay::sync::sync_once;
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const TOKEN: &str = "test-device-token";
const DELIVERY_ID: &str = "delivery-http-1";

fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(b"MiRelay HTTP integration test");
    bytes
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn http_config(root: &TempDir, base_url: String, page_size: u32) -> Config {
    Config {
        schema_version: 1,
        directory_sync: false,
        device_id: "http-test-device".into(),
        server: ServerConfig::Http {
            base_url,
            token_env: "MIRELAY_TEST_TOKEN".into(),
            request_timeout_seconds: 5,
            page_size,
            retry_max_attempts: 3,
            retry_base_delay_milliseconds: 1,
            retry_max_delay_milliseconds: 5,
            allow_insecure_http: true,
        },
        storage: StorageConfig {
            library_dir: root.path().join("library"),
            state_file: root.path().join("state.json"),
        },
        wallpaper: WallpaperConfig {
            command: vec![],
            timeout_seconds: 2,
        },
        limits: LimitsConfig {
            max_file_size_bytes: 1024 * 1024,
        },
    }
}

fn index_item(id: &str, bytes: &[u8], created_at_unix: u64) -> serde_json::Value {
    json!({
        "delivery_id": id,
        "original_name": format!("{id}.png"),
        "size_bytes": bytes.len(),
        "sha256": digest(bytes),
        "media_type": "image/png",
        "created_at_unix": created_at_unix,
        "future_additive_field": true
    })
}

#[test]
fn http_sync_downloads_verifies_persists_and_acknowledges() {
    let root = tempfile::tempdir().unwrap();
    let server = MockServer::start();
    let bytes = png_bytes();
    let sha256 = digest(&bytes);
    let etag = format!("\"sha256:{sha256}\"");
    let api_path = "/gateway/api/v1/deliveries";

    let list = server.mock(|when, then| {
        when.method(GET)
            .path(api_path)
            .query_param("status", "pending")
            .query_param("limit", "50")
            .query_param_missing("cursor")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("mirelay-protocol-version", "1")
            .header("cache-control", "no-store")
            .header("accept", "application/json");
        then.status(200)
            .header("content-type", "application/json; charset=utf-8")
            .json_body(json!({
                "schema_version": 1,
                "items": [index_item(DELIVERY_ID, &bytes, 42)],
                "next_cursor": null,
                "future_page_field": "ignored"
            }));
    });
    let content = server.mock(|when, then| {
        when.method(GET)
            .path(format!("{api_path}/{DELIVERY_ID}/content"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("mirelay-protocol-version", "1")
            .header("if-match", &etag)
            .header("accept", "image/png")
            .header("accept-encoding", "identity");
        then.status(200)
            .header("content-type", "image/png")
            .header("content-length", bytes.len().to_string())
            .header("etag", &etag)
            .body(&bytes);
    });
    let ack = server.mock(|when, then| {
        when.method(PUT)
            .path(format!("{api_path}/{DELIVERY_ID}/ack"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("mirelay-protocol-version", "1")
            .header("content-type", "application/json")
            .json_body(json!({ "sha256": sha256 }));
        then.status(204);
    });

    let config = http_config(&root, format!("{}/gateway", server.base_url()), 50);
    let source = HttpSource::new(
        match &config.server {
            ServerConfig::Http { base_url, .. } => base_url,
            ServerConfig::Filesystem { .. } => unreachable!(),
        },
        TOKEN,
        5,
        50,
        true,
    )
    .unwrap();

    let summary = sync_once(&config, &source).unwrap();

    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 1);
    assert_eq!(summary.acknowledged, 1);
    assert_eq!(summary.wallpaper_not_configured, 1);
    list.assert_calls(1);
    content.assert_calls(1);
    ack.assert_calls(1);

    let state = StateStore::new(config.storage.state_file.clone())
        .load()
        .unwrap();
    let record = &state.deliveries[DELIVERY_ID];
    assert_eq!(record.delivery_status, DeliveryStatus::Acknowledged);
    assert_eq!(record.wallpaper_status, WallpaperStatus::NotConfigured);
    assert_eq!(std::fs::read(&record.stored_path).unwrap(), bytes);
}

#[test]
fn http_scan_follows_opaque_cursors_and_isolates_bad_items() {
    let server = MockServer::start();
    let first_bytes = png_bytes();
    let mut second_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    second_bytes.extend_from_slice(b"second image");
    let api_path = "/api/v1/deliveries";

    let first_page = server.mock(|when, then| {
        when.method(GET)
            .path(api_path)
            .query_param("status", "pending")
            .query_param("limit", "2")
            .query_param_missing("cursor");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema_version": 1,
                "items": [
                    index_item("delivery-a", &first_bytes, 20),
                    { "delivery_id": "invalid item" }
                ],
                "next_cursor": "opaque:page/2"
            }));
    });
    let second_page = server.mock(|when, then| {
        when.method(GET)
            .path(api_path)
            .query_param("status", "pending")
            .query_param("limit", "2")
            .query_param("cursor", "opaque:page/2");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema_version": 1,
                "items": [index_item("delivery-b", &second_bytes, 10)],
                "next_cursor": null
            }));
    });

    let source = HttpSource::new(&server.base_url(), TOKEN, 5, 2, true).unwrap();
    let scan = source.scan_pending(1024 * 1024).unwrap();

    first_page.assert_calls(1);
    second_page.assert_calls(1);
    assert_eq!(scan.issues.len(), 1);
    assert!(scan.issues[0].message.contains("invalid delivery object"));
    assert_eq!(
        scan.deliveries
            .iter()
            .map(|delivery| delivery.id.as_str())
            .collect::<Vec<_>>(),
        ["delivery-b", "delivery-a"]
    );
}

#[test]
fn mismatched_content_etag_prevents_storage_and_ack() {
    let root = tempfile::tempdir().unwrap();
    let server = MockServer::start();
    let bytes = png_bytes();
    let api_path = "/api/v1/deliveries";

    let list = server.mock(|when, then| {
        when.method(GET).path(api_path);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema_version": 1,
                "items": [index_item(DELIVERY_ID, &bytes, 42)],
                "next_cursor": null
            }));
    });
    let content = server.mock(|when, then| {
        when.method(GET)
            .path(format!("{api_path}/{DELIVERY_ID}/content"));
        then.status(200)
            .header("content-type", "image/png")
            .header("content-length", bytes.len().to_string())
            .header("etag", "\"sha256:wrong\"")
            .body(&bytes);
    });
    let ack = server.mock(|when, then| {
        when.method(PUT)
            .path(format!("{api_path}/{DELIVERY_ID}/ack"));
        then.status(204);
    });

    let config = http_config(&root, server.base_url(), 50);
    let source = HttpSource::new_with_retry(
        &server.base_url(),
        TOKEN,
        5,
        50,
        true,
        HttpRetryPolicy::new(1, Duration::ZERO, Duration::ZERO).unwrap(),
    )
    .unwrap();
    let summary = sync_once(&config, &source).unwrap();

    list.assert_calls(1);
    content.assert_calls(1);
    ack.assert_calls(0);
    assert_eq!(summary.received, 0);
    assert_eq!(summary.acknowledged, 0);
    assert_eq!(summary.failures.len(), 1);
    assert!(summary.failures[0].message.contains("ETag mismatch"));
    assert!(
        StateStore::new(config.storage.state_file.clone())
            .load()
            .unwrap()
            .deliveries
            .is_empty()
    );
}

#[test]
fn http_problem_details_and_retry_after_are_reported() {
    let server = MockServer::start();
    let unavailable = server.mock(|when, then| {
        when.method(GET).path("/api/v1/deliveries");
        then.status(503)
            .header("content-type", "application/problem+json")
            .header("retry-after", "30")
            .json_body(json!({
                "code": "temporarily_unavailable",
                "detail": "delivery store is restarting",
                "request_id": "request-123"
            }));
    });
    let source = HttpSource::new_with_retry(
        &server.base_url(),
        TOKEN,
        5,
        50,
        true,
        HttpRetryPolicy::new(1, Duration::ZERO, Duration::ZERO).unwrap(),
    )
    .unwrap();

    let error = source.scan_pending(1024 * 1024).unwrap_err();
    let message = format!("{error:#}");

    unavailable.assert_calls(1);
    assert!(message.contains("HTTP 503"));
    assert!(message.contains("temporarily_unavailable"));
    assert!(message.contains("request_id=request-123"));
    assert!(message.contains("retry_after=30"));
}

#[test]
fn retryable_http_status_is_attempted_with_a_bounded_budget() {
    let server = MockServer::start();
    let unavailable = server.mock(|when, then| {
        when.method(GET).path("/api/v1/deliveries");
        then.status(503)
            .header("content-type", "application/problem+json")
            .header("retry-after", "0")
            .json_body(json!({
                "code": "temporarily_unavailable",
                "detail": "try again"
            }));
    });
    let source = HttpSource::new_with_retry(
        &server.base_url(),
        TOKEN,
        5,
        50,
        true,
        HttpRetryPolicy::new(3, Duration::ZERO, Duration::ZERO).unwrap(),
    )
    .unwrap();

    let error = source.scan_pending(1024 * 1024).unwrap_err();

    unavailable.assert_calls(3);
    assert!(format!("{error:#}").contains("failed after 3 attempts"));
}

#[test]
fn cli_initializes_and_uses_an_http_source() {
    let root = tempfile::tempdir().unwrap();
    let server = MockServer::start();
    let config_path = root.path().join("config.toml");
    let data_path = root.path().join("data");
    let list = server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/deliveries")
            .header("authorization", format!("Bearer {TOKEN}"));
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema_version": 1,
                "items": [],
                "next_cursor": null
            }));
    });

    Command::cargo_bin("mirelay")
        .unwrap()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "init",
            "--data-dir",
            data_path.to_str().unwrap(),
            "--server-url",
            &server.base_url(),
            "--token-env",
            "MIRELAY_HTTP_CLI_TEST_TOKEN",
            "--allow-insecure-http",
        ])
        .assert()
        .success();

    let config = Config::load(&config_path).unwrap();
    assert!(matches!(config.server, ServerConfig::Http { .. }));

    Command::cargo_bin("mirelay")
        .unwrap()
        .env("MIRELAY_HTTP_CLI_TEST_TOKEN", TOKEN)
        .args(["--config", config_path.to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicates::str::contains("source pending:          0"));

    list.assert_calls(1);
}
