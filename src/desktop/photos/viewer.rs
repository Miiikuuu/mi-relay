use super::grid::Photo;
use super::*;

struct Viewer {
    window: glib::WeakRef<adw::Window>,
    pages: Vec<Photo>,
    index: Cell<usize>,
    zoom: Cell<f64>,
    scroll: gtk::ScrolledWindow,
    title: adw::WindowTitle,
    previous: gtk::Button,
    next: gtk::Button,
    fit: gtk::Button,
    details: gtk::Label,
}

pub(super) fn bounded_zoom(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(1.0, 4.0)
    } else {
        1.0
    }
}

impl Viewer {
    fn show(&self, index: usize) {
        let Some(photo) = self.pages.get(index) else {
            return;
        };
        self.index.set(index);
        self.zoom.set(1.0);
        self.fit.set_label("Fit");
        self.title.set_title(&photo.name);
        self.title
            .set_subtitle(&format!("{} / {}", index + 1, self.pages.len()));
        self.previous.set_sensitive(index > 0);
        self.next.set_sensitive(index + 1 < self.pages.len());
        self.details.set_label(&format!("{}\n\n{}\n\nOptimized local preview · original unchanged\nPNG / JPEG · up to 32 MiB and 16 megapixels\n\n← / →  Previous / next\n+ / −  Zoom · 0  Fit\nCtrl + scroll or pinch to zoom\nDrag to pan · Double-click to zoom\nF11  Fullscreen · Esc  Back", photo.name,
            photo.source.as_ref().map(|s| format!("{}\n{} bytes", s.relative, s.size)).unwrap_or_else(|| "Source unavailable".into())));
        self.scroll
            .set_child(Some(&preview(photo.source.clone(), 1024, 120)));
        self.scroll.hadjustment().set_value(0.0);
        self.scroll.vadjustment().set_value(0.0);
    }
    fn step(&self, delta: isize) {
        if let Some(index) = self.index.get().checked_add_signed(delta) {
            self.show(index);
        }
    }
    fn set_zoom(&self, value: f64) {
        let value = bounded_zoom(value);
        self.zoom.set(value);
        self.fit
            .set_label(if value == 1.0 { "Fit" } else { "Reset zoom" });
        if let Some(child) = self.scroll.child() {
            // ScrolledWindow wraps non-scrollable widgets in a viewport.
            let child = child
                .downcast_ref::<gtk::Viewport>()
                .and_then(|view| view.child())
                .unwrap_or(child);
            if value == 1.0 {
                child.set_size_request(120, 120);
            } else {
                child.set_size_request(
                    (f64::from(self.scroll.width()) * value).round() as i32,
                    (f64::from(self.scroll.height()) * value).round() as i32,
                );
            }
        }
    }
    fn fullscreen(&self) {
        if let Some(window) = self.window.upgrade() {
            if window.is_fullscreen() {
                window.unfullscreen();
            } else {
                window.fullscreen();
            }
        }
    }
}

