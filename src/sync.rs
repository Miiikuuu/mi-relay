use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::fsutil::unix_now;
use crate::model::{AppState, Delivery, DeliveryRecord, DeliveryStatus, WallpaperStatus};
use crate::source::DeliverySource;
use crate::state::StateStore;
use crate::storage::{
    StoredAsset, cleanup_staging, is_image_media_type, lock_library, quarantine_content_object,
    receive, verify_recorded_file,
};
use crate::wallpaper::{self, ApplyOutcome};

#[derive(Debug, Clone)]
pub struct SyncFailure {
    pub item: String,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct SyncSummary {
    pub stale_staging_files_removed: usize,
    pub received: usize,
    pub repaired: usize,
    pub already_stored: usize,
    pub acknowledged: usize,
    pub wallpaper_applied: usize,
    pub wallpaper_not_applicable: usize,
    pub wallpaper_not_configured: usize,
    pub failures: Vec<SyncFailure>,
}

#[derive(Debug, Default, Clone)]
pub struct RetrySummary {
    pub attempted: usize,
    pub applied: usize,
    pub not_configured: usize,
    pub skipped_uncertain: usize,
    pub failures: Vec<SyncFailure>,
}

pub fn sync_once(config: &Config, source: &dyn DeliverySource) -> Result<SyncSummary> {
    config.validate()?;
    let store = StateStore::new(config.storage.state_file.clone());
    let _lock = store.lock_exclusive()?;
    let _library_lock = lock_library(&config.storage.library_dir)?;
    let mut state = store.load()?;
    let mut summary = SyncSummary {
        stale_staging_files_removed: cleanup_staging(&config.storage.library_dir)?,
        ..Default::default()
    };

    reconcile_interrupted_wallpapers(&store, &mut state)?;
    let mut reconciliation =
        reconcile_pending_acks(config, source, &store, &mut state, &mut summary)?;

    let scan = source.scan_pending(config.limits.max_file_size_bytes)?;
    summary
        .failures
        .extend(scan.issues.into_iter().map(|issue| SyncFailure {
            item: issue.item,
            message: issue.message,
        }));

    for delivery in scan.deliveries {
        if reconciliation.handled.contains(&delivery.id) {
            summary.already_stored += 1;
            continue;
        }
        reconciliation.repair_needed.remove(&delivery.id);
        if let Err(error) =
            process_delivery(config, source, &store, &mut state, &delivery, &mut summary)
        {
            summary.failures.push(SyncFailure {
                item: delivery.id.clone(),
                message: format!("{error:#}"),
            });
        }
    }
    for (id, message) in reconciliation.repair_needed {
        summary.failures.push(SyncFailure {
            item: id,
            message: format!(
                "{message}; the server did not list this delivery, so it could not be repaired"
            ),
        });
    }

    process_wallpaper_queue(
        config,
        &store,
        &mut state,
        WallpaperRunMode::Automatic,
        &mut summary,
    )?;
    Ok(summary)
}

pub fn retry_wallpapers(config: &Config, include_uncertain: bool) -> Result<RetrySummary> {
    config.validate()?;
    let store = StateStore::new(config.storage.state_file.clone());
    let _lock = store.lock_exclusive()?;
    let mut state = store.load()?;
    reconcile_interrupted_wallpapers(&store, &mut state)?;
    let skipped_uncertain = if include_uncertain {
        0
    } else {
        state
            .deliveries
            .values()
            .filter(|record| record.wallpaper_status == WallpaperStatus::Uncertain)
            .count()
    };

    let mut sync_summary = SyncSummary::default();
    process_wallpaper_queue(
        config,
        &store,
        &mut state,
        WallpaperRunMode::ExplicitRetry { include_uncertain },
        &mut sync_summary,
    )?;

    Ok(RetrySummary {
        attempted: sync_summary.wallpaper_applied
            + sync_summary.wallpaper_not_configured
            + sync_summary.failures.len(),
        applied: sync_summary.wallpaper_applied,
        not_configured: sync_summary.wallpaper_not_configured,
        skipped_uncertain,
        failures: sync_summary.failures,
    })
}

fn reconcile_interrupted_wallpapers(store: &StateStore, state: &mut AppState) -> Result<()> {
    let interrupted = state
        .deliveries
        .iter()
        .filter(|(_, record)| record.wallpaper_status == WallpaperStatus::Running)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if interrupted.is_empty() {
        return Ok(());
    }

    for id in interrupted {
        let record = state.deliveries.get_mut(&id).expect("id came from state");
        record.wallpaper_status = WallpaperStatus::Uncertain;
        record.wallpaper_error = Some(
            "the previous CLI run ended while the wallpaper command was running; use retry --include-uncertain only if repeating it is safe"
                .into(),
        );
    }
    store.save(state)
}

struct AckReconciliation {
    handled: BTreeSet<String>,
    repair_needed: BTreeMap<String, String>,
}

fn reconcile_pending_acks(
    config: &Config,
    source: &dyn DeliverySource,
    store: &StateStore,
    state: &mut AppState,
    summary: &mut SyncSummary,
) -> Result<AckReconciliation> {
    let pending = state
        .deliveries
        .iter()
        .filter(|(_, record)| record.delivery_status == DeliveryStatus::AckPending)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();

    let mut handled = BTreeSet::new();
    let mut repair_needed = BTreeMap::new();
    for id in pending {
        let snapshot = state.deliveries[&id].clone();
        let verification = verify_recorded_file(
            &snapshot.stored_path,
            snapshot.size,
            &snapshot.sha256,
            &snapshot.media_type,
            config.limits.max_file_size_bytes,
        );
        let Err(error) = verification else {
            handled.insert(id.clone());
            match source.acknowledge(&snapshot.id, &snapshot.sha256) {
                Ok(()) => {
                    let record = state.deliveries.get_mut(&id).expect("id came from state");
                    record.delivery_status = DeliveryStatus::Acknowledged;
                    record.acknowledged_at_unix = Some(unix_now());
                    record.delivery_error = None;
                    store.save(state)?;
                    summary.acknowledged += 1;
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    state
                        .deliveries
                        .get_mut(&id)
                        .expect("id came from state")
                        .delivery_error = Some(message.clone());
                    store.save(state)?;
                    summary.failures.push(SyncFailure { item: id, message });
                }
            }
            continue;
        };

        let message = format!("{error:#}");
        state
            .deliveries
            .get_mut(&id)
            .expect("id came from state")
            .delivery_error = Some(message.clone());
        store.save(state)?;
        repair_needed.insert(id, message);
    }
    Ok(AckReconciliation {
        handled,
        repair_needed,
    })
}

fn process_delivery(
    config: &Config,
    source: &dyn DeliverySource,
    store: &StateStore,
    state: &mut AppState,
    delivery: &Delivery,
    summary: &mut SyncSummary,
) -> Result<()> {
    if let Some(existing) = state.deliveries.get(&delivery.id).cloned() {
        if existing.sha256 != delivery.sha256
            || existing.size != delivery.size
            || existing.media_type != delivery.media_type
        {
            anyhow::bail!(
                "delivery id collision: stored metadata does not match the incoming delivery"
            );
        }
        let mut repaired = false;
        if verify_recorded_file(
            &existing.stored_path,
            existing.size,
            &existing.sha256,
            &existing.media_type,
            config.limits.max_file_size_bytes,
        )
        .is_err()
        {
            quarantine_content_object(
                &config.storage.library_dir,
                &existing.sha256,
                &existing.media_type,
            )?;
            let stored = match receive_delivery_with_retry(config, source, delivery) {
                Ok(stored) => stored,
                Err(error) => {
                    let message = format!("{error:#}");
                    state
                        .deliveries
                        .get_mut(&delivery.id)
                        .expect("record was just found")
                        .delivery_error = Some(message);
                    store.save(state)?;
                    return Err(error);
                }
            };
            summary.stale_staging_files_removed += cleanup_staging(&config.storage.library_dir)?;
            let record = state
                .deliveries
                .get_mut(&delivery.id)
                .expect("record was just found");
            record.stored_path = stored.path;
            record.size = stored.size;
            record.sha256 = stored.sha256;
            record.media_type = stored.media_type;
            record.delivery_error = None;
            store.save(state)?;
            summary.repaired += 1;
            repaired = true;
        }
        let snapshot = state.deliveries[&delivery.id].clone();
        if let Err(error) = source.acknowledge(&snapshot.id, &snapshot.sha256) {
            let message = format!("{error:#}");
            state
                .deliveries
                .get_mut(&delivery.id)
                .expect("record was just found")
                .delivery_error = Some(message.clone());
            store.save(state)?;
            anyhow::bail!(message);
        }
        if snapshot.delivery_status != DeliveryStatus::Acknowledged
            || snapshot.delivery_error.is_some()
        {
            let newly_acknowledged = snapshot.delivery_status != DeliveryStatus::Acknowledged;
            let record = state
                .deliveries
                .get_mut(&delivery.id)
                .expect("record was just found");
            record.delivery_status = DeliveryStatus::Acknowledged;
            if newly_acknowledged {
                record.acknowledged_at_unix = Some(unix_now());
            }
            record.delivery_error = None;
            store.save(state)?;
            if newly_acknowledged {
                summary.acknowledged += 1;
            }
        }
        if !repaired {
            summary.already_stored += 1;
        }
        return Ok(());
    }

    let stored = receive_delivery_with_retry(config, source, delivery)?;
    let now = unix_now();
    let wallpaper_status = if is_image_media_type(&stored.media_type) {
        WallpaperStatus::Pending
    } else {
        summary.wallpaper_not_applicable += 1;
        WallpaperStatus::NotApplicable
    };
    let record = DeliveryRecord {
        id: delivery.id.clone(),
        original_name: delivery.original_name.clone(),
        media_type: stored.media_type,
        sha256: stored.sha256,
        size: stored.size,
        stored_path: stored.path,
        delivery_status: DeliveryStatus::AckPending,
        wallpaper_status,
        wallpaper_attempts: 0,
        delivery_error: None,
        wallpaper_error: None,
        source_created_at_unix: delivery.created_at_unix,
        received_at_unix: now,
        acknowledged_at_unix: None,
        imported_at_unix: None,
    };
    state.deliveries.insert(delivery.id.clone(), record);
    store
        .save(state)
        .context("file was stored, but its state could not be persisted")?;
    summary.received += 1;

    match source.acknowledge(&delivery.id, &delivery.sha256) {
        Ok(()) => {
            let record = state
                .deliveries
                .get_mut(&delivery.id)
                .expect("record was just inserted");
            record.delivery_status = DeliveryStatus::Acknowledged;
            record.acknowledged_at_unix = Some(unix_now());
            record.delivery_error = None;
            store.save(state)?;
            summary.acknowledged += 1;
        }
        Err(error) => {
            let message = format!("{error:#}");
            state
                .deliveries
                .get_mut(&delivery.id)
                .expect("record was just inserted")
                .delivery_error = Some(message.clone());
            store.save(state)?;
            summary.failures.push(SyncFailure {
                item: delivery.id.clone(),
                message,
            });
        }
    }
    Ok(())
}

fn receive_delivery_with_retry(
    config: &Config,
    source: &dyn DeliverySource,
    delivery: &Delivery,
) -> Result<StoredAsset> {
    let mut failed_attempts = 0_u32;
    loop {
        let result = source.open_payload(delivery).and_then(|reader| {
            receive(
                delivery,
                reader,
                &config.storage.library_dir,
                config.limits.max_file_size_bytes,
            )
        });
        match result {
            Ok(stored) => return Ok(stored),
            Err(error) => {
                failed_attempts = failed_attempts.saturating_add(1);
                let Some(delay) = source.payload_retry_delay(&error, failed_attempts) else {
                    return if failed_attempts > 1 {
                        Err(error).with_context(|| {
                            format!(
                                "downloading delivery {} failed after {} attempts",
                                delivery.id, failed_attempts
                            )
                        })
                    } else {
                        Err(error)
                    };
                };
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WallpaperRunMode {
    Automatic,
    ExplicitRetry { include_uncertain: bool },
}

fn process_wallpaper_queue(
    config: &Config,
    store: &StateStore,
    state: &mut AppState,
    mode: WallpaperRunMode,
    summary: &mut SyncSummary,
) -> Result<()> {
    let legacy_non_images = state
        .deliveries
        .iter()
        .filter(|(_, record)| {
            !is_image_media_type(&record.media_type)
                && record.wallpaper_status != WallpaperStatus::NotApplicable
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if !legacy_non_images.is_empty() {
        for id in legacy_non_images {
            let record = state.deliveries.get_mut(&id).expect("id came from state");
            record.wallpaper_status = WallpaperStatus::NotApplicable;
            record.wallpaper_error = None;
            record.imported_at_unix = None;
            summary.wallpaper_not_applicable += 1;
        }
        store.save(state)?;
    }

    let mut candidates = state
        .deliveries
        .iter()
        .filter_map(|(id, record)| {
            if record.delivery_status != DeliveryStatus::Acknowledged {
                return None;
            }
            let eligible = match mode {
                WallpaperRunMode::Automatic => matches!(
                    record.wallpaper_status,
                    WallpaperStatus::Pending | WallpaperStatus::NotConfigured
                ),
                WallpaperRunMode::ExplicitRetry { include_uncertain } => {
                    matches!(
                        record.wallpaper_status,
                        WallpaperStatus::Pending
                            | WallpaperStatus::NotConfigured
                            | WallpaperStatus::Failed
                    ) || (include_uncertain
                        && record.wallpaper_status == WallpaperStatus::Uncertain)
                }
            };
            eligible.then(|| {
                (
                    record
                        .source_created_at_unix
                        .unwrap_or(record.received_at_unix),
                    id.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort();

    for (_, id) in candidates {
        let snapshot = state.deliveries[&id].clone();
        if config.wallpaper.command.is_empty() {
            let record = state.deliveries.get_mut(&id).expect("id came from state");
            record.wallpaper_status = WallpaperStatus::NotConfigured;
            record.wallpaper_error = None;
            store.save(state)?;
            summary.wallpaper_not_configured += 1;
            continue;
        }

        if let Err(error) = verify_recorded_file(
            &snapshot.stored_path,
            snapshot.size,
            &snapshot.sha256,
            &snapshot.media_type,
            config.limits.max_file_size_bytes,
        ) {
            let message = format!("{error:#}");
            {
                let record = state.deliveries.get_mut(&id).expect("id came from state");
                record.delivery_error = Some(message.clone());
                record.wallpaper_status = WallpaperStatus::Failed;
                record.wallpaper_error = Some(message.clone());
            }
            store.save(state)?;
            summary.failures.push(SyncFailure { item: id, message });
            continue;
        }
        {
            let record = state.deliveries.get_mut(&id).expect("id came from state");
            record.delivery_error = None;
            record.wallpaper_status = WallpaperStatus::Running;
            record.wallpaper_attempts = record.wallpaper_attempts.saturating_add(1);
            record.wallpaper_error = None;
        }
        store.save(state)?;

        match wallpaper::apply(&config.wallpaper, &snapshot) {
            Ok(ApplyOutcome::Applied) => {
                let record = state.deliveries.get_mut(&id).expect("id came from state");
                record.wallpaper_status = WallpaperStatus::Applied;
                record.imported_at_unix = Some(unix_now());
                record.wallpaper_error = None;
                store.save(state)?;
                summary.wallpaper_applied += 1;
            }
            Ok(ApplyOutcome::NotConfigured) => {
                let record = state.deliveries.get_mut(&id).expect("id came from state");
                record.wallpaper_status = WallpaperStatus::NotConfigured;
                store.save(state)?;
                summary.wallpaper_not_configured += 1;
            }
            Err(error) => {
                let record = state.deliveries.get_mut(&id).expect("id came from state");
                record.wallpaper_status = if error.uncertain {
                    WallpaperStatus::Uncertain
                } else {
                    WallpaperStatus::Failed
                };
                record.wallpaper_error = Some(error.message.clone());
                store.save(state)?;
                summary.failures.push(SyncFailure {
                    item: id,
                    message: error.message,
                });
            }
        }
    }
    Ok(())
}

pub fn status_counts(state: &AppState) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::from([
        ("total", 0),
        ("ack_pending", 0),
        ("acknowledged", 0),
        ("wallpaper_pending", 0),
        ("wallpaper_not_applicable", 0),
        ("wallpaper_applied", 0),
        ("wallpaper_failed", 0),
        ("wallpaper_uncertain", 0),
        ("wallpaper_not_configured", 0),
    ]);
    for record in state.deliveries.values() {
        *counts.get_mut("total").unwrap() += 1;
        match record.delivery_status {
            DeliveryStatus::AckPending => *counts.get_mut("ack_pending").unwrap() += 1,
            DeliveryStatus::Acknowledged => *counts.get_mut("acknowledged").unwrap() += 1,
        }
        match record.wallpaper_status {
            WallpaperStatus::Pending | WallpaperStatus::Running => {
                *counts.get_mut("wallpaper_pending").unwrap() += 1
            }
            WallpaperStatus::NotConfigured => {
                *counts.get_mut("wallpaper_not_configured").unwrap() += 1
            }
            WallpaperStatus::NotApplicable => {
                *counts.get_mut("wallpaper_not_applicable").unwrap() += 1
            }
            WallpaperStatus::Applied => *counts.get_mut("wallpaper_applied").unwrap() += 1,
            WallpaperStatus::Failed => *counts.get_mut("wallpaper_failed").unwrap() += 1,
            WallpaperStatus::Uncertain => *counts.get_mut("wallpaper_uncertain").unwrap() += 1,
        }
    }
    counts
}
