//! Native adaptive Folder navigation, with one sidebar instance at every width.
//! Flap is available at our existing libadwaita minimum; no new runtime baseline.
use super::*;

const SIDEBAR_MIN: i32 = 208;
const SIDEBAR_MAX: i32 = 320;
const CONTENT_MIN: i32 = 520;
const HANDLE: i32 = 14;

fn sidebar_width(requested: f64, available: i32) -> i32 {
    let maximum = available
        .saturating_sub(CONTENT_MIN + HANDLE)
        .clamp(SIDEBAR_MIN, SIDEBAR_MAX);
    if !requested.is_finite() {
        return SIDEBAR_MIN;
    }
    requested.round().clamp(SIDEBAR_MIN as f64, maximum as f64) as i32
}

#[derive(Clone)]
pub(super) struct Navigation {
    pub(super) view: adw::Flap,
    #[cfg(test)]
    pub(super) toggle: gtk::ToggleButton,
    #[cfg(test)]
    pub(super) close: gtk::Button,
}

impl Navigation {
    pub(super) fn new(
        sidebar: &gtk::Box,
        sidebar_brand: &gtk::Box,
        content: &gtk::Box,
        header: &adw::HeaderBar,
        folders: &gtk::ListBox,
    ) -> Self {
        sidebar.set_hexpand(false);
        content.set_width_request(CONTENT_MIN);
        let handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
        handle.add_css_class("sidebar-resize");
        handle.set_width_request(HANDLE);
        handle.set_focusable(true);
        handle.set_cursor_from_name(Some("col-resize"));
        handle.set_tooltip_text(Some("Resize Folder sidebar · arrow keys, Home or End"));
        handle.update_property(&[gtk::accessible::Property::Label("Resize Folder sidebar")]);
        let view = adw::Flap::builder()
            .flap(sidebar)
            .content(content)
            .separator(&handle)
            .fold_policy(adw::FlapFoldPolicy::Auto)
            .fold_threshold_policy(adw::FoldThresholdPolicy::Minimum)
            .transition_type(adw::FlapTransitionType::Over)
            .modal(true)
            .vexpand(true)
            .build();
        view.add_css_class("workspace");
        let toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Show Folders")
            .visible(false)
            .build();
        toggle.add_css_class("flat");
        toggle.update_property(&[gtk::accessible::Property::Label("Show Folders")]);
        header.pack_start(&toggle);
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Hide Folders")
            .valign(gtk::Align::Center)
            .margin_end(12)
            .visible(false)
            .build();
        close.add_css_class("flat");
        sidebar_brand.append(&close);
        close.connect_clicked(glib::clone!(
            #[weak]
            view,
            #[weak]
            toggle,
            move |_| {
                view.set_reveal_flap(false);
                toggle.grab_focus();
            }
        ));
        view.bind_property("folded", &close, "visible")
            .sync_create()
            .build();
        view.bind_property("reveal-flap", &toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        view.connect_folded_notify(glib::clone!(
            #[weak]
            toggle,
            #[weak]
            sidebar,
            move |view| {
                toggle.set_visible(view.is_folded());
                if view.is_folded() {
                    sidebar.add_css_class("folder-drawer");
                } else {
                    sidebar.remove_css_class("folder-drawer");
                }
            }
        ));
        // Close only for explicit activation, not selection changes caused by
        // filtering, refresh or keyboard navigation through the Folder list.
        folders.connect_row_activated(glib::clone!(
            #[weak]
            view,
            #[weak]
            toggle,
            move |_, _| {
                if view.is_folded() {
                    view.set_reveal_flap(false);
                    toggle.grab_focus();
                }
            }
        ));
        let initial = Rc::new(Cell::new(SIDEBAR_MIN));
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.connect_drag_begin(glib::clone!(
            #[weak]
            sidebar,
            #[strong]
            initial,
            move |_, _, _| initial.set(sidebar.width_request())
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak]
            sidebar,
            #[weak]
            view,
            move |_, dx, _| {
                if !view.is_folded() {
                    let direction = if view.direction() == gtk::TextDirection::Rtl {
                        -1.0
                    } else {
                        1.0
                    };
                    sidebar.set_width_request(sidebar_width(
                        initial.get() as f64 + dx * direction,
                        view.width(),
                    ));
                }
            }
        ));
        handle.add_controller(drag);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            sidebar,
            #[weak]
            view,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if view.is_folded() || !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let direction = if view.direction() == gtk::TextDirection::Rtl {
                    -16
                } else {
                    16
                };
                let requested = match key {
                    gtk::gdk::Key::Left => sidebar.width_request() - direction,
                    gtk::gdk::Key::Right => sidebar.width_request() + direction,
                    gtk::gdk::Key::Home => SIDEBAR_MIN,
                    gtk::gdk::Key::End => SIDEBAR_MAX,
                    _ => return glib::Propagation::Proceed,
                };
                sidebar.set_width_request(sidebar_width(requested as f64, view.width()));
                glib::Propagation::Stop
            }
        ));
        handle.add_controller(keys);
        Self {
            view,
            #[cfg(test)]
            toggle,
            #[cfg(test)]
            close,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_is_bounded_and_preserves_content_space() {
        assert_eq!(sidebar_width(999.0, 1000), 320);
        assert_eq!(sidebar_width(-999.0, 1000), 208);
        assert_eq!(sidebar_width(300.0, 820), 286);
        assert_eq!(sidebar_width(300.0, 560), 208);
        assert_eq!(sidebar_width(f64::NAN, 1000), 208);
        assert_eq!(sidebar_width(f64::INFINITY, i32::MAX), 208);
        assert_eq!(sidebar_width(300.0, i32::MIN), 208);
    }
}
