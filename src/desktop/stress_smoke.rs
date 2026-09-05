//! Opt-in diagnostics, called only from the isolated file smoke-test path.
use std::time::Instant;

use anyhow::ensure;

use super::*;

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
        })
        .unwrap_or(0)
}

fn measure(label: &str, count: usize, repeats: usize, mut run: impl FnMut()) {
    run(); // Warm the allocation/cache paths; report subsequent synchronous work.
    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let started = Instant::now();
        run();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "QA_PERF {}",
        serde_json::json!({
            "operation": label, "items": count, "samples": repeats,
            "median_ms": samples[repeats / 2], "max_ms": samples[repeats - 1],
            "rss_kib": rss_kib(), "build": if cfg!(debug_assertions) { "debug" } else { "release" },
            "scope": "synchronous model/widget work; excludes subsequent frame rendering"
        })
    );
}

fn row_count(list: &gtk::ListBox) -> usize {
    let mut count = 0;
    let mut child = list.first_child();
    while let Some(widget) = child {
        count += 1;
        child = widget.next_sibling();
    }
    count
}

pub(super) fn run(ui: &DesktopUi) -> Result<()> {
    let original = ui.bridges.borrow().clone();
    let views = ui.file_views.borrow().clone();
    let activity = ui.file_activity.borrow().clone();
    let registry = ui.paths.registry.load()?;
    let selected = ui
        .selected_bridge()
        .context("stress preview needs a selected Folder")?;
    let id = selected.registration.id.clone();
    let template = selected
        .snapshot
        .as_ref()
        .and_then(|s| s.deliveries.first())
        .context("stress preview needs a sample file")?
        .clone();
    let baseline_rss = rss_kib();
    let result = (|| -> Result<()> {
        // Invalid actions and stale row indices must leave valid view state untouched.
        for action in [
            "file-sort",
            "file-filter",
            "file-kind",
            "folder-sort",
            "folder-filter",
        ] {
            let action = ui.widgets.window.lookup_action(action).unwrap();
            let before = action.state();
            for key in ["", "unknown", "../../../", "升序", "name-asc; rm"] {
                action.activate(Some(&key.to_variant()));
                ensure!(
                    action.state() == before,
                    "invalid action target changed state"
                );
            }
        }
        ui.select_bridge_at(-1);
        ui.select_bridge_at(i32::MAX);
        ensure!(ui.selected_bridge_id.borrow().as_deref() == Some(id.as_str()));
        ensure!(ui.begin_task("Stress guard check"));
        ensure!(
            !ui.begin_task("Duplicate receive"),
            "busy guard accepted a second task"
        );
        ensure!(!ui.widgets.sync_button.is_sensitive());
        ensure!(!ui.widgets.settings_button.is_sensitive());
        ui.finish_task();

        for count in [1_000, 10_000, 50_000] {
            let records: Vec<_> = (0..count)
                .map(|index| {
                    let mut record = template.clone();
                    let shuffled = (index * 7919) % count;
                    record.id = format!("stress-{index:06}");
                    record.original_name = format!("Document {shuffled:06} — 中文.txt");
                    record.size = shuffled as u64 * 4096;
                    record.received_at_unix = 1_700_000_000 + shuffled as u64;
                    record.delivery_status = DeliveryStatus::Acknowledged;
                    record.wallpaper_status = WallpaperStatus::NotApplicable;
                    record.delivery_error = None;
                    record.wallpaper_error = None;
                    record
                })
                .collect();
            for (label, sort) in [
                ("model-time-sort", FileSort::Newest),
                ("model-name-sort", FileSort::NameAsc),
            ] {
                measure(label, count, 5, || {
                    let result =
                        visible_files(&records, &[], sort, FileFilter::All, FileKind::All, "");
                    assert_eq!(result.len(), count);
                    std::hint::black_box(result);
                });
            }
            {
                let mut bridges = ui.bridges.borrow_mut();
                let snapshot = bridges
                    .iter_mut()
                    .find(|b| b.registration.id == id)
                    .and_then(|b| b.snapshot.as_mut())
                    .unwrap();
                snapshot.deliveries = Arc::new(records.into());
            }
            ui.file_views
                .borrow_mut()
                .insert(id.clone(), FileViewOptions::default());
            ui.file_activity.borrow_mut().clear();
            let sort = ui.widgets.window.lookup_action("file-sort").unwrap();
            measure("gtk-name-sort-first-page", count, 5, || {
                sort.activate(Some(&"name-asc".to_variant()));
                assert_eq!(row_count(&ui.widgets.delivery_list), FILE_PAGE_SIZE);
            });
            measure("gtk-alternating-sort-first-page", count, 5, || {
                let next = if ui.file_views.borrow()[&id].sort == FileSort::NameAsc {
                    "name-desc"
                } else {
                    "name-asc"
                };
                sort.activate(Some(&next.to_variant()));
                assert_eq!(row_count(&ui.widgets.delivery_list), FILE_PAGE_SIZE);
            });
            measure("gtk-search-miss", count, 5, || {
                // Set view text through the same handler path, even on repeated queries.
                ui.change_file_view(true, |view| {
                    view.query = "not-present-in-this-fixture".into()
                });
                assert_eq!(row_count(&ui.widgets.delivery_list), 0);
            });
            ui.change_file_view(true, |view| view.query.clear());
            let mut phase = SyncPhase::Downloading;
            measure("gtk-changing-phase-and-flush", count, 5, || {
                phase = if phase == SyncPhase::Downloading {
                    SyncPhase::Verifying
                } else {
                    SyncPhase::Downloading
                };
                ui.handle_file_progress(
                    &id,
                    SyncEvent::FileActive {
                        id: "stress-changing-phase".into(),
                        original_name: "Changing phase.txt".into(),
                        media_type: "text/plain".into(),
                        size: 42,
                        phase,
                    },
                );
            });
            ui.file_activity.borrow_mut().clear();
            if count == 10_000 {
                let before_cycles = rss_kib();
                measure("gtk-alternating-sort-100-cycles", count, 100, || {
                    let next = if ui.file_views.borrow()[&id].sort == FileSort::NameAsc {
                        "name-desc"
                    } else {
                        "name-asc"
                    };
                    sort.activate(Some(&next.to_variant()));
                });
                println!(
                    "QA_MEMORY {}",
                    serde_json::json!({
                        "scenario": "100-sort-cycles", "before_kib": before_cycles,
                        "after_kib": rss_kib(), "note": "RSS is not a leak proof; allocator/GTK caches included"
                    })
                );
                // 2,048 phase updates for one active file, flushed in 32 batches.
                measure("gtk-64-progress-events-and-flush", count, 32, || {
                    ui.processing_worker_batch.set(true);
                    for _ in 0..64 {
                        ui.handle_file_progress(
                            &id,
                            SyncEvent::FileActive {
                                id: "stress-live".into(),
                                original_name: "Live transfer.txt".into(),
                                media_type: "text/plain".into(),
                                size: 42,
                                phase: SyncPhase::Downloading,
                            },
                        );
                    }
                    ui.processing_worker_batch.set(false);
                    ui.flush_file_render();
                });
                ensure!(
                    ui.file_activity.borrow()[&id].len() == 1,
                    "progress events duplicated a live row"
                );
                ui.file_activity.borrow_mut().clear();
            }
        }
        for count in [100, 1_000] {
            let folders: Vec<_> = (0..count)
                .map(|index| {
                    let mut view = selected.clone();
                    if index != 0 {
                        view.registration.id = format!("stress-folder-{index}");
                    }
                    view.registration.name = format!("Folder {:04}", (index * 7919) % count);
                    view.snapshot.as_mut().unwrap().deliveries = Arc::new(Vec::new().into());
                    view
                })
                .collect();
            ui.bridges.replace(folders);
            measure("gtk-folder-list-render", count, 5, || {
                ui.render_bridge_list()
            });
            ensure!(row_count(&ui.widgets.bridge_list) == count);
        }
        ensure!(
            ui.paths.registry.load()? == registry,
            "stress controls modified registry"
        );
        println!("QA_GUARDS invalid-targets=25 stale-indices=2 duplicate-task=rejected");
        Ok(())
    })();
    ui.bridges.replace(original);
    ui.file_views.replace(views);
    ui.file_activity.replace(activity);
    ui.processing_worker_batch.set(false);
    ui.finish_task();
    ui.render_bridge_list();
    println!(
        "QA_MEMORY {}",
        serde_json::json!({
            "scenario": "restored-original-fixture", "before_kib": baseline_rss, "after_kib": rss_kib()
        })
    );
    result
}
