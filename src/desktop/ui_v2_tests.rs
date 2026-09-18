use super::*;

#[test]
#[ignore = "requires an isolated graphical session; connection lifecycle UI regression"]
fn native_connection_states_gate_receive_and_update_empty_copy() {
    use crate::config::ConnectionState;
    adw::init().unwrap();
    brand::register_resources().unwrap();
    brand::install_icons();
    let application = adw::Application::builder()
        .application_id("io.mirelay.ConnectionUiTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(gio::Cancellable::NONE).unwrap();
    let (widgets, _) = build_widgets(&application);
    let root = tempfile::tempdir().unwrap();
    let paths = resolve_desktop_paths(None, Some(root.path().join("bridges.toml"))).unwrap();
    let (sender, receiver) = mpsc::channel();
    let ui = DesktopUi {
        paths,
        bridges: RefCell::new(Vec::new()),
        bridge_ids: RefCell::new(Vec::new()),
        selected_bridge_id: RefCell::new(Some("fixture".into())),
        folder_sort: Cell::new(FolderSort::default()),
        folder_filter: Cell::new(FolderFilter::default()),
        unread_folders: RefCell::new(HashSet::new()),
        sync_failed_folders: RefCell::new(HashSet::new()),
        rendering_list: Cell::new(false),
        file_views: RefCell::new(HashMap::new()),
        file_activity: RefCell::new(HashMap::new()),
        rendering_file_controls: Cell::new(false),
        processing_worker_batch: Cell::new(false),
        file_render_pending: Cell::new(false),
        rendered_files: RefCell::new(Vec::new()),
        rendered_context: RefCell::new(None),
        tokens: RefCell::new(HashMap::new()),
        busy: Cell::new(false),
        editor_open: Cell::new(false),
        sender,
        widgets,
    };
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(root.path().join("data")),
        library_dir: Some(root.path().join("originals")),
        ..Default::default()
    })
    .unwrap();
    // End with Connected to prove that selecting another active Folder restores controls.
    for state in [
        ConnectionState::Connected,
        ConnectionState::ExitPending,
        ConnectionState::ExitLocalCleaned,
        ConnectionState::Exited,
        ConnectionState::DisconnectPending,
        ConnectionState::Disconnected,
        ConnectionState::Connected,
    ] {
        config.connection_state = state;
        let snapshot = snapshot_from_config(config.clone()).unwrap();
        ui.bridges.replace(vec![BridgeView {
            registration: BridgeRegistration {
                kind: Default::default(),
                id: "fixture".into(),
                name: "Fixture".into(),
                config_path: root.path().join("fixture.toml"),
                auto_receive: false,
            },
            snapshot: Some(snapshot),
            error: None,
        }]);
        ui.render_current_bridge();
        assert_eq!(ui.widgets.bridge_state.text(), connection::status(state));
        assert_eq!(ui.widgets.sync_button.is_sensitive(), state.is_connected());
        assert!(ui.widgets.open_library_button.is_sensitive());
        assert_eq!(
            ui.widgets.activity_empty.text(),
            connection::empty_message(state, false)
        );
        assert_eq!(
            ui.widgets.activity_empty_title.text(),
            if state.is_connected() {
                "No transfers yet"
            } else {
                connection::status(state)
            }
        );
        // Filters trigger a second render and must not restore Ready or Receive.
        ui.change_file_view(true, |view| view.query = "missing".into());
        assert_eq!(ui.widgets.bridge_state.text(), connection::status(state));
        if !state.is_connected() {
            ui.start_sync();
            assert!(!ui.busy.get());
            assert!(receiver.try_recv().is_err());
        }
        ui.busy.set(true);
        ui.update_action_sensitivity();
        assert!(!ui.widgets.sync_button.is_sensitive());
        ui.busy.set(false);
    }
    let ui = Rc::new(ui);
    let registration = ui.bridges.borrow()[0].registration.clone();
    let editor = adw::Window::builder()
        .transient_for(&ui.widgets.window)
        .build();
    for state in [ConnectionState::Exited, ConnectionState::Connected] {
        config.connection_state = state;
        config.save(&registration.config_path, true).unwrap();
        confirm_remove_bridge(&ui, &editor, registration.clone());
        let dialogs = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::MessageDialog>().ok())
            .filter(|dialog| dialog.is_visible())
            .collect::<Vec<_>>();
        assert_eq!(dialogs.len(), 1);
        assert_eq!(
            dialogs[0].secondary_text().as_deref(),
            Some(connection::removal_message(state))
        );
        dialogs[0].response(gtk::ResponseType::Cancel);
        assert!(
            registration.config_path.exists(),
            "Cancel must keep the configuration"
        );
    }
    editor.close();
    ui.widgets.window.close();
}

