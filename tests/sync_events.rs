use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use anyhow::{Result, bail};
use mirelay::config::{Config, InitOverrides};
use mirelay::model::{
    Delivery, DeliveryRecord, DeliveryStatus, MANIFEST_SCHEMA_VERSION, WallpaperStatus,
};
use mirelay::source::{DeliverySource, SourceScan};
use mirelay::state::StateStore;
use mirelay::sync::{SyncEvent, SyncPhase, SyncSummary, sync_once, sync_once_with_events};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct EventSource {
    delivery: Delivery,
    payload: Vec<u8>,
    acknowledged: AtomicBool,
    hide_listing: AtomicBool,
    relist_completed: AtomicBool,
    opens: AtomicUsize,
    ack_attempts: AtomicUsize,
    fail_opens: usize,
    fail_acks: usize,
    before_open: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl EventSource {
    fn new(payload: &[u8], media_type: &str) -> Self {
        Self {
            delivery: Delivery {
                schema_version: MANIFEST_SCHEMA_VERSION,
                id: "events-file".into(),
                original_name: "example.txt".into(),
                payload: "events-file.payload".into(),
                size: payload.len() as u64,
                sha256: hex::encode(Sha256::digest(payload)),
                media_type: media_type.into(),
                created_at_unix: Some(1),
            },
            payload: payload.to_vec(),
            acknowledged: AtomicBool::new(false),
            hide_listing: AtomicBool::new(false),
            relist_completed: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            ack_attempts: AtomicUsize::new(0),
            fail_opens: 0,
            fail_acks: 0,
            before_open: None,
        }
    }

    fn text() -> Self {
        Self::new(b"MiRelay progress test\n", "text/plain")
    }

    fn image() -> Self {
        Self::new(b"\x89PNG\r\n\x1a\nMiRelay progress test", "image/png")
    }
}

impl DeliverySource for EventSource {
    fn scan_pending(&self, _max_file_size: u64) -> Result<SourceScan> {
        let listed = !self.hide_listing.load(Ordering::SeqCst)
            && (!self.acknowledged.load(Ordering::SeqCst)
                || self.relist_completed.load(Ordering::SeqCst));
        Ok(SourceScan {
            deliveries: if listed {
                vec![self.delivery.clone()]
            } else {
                vec![]
            },
            issues: vec![],
        })
    }

    fn open_payload(&self, _delivery: &Delivery) -> Result<Box<dyn Read + Send>> {
        if let Some(before_open) = &self.before_open {
            before_open();
        }
        if self.opens.fetch_add(1, Ordering::SeqCst) < self.fail_opens {
            return Err(
                io::Error::new(io::ErrorKind::ConnectionReset, "injected disconnect").into(),
            );
        }
        Ok(Box::new(Cursor::new(self.payload.clone())))
    }

    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()> {
        assert_eq!(id, self.delivery.id);
        assert_eq!(sha256, self.delivery.sha256);
        if self.ack_attempts.fetch_add(1, Ordering::SeqCst) < self.fail_acks {
            bail!("injected ACK failure");
        }
        self.acknowledged.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn payload_retry_delay(&self, error: &anyhow::Error, failed_attempts: u32) -> Option<Duration> {
        (failed_attempts < 3 && error.downcast_ref::<io::Error>().is_some())
            .then_some(Duration::ZERO)
    }
}

fn config(root: &TempDir) -> Config {
    Config::defaults(InitOverrides {
        data_dir: Some(root.path().to_path_buf()),
        ..Default::default()
    })
    .unwrap()
}

fn collect(config: &Config, source: &EventSource) -> (SyncSummary, Vec<SyncEvent>) {
    let events = Mutex::new(Vec::new());
    let summary = sync_once_with_events(config, source, &|event| {
        events.lock().unwrap().push(event);
    })
    .unwrap();
    (summary, events.into_inner().unwrap())
}

fn phases(events: &[SyncEvent]) -> Vec<SyncPhase> {
    events
        .iter()
        .filter_map(|event| match event {
            SyncEvent::FileActive { phase, .. } => Some(*phase),
            SyncEvent::FileSettled { .. } => None,
        })
        .collect()
}

fn last_settled(events: &[SyncEvent]) -> (Option<DeliveryRecord>, &Option<String>) {
    let Some(SyncEvent::FileSettled { id, record, error }) = events.last() else {
        panic!("file must finish with a settled event: {events:?}");
    };
    assert_eq!(id, "events-file");
    (record.as_deref().cloned(), error)
}

fn load_record(config: &Config) -> DeliveryRecord {
    StateStore::new(config.storage.state_file.clone())
        .load()
        .unwrap()
        .deliveries
        .remove("events-file")
        .unwrap()
}

#[test]
fn new_file_reports_phases_then_the_persisted_record() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let source = EventSource::text();
    let (summary, events) = collect(&config, &source);

    assert!(summary.failures.is_empty());
    assert_eq!(events.len(), 3);
    assert_eq!(
        phases(&events),
        [SyncPhase::Downloading, SyncPhase::Confirming]
    );
    assert!(matches!(
        &events[0],
        SyncEvent::FileActive { id, original_name, media_type, size, .. }
            if id == "events-file" && original_name == "example.txt"
                && media_type == "text/plain" && *size == source.delivery.size
    ));
    let (record, error) = last_settled(&events);
    assert!(error.is_none());
    assert_eq!(record.as_ref().unwrap(), &load_record(&config));
    assert_eq!(
        record.as_ref().unwrap().delivery_status,
        DeliveryStatus::Acknowledged
    );
}

#[test]
fn downloading_event_arrives_while_payload_open_is_still_blocked() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let (release_tx, release_rx) = mpsc::channel();
    let (opened_tx, opened_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let mut source = EventSource::text();
    source.before_open = Some(Arc::new(move || {
        opened_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    }));
    let (events_tx, events_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        sync_once_with_events(&config, &source, &|event| events_tx.send(event).unwrap()).unwrap()
    });

    let first = events_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        first,
        SyncEvent::FileActive {
            phase: SyncPhase::Downloading,
            ..
        }
    ));
    opened_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(events_rx.try_recv().is_err());
    release_tx.send(()).unwrap();
    let summary = worker.join().unwrap();
    assert!(summary.failures.is_empty());
    let remaining = events_rx.into_iter().collect::<Vec<_>>();
    assert!(last_settled(&remaining).1.is_none());
}

