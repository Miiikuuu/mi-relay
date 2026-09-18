//! Read-only, event-driven power-profiles-daemon integration, without raising
//! the GLib minimum. Own the connection so losing the bus cannot exit the app.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{gio, glib, prelude::*};

const PROVIDERS: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    #[default]
    Checking,
    Unavailable,
    Normal,
    Saver,
}

impl State {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking power profile. Background motion is paused.",
            Self::Unavailable => {
                "Power profile unavailable. Background motion stays paused; transfers are unaffected."
            }
            Self::Normal => "Power profile allows motion. Your appearance settings still apply.",
            Self::Saver => {
                "Battery saver is active. Background motion is paused; transfers are unaffected."
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Observation {
    Pending,
    Absent,
    Unknown,
    Normal,
    Saver,
}

fn profile(value: Option<&glib::Variant>) -> Observation {
    match value.and_then(glib::Variant::str) {
        Some("balanced" | "performance") => Observation::Normal,
        Some("power-saver") => Observation::Saver,
        _ => Observation::Unknown,
    }
}

fn aggregate(values: [Observation; 2]) -> State {
    if values.contains(&Observation::Saver) {
        return State::Saver;
    }
    if values.contains(&Observation::Pending) {
        return State::Checking;
    }
    // Prefer the modern interface, but never ignore a saver report from either.
    let selected = if values[0] == Observation::Absent {
        values[1]
    } else {
        values[0]
    };
    if selected == Observation::Normal {
        State::Normal
    } else {
        State::Unavailable
    }
}

pub(super) struct Monitor {
    connection: RefCell<Option<gio::DBusConnection>>,
    proxies: [RefCell<Option<gio::DBusProxy>>; 2],
    observations: Cell<[Observation; 2]>,
    epochs: Cell<[u64; 2]>,
    state: Cell<State>,
    setup: gio::Cancellable,
    reads: gio::Cancellable,
    deadline: RefCell<Option<glib::SourceId>>,
    changed: Box<dyn Fn(State)>,
}

impl Monitor {
    pub(super) fn system(changed: impl Fn(State) + 'static) -> Rc<Self> {
        let address =
            gio::dbus_address_get_for_bus_sync(gio::BusType::System, gio::Cancellable::NONE);
        Self::start(address.ok().as_deref(), changed)
    }

    fn start(address: Option<&str>, changed: impl Fn(State) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            connection: RefCell::default(),
            proxies: Default::default(),
            observations: Cell::new([Observation::Pending; 2]),
            epochs: Cell::new([0; 2]),
            state: Cell::new(State::Checking),
            setup: gio::Cancellable::new(),
            reads: gio::Cancellable::new(),
            deadline: RefCell::default(),
            changed: Box::new(changed),
        });
        let Some(address) = address else {
            this.unavailable();
            return this;
        };
        let weak = Rc::downgrade(&this);
        *this.deadline.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_secs(3),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.deadline.borrow_mut().take();
                    this.setup.cancel();
                    let values = this.observations.get().map(|v| {
                        if v == Observation::Pending {
                            Observation::Unknown
                        } else {
                            v
                        }
                    });
                    this.observations.set(values);
                    this.publish();
                }
            },
        ));
        let weak = Rc::downgrade(&this);
        gio::DBusConnection::for_address(
            address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            Some(&this.setup),
            move |result| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                let Ok(connection) = result else {
                    this.unavailable();
                    return;
                };
                connection.set_exit_on_close(false);
                let weak = Rc::downgrade(&this);
                // This private connection was created on the GTK main context.
                connection.connect_local("closed", false, move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.unavailable();
                    }
                    None
                });
                *this.connection.borrow_mut() = Some(connection.clone());
                for (index, (name, path)) in PROVIDERS.iter().enumerate() {
                    let weak = Rc::downgrade(&this);
                    gio::DBusProxy::new(
                        &connection,
                        gio::DBusProxyFlags::DO_NOT_AUTO_START,
                        None,
                        Some(name),
                        path,
                        name,
                        Some(&this.setup),
                        move |result| {
                            let Some(this) = weak.upgrade() else {
                                return;
                            };
                            let Ok(proxy) = result else {
                                this.set(index, Observation::Unknown);
                                return;
                            };
                            proxy.set_default_timeout(1000);
                            let weak = Rc::downgrade(&this);
                            proxy.connect_local("notify::g-name-owner", false, move |_| {
                                if let Some(this) = weak.upgrade() {
                                    this.refresh(index);
                                }
                                None
                            });
                            let weak = Rc::downgrade(&this);
                            proxy.connect_local("g-properties-changed", false, move |values| {
                                if let Some(this) = weak.upgrade() {
                                    let changed = values[1].get::<glib::Variant>().ok();
                                    let invalidated = values[2].get::<glib::StrV>().ok();
                                    if changed.as_ref().is_some_and(|v| {
                                        glib::VariantDict::new(Some(v)).contains("ActiveProfile")
                                    }) || invalidated
                                        .is_some_and(|v| v.iter().any(|key| key == "ActiveProfile"))
                                    {
                                        this.refresh(index);
                                    }
                                }
                                None
                            });
                            *this.proxies[index].borrow_mut() = Some(proxy);
                            this.refresh(index);
                        },
                    );
                }
            },
        );
        this
    }

    fn publish(&self) {
        let values = self.observations.get();
        if !values.contains(&Observation::Pending)
            && let Some(deadline) = self.deadline.borrow_mut().take()
        {
            deadline.remove();
        }
        let state = aggregate(values);
        if self.state.replace(state) != state {
            (self.changed)(state);
        }
    }

    fn unavailable(&self) {
        self.observations.set([Observation::Unknown; 2]);
        self.epochs
            .set(self.epochs.get().map(|v| v.wrapping_add(1)));
        self.publish();
    }

    fn set(&self, index: usize, value: Observation) {
        let mut values = self.observations.get();
        values[index] = value;
        self.observations.set(values);
        self.publish();
    }

    fn refresh(self: &Rc<Self>, index: usize) {
        let mut epochs = self.epochs.get();
        epochs[index] = epochs[index].wrapping_add(1);
        self.epochs.set(epochs);
        let epoch = epochs[index];
        let Some(proxy) = self.proxies[index].borrow().clone() else {
            return;
        };
        if proxy.connection().is_closed() {
            self.unavailable();
            return;
        }
        let Some(owner) = proxy.g_name_owner() else {
            self.set(index, Observation::Absent);
            return;
        };
        if let Some(value) = proxy.cached_property("ActiveProfile") {
            self.set(index, profile(Some(&value)));
            return;
        }
        // An invalidated property is not permission to keep animating. Refresh
        // asynchronously; a newer owner/value always wins over an old reply.
        self.set(index, Observation::Unknown);
        let weak = Rc::downgrade(self);
        let pending_proxy = proxy.clone();
        proxy.call(
            "org.freedesktop.DBus.Properties.Get",
            Some(&(PROVIDERS[index].0, "ActiveProfile").to_variant()),
            gio::DBusCallFlags::NO_AUTO_START,
            1000,
            Some(&self.reads),
            move |reply| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                if this.epochs.get()[index] != epoch
                    || pending_proxy.g_name_owner().as_ref() != Some(&owner)
                {
                    return;
                }
                let value = reply
                    .ok()
                    .and_then(|v| v.get::<(glib::Variant,)>().map(|tuple| tuple.0));
                this.set(index, profile(value.as_ref()));
            },
        );
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.setup.cancel();
        self.reads.cancel();
        if let Some(deadline) = self.deadline.get_mut().take() {
            deadline.remove();
        }
        if let Some(connection) = self.connection.get_mut().take() {
            connection.close(gio::Cancellable::NONE, |_| {});
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
