use super::*;
use std::sync::OnceLock;

#[cfg(test)]
const ICON: &str = "/io/mirelay/Desktop/icons/128x128/apps/io.mirelay.Desktop.png";
const WORDMARK: &str = "/io/mirelay/Desktop/brand/wordmark.png";
const SIDEBAR_WORDMARK: &str = "/io/mirelay/Desktop/brand/wordmark-transparent.png";

pub(super) fn register_resources() -> Result<()> {
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    match REGISTERED.get_or_init(|| {
        gio::resources_register_include!("mirelay.gresource").map_err(|error| error.to_string())
    }) {
        Ok(()) => Ok(()),
        Err(error) => bail!("Could not load MiRelay brand resources: {error}"),
    }
}

pub(super) fn install_icons() {
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/io/mirelay/Desktop/icons");
    }
    gtk::Window::set_default_icon_name(APPLICATION_ID);
}

pub(super) fn about_button(parent: &adw::ApplicationWindow) -> gtk::Button {
    let image = gtk::Picture::new();
    // This offline-derived asset only removes the exterior canvas. No redraw,
    // runtime pixel processing, cropping, or tint; About retains the original.
    if let Ok(pixbuf) =
        gtk::gdk_pixbuf::Pixbuf::from_resource_at_scale(SIDEBAR_WORDMARK, 112, 70, true)
    {
        image.set_pixbuf(Some(&pixbuf));
    } else {
        image.set_resource(Some(SIDEBAR_WORDMARK));
    }
    image.set_keep_aspect_ratio(true);
    image.set_can_shrink(true);
    image.set_alternative_text(Some("MiRelay — About"));
    let button = gtk::Button::builder()
        .child(&image)
        .tooltip_text("About MiRelay")
        .valign(gtk::Align::Center)
        .build();
    button.set_widget_name("brand-about");
    button.add_css_class("flat");
    button.add_css_class("brand-button");
    let parent = parent.downgrade();
    button.connect_clicked(move |_| {
        if let Some(parent) = parent.upgrade() {
            about_window(&parent).present();
        }
    });
    button
}

pub(super) fn folder_icon(kind: crate::bridge_registry::FolderKind) -> &'static str {
    if kind.is_general() {
        "relay-folder-symbolic"
    } else {
        "relay-photos-symbolic"
    }
}

