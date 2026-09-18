//! App-local appearance only. No transport state, worker, or CSS animation.
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use anyhow::{Context, Result, bail};
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod gpu_tests;
mod power;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub motion: bool,
    pub reduce_transparency: bool,
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    #[serde(flatten)]
    preferences: Preferences,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

fn load(path: &Path) -> Result<Document> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document {
                version: 1,
                preferences: Preferences::default(),
                extra: BTreeMap::new(),
            });
        }
        Err(e) => return Err(e.into()),
        Ok(meta) if !meta.is_file() => bail!("Appearance settings must be a regular file"),
        _ => {}
    }
    let raw = crate::fsutil::read_limited(path, 16 * 1024)?;
    let document: Document = serde_json::from_slice(&raw).context("Invalid appearance settings")?;
    if document.version != 1 {
        bail!("Unsupported appearance settings version");
    }
    Ok(document)
}

fn save(path: &Path, preferences: Preferences) -> Result<()> {
    // Re-read before writing; never silently destroy a malformed/future file.
    let mut document = load(path)?;
    document.preferences = preferences;
    crate::fsutil::atomic_write(path, &serde_json::to_vec_pretty(&document)?)
}

fn should_animate(
    p: Preferences,
    mapped: bool,
    active: bool,
    animations: bool,
    high_contrast: bool,
    power_allows_motion: bool,
) -> bool {
    p.motion
        && !p.reduce_transparency
        && mapped
        && active
        && animations
        && !high_contrast
        && power_allows_motion
}

