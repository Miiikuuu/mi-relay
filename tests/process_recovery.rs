#![cfg(unix)]

use std::fs;
use std::io::{self, Read};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use mirelay::config::{Config, InitOverrides, ServerConfig};
use mirelay::model::{Delivery, DeliveryStatus, WallpaperStatus};
use mirelay::source::{DeliverySource, FilesystemSource, SourceScan};
use mirelay::state::StateStore;
use mirelay::sync::{SyncEvent, SyncPhase, retry_wallpapers, sync_once, sync_once_with_events};

// Only children created by this test are ever terminated, including on assertion failure.
struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn gate(root: &Path) {
    fs::write(root.join("crash-ready"), b"ready").unwrap();
    // Fail closed if a parent disappears; no indefinitely orphaned test process.
    thread::sleep(Duration::from_secs(15));
    panic!("parent did not terminate the crash-test child");
}

struct InterruptedReader {
    inner: Box<dyn Read + Send>,
    root: PathBuf,
    read_once: bool,
}

impl Read for InterruptedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.read_once {
            gate(&self.root);
        }
        let size = self.inner.read(buffer)?;
        self.read_once = size > 0;
        Ok(size)
    }
}

struct CrashSource {
    inner: FilesystemSource,
    root: PathBuf,
    phase: String,
}

impl DeliverySource for CrashSource {
    fn scan_pending(&self, max: u64) -> Result<SourceScan> {
        self.inner.scan_pending(max)
    }

    fn open_payload(&self, delivery: &Delivery) -> Result<Box<dyn Read + Send>> {
        let reader = self.inner.open_payload(delivery)?;
        if self.phase == "download" {
            Ok(Box::new(InterruptedReader {
                inner: reader,
                root: self.root.clone(),
                read_once: false,
            }))
        } else {
            Ok(reader)
        }
    }

    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()> {
        if self.phase == "before-ack" {
            gate(&self.root);
        }
        self.inner.acknowledge(id, sha256)?;
        if self.phase == "after-ack" {
            gate(&self.root);
        }
        Ok(())
    }
}

#[test]
#[ignore = "subprocess helper, invoked only by the crash recovery tests"]
fn crash_worker() {
    let root = PathBuf::from(std::env::var_os("MIRELAY_CRASH_TEST_ROOT").unwrap());
    let phase = std::env::var("MIRELAY_CRASH_TEST_PHASE").unwrap();
    let config = Config::load(&root.join("config.toml")).unwrap();
    let ServerConfig::Filesystem { inbox_dir } = &config.server else {
        panic!("crash tests must not contact a server");
    };
    let source = CrashSource {
        inner: FilesystemSource::new(inbox_dir.clone()),
        root: root.clone(),
        phase: phase.clone(),
    };
    sync_once_with_events(&config, &source, &|event| {
        if phase == "wallpaper-running"
            && matches!(
                event,
                SyncEvent::FileActive {
                    phase: SyncPhase::ApplyingWallpaper,
                    ..
                }
            )
        {
            gate(&root);
        }
    })
    .unwrap();
    panic!("crash gate was not reached");
}

fn crash_and_recover(phase: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(root.path().join("data")),
        ..Default::default()
    })
    .unwrap();
    let mut bytes = vec![b'x'; 256 * 1024];
    if phase == "wallpaper-running" {
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        // A harmless hook, never the user's wallpaper command.
        config.wallpaper.command = vec!["/bin/true".into(), "{path}".into()];
    }
    config.ensure_directories().unwrap();
    config
        .save(&root.path().join("config.toml"), false)
        .unwrap();
    let ServerConfig::Filesystem { inbox_dir } = &config.server else {
        unreachable!()
    };
    let source = FilesystemSource::new(inbox_dir.clone());
    let input = root.path().join("payload.bin");
    fs::write(&input, &bytes).unwrap();
    let delivery = source
        .enqueue(&input, config.limits.max_file_size_bytes)
        .unwrap();
    let mut child = TestChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_worker", "--ignored", "--nocapture"])
            .env("MIRELAY_CRASH_TEST_ROOT", root.path())
            .env("MIRELAY_CRASH_TEST_PHASE", phase)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("crash-ready").exists() {
        assert!(Instant::now() < deadline, "crash gate timed out: {phase}");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {phase}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));

    let store = StateStore::new(config.storage.state_file.clone());
    // Taking the lock proves SIGKILL released the dead receiver's state lock.
    let before = {
        let _guard = store.lock_shared().unwrap();
        store.load().unwrap()
    };
    let ack_path = inbox_dir
        .join(".acks")
        .join(format!("{}.json", delivery.id));
    if phase == "download" {
        assert!(before.deliveries.is_empty());
        assert!(!ack_path.exists());
    } else {
        let record = &before.deliveries[&delivery.id];
        assert_eq!(fs::read(&record.stored_path).unwrap(), bytes);
        assert_eq!(ack_path.exists(), phase != "before-ack");
        assert_eq!(
            record.delivery_status,
            if phase == "wallpaper-running" {
                DeliveryStatus::Acknowledged
            } else {
                DeliveryStatus::AckPending
            }
        );
    }
    let resumed = sync_once(&config, &source).unwrap();
    assert!(resumed.failures.is_empty(), "{:#?}", resumed.failures);
    assert_eq!(resumed.received, usize::from(phase == "download"));
    assert_eq!(
        resumed.stale_staging_files_removed,
        usize::from(phase == "download")
    );
    let recovered = store.load().unwrap();
    assert_eq!(recovered.deliveries.len(), 1);
    let record = &recovered.deliveries[&delivery.id];
    assert_eq!(record.delivery_status, DeliveryStatus::Acknowledged);
    assert_eq!(record.sha256, delivery.sha256);
    assert_eq!(fs::read(&record.stored_path).unwrap(), bytes);
    assert!(
        source
            .scan_pending(config.limits.max_file_size_bytes)
            .unwrap()
            .deliveries
            .is_empty()
    );
    if phase == "wallpaper-running" {
        assert_eq!(record.wallpaper_status, WallpaperStatus::Uncertain);
        assert_eq!(record.wallpaper_attempts, 1);
        assert_eq!(retry_wallpapers(&config, false).unwrap().attempted, 0);
        assert_eq!(retry_wallpapers(&config, true).unwrap().applied, 1);
    }
    let second = sync_once(&config, &source).unwrap();
    assert_eq!(second.received, 0);
    assert_eq!(second.acknowledged, 0);
    assert_eq!(store.load().unwrap().deliveries.len(), 1);
}

#[test]
fn sigkill_during_download_removes_partial_content_and_recovers() {
    crash_and_recover("download");
}

#[test]
fn sigkill_before_ack_reuses_the_durable_file() {
    crash_and_recover("before-ack");
}

#[test]
fn sigkill_after_remote_ack_reconciles_without_duplicate_delivery() {
    crash_and_recover("after-ack");
}

#[test]
fn sigkill_after_marking_wallpaper_running_requires_explicit_retry() {
    crash_and_recover("wallpaper-running");
}
