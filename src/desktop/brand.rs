use super::*;
use std::sync::OnceLock;

const ICON: &str = "/io/mirelay/Desktop/icons/128x128/apps/io.mirelay.Desktop.png";
const WORDMARK: &str = "/io/mirelay/Desktop/brand/wordmark.png";

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
    let image = gtk::Image::from_resource(ICON);
    image.set_pixel_size(32);
    let button = gtk::Button::builder()
        .child(&image)
        .tooltip_text("About MiRelay")
        .valign(gtk::Align::Center)
        .build();
    button.set_widget_name("brand-about");
    button.add_css_class("brand-button");
    let parent = parent.downgrade();
    button.connect_clicked(move |_| {
        if let Some(parent) = parent.upgrade() {
            about_window(&parent).present();
        }
    });
    button
}

fn about_window(parent: &adw::ApplicationWindow) -> adw::Window {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("About MiRelay")
        .default_width(480)
        .resizable(false)
        .build();
    dialog.add_css_class("mirelay");
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
            .default_width(400)
            .default_height(100)
            .build();
        let button = about_button(&parent);
        parent.set_content(Some(&button));
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
        assert!(dialog.is_modal());
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
            include_bytes!("../../assets/brand/MiRelay-brand-kit-v1/icons/png/mirelay-128.png")
        );
        let wordmark =
            gio::resources_lookup_data(WORDMARK, gio::ResourceLookupFlags::NONE).unwrap();
        assert_eq!(
            wordmark.as_ref(),
            include_bytes!("../../assets/brand/MiRelay-brand-kit-v1/wordmark/mirelay-wordmark.png")
        );
    }
}