fn wave(micros: i64, seconds: i64) -> f32 {
    let phase = micros.rem_euclid(seconds * 1_000_000) as f64 / (seconds * 1_000_000) as f64;
    ((1.0 - (phase * std::f64::consts::TAU).cos()) * 0.5) as f32
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Ambient {
        pub(super) preferences: Cell<Preferences>,
        pub window: glib::WeakRef<adw::ApplicationWindow>,
        pub elapsed: Cell<i64>,
        pub previous: Cell<i64>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        pub nodes: RefCell<Vec<gsk::RadialGradientNode>>,
        pub dimensions: Cell<(i32, i32)>,
        pub(super) power_state: Cell<power::State>,
        pub(super) power_monitor: RefCell<Option<Rc<power::Monitor>>>,
        pub(super) power_labels: RefCell<Vec<glib::WeakRef<gtk::Label>>>,
        #[cfg(test)]
        pub draws: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Ambient {
        const NAME: &'static str = "MiRelayAmbientBackdrop";
        type Type = super::Ambient;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for Ambient {
        fn dispose(&self) {
            self.obj().stop();
            self.power_monitor.borrow_mut().take();
            self.nodes.borrow_mut().clear();
        }
    }
    impl WidgetImpl for Ambient {
        fn map(&self) {
            self.parent_map();
            self.obj().update();
        }
        fn unmap(&self) {
            self.obj().stop();
            self.parent_unmap();
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            #[cfg(test)]
            self.draws.set(self.draws.get() + 1);
            if self.preferences.get().reduce_transparency
                || adw::StyleManager::default().is_high_contrast()
            {
                return;
            }
            let obj = self.obj();
            let (width, height) = (obj.width(), obj.height());
            if width <= 0 || height <= 0 {
                return;
            }
            if self.dimensions.replace((width, height)) != (width, height)
                || self.nodes.borrow().is_empty()
            {
                *self.nodes.borrow_mut() = gradients(width as f32, height as f32);
            }
            let t = self.elapsed.get();
            let mix = wave(t, 22);
            snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            for (i, period) in [12, 15, 14].iter().enumerate() {
                let position = wave(t, *period) - 0.5;
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    width as f32 * position * if i == 1 { -0.3 } else { 0.3 },
                    height as f32 * position * 0.15,
                ));
                for (variant, opacity) in [(0, 1.0 - mix), (1, mix)] {
                    snapshot.push_opacity(opacity as f64);
                    snapshot.append_node(&self.nodes.borrow()[i * 2 + variant]);
                    snapshot.pop();
                }
                snapshot.restore();
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct Ambient(ObjectSubclass<imp::Ambient>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn gradients(width: f32, height: f32) -> Vec<gsk::RadialGradientNode> {
    let colors = [
        gdk::RGBA::new(0.725, 0.851, 0.804, 0.48),
        gdk::RGBA::new(0.733, 0.792, 0.875, 0.40),
        gdk::RGBA::new(0.898, 0.808, 0.776, 0.32),
    ];
    let centers = [(0.08, 0.28), (0.95, 0.45), (0.35, 0.95)];
    let bounds = graphene::Rect::new(-width, -height, width * 3.0, height * 3.0);
    centers
        .iter()
        .enumerate()
        .flat_map(|(i, (x, y))| {
            let center = graphene::Point::new(width * x, height * y);
            [0, 1].map(|variant| {
                gsk::RadialGradientNode::new(
                    &bounds,
                    &center,
                    width * 0.8,
                    height * 0.8,
                    0.0,
                    1.0,
                    &[
                        gsk::ColorStop::new(0.0, colors[(i + variant) % 3]),
                        gsk::ColorStop::new(1.0, gdk::RGBA::TRANSPARENT),
                    ],
                )
            })
        })
        .collect()
}

impl Ambient {
    pub(super) fn monitor_power(&self) {
        self.imp().power_monitor.borrow_mut().take();
        self.set_power_state(power::State::Checking);
        let weak = self.downgrade();
        let monitor = power::Monitor::system(move |state| {
            if let Some(obj) = weak.upgrade() {
                obj.set_power_state(state);
            }
        });
        *self.imp().power_monitor.borrow_mut() = Some(monitor);
    }

    fn set_power_state(&self, state: power::State) {
        self.imp().power_state.set(state);
        self.imp().power_labels.borrow_mut().retain(|weak| {
            if let Some(label) = weak.upgrade() {
                label.set_label(state.label());
                true
            } else {
                false
            }
        });
        self.update();
    }

    pub(super) fn new(window: &adw::ApplicationWindow) -> Self {
        let obj: Self = glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", true)
            .property("can-target", false)
            .build();
        obj.imp().window.set(Some(window));
        window.connect_is_active_notify(glib::clone!(
            #[weak]
            obj,
            move |_| obj.update()
        ));
        // Watched closures are invalidated with the widget, unlike global
        // signal handlers that merely keep a dead weak reference forever.
        obj.settings().connect_closure(
            "notify::gtk-enable-animations",
            false,
            glib::closure_local!(
                #[watch]
                obj,
                move |_settings: gtk::Settings, _property: glib::ParamSpec| obj.update()
            ),
        );
        adw::StyleManager::default().connect_closure(
            "notify::high-contrast",
            false,
            glib::closure_local!(
                #[watch]
                obj,
                move |_style: adw::StyleManager, _property: glib::ParamSpec| {
                    obj.apply_surface();
                    obj.update();
                    obj.queue_draw();
                }
            ),
        );
        obj
    }

    fn apply_surface(&self) {
        if let Some(window) = self.imp().window.upgrade() {
            if self.imp().preferences.get().reduce_transparency
                || adw::StyleManager::default().is_high_contrast()
            {
                window.add_css_class("reduced-transparency");
            } else {
                window.remove_css_class("reduced-transparency");
            }
        }
    }

    fn apply(&self, preferences: Preferences) {
        self.imp().preferences.set(preferences);
        self.apply_surface();
        self.update();
        self.queue_draw();
    }

    fn enabled(&self) -> bool {
        should_animate(
            self.imp().preferences.get(),
            self.is_mapped(),
            self.imp().window.upgrade().is_some_and(|w| w.is_active()),
            self.settings().is_gtk_enable_animations(),
            adw::StyleManager::default().is_high_contrast(),
            self.imp().power_state.get() == power::State::Normal,
        )
    }

    fn stop(&self) {
        if let Some(tick) = self.imp().tick.borrow_mut().take() {
            tick.remove();
        }
        self.imp().previous.set(0);
    }

    fn update(&self) {
        if !self.enabled() {
            self.stop();
            return;
        }
        if self.imp().tick.borrow().is_some() {
            return;
        }
        let tick = self.add_tick_callback(|obj, clock| {
            if !obj.enabled() {
                obj.imp().tick.borrow_mut().take();
                obj.imp().previous.set(0);
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let previous = obj.imp().previous.get();
            if previous == 0 {
                obj.imp().previous.set(now);
            } else if now.saturating_sub(previous) >= 33_334 {
                obj.imp().elapsed.set(
                    obj.imp()
                        .elapsed
                        .get()
                        .saturating_add(now.saturating_sub(previous).min(100_000)),
                );
                obj.imp().previous.set(now);
                obj.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        *self.imp().tick.borrow_mut() = Some(tick);
    }
}

pub(super) fn install(
    window: &adw::ApplicationWindow,
    ambient: &Ambient,
    button: &gtk::Button,
    toasts: &adw::ToastOverlay,
    path: PathBuf,
) {
    match load(&path) {
        Ok(document) => ambient.apply(document.preferences),
        Err(_) => toasts.add_toast(adw::Toast::new(
            "Could not read appearance settings. Using motion off; the file is unchanged.",
        )),
    }
    let open_dialog: Rc<RefCell<glib::WeakRef<adw::Window>>> = Rc::default();
    button.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        ambient,
        move |_| {
            if let Some(dialog) = open_dialog.borrow().upgrade() {
                dialog.present();
                return;
            }
            let dialog = settings_dialog(&window, &ambient, path.clone());
            open_dialog.borrow_mut().set(Some(&dialog));
            dialog.present();
        }
    ));
    let action = gio::SimpleAction::new("appearance", None);
    action.connect_activate(glib::clone!(
        #[weak]
        button,
        move |_, _| button.emit_clicked()
    ));
    window.add_action(&action);
}

fn settings_dialog(
    window: &adw::ApplicationWindow,
    ambient: &Ambient,
    path: PathBuf,
) -> adw::Window {
    let dialog = adw::Window::builder()
        .title("Settings")
        .transient_for(window)
        .modal(true)
        .default_width(460)
        .default_height(480)
        .build();
    dialog.add_css_class("mirelay");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&adw::HeaderBar::new());
    let group = adw::PreferencesGroup::builder().title("Appearance").description("Applies to all Folders in this local profile. Transfers and automatic receive are unchanged.").build();
    let motion = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(ambient.imp().preferences.get().motion)
        .build();
    let reduced = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(ambient.imp().preferences.get().reduce_transparency)
        .build();
    for (title, subtitle, switch) in [
        (
            "Background motion",
            "Slow light bands. Off by default. Pauses in battery saver, when hidden or unfocused, or when system animations are off.",
            &motion,
        ),
        (
            "Reduce transparency",
            "Use solid panels and stop background motion. High contrast also uses this fallback.",
            &reduced,
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .activatable_widget(switch)
            .build();
        row.add_suffix(switch);
        group.add(&row);
    }
    let form = gtk::Box::new(gtk::Orientation::Vertical, 16);
    form.set_margin_start(24);
    form.set_margin_end(24);
    form.set_margin_top(16);
    form.set_margin_bottom(24);
    form.append(&group);
    let power_status = gtk::Label::builder()
        .label(ambient.imp().power_state.get().label())
        .wrap(true)
        .xalign(0.0)
        .build();
    power_status.add_css_class("secondary-text");
    power_status.set_hexpand(true);
    ambient
        .imp()
        .power_labels
        .borrow_mut()
        .retain(|label| label.upgrade().is_some());
    ambient
        .imp()
        .power_labels
        .borrow_mut()
        .push(power_status.downgrade());
    let power_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    power_row.append(&power_status);
    let refresh_power = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_power.set_tooltip_text(Some("Refresh power status"));
    refresh_power.add_css_class("flat");
    refresh_power.set_valign(gtk::Align::Center);
    refresh_power.connect_clicked(glib::clone!(
        #[weak]
        ambient,
        move |_| ambient.monitor_power()
    ));
    power_row.append(&refresh_power);
    form.append(&power_row);
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    error.add_css_class("error");
    form.append(&error);
    let save_button = gtk::Button::with_label("Save");
    save_button.add_css_class("suggested-action");
    save_button.set_halign(gtk::Align::End);
    form.append(&save_button);
    let saving = Rc::new(Cell::new(false));
    dialog.connect_close_request(glib::clone!(
        #[strong]
        saving,
        move |_| if saving.get() {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    ));
    save_button.connect_clicked(glib::clone!(#[weak] dialog, #[weak] ambient, #[weak] form, #[weak] error, move |_| {
        if saving.replace(true) { return; }
        let preferences = Preferences { motion: motion.is_active(), reduce_transparency: reduced.is_active() };
        form.set_sensitive(false); error.set_visible(false);
        let path = path.clone();
        glib::spawn_future_local(glib::clone!(#[strong] saving, #[strong] dialog, #[weak] ambient, #[strong] form, #[strong] error, async move {
            let result = gio::spawn_blocking(move || save(&path, preferences)).await;
            saving.set(false); form.set_sensitive(true);
            match result {
                Ok(Ok(())) => { ambient.apply(preferences); dialog.close(); }
                _ => { error.set_label("Could not save or confirm appearance settings. Check the file format, version and directory permissions, then retry."); error.set_visible(true); }
            }
        }));
    }));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&form)
        .build();
    root.append(&scroll);
    dialog.set_content(Some(&root));
    dialog
}

#[cfg(test)]
mod tests;
