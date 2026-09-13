//! Opt-in real GTK checks and bounded, reproducible album measurements.
use super::*;
use std::time::Instant;

fn pump(ms: u64) {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        for _ in 0..32 {
            if context.pending() {
                context.iteration(false);
            } else {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![root.clone()];
    let mut next = root.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        result.extend(widgets(&child));
    }
    result
}

fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
        .unwrap()
}

#[test]
#[ignore = "requires a graphical session; album virtualization, viewer, faults and performance"]
fn album_virtualization_viewer_and_performance() {
    adw::init().unwrap();
    install_css();
    let root = tempfile::Builder::new()
        .prefix("album-qa-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    let mut sources = Vec::new();
    for i in 0..32 {
        let width = if i % 2 == 0 { 640 } else { 360 };
        let image = Pixbuf::new(Colorspace::Rgb, false, 8, width, 480).unwrap();
        image.fill(((40 + i * 5) << 24) | ((180 - i * 3) << 16) | ((90 + i * 3) << 8) | 255);
        let bytes = image.save_to_bufferv("png", &[]).unwrap();
        let path = root.path().join(format!("sample-{i}.png"));
        std::fs::write(&path, &bytes).unwrap();
        sources.push((
            path,
            bytes.len() as u64,
            hex::encode(Sha256::digest(&bytes)),
        ));
    }
    let records = |count: usize| -> Vec<FileEntry> {
        (0..count)
            .map(|i| {
                let (path, size, hash) = &sources[i % sources.len()];
                FileEntry::Saved(DeliveryRecord {
                    id: format!("image-{i}"),
                    original_name: format!("Photo {i:05} — 相册.png"),
                    media_type: "image/png".into(),
                    sha256: hash.clone(),
                    size: *size,
                    stored_path: path.clone(),
                    delivery_status: DeliveryStatus::Acknowledged,
                    wallpaper_status: WallpaperStatus::NotApplicable,
                    wallpaper_attempts: 0,
                    delivery_error: None,
                    wallpaper_error: None,
                    source_created_at_unix: None,
                    received_at_unix: i as u64,
                    acknowledged_at_unix: Some(i as u64),
                    imported_at_unix: None,
                })
            })
            .collect()
    };
    let parent = adw::ApplicationWindow::builder()
        .default_width(1100)
        .default_height(760)
        .build();
    parent.add_css_class("mirelay");
    let album = grid::AlbumGrid::new(&parent);
    let overlay_preference = album.widget.is_overlay_scrolling();
    let solid_control = std::env::var_os("MIRELAY_ALBUM_QA_SOLID_SCROLLBARS").is_some();
    let overlay_control = std::env::var_os("MIRELAY_ALBUM_QA_OVERLAY_SCROLLBARS").is_some();
    assert!(
        !(solid_control && overlay_control),
        "Choose only one scrollbar control"
    );
    if solid_control || overlay_control {
        // Runs after the product's map handler, also on remap. Only the test
        // harness can bypass the renderer-aware policy for the old-path control.
        album.widget.connect_map(move |widget| {
            widget.set_overlay_scrolling(overlay_control);
        });
    }
    parent.set_content(Some(&album.widget));
    let requested_monitor = std::env::var("MIRELAY_ALBUM_QA_MONITOR").ok();
    if let Some(connector) = &requested_monitor {
        let monitors = gtk::gdk::Display::default().unwrap().monitors();
        let monitor = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
            .find(|monitor| monitor.connector().as_deref() == Some(connector.as_str()))
            .expect("Requested QA monitor is unavailable");
        parent.fullscreen_on_monitor(&monitor);
    }
    parent.present();
    pump(300);
    let expected_overlay = if solid_control || overlay_control {
        overlay_control
    } else {
        overlay_preference && !parent.renderer().unwrap().is::<gtk::gsk::CairoRenderer>()
    };
    assert_eq!(album.widget.is_overlay_scrolling(), expected_overlay);
    let actual_monitor = parent.surface().and_then(|surface| {
        surface
            .display()
            .monitor_at_surface(&surface)
            .and_then(|monitor| monitor.connector())
    });
    if let Some(connector) = &requested_monitor {
        assert_eq!(
            actual_monitor.as_deref(),
            Some(connector.as_str()),
            "Compositor must place the test on the requested monitor"
        );
        assert!(
            parent.is_fullscreen(),
            "Monitor-targeted test must enter fullscreen"
        );
    }
    println!(
        "ALBUM_ENV {}",
        serde_json::json!({
            "renderer": parent.renderer().map(|renderer| renderer.type_().name().to_owned()),
            "display": gtk::gdk::Display::default().map(|display| display.type_().name().to_owned()),
            "requested_renderer": std::env::var("GSK_RENDERER").unwrap_or_else(|_| "default".into()),
            "requested_backend": std::env::var("GDK_BACKEND").unwrap_or_else(|_| "default".into()),
            "monitor": actual_monitor.map(|connector| connector.to_string()),
            "width": parent.width(), "height": parent.height(), "scale_factor": parent.scale_factor(),
            "overlay_scrolling": album.widget.is_overlay_scrolling(),
        })
    );
    pump(100);
    let baseline = rss();
    for count in [1000, 10000] {
        let entries = records(count);
        let start = Instant::now();
        album.set_entries(&entries, root.path());
        let bind_ms = start.elapsed().as_secs_f64() * 1000.0;
        pump(750);
        assert_eq!(album.len(), count as u32);
        let bound = widgets(album.view.upcast_ref())
            .iter()
            .filter(|widget| widget.is::<gtk::Picture>())
            .count();
        assert!(
            bound > 0 && bound < 600,
            "Grid must virtualize: {bound} pictures for {count} entries"
        );
        let visible = widgets(album.view.upcast_ref())
            .iter()
            .filter(|widget| {
                widget
                    .downcast_ref::<gtk::Picture>()
                    .is_some_and(|picture| picture.is_mapped() && picture.paintable().is_some())
            })
            .count();
        assert!(visible > 0, "Must display real decoded pixels");
        let identity = album.item(0).unwrap();
        album.set_entries(&entries, root.path());
        assert_eq!(album.item(0).unwrap(), identity);
        println!(
            "ALBUM_MODEL {}",
            serde_json::json!({"entries": count, "set_entries_ms": bind_ms, "bound_pictures": bound, "mapped_loaded_pictures": visible, "rss_kib": rss(), "build": "debug", "distinct_source_images": 32})
        );
    }
    let ticks = Rc::new(RefCell::new(Vec::<i64>::new()));
    let frames = ticks.clone();
    let tick = album.view.add_tick_callback(move |_, clock| {
        frames.borrow_mut().push(clock.frame_time());
        glib::ControlFlow::Continue
    });
    let started = Instant::now();
    for step in 0..80 {
        let adjustment = album.widget.vadjustment();
        let maximum = (adjustment.upper() - adjustment.page_size()).max(0.0);
        adjustment.set_value(maximum * f64::from(step % 40) / 39.0);
        pump(20);
    }
    tick.remove();
    let times = ticks.borrow();
    let mut intervals: Vec<_> = times
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1000.0)
        .collect();
    intervals.sort_by(f64::total_cmp);
    assert!(!intervals.is_empty());
    println!(
        "ALBUM_SCROLL {}",
        serde_json::json!({"scroll_steps": 80, "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0, "frame_intervals": intervals.len(), "frame_interval_median_ms": intervals[intervals.len()/2], "frame_interval_p95_ms": intervals[(intervals.len()-1)*95/100], "rss_kib": rss(), "baseline_rss_kib": baseline, "scope": "GTK debug scrolling; backend/renderer/monitor in ALBUM_ENV; 10000 records referencing 32 distinct local PNGs, not a release FPS guarantee"})
    );
    drop(times);
    ticks.borrow_mut().clear();
    album.widget.vadjustment().set_value(0.0);
    pump(500);
    let frames = ticks.clone();
    let tick = album.view.add_tick_callback(move |_, clock| {
        frames.borrow_mut().push(clock.frame_time());
        glib::ControlFlow::Continue
    });
    for step in 0..120 {
        album.widget.vadjustment().set_value(f64::from(step * 48));
        pump(20);
    }
    tick.remove();
    let mut smooth: Vec<_> = ticks
        .borrow()
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1000.0)
        .collect();
    smooth.sort_by(f64::total_cmp);
    assert!(!smooth.is_empty());
    println!(
        "ALBUM_SMOOTH {}",
        serde_json::json!({"steps": 120, "pixels_per_step": 48, "frame_interval_median_ms": smooth[smooth.len()/2], "frame_interval_p95_ms": smooth[(smooth.len()-1)*95/100], "rss_kib": rss()})
    );
    pump(500);
    let loading = widgets(album.view.upcast_ref())
        .iter()
        .filter(|widget| {
            widget.downcast_ref::<gtk::Stack>().is_some_and(|stack| {
                stack.is_mapped() && stack.visible_child_name().as_deref() == Some("loading")
            })
        })
        .count();
    println!("ALBUM_PENDING {loading}");
    assert_eq!(
        loading, 0,
        "Visible thumbnail requests must settle before idle measurement"
    );
    let cpu_ticks = || {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    let hz: f64 = String::from_utf8(
        std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    let idle = glib::MainLoop::new(None, false);
    // Observe real paint signals without requesting frames, unlike a tick callback.
    // A completed preview queue does not mean GTK's scrollbar fade has finished.
    let clock = parent.frame_clock().unwrap();
    let paints = Rc::new(Cell::new(0u32));
    let observed = paints.clone();
    let paint_handler = clock.connect_after_paint(move |_| observed.set(observed.get() + 1));
    let scrollbar = album.widget.vscrollbar();
    let opacity_changes = Rc::new(Cell::new(0u32));
    let changes = opacity_changes.clone();
    let opacity_handler = scrollbar.connect_opacity_notify(move |_| changes.set(changes.get() + 1));
    let initial_opacity = scrollbar.opacity();
    let quit = idle.clone();
    // Include the entire delayed GTK fade, not a two-second window that may
    // arbitrarily catch only its beginning (or miss it altogether).
    glib::timeout_add_local_once(Duration::from_secs(4), move || quit.quit());
    let before = cpu_ticks();
    let polls_before = PREVIEW_POLLS.load(Ordering::Relaxed);
    let frames_before = ticks.borrow().len();
    let start = Instant::now();
    idle.run();
    assert_eq!(
        PREVIEW_POLLS.load(Ordering::Relaxed),
        polls_before,
        "Settled album must not keep polling preview jobs"
    );
    assert_eq!(
        ticks.borrow().len(),
        frames_before,
        "Measurement callback must stop before idle sampling"
    );
    println!(
        "ALBUM_POST_SCROLL {}",
        serde_json::json!({"seconds": start.elapsed().as_secs_f64(), "cpu_percent_one_core": (cpu_ticks() - before) as f64 / hz / start.elapsed().as_secs_f64() * 100.0, "rss_kib": rss(), "paints": paints.get(), "scrollbar_opacity_start": initial_opacity, "scrollbar_opacity_end": scrollbar.opacity(), "scrollbar_opacity_changes": opacity_changes.get(), "scope": "10000-record visible album; preview polling stopped, but GTK scrollbar fade may still run"})
    );
    if !expected_overlay {
        assert_eq!(opacity_changes.get(), 0, "Regular scrollbar must not fade");
        assert!(
            paints.get() <= 2,
            "Settled non-overlay album must not keep repainting"
        );
    }
    let settle = glib::MainLoop::new(None, false);
    let quit = settle.clone();
    glib::timeout_add_local_once(Duration::from_secs(10), move || quit.quit());
    settle.run();
    let quit = idle.clone();
    glib::timeout_add_local_once(Duration::from_secs(3), move || quit.quit());
    let before = cpu_ticks();
    let start = Instant::now();
    let late_paints = paints.get();
    let late_opacity_changes = opacity_changes.get();
    idle.run();
    println!(
        "ALBUM_SETTLED_IDLE {}",
        serde_json::json!({"seconds": start.elapsed().as_secs_f64(), "cpu_percent_one_core": (cpu_ticks() - before) as f64 / hz / start.elapsed().as_secs_f64() * 100.0, "rss_kib": rss(), "paints": paints.get() - late_paints, "scrollbar_opacity_changes": opacity_changes.get() - late_opacity_changes})
    );
    clock.disconnect(paint_handler);
    scrollbar.disconnect(opacity_handler);
    parent.set_content(Some(&gtk::Label::new(Some("Idle control"))));
    let quit = idle.clone();
    glib::timeout_add_local_once(Duration::from_secs(3), move || quit.quit());
    let before = cpu_ticks();
    let start = Instant::now();
    idle.run();
    println!(
        "ALBUM_IDLE_HIDDEN {}",
        serde_json::json!({"cpu_percent_one_core": (cpu_ticks() - before) as f64 / hz / start.elapsed().as_secs_f64() * 100.0})
    );
    parent.set_content(Some(&album.widget));
    pump(500);
    assert_eq!(
        album.widget.is_overlay_scrolling(),
        expected_overlay,
        "Renderer policy must survive album unmap/remap"
    );
    let pages = vec![
        grid::Photo {
            id: "first".into(),
            name: "Landscape.png".into(),
            source: Some(Source {
                root: root.path().to_owned(),
                relative: "sample-0.png".into(),
                size: sources[0].1,
                hash: sources[0].2.clone(),
            }),
        },
        grid::Photo {
            id: "missing".into(),
            name: "Missing.png".into(),
            source: None,
        },
    ];
    assert!(viewer::open(&parent, vec![], 0).is_none());
    assert!(viewer::open(&parent, pages.clone(), usize::MAX).is_none());
    let view = viewer::open(&parent, pages, 0).unwrap();
    pump(500);
    let buttons = widgets(view.upcast_ref());
    let controllers = view.observe_controllers();
    let key = (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .unwrap();
    assert_eq!(key.propagation_phase(), gtk::PropagationPhase::Capture);
    let button = |label: &str| {
        buttons
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
            .find(|button| button.tooltip_text().as_deref() == Some(label))
            .unwrap()
    };
    assert!(!button("Previous photo (Left)").is_sensitive());
    button("Zoom in (+)").emit_clicked();
    pump(80);
    assert!(
        widgets(view.upcast_ref())
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
            .any(|button| button.label().as_deref() == Some("Reset zoom"))
    );
    button("Fit image (0)").emit_clicked();
    button("Next photo (Right)").emit_clicked();
    pump(80);
    assert!(!button("Next photo (Right)").is_sensitive());
    assert!(
        widgets(view.upcast_ref())
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Stack>())
            .any(|stack| stack.visible_child_name().as_deref() == Some("missing"))
    );
    // Synthetic repeated activation must not index out of range, even when disabled.
    for _ in 0..50 {
        button("Next photo (Right)").emit_clicked();
    }
    button("Previous photo (Left)").emit_clicked();
    pump(150);
    assert!(key.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Right,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert!(!button("Next photo (Right)").is_sensitive());
    assert!(key.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Left,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert!(!button("Previous photo (Left)").is_sensitive());
    pump(150);
    if let Some(out) = std::env::var_os("MIRELAY_ALBUM_QA_DIR") {
        let out = PathBuf::from(out);
        assert!(out.is_dir());
        capture_widget(view.upcast_ref(), &out.join("viewer.png")).unwrap();
        capture_widget(parent.upcast_ref(), &out.join("grid.png")).unwrap();
    }
    view.close();
    let closed = view.downgrade();
    drop(buttons);
    drop(view);
    pump(100);
    assert!(
        closed.upgrade().is_none(),
        "Closed viewer must release its window"
    );
    for _ in 0..40 {
        album.clear();
        album.set_entries(&records(200), root.path());
    }
    album.clear();
    pump(200);
    assert_eq!(album.len(), 0);
    println!(
        "ALBUM_FINAL {}",
        serde_json::json!({"rss_kib": rss(), "clear_repopulate_cycles": 40, "remaining_model_entries": album.len()})
    );
    for (path, _, hash) in sources {
        assert_eq!(
            hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
            hash
        );
    }
    parent.close();
}

#[test]
#[ignore = "requires a graphical session; recycled results, unmap cancellation and viewer lifetime"]
fn album_preview_lifecycle_faults() {
    adw::init().unwrap();
    install_css();
    let root = tempfile::tempdir().unwrap();
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/brand/MiRelay-brand-kit-v1/icons/png/mirelay-512.png"
    ));
    std::fs::write(root.path().join("original.png"), bytes).unwrap();
    let source = Source {
        root: root.path().into(),
        relative: "original.png".into(),
        hash: hex::encode(Sha256::digest(bytes)),
        size: bytes.len() as u64,
    };
    let parent = adw::ApplicationWindow::builder()
        .default_width(640)
        .default_height(480)
        .build();
    let cell = Preview::new(256, 128, true);
    parent.set_content(Some(&cell.stack));
    parent.present();
    for _ in 0..20 {
        cell.bind(Some(source.clone()));
        pump(22); // Admit a request, then invalidate the cell before consuming its result.
        cell.bind(None);
        pump(50);
        assert_eq!(cell.stack.visible_child_name().as_deref(), Some("missing"));
        assert!(cell.job.borrow().is_none());
        assert!(
            cell.stack
                .child_by_name("image")
                .and_downcast::<gtk::Picture>()
                .unwrap()
                .paintable()
                .is_none()
        );
    }
    cell.bind(Some(source.clone()));
    parent.set_content(gtk::Widget::NONE);
    assert!(
        cell.job.borrow().is_none(),
        "Unmapping must remove the result timer"
    );
    parent.set_content(Some(&cell.stack));
    let until = Instant::now() + Duration::from_secs(5);
    while cell.stack.visible_child_name().as_deref() == Some("loading") && Instant::now() < until {
        pump(10);
    }
    assert_eq!(cell.stack.visible_child_name().as_deref(), Some("image"));
    let pages = vec![grid::Photo {
        id: "one".into(),
        name: "Original.png".into(),
        source: Some(source.clone()),
    }];
    for _ in 0..20 {
        let viewer = viewer::open(&parent, pages.clone(), 0).unwrap();
        let weak = viewer.downgrade();
        pump(22);
        viewer.close();
        drop(viewer);
        pump(10);
        assert!(
            weak.upgrade().is_none(),
            "Repeated viewer close leaked a window"
        );
    }
    // A cached preview must not resurrect a file that was removed in the meantime.
    std::fs::remove_file(root.path().join("original.png")).unwrap();
    cell.bind(Some(source));
    let until = Instant::now() + Duration::from_secs(5);
    while cell.stack.visible_child_name().as_deref() == Some("loading") && Instant::now() < until {
        pump(10);
    }
    assert_eq!(cell.stack.visible_child_name().as_deref(), Some("missing"));
    assert!(cell.job.borrow().is_none());
    parent.close();
    println!(
        "ALBUM_LIFECYCLE stale_results=20 unmap_remap=passed viewer_close=20 deleted_cached_source=passed"
    );
}

#[test]
#[ignore = "native decoder-only CPU control; no GTK window or scrolling"]
fn album_decoder_idle_control() {
    let root = tempfile::tempdir().unwrap();
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/brand/MiRelay-brand-kit-v1/icons/png/mirelay-512.png"
    ));
    let alive = Arc::new(());
    for index in 0..32 {
        let relative = format!("{index}.png");
        std::fs::write(root.path().join(&relative), bytes).unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        worker()
            .send(Request {
                source: Source {
                    root: root.path().into(),
                    relative,
                    hash: hex::encode(Sha256::digest(bytes)),
                    size: bytes.len() as u64,
                },
                edge: 256,
                alive: Arc::downgrade(&alive),
                reply: send,
            })
            .unwrap();
        assert!(
            receive
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_some()
        );
    }
    let hz: f64 = String::from_utf8(
        std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    let ticks = || {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    for index in 0..4 {
        let main_loop = glib::MainLoop::new(None, false);
        let quit = main_loop.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || quit.quit());
        let before = ticks();
        let start = Instant::now();
        main_loop.run();
        println!(
            "ALBUM_DECODER_IDLE {}",
            serde_json::json!({"sample": index, "cpu_percent_one_core": (ticks()-before) as f64 / hz / start.elapsed().as_secs_f64() * 100.0, "rss_kib": rss(), "gtk_windows": 0})
        );
    }
}
