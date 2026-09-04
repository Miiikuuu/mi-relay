use std::fmt;
use std::io::{self, Cursor, Read};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{Result, bail};
use mirelay::config::{Config, LimitsConfig, ServerConfig, StorageConfig, WallpaperConfig};
use mirelay::model::{Delivery, DeliveryStatus, MANIFEST_SCHEMA_VERSION, WallpaperStatus};
use mirelay::source::{DeliverySource, SourceScan};
use mirelay::state::StateStore;
use mirelay::sync::{retry_wallpapers, sync_once};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct TestSource {
    delivery: Delivery,
    bytes: Vec<u8>,
    acknowledged: AtomicBool,
    ack_failures_remaining: Mutex<usize>,
    payload_failures_remaining: Mutex<usize>,
    opens: AtomicUsize,
    ack_attempts: AtomicUsize,
}

impl TestSource {
    fn new(bytes: Vec<u8>, ack_failures: usize) -> Self {
        let delivery = Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: "recovery-delivery".into(),
            original_name: "wallpaper.png".into(),
            payload: "recovery-delivery.payload".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
            media_type: "image/png".into(),
            created_at_unix: Some(1),
        };
        Self {
            delivery,
            bytes,
            acknowledged: AtomicBool::new(false),
            ack_failures_remaining: Mutex::new(ack_failures),
            payload_failures_remaining: Mutex::new(0),
            opens: AtomicUsize::new(0),
            ack_attempts: AtomicUsize::new(0),
        }
    }

    fn with_payload_failures(self, failures: usize) -> Self {
        *self.payload_failures_remaining.lock().unwrap() = failures;
        self
    }
}

#[derive(Debug)]
struct InjectedTransientReadError;

impl fmt::Display for InjectedTransientReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("injected transient payload read failure")
    }
}

impl std::error::Error for InjectedTransientReadError {}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            InjectedTransientReadError,
        ))
    }
}

impl DeliverySource for TestSource {
    fn scan_pending(&self, _max_file_size: u64) -> Result<SourceScan> {
        Ok(SourceScan {
            deliveries: if self.acknowledged.load(Ordering::SeqCst) {
                vec![]
            } else {
                vec![self.delivery.clone()]
            },
            issues: vec![],
        })
    }

    fn open_payload(&self, _delivery: &Delivery) -> Result<Box<dyn Read + Send>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let mut failures = self.payload_failures_remaining.lock().unwrap();
        if *failures > 0 {
            *failures -= 1;
            return Ok(Box::new(FailingReader));
        }
        Ok(Box::new(Cursor::new(self.bytes.clone())))
    }

    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()> {
        assert_eq!(id, self.delivery.id);
        assert_eq!(sha256, self.delivery.sha256);
        self.ack_attempts.fetch_add(1, Ordering::SeqCst);
        let mut failures = self.ack_failures_remaining.lock().unwrap();
        if *failures > 0 {
            *failures -= 1;
            bail!("injected ACK failure");
        }
        self.acknowledged.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn payload_retry_delay(
        &self,
        error: &anyhow::Error,
        failed_attempts: u32,
    ) -> Option<std::time::Duration> {
        (failed_attempts < 3
            && error.chain().any(|cause| {
                cause.downcast_ref::<InjectedTransientReadError>().is_some()
                    || cause
                        .downcast_ref::<io::Error>()
                        .and_then(io::Error::get_ref)
                        .and_then(|inner| inner.downcast_ref::<InjectedTransientReadError>())
                        .is_some()
            }))
        .then_some(std::time::Duration::ZERO)
    }
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(b"MiRelay recovery test");
    bytes
}

fn config(root: &TempDir, command: Vec<String>) -> Config {
    Config {
        schema_version: 1,
        device_id: "test-device".into(),
        server: ServerConfig::Filesystem {
            inbox_dir: root.path().join("unused-inbox"),
        },
        storage: StorageConfig {
            library_dir: root.path().join("library"),
            state_file: root.path().join("state.json"),
        },
        wallpaper: WallpaperConfig {
            command,
            timeout_seconds: 2,
        },
        limits: LimitsConfig {
            max_file_size_bytes: 1024,
        },
    }
}

fn load_only_record(config: &Config) -> mirelay::model::DeliveryRecord {
    let state = StateStore::new(config.storage.state_file.clone())
        .load()
        .unwrap();
    assert_eq!(state.deliveries.len(), 1);
    state.deliveries.into_values().next().unwrap()
}