fn control(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(label)
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

pub(super) fn open(
    parent: &adw::ApplicationWindow,
    pages: Vec<Photo>,
    index: usize,
) -> Option<adw::Window> {
    if index >= pages.len() {
        return None;
    }
    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Photos — MiRelay")
        .default_width(960)
        .default_height(720)
        .build();
    window.add_css_class("mirelay");
    window.add_css_class("album-viewer");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("", "");
    header.set_title_widget(Some(&title));
    let close = control("go-previous-symbolic", "Close photo (Esc)");
    let previous = control("go-previous-symbolic", "Previous photo (Left)");
    let next = control("go-next-symbolic", "Next photo (Right)");
    let minus = control("zoom-out-symbolic", "Zoom out (-)");
    let plus = control("zoom-in-symbolic", "Zoom in (+)");
    let fit = gtk::Button::with_label("Fit");
    fit.add_css_class("flat");
    fit.set_tooltip_text(Some("Fit image (0)"));
    let fullscreen = control("view-fullscreen-symbolic", "Toggle fullscreen (F11)");
    let details = gtk::Label::new(None);
    details.set_wrap(true);
    details.set_max_width_chars(48);
    details.set_selectable(true);
    details.set_margin_top(16);
    details.set_margin_bottom(16);
    details.set_margin_start(16);
    details.set_margin_end(16);
    let info = gtk::MenuButton::builder()
        .icon_name("info-outline-symbolic")
        .tooltip_text("Photo details and shortcuts")
        .popover(&gtk::Popover::builder().child(&details).build())
        .build();
    header.pack_start(&close);
    header.pack_end(&fullscreen);
    header.pack_end(&info);
    content.append(&header);
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();
    scroll.set_widget_name("album-viewer-canvas");
    content.append(&scroll);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.add_css_class("album-viewer-controls");
    toolbar.set_halign(gtk::Align::Center);
    for button in [&previous, &minus, &fit, &plus, &next] {
        toolbar.append(button);
    }
    content.append(&toolbar);
    window.set_content(Some(&content));
    let viewer = Rc::new(Viewer {
        window: window.downgrade(),
        pages,
        index: Cell::new(index),
        zoom: Cell::new(1.0),
        scroll: scroll.clone(),
        title,
        previous: previous.clone(),
        next: next.clone(),
        fit: fit.clone(),
        details,
    });
    for (button, delta) in [(&previous, -1), (&next, 1)] {
        let weak = Rc::downgrade(&viewer);
        button.connect_clicked(move |_| {
            if let Some(viewer) = weak.upgrade() {
                viewer.step(delta);
            }
        });
    }
    for (button, factor) in [(&minus, 0.8), (&plus, 1.25), (&fit, 0.0)] {
        let weak = Rc::downgrade(&viewer);
        button.connect_clicked(move |_| {
            if let Some(viewer) = weak.upgrade() {
                viewer.set_zoom(viewer.zoom.get() * factor);
            }
        });
    }
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    let weak = Rc::downgrade(&viewer);
    fullscreen.connect_clicked(move |_| {
        if let Some(viewer) = weak.upgrade() {
            viewer.fullscreen();
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&viewer);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(viewer) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        use gtk::gdk::Key;
        match key {
            Key::Left => viewer.step(-1),
            Key::Right => viewer.step(1),
            Key::plus | Key::equal | Key::KP_Add => viewer.set_zoom(viewer.zoom.get() * 1.25),
            Key::minus | Key::KP_Subtract => viewer.set_zoom(viewer.zoom.get() * 0.8),
            Key::_0 | Key::KP_0 => viewer.set_zoom(1.0),
            Key::F11 => viewer.fullscreen(),
            Key::Escape => {
                if let Some(window) = viewer.window.upgrade() {
                    if window.is_fullscreen() {
                        window.unfullscreen();
                    } else {
                        window.close();
                    }
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let weak = Rc::downgrade(&viewer);
    wheel.connect_scroll(move |controller, _, dy| {
        if !controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        if let Some(viewer) = weak.upgrade() {
            viewer.set_zoom(viewer.zoom.get() * (-dy * 0.15).exp());
        }
        glib::Propagation::Stop
    });
    scroll.add_controller(wheel);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&viewer);
    click.connect_pressed(move |_, count, _, _| {
        if count == 2 {
            if let Some(viewer) = weak.upgrade() {
                viewer.set_zoom(if viewer.zoom.get() > 1.0 { 1.0 } else { 2.0 });
            }
        }
    });
    scroll.add_controller(click);
    let drag = gtk::GestureDrag::new();
    let origin = Rc::new(Cell::new((0.0, 0.0)));
    let pos = origin.clone();
    let weak = scroll.downgrade();
    drag.connect_drag_begin(move |_, _, _| {
        if let Some(scroll) = weak.upgrade() {
            pos.set((scroll.hadjustment().value(), scroll.vadjustment().value()));
        }
    });
    let weak = scroll.downgrade();
    drag.connect_drag_update(move |_, dx, dy| {
        if let Some(scroll) = weak.upgrade() {
            let (x, y) = origin.get();
            scroll.hadjustment().set_value(x - dx);
            scroll.vadjustment().set_value(y - dy);
        }
    });
    scroll.add_controller(drag);
    let pinch = gtk::GestureZoom::new();
    let initial = Rc::new(Cell::new(1.0));
    let begin = initial.clone();
    let weak = Rc::downgrade(&viewer);
    pinch.connect_begin(move |_, _| {
        if let Some(viewer) = weak.upgrade() {
            begin.set(viewer.zoom.get());
        }
    });
    let weak = Rc::downgrade(&viewer);
    pinch.connect_scale_changed(move |_, scale| {
        if let Some(viewer) = weak.upgrade() {
            viewer.set_zoom(initial.get() * scale);
        }
    });
    scroll.add_controller(pinch);
    viewer.show(index);
    // Window owns the controller state; the state has only a weak window ref.
    window.connect_close_request(move |_| {
        viewer.scroll.set_child(gtk::Widget::NONE);
        glib::Propagation::Proceed
    });
    window.present();
    Some(window)
}
