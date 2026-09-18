use super::*;

#[test]
fn preferences_are_opt_in_and_preserve_unknown_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("appearance.json");
    assert_eq!(load(&path).unwrap().preferences, Preferences::default());
    assert!(!path.exists());
    std::fs::write(&path, br#"{"version":1,"future":{"keep":true}}"#).unwrap();
    let preferences = Preferences {
        motion: true,
        reduce_transparency: true,
    };
    save(&path, preferences).unwrap();
    let document = load(&path).unwrap();
    assert_eq!(document.preferences, preferences);
    assert_eq!(document.extra["future"]["keep"], true);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn corrupt_future_and_oversized_settings_are_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("appearance.json");
    for bytes in [
        b"broken".to_vec(),
        br#"{"version":2,"motion":true}"#.to_vec(),
        br#"{"version":1,"motion":"true"}"#.to_vec(),
        vec![b' '; 16 * 1024 + 1],
    ] {
        std::fs::write(&path, &bytes).unwrap();
        assert!(save(&path, Preferences::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn symlinks_and_directories_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.json");
    let link = directory.path().join("appearance.json");
    save(&target, Preferences::default()).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        save(
            &link,
            Preferences {
                motion: true,
                reduce_transparency: false
            }
        )
        .is_err()
    );
    assert_eq!(load(&target).unwrap().preferences, Preferences::default());
    assert!(load(directory.path()).is_err());
}

#[test]
fn every_motion_gate_is_required() {
    let on = Preferences {
        motion: true,
        reduce_transparency: false,
    };
    assert!(should_animate(on, true, true, true, false, true));
    assert!(!should_animate(on, true, true, true, false, false));
    for mask in 0..32 {
        let motion = mask & 1 != 0;
        let reduced = mask & 2 != 0;
        let mapped = mask & 4 != 0;
        let active = mask & 8 != 0;
        let animations = mask & 16 != 0;
        for contrast in [false, true] {
            assert_eq!(
                should_animate(
                    Preferences {
                        motion,
                        reduce_transparency: reduced
                    },
                    mapped,
                    active,
                    animations,
                    contrast,
                    true
                ),
                motion && !reduced && mapped && active && animations && !contrast
            );
        }
    }
}

#[test]
fn curves_are_finite_bounded_periodic_and_slow() {
    for period in [12, 15, 14, 22] {
        for time in [i64::MIN, -1, 0, 1, 5_000_000, i64::MAX] {
            assert!((0.0..=1.0).contains(&wave(time, period)));
        }
        assert_eq!(wave(0, period), 0.0);
        assert_eq!(wave(period * 500_000, period), 1.0);
        assert_eq!(
            wave(123_456, period),
            wave(123_456 + period * 1_000_000, period)
        );
        for frame in 0..1000 {
            assert!(
                (wave(frame * 33_334, period) - wave((frame + 1) * 33_334, period)).abs() < 0.01
            );
        }
    }
}

pub(super) fn settle(ms: u64) {
    let context = glib::MainContext::default();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < until {
        for _ in 0..32 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn capture(widget: &impl IsA<gtk::Widget>, name: &str) {
    if let Some(directory) = std::env::var_os("MIRELAY_APPEARANCE_QA_DIR") {
        super::super::capture_widget(widget.as_ref(), &PathBuf::from(directory).join(name))
            .unwrap();
    }
}

#[test]
#[ignore = "requires an isolated graphical session with an attached viewer"]
fn native_background_stops_and_settings_save_without_touching_folders() {
    adw::init().unwrap();
    super::super::brand::register_resources().unwrap();
    super::super::brand::install_icons();
    super::super::install_css();
    let app = adw::Application::builder()
        .application_id("io.mirelay.AppearanceQA")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let (widgets, _) = super::super::build_widgets(&app);
    // Count foreground drawing through an actual child in the main content.
    // The existing Add Folder action remains present and usable.
    let foreground_paints = Rc::new(Cell::new(0u64));
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    let count = foreground_paints.clone();
    probe.set_draw_func(move |_, _, _, _| count.set(count.get() + 1));
    widgets.empty_page.set_child(gtk::Widget::NONE);
    let probe_layer = gtk::Overlay::new();
    probe_layer.set_child(Some(&widgets.empty_action));
    probe_layer.add_overlay(&probe);
    widgets.empty_page.set_child(Some(&probe_layer));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("appearance.json");
    let folder = directory.path().join("bridges.toml");
    std::fs::write(&folder, "unmodified folder registry fixture").unwrap();
    install(
        &widgets.window,
        &widgets.ambient,
        &widgets.appearance_button,
        &widgets.toast_overlay,
        path.clone(),
    );
    widgets.window.present();
    // Drive real proxy callbacks using only our private bus, never the host's
    // power policy. The dedicated provider test covers the remaining failures.
    let power_fixture = power::tests::Fixture::new();
    power_fixture.own(0, true);
    let weak = widgets.ambient.downgrade();
    *widgets.ambient.imp().power_monitor.borrow_mut() = Some(power_fixture.monitor(move |state| {
        if let Some(ambient) = weak.upgrade() {
            ambient.set_power_state(state);
        }
    }));
    settle(700);
    assert!(widgets.appearance_button.is_mapped());
    assert!(widgets.settings_button.is_mapped());
    let ambient = &widgets.ambient;
    assert!(ambient.imp().tick.borrow().is_none());
    assert_eq!(ambient.imp().nodes.borrow().len(), 6);
    let before = ambient.imp().draws.get();
    settle(700);
    assert_eq!(
        ambient.imp().draws.get(),
        before,
        "default-off background keeps redrawing"
    );
    capture(&widgets.window, "static.png");
    let settings = ambient.settings();
    let original = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(true);
    ambient.apply(Preferences {
        motion: true,
        reduce_transparency: false,
    });
    settle(1000);
    assert!(
        widgets.window.is_active(),
        "viewer must focus the test window to validate motion"
    );
    assert!(ambient.imp().elapsed.get() > 0);
    capture(&widgets.window, "motion.png");
    let before = ambient.imp().draws.get();
    let foreground_before = foreground_paints.get();
    let started = std::time::Instant::now();
    settle(1000);
    let seconds = started.elapsed().as_secs_f64();
    let delta = ambient.imp().draws.get() - before;
    assert!(
        delta > 0 && delta <= (seconds * 30.0).ceil() as u64 + 1,
        "expected bounded real redraws, observed {delta} over {seconds:.3}s"
    );
    assert!(foreground_before > 0, "foreground probe was never drawn");
    assert_eq!(
        foreground_paints.get(),
        foreground_before,
        "ambient motion invalidated foreground drawing"
    );
    println!(
        "appearance probe: moving_background_draws={delta}/{seconds:.3}s, foreground_draws=0, cached_gradient_nodes={}",
        ambient.imp().nodes.borrow().len()
    );
    power_fixture.profile(0, "power-saver", false);
    settle(250);
    assert_eq!(ambient.imp().power_state.get(), power::State::Saver);
    assert!(ambient.imp().tick.borrow().is_none());
    let paused = ambient.imp().elapsed.get();
    settle(300);
    assert_eq!(ambient.imp().elapsed.get(), paused);
    assert!(
        ambient.imp().preferences.get().motion,
        "system policy must not overwrite user preference"
    );
    power_fixture.profile(0, "balanced", false);
    settle(250);
    assert!(ambient.imp().tick.borrow().is_some());
    power_fixture.profile(0, "unknown-profile", false);
    settle(250);
    assert!(ambient.imp().tick.borrow().is_none());
    power_fixture.profile(0, "balanced", false);
    settle(250);
    assert!(ambient.imp().tick.borrow().is_some());
    println!(
        "power policy: saver stops real ticks, balanced resumes, unknown stops; saved motion preference unchanged"
    );
    settings.set_gtk_enable_animations(false);
    settle(200);
    assert!(ambient.imp().tick.borrow().is_none());
    let before = ambient.imp().draws.get();
    settle(600);
    assert_eq!(ambient.imp().draws.get(), before);
    println!(
        "appearance probe: system-animation-off background_draws=0/600ms; default-off background_draws=0/700ms"
    );
    settings.set_gtk_enable_animations(true);
    widgets.window.set_visible(false);
    settle(200);
    assert!(ambient.imp().tick.borrow().is_none());
    widgets.window.present();
    settle(300);
    widgets.appearance_button.emit_clicked();
    settle(300);
    assert!(
        ambient.imp().tick.borrow().is_none(),
        "modal Settings must suspend background motion"
    );
    let dialog = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<adw::Window>().ok())
        .find(|w| w.title().as_deref() == Some("Settings"))
        .unwrap();
    capture(&dialog, "settings.png");
    fn descendants(widget: &gtk::Widget, list: &mut Vec<gtk::Widget>) {
        list.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(w) = child {
            child = w.next_sibling();
            descendants(&w, list);
        }
    }
    let mut children = Vec::new();
    descendants(dialog.upcast_ref(), &mut children);
    let switches: Vec<gtk::Switch> = children
        .iter()
        .filter_map(|w| w.clone().downcast().ok())
        .collect();
    assert_eq!(switches.len(), 2);
    switches[1].set_active(true);
    let save_button = children
        .iter()
        .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
        .find(|w| w.label().as_deref() == Some("Save"))
        .unwrap();
    save_button.emit_clicked();
    settle(500);
    assert_eq!(
        load(&path).unwrap().preferences,
        Preferences {
            motion: true,
            reduce_transparency: true
        }
    );
    assert!(widgets.window.has_css_class("reduced-transparency"));
    capture(&widgets.window, "opaque.png");
    assert!(ambient.imp().tick.borrow().is_none());
    assert_eq!(
        std::fs::read_to_string(folder).unwrap(),
        "unmodified folder registry fixture"
    );
    let bad_path = directory.path().join("bad-appearance.json");
    std::fs::write(&bad_path, "invalid-json-retain-me").unwrap();
    let failed = settings_dialog(&widgets.window, ambient, bad_path.clone());
    failed.present();
    settle(250);
    let mut children = Vec::new();
    descendants(failed.upcast_ref(), &mut children);
    let retry = children
        .iter()
        .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
        .find(|w| w.label().as_deref() == Some("Save"))
        .unwrap();
    retry.emit_clicked();
    settle(500);
    assert!(retry.is_sensitive());
    assert!(
        children
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::Label>().ok())
            .any(|w| w.is_visible() && w.text().contains("Could not save or confirm"))
    );
    assert_eq!(
        std::fs::read_to_string(bad_path).unwrap(),
        "invalid-json-retain-me"
    );
    assert!(ambient.imp().preferences.get().reduce_transparency);
    capture(&failed, "save-error.png");
    failed.close();
    settle(200);
    settings.set_gtk_enable_animations(original);
    assert_eq!(power_fixture.writes.get(), 0);
    widgets.window.close();
    settle(200);
    assert!(ambient.imp().tick.borrow().is_none());
}