fn about_window(parent: &adw::ApplicationWindow) -> adw::Window {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(false)
        .destroy_with_parent(true)
        .title("About MiRelay")
        .default_width(480)
        .resizable(false)
        .build();
    dialog.add_css_class("mirelay");
    let keys = gtk::EventControllerKey::new();
    let weak_dialog = dialog.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(keys);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    // The approved artwork is opaque. Keep the entire canvas and its original
    // aspect ratio on an explicit white surface, including in dark mode.
    let wordmark = gtk::Picture::for_resource(WORDMARK);
    wordmark.set_keep_aspect_ratio(true);
    wordmark.set_can_shrink(true);
    wordmark.set_size_request(400, 250);
    wordmark.add_css_class("brand-canvas");
    wordmark.set_alternative_text(Some("MiRelay — Send · Receive — by MiiiKuuu"));
    content.append(&wordmark);
    let version = gtk::Label::new(Some(concat!("MiRelay ", env!("CARGO_PKG_VERSION"))));
    version.add_css_class("title-2");
    content.append(&version);
    content.append(&gtk::Label::new(Some("Created by MiiiKuuu")));
    let description = gtk::Label::new(Some("Your files. Your devices. Your relay."));
    description.set_wrap(true);
    description.add_css_class("dim-label");
    content.append(&description);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::Center);
    let weak_dialog = dialog.downgrade();
    close.connect_clicked(move |_| {
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.close();
        }
    });
    content.append(&close);
    dialog.set_content(Some(&content));
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a graphical session; verifies branded About in both themes"]
    fn gtk_about_opens_with_complete_artwork_in_both_themes() {
        adw::init().unwrap();
        register_resources().unwrap();
        install_icons();
        super::super::install_css();
        let display = gtk::gdk::Display::default().unwrap();
        assert!(gtk::IconTheme::for_display(&display).has_icon(APPLICATION_ID));
        let parent = adw::ApplicationWindow::builder()
            .default_width(560)
            .default_height(180)
            .build();
        let button = about_button(&parent);
        let preview = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        preview.set_margin_start(16);
        preview.set_margin_end(16);
        preview.append(&button);
        for size in [16, 32, 48, 64, 128] {
            // Verify the new embedded pixels even when an older hicolor icon
            // is installed on the test host and wins named-theme lookup.
            let icon = gtk::Image::from_resource(&format!(
                "/io/mirelay/Desktop/icons/{size}x{size}/apps/io.mirelay.Desktop.png"
            ));
            icon.set_pixel_size(size);
            preview.append(&icon);
        }
        parent.set_content(Some(&preview));
        parent.present();
        let context = glib::MainContext::default();
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        settle();
        button.emit_clicked();
        settle();
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<adw::Window>().ok())
            .find(|window| window.title().as_deref() == Some("About MiRelay"))
            .expect("brand button opens About");
        assert!(!dialog.is_modal(), "About must not block the Folder window");
        assert!(dialog.must_destroy_with_parent());
        let content = dialog.content().unwrap().downcast::<gtk::Box>().unwrap();
        let picture = content
            .first_child()
            .unwrap()
            .downcast::<gtk::Picture>()
            .unwrap();
        assert!(picture.is_keep_aspect_ratio());
        let paintable = picture.paintable().unwrap();
        assert_eq!(
            (paintable.intrinsic_width(), paintable.intrinsic_height()),
            (1586, 992)
        );
        let style = adw::StyleManager::default();
        let original = style.color_scheme();
        for (name, scheme) in [
            ("light", adw::ColorScheme::ForceLight),
            ("dark", adw::ColorScheme::ForceDark),
        ] {
            style.set_color_scheme(scheme);
            settle();
            assert!(picture.is_mapped());
            if let Some(directory) = std::env::var_os("MIRELAY_BRAND_TEST_SCREENSHOTS") {
                let directory = PathBuf::from(directory);
                assert!(directory.is_absolute() && directory.is_dir());
                super::super::capture_widget(
                    parent.upcast_ref(),
                    &directory.join(format!("launcher-{name}.png")),
                )
                .unwrap();
                let path = directory.join(format!("about-{name}.png"));
                assert!(!path.exists());
                super::super::capture_widget(dialog.upcast_ref(), &path).unwrap();
                let pixels = gtk::gdk_pixbuf::Pixbuf::from_file(&path).unwrap();
                let bounds = picture.compute_bounds(&dialog).unwrap();
                let bytes = pixels.read_pixel_bytes();
                let stride = pixels.rowstride() as usize;
                let channels = pixels.n_channels() as usize;
                let mut ink = 0;
                for y in bounds.y().ceil() as usize..(bounds.y() + bounds.height()).floor() as usize
                {
                    for x in
                        bounds.x().ceil() as usize..(bounds.x() + bounds.width()).floor() as usize
                    {
                        let offset = y * stride + x * channels;
                        if bytes[offset..offset + 3]
                            .iter()
                            .all(|channel| *channel < 160)
                        {
                            ink += 1;
                        }
                    }
                }
                assert!(
                    ink > 1000,
                    "{name} screenshot must contain artwork, not an empty white canvas"
                );
            }
        }
        style.set_color_scheme(original);
        content
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        settle();
        assert!(!dialog.is_visible());
        parent.close();
    }

    #[test]
    fn embedded_brand_assets_are_available_without_the_source_tree() {
        register_resources().unwrap();
        register_resources().unwrap(); // Repeated initialization is harmless.
        let icon = gio::resources_lookup_data(ICON, gio::ResourceLookupFlags::NONE).unwrap();
        assert_eq!(
            icon.as_ref(),
            include_bytes!("../../assets/ui/v2/brand/icons/mirelay-128.png")
        );
        for size in [16, 24, 32, 48, 64, 96, 128, 192, 256, 512, 1024] {
            let path =
                format!("/io/mirelay/Desktop/icons/{size}x{size}/apps/io.mirelay.Desktop.png");
            let pixels = gtk::gdk_pixbuf::Pixbuf::from_resource(&path).unwrap();
            assert_eq!((pixels.width(), pixels.height()), (size, size));
            assert!(pixels.has_alpha(), "launcher icon {size} must have alpha");
            assert_eq!(pixels.read_pixel_bytes()[3], 0, "launcher canvas {size}");
        }
        let wordmark =
            gio::resources_lookup_data(WORDMARK, gio::ResourceLookupFlags::NONE).unwrap();
        assert_eq!(
            wordmark.as_ref(),
            include_bytes!("../../assets/brand/MiRelay-brand-kit-v1/wordmark/mirelay-wordmark.png")
        );
        let sidebar =
            gio::resources_lookup_data(SIDEBAR_WORDMARK, gio::ResourceLookupFlags::NONE).unwrap();
        assert_eq!(
            sidebar.as_ref(),
            include_bytes!("../../assets/ui/v2/brand/mirelay-wordmark-transparent.png")
        );
        let pixels = gtk::gdk_pixbuf::Pixbuf::from_resource(SIDEBAR_WORDMARK).unwrap();
        assert!(pixels.has_alpha());
        assert_eq!((pixels.width(), pixels.height()), (1586, 992));
        assert_eq!(
            pixels.read_pixel_bytes()[3],
            0,
            "canvas corner is transparent"
        );
    }
}