fn settle() {
    let context = glib::MainContext::default();
    let until = std::time::Instant::now() + Duration::from_millis(200);
    while std::time::Instant::now() < until {
        for _ in 0..32 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires an isolated graphical session; native v2 layout and resource checks"]
fn native_v2_preserves_details_and_category_switches() {
    adw::init().unwrap();
    brand::register_resources().unwrap();
    brand::install_icons();
    let provider = gtk::CssProvider::new();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let messages = errors.clone();
    provider
        .connect_parsing_error(move |_, _, error| messages.borrow_mut().push(error.to_string()));
    load_desktop_css(&provider, false);
    load_desktop_css(&provider, true);
    assert!(
        errors.borrow().is_empty(),
        "CSS diagnostics: {:?}",
        errors.borrow()
    );
    install_css();
    let application = adw::Application::builder()
        .application_id("io.mirelay.UiV2Test")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(gio::Cancellable::NONE).unwrap();
    let (widgets, _) = build_widgets(&application);
    widgets.window.present();
    settle();
    let header = widgets
        .settings_button
        .ancestor(adw::HeaderBar::static_type())
        .unwrap()
        .downcast::<adw::HeaderBar>()
        .unwrap();
    let title_slot = header.title_widget().unwrap();
    assert!(title_slot.is::<gtk::Box>());
    assert!(
        title_slot.first_child().is_none(),
        "no duplicate centered Folder title"
    );
    assert_eq!(widgets.window.title().as_deref(), Some("MiRelay"));
    let icons = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap());
    for icon in [
        "relay-folder-symbolic",
        "relay-photos-symbolic",
        "relay-link-symbolic",
        "relay-add-symbolic",
        "relay-settings-symbolic",
        "relay-filter-symbolic",
        "sidebar-show-symbolic",
        "window-close-symbolic",
    ] {
        assert!(icons.has_icon(icon), "missing {icon}");
    }
    assert!(widgets.add_bridge_button.is_mapped());
    assert!(widgets.settings_button.is_mapped());
    assert!(widgets.sync_button.is_mapped());
    let first = gtk::Label::new(Some("Photos · fixture"));
    let second = gtk::Label::new(Some("Documents · fixture"));
    widgets.bridge_list.append(&first);
    widgets.bridge_list.append(&second);
    let first_row = widgets.bridge_list.row_at_index(0).unwrap();
    let second_row = widgets.bridge_list.row_at_index(1).unwrap();
    widgets.bridge_list.select_row(Some(&first_row));
    widgets.folder_search.set_text("retained query");
    for width in [1000, 820, 640, 560, 820, 560, 1000] {
        widgets.window.set_default_size(width, 680);
        for _ in 0..4 {
            settle();
        }
        let navigation = &widgets.navigation;
        let narrow = width < 800;
        assert!(
            navigation.view.flap().unwrap().width() <= 320,
            "sidebar expanded beyond its bound"
        );
        assert_eq!(navigation.view.is_folded(), narrow, "width {width}");
        assert_eq!(navigation.toggle.is_visible(), narrow);
        assert!(
            widgets.window.width() <= width,
            "window refused width {width}: {}",
            widgets.window.width()
        );
        if narrow {
            assert!(!navigation.view.reveals_flap());
            navigation.toggle.set_active(true);
            for _ in 0..3 {
                settle();
            }
            for widget in [
                widgets.add_bridge_button.upcast_ref::<gtk::Widget>(),
                widgets.appearance_button.upcast_ref(),
                widgets.folder_sort_button.upcast_ref(),
                widgets.folder_filter_button.upcast_ref(),
                widgets.bridge_list.upcast_ref(),
            ] {
                assert!(widget.is_mapped(), "Folder action unavailable in drawer");
            }
            // A background refresh/selection must not unexpectedly close it.
            widgets.bridge_list.select_row(Some(&second_row));
            assert!(navigation.view.reveals_flap());
            if let Some(directory) = std::env::var_os("MIRELAY_APPEARANCE_QA_DIR") {
                capture_widget(
                    widgets.window.upcast_ref(),
                    &PathBuf::from(directory).join(format!("drawer-{width}.png")),
                )
                .unwrap();
            }
            widgets
                .bridge_list
                .emit_by_name::<()>("row-activated", &[&second_row]);
            for _ in 0..3 {
                settle();
            }
            assert!(!navigation.view.reveals_flap());
            assert!(!navigation.toggle.is_active());
            navigation.toggle.set_active(true);
            for _ in 0..3 {
                settle();
            }
            assert!(navigation.close.is_mapped());
            navigation.close.emit_clicked();
            for _ in 0..3 {
                settle();
            }
            assert!(!navigation.view.reveals_flap());
        } else {
            assert!(navigation.view.reveals_flap());
            assert!(widgets.add_bridge_button.is_mapped());
            let handle = navigation.view.separator().unwrap();
            let controllers = handle.observe_controllers();
            let drag = (0..controllers.n_items())
                .filter_map(|i| {
                    controllers
                        .item(i)
                        .and_then(|v| v.downcast::<gtk::GestureDrag>().ok())
                })
                .next()
                .unwrap();
            drag.emit_by_name::<()>("drag-begin", &[&0.0f64, &0.0f64]);
            drag.emit_by_name::<()>("drag-update", &[&1000.0f64, &0.0f64]);
            settle();
            assert!(navigation.view.flap().unwrap().width_request() <= 320);
            assert!(
                !navigation.view.is_folded(),
                "resizing must preserve room for content"
            );
            drag.emit_by_name::<()>("drag-update", &[&-1000.0f64, &0.0f64]);
            settle();
            assert_eq!(navigation.view.flap().unwrap().width_request(), 208);
        }
        assert_eq!(widgets.folder_search.text(), "retained query");
        assert!(widgets.settings_button.is_mapped());
        assert!(widgets.sync_button.is_mapped());
        for photos in [false, true, false, true, false] {
            widgets.album_layout.apply(&widgets, photos);
            settle();
            let details = &widgets.album_layout.details;
            assert!(details.is_mapped());
            details.popup();
            settle();
            for value in [
                &widgets.source_value,
                &widgets.folder_value,
                &widgets.device_value,
                &widgets.size_limit_value,
                &widgets.auto_receive_value,
                &widgets.wallpaper_value,
            ] {
                assert!(
                    value.is_mapped(),
                    "Folder property lost after category switch"
                );
            }
            details.popdown();
            assert_eq!(widgets.photo_grid.view.min_columns(), 3);
            assert_eq!(widgets.photo_grid.view.max_columns(), 3);
            assert_eq!(widgets.photo_grid.widget.is_visible(), photos);
        }
        settle();
        if let Some(directory) = std::env::var_os("MIRELAY_APPEARANCE_QA_DIR") {
            capture_widget(
                widgets.window.upcast_ref(),
                &PathBuf::from(directory).join(format!("content-{width}.png")),
            )
            .unwrap();
        }
    }
    let style = adw::StyleManager::default();
    let original_scheme = style.color_scheme();
    for (name, scheme) in [
        ("light", adw::ColorScheme::ForceLight),
        ("dark", adw::ColorScheme::ForceDark),
    ] {
        style.set_color_scheme(scheme);
        for _ in 0..3 {
            settle();
        }
        if let Some(directory) = std::env::var_os("MIRELAY_APPEARANCE_QA_DIR") {
            capture_widget(
                widgets.window.upcast_ref(),
                &PathBuf::from(directory).join(format!("brand-{name}.png")),
            )
            .unwrap();
        }
    }
    style.set_color_scheme(original_scheme);
    println!(
        "adaptive navigation: 1000/820/640/560 px, repeated folding, Folder controls, activation, retained search and General/Photos details passed"
    );
    widgets.window.close();
    settle();
}