#[test]
fn checksum_failure_settles_without_a_record_or_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.delivery.sha256 = "0".repeat(64);
    let (summary, events) = collect(&config, &source);

    assert_eq!(summary.failures.len(), 1);
    assert_eq!(phases(&events), [SyncPhase::Downloading]);
    let (record, error) = last_settled(&events);
    assert!(record.is_none());
    assert!(error.as_ref().unwrap().contains("checksum mismatch"));
    assert_eq!(source.ack_attempts.load(Ordering::SeqCst), 0);
}

#[test]
fn retry_reports_each_attempt_and_settles_after_success() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.fail_opens = 1;
    let (summary, events) = collect(&config, &source);

    assert!(summary.failures.is_empty());
    assert_eq!(
        phases(&events),
        [
            SyncPhase::Downloading,
            SyncPhase::Retrying,
            SyncPhase::Downloading,
            SyncPhase::Confirming
        ]
    );
    assert_eq!(source.opens.load(Ordering::SeqCst), 2);
    assert!(last_settled(&events).1.is_none());
}

#[test]
fn exhausted_retries_end_in_a_settled_error() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.fail_opens = 3;
    let (summary, events) = collect(&config, &source);

    assert_eq!(summary.failures.len(), 1);
    assert_eq!(
        phases(&events),
        [
            SyncPhase::Downloading,
            SyncPhase::Retrying,
            SyncPhase::Downloading,
            SyncPhase::Retrying,
            SyncPhase::Downloading
        ]
    );
    let (record, error) = last_settled(&events);
    assert!(record.is_none());
    assert!(error.as_ref().unwrap().contains("failed after 3 attempts"));
}

#[test]
fn ack_failures_and_reconciliation_always_settle_without_redownloading() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.fail_acks = 2;

    for attempt in 0..3 {
        let (summary, events) = collect(&config, &source);
        assert_eq!(
            phases(&events),
            [
                if attempt == 0 {
                    SyncPhase::Downloading
                } else {
                    SyncPhase::Verifying
                },
                SyncPhase::Confirming
            ]
        );
        let (record, error) = last_settled(&events);
        assert_eq!(record.as_ref().unwrap(), &load_record(&config));
        if attempt < 2 {
            assert_eq!(summary.failures.len(), 1);
            assert_eq!(
                record.as_ref().unwrap().delivery_status,
                DeliveryStatus::AckPending
            );
            assert!(error.as_ref().unwrap().contains("ACK failure"));
        } else {
            assert!(summary.failures.is_empty());
            assert_eq!(
                record.as_ref().unwrap().delivery_status,
                DeliveryStatus::Acknowledged
            );
            assert!(error.is_none());
        }
    }
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
}

#[test]
fn already_completed_unchanged_files_do_not_report_activity_even_if_relisted() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let source = EventSource::text();
    sync_once(&config, &source).unwrap();

    assert!(collect(&config, &source).1.is_empty());
    source.relist_completed.store(true, Ordering::SeqCst);
    let (summary, events) = collect(&config, &source);
    assert_eq!(summary.already_stored, 1);
    assert!(events.is_empty());
}