#[test]
fn ack_failure_recovers_without_downloading_again() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root, vec![]);
    let source = TestSource::new(png_bytes(), 1);

    let first = sync_once(&config, &source).unwrap();
    assert_eq!(first.received, 1);
    assert_eq!(first.failures.len(), 1);
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(
        load_only_record(&config).delivery_status,
        DeliveryStatus::AckPending
    );

    let second = sync_once(&config, &source).unwrap();
    assert!(second.failures.is_empty());
    assert_eq!(second.received, 0);
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(source.ack_attempts.load(Ordering::SeqCst), 2);
    let record = load_only_record(&config);
    assert_eq!(record.delivery_status, DeliveryStatus::Acknowledged);
    assert_eq!(record.wallpaper_status, WallpaperStatus::NotConfigured);
}

#[test]
fn transient_payload_read_failure_restarts_the_full_download() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root, vec![]);
    let source = TestSource::new(png_bytes(), 0).with_payload_failures(1);

    let summary = sync_once(&config, &source).unwrap();

    assert!(summary.failures.is_empty());
    assert_eq!(summary.received, 1);
    assert_eq!(summary.acknowledged, 1);
    assert_eq!(source.opens.load(Ordering::SeqCst), 2);
    assert_eq!(source.ack_attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn damaged_ack_pending_content_is_downloaded_again_and_repaired() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root, vec![]);
    let source = TestSource::new(png_bytes(), 1);

    let first = sync_once(&config, &source).unwrap();
    assert_eq!(first.received, 1);
    assert_eq!(first.failures.len(), 1);
    let damaged_path = load_only_record(&config).stored_path;
    std::fs::write(&damaged_path, b"damaged local bytes").unwrap();

    let second = sync_once(&config, &source).unwrap();
    assert!(second.failures.is_empty());
    assert_eq!(second.repaired, 1);
    assert_eq!(second.acknowledged, 1);
    assert_eq!(source.opens.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&damaged_path).unwrap(), source.bytes);
    let record = load_only_record(&config);
    assert_eq!(record.delivery_status, DeliveryStatus::Acknowledged);
    assert!(record.delivery_error.is_none());
}

#[test]
fn wallpaper_failure_does_not_redownload_or_repeat_automatically() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root, vec!["/bin/false".into(), "{path}".into()]);
    let source = TestSource::new(png_bytes(), 0);

    let first = sync_once(&config, &source).unwrap();
    assert_eq!(first.received, 1);
    assert_eq!(first.failures.len(), 1);
    let record = load_only_record(&config);
    assert_eq!(record.delivery_status, DeliveryStatus::Acknowledged);
    assert_eq!(record.wallpaper_status, WallpaperStatus::Failed);
    assert_eq!(record.wallpaper_attempts, 1);

    let second = sync_once(&config, &source).unwrap();
    assert!(second.failures.is_empty());
    assert_eq!(load_only_record(&config).wallpaper_attempts, 1);
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(source.ack_attempts.load(Ordering::SeqCst), 1);

    let retry = retry_wallpapers(&config, false).unwrap();
    assert_eq!(retry.attempted, 1);
    assert_eq!(retry.failures.len(), 1);
    assert_eq!(load_only_record(&config).wallpaper_attempts, 2);
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(source.ack_attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn interrupted_wallpaper_requires_explicit_uncertain_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(&root, vec![]);
    let source = TestSource::new(png_bytes(), 0);
    sync_once(&config, &source).unwrap();

    let store = StateStore::new(config.storage.state_file.clone());
    let mut state = store.load().unwrap();
    state
        .deliveries
        .values_mut()
        .next()
        .unwrap()
        .wallpaper_status = WallpaperStatus::Running;
    store.save(&state).unwrap();

    config.wallpaper.command = vec!["/bin/true".into(), "{path}".into()];
    let safe_retry = retry_wallpapers(&config, false).unwrap();
    assert_eq!(safe_retry.skipped_uncertain, 1);
    assert_eq!(safe_retry.attempted, 0);
    assert_eq!(
        load_only_record(&config).wallpaper_status,
        WallpaperStatus::Uncertain
    );

    let explicit_retry = retry_wallpapers(&config, true).unwrap();
    assert_eq!(explicit_retry.applied, 1);
    assert_eq!(
        load_only_record(&config).wallpaper_status,
        WallpaperStatus::Applied
    );
}
