//! Opt-in offscreen hardware-rendering cost sample, not display FPS acceptance.
use super::*;
use gtk::subclass::prelude::WidgetImpl;

fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[test]
#[ignore = "requires an owned hardware-GL Wayland compositor; includes GPU readback, not screen FPS"]
fn offscreen_hardware_backdrop_cost() {
    adw::init().unwrap();
    super::super::brand::register_resources().unwrap();
    super::super::brand::install_icons();
    super::super::install_css();
    let app = adw::Application::builder()
        .application_id("io.mirelay.GpuQA")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let (widgets, _) = super::super::build_widgets(&app);
    widgets.window.present();
    super::tests::settle(700);
    let renderer = widgets.window.renderer().unwrap();
    assert!(
        renderer.type_().name().contains("GLRenderer"),
        "not a GL renderer: {}",
        renderer.type_().name()
    );
    // The launcher records GDK's actual GL device diagnostics, separately from
    // the compositor's device. Keep the crate's forbid(unsafe_code) invariant.
    println!("GTK renderer: {}", renderer.type_().name());
    let ambient = &widgets.ambient;
    let before = ambient.imp().draws.get();
    let cpu_start = cpu_ticks();
    let idle_start = std::time::Instant::now();
    let idle = glib::MainLoop::new(None, false);
    let done = idle.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || done.quit());
    idle.run();
    let idle_cpu = cpu_ticks() - cpu_start;
    assert_eq!(ambient.imp().draws.get(), before);
    assert!(ambient.imp().tick.borrow().is_none());
    println!(
        "static idle: wall={:.3}s process_cpu_ticks={idle_cpu} background_draws=0",
        idle_start.elapsed().as_secs_f64()
    );
    for (width, height) in [(1000, 680), (560, 680)] {
        ambient.allocate(width, height, -1, None);
        let viewport = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let mut samples = Vec::new();
        let mut first_frame = None;
        let cpu_start = cpu_ticks();
        for frame in 0..65 {
            ambient.imp().elapsed.set(frame * 33_334);
            let start = std::time::Instant::now();
            let snapshot = gtk::Snapshot::new();
            ambient.imp().snapshot(&snapshot);
            let node = snapshot.to_node().unwrap();
            let texture = renderer.render_texture(&node, Some(&viewport));
            // Explicit readback completes GPU work; record its cost honestly.
            texture.download(&mut pixels, width as usize * 4);
            if frame >= 5 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            if frame == 5 {
                first_frame = Some(pixels.clone());
            }
        }
        samples.sort_by(f64::total_cmp);
        assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
        assert_ne!(
            first_frame.unwrap(),
            pixels,
            "motion samples reused a static image"
        );
        assert_eq!(ambient.imp().nodes.borrow().len(), 6);
        println!(
            "offscreen {width}x{height}: samples=60 warmup=5 p50_ms={:.3} p95_ms={:.3} max_ms={:.3} process_cpu_ticks_including_warmup={}; snapshot+GPU+readback, not display FPS",
            samples[30],
            samples[56],
            samples[59],
            cpu_ticks() - cpu_start
        );
    }
    widgets.window.close();
    super::tests::settle(200);
}