#[test]
fn damaged_pending_file_settles_verification_then_reports_repair() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.fail_acks = 1;
    sync_once(&config, &source).unwrap();
    std::fs::write(load_record(&config).stored_path, b"corrupted").unwrap();

    let (summary, events) = collect(&config, &source);
    assert!(summary.failures.is_empty());
    assert_eq!(summary.repaired, 1);
    assert_eq!(
        phases(&events),
        [
            SyncPhase::Verifying,
            SyncPhase::Verifying,
            SyncPhase::Retrying,
            SyncPhase::Downloading,
            SyncPhase::Confirming
        ]
    );
    assert!(matches!(
        &events[1],
        SyncEvent::FileSettled { error: Some(_), .. }
    ));
    let (record, error) = last_settled(&events);
    assert!(error.is_none());
    assert_eq!(record.as_ref().unwrap(), &load_record(&config));
}

#[test]
fn unrepairable_pending_file_does_not_leave_active_verification() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut source = EventSource::text();
    source.fail_acks = 1;
    sync_once(&config, &source).unwrap();
    std::fs::write(load_record(&config).stored_path, b"corrupted").unwrap();
    source.hide_listing.store(true, Ordering::SeqCst);

    let (summary, events) = collect(&config, &source);
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(phases(&events), [SyncPhase::Verifying]);
    assert!(last_settled(&events).1.is_some());
}

#[test]
fn disabled_wallpaper_does_not_report_application_activity() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let source = EventSource::image();
    let (_, events) = collect(&config, &source);

    assert_eq!(
        phases(&events),
        [SyncPhase::Downloading, SyncPhase::Confirming]
    );
    assert_eq!(
        load_record(&config).wallpaper_status,
        WallpaperStatus::NotConfigured
    );
    assert!(collect(&config, &source).1.is_empty());
}

#[cfg(unix)]
#[test]
fn real_wallpaper_application_reports_stages_and_settles_success_or_failure() {
    for (command, expected_status) in [
        ("/bin/true", WallpaperStatus::Applied),
        ("/bin/false", WallpaperStatus::Failed),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut config = config(&root);
        config.wallpaper.command = vec![command.into(), "{path}".into()];
        let source = EventSource::image();
        let (_, events) = collect(&config, &source);

        assert_eq!(
            phases(&events),
            [
                SyncPhase::Downloading,
                SyncPhase::Confirming,
                SyncPhase::Verifying,
                SyncPhase::ApplyingWallpaper
            ]
        );
        let (record, error) = last_settled(&events);
        assert_eq!(record.as_ref().unwrap(), &load_record(&config));
        assert_eq!(record.as_ref().unwrap().wallpaper_status, expected_status);
        assert_eq!(error.is_some(), expected_status == WallpaperStatus::Failed);
        assert!(collect(&config, &source).1.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn failed_wallpaper_verification_settles_without_applying() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(&root);
    let source = EventSource::image();
    sync_once(&config, &source).unwrap();
    config.wallpaper.command = vec!["/bin/true".into(), "{path}".into()];
    std::fs::write(load_record(&config).stored_path, b"corrupted").unwrap();

    let (summary, events) = collect(&config, &source);
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(phases(&events), [SyncPhase::Verifying]);
    let (record, error) = last_settled(&events);
    assert_eq!(
        record.as_ref().unwrap().wallpaper_status,
        WallpaperStatus::Failed
    );
    assert!(error.is_some());
}

#[test]
fn failed_state_write_never_reports_an_unpersisted_acknowledgement() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let source = EventSource::text();
    let events = Mutex::new(Vec::new());
    let saved_state = root.path().join("last-saved-state.json");
    let summary = sync_once_with_events(&config, &source, &|event| {
        if matches!(
            event,
            SyncEvent::FileActive {
                phase: SyncPhase::Confirming,
                ..
            }
        ) {
            std::fs::rename(&config.storage.state_file, &saved_state).unwrap();
            std::fs::create_dir(&config.storage.state_file).unwrap();
        }
        events.lock().unwrap().push(event);
    })
    .unwrap();
    assert_eq!(summary.failures.len(), 1);
    let events = events.into_inner().unwrap();
    let (record, error) = last_settled(&events);
    assert!(error.is_some());
    assert_eq!(
        record.as_ref().unwrap().delivery_status,
        DeliveryStatus::AckPending
    );
    let persisted = StateStore::new(saved_state).load().unwrap();
    assert_eq!(record.as_ref(), persisted.deliveries.get("events-file"));
}
