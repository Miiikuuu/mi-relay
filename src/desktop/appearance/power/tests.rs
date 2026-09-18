use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

#[test]
fn only_known_non_saver_profiles_allow_motion() {
    for value in [
        None,
        Some(false.to_variant()),
        Some("unknown".to_variant()),
        Some("".to_variant()),
        Some("Balanced".to_variant()),
    ] {
        assert!(profile(value.as_ref()) == Observation::Unknown);
    }
    assert!(profile(Some(&"power-saver".to_variant())) == Observation::Saver);
    for value in ["balanced", "performance"] {
        assert!(profile(Some(&value.to_variant())) == Observation::Normal);
    }
}

#[test]
fn alias_precedence_and_conflicts_fail_closed() {
    use Observation::*;
    assert_eq!(aggregate([Absent, Normal]), State::Normal);
    assert_eq!(aggregate([Normal, Absent]), State::Normal);
    assert_eq!(aggregate([Absent, Absent]), State::Unavailable);
    assert_eq!(aggregate([Unknown, Normal]), State::Unavailable);
    for other in [Pending, Absent, Unknown, Normal, Saver] {
        assert_eq!(aggregate([Saver, other]), State::Saver);
        assert_eq!(aggregate([other, Saver]), State::Saver);
    }
    assert_eq!(aggregate([Normal, Pending]), State::Checking);
}

pub(crate) struct Fixture {
    bus: OwnedBus,
    pub(crate) address: String,
    connection: gio::DBusConnection,
    values: [Rc<RefCell<String>>; 2],
    pub(crate) reads: Rc<Cell<u32>>,
    pub(crate) writes: Rc<Cell<u32>>,
}

struct OwnedBus(Child);
impl OwnedBus {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for OwnedBus {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let mut bus = OwnedBus(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1", "--nopidfile"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("private QA bus"),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_owned();
        let connection = gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap();
        connection.set_exit_on_close(false);
        let this = Self {
            bus,
            address,
            connection,
            values: std::array::from_fn(|_| Rc::new(RefCell::new("balanced".into()))),
            reads: Rc::default(),
            writes: Rc::default(),
        };
        for (index, (name, path)) in PROVIDERS.iter().enumerate() {
            let xml = format!(
                "<node><interface name='{name}'><property name='ActiveProfile' type='s' access='readwrite'/></interface></node>"
            );
            let info = gio::DBusNodeInfo::for_xml(&xml)
                .unwrap()
                .lookup_interface(name)
                .unwrap();
            let value = this.values[index].clone();
            let reads = this.reads.clone();
            let writes = this.writes.clone();
            this.connection
                .register_object(path, &info)
                .property(move |_, _, _, _, property| {
                    assert_eq!(property, "ActiveProfile");
                    reads.set(reads.get() + 1);
                    value.borrow().to_variant()
                })
                .set_property(move |_, _, _, _, _, _| {
                    writes.set(writes.get() + 1);
                    false
                })
                .build()
                .unwrap();
        }
        this
    }

    pub(crate) fn own(&self, index: usize, own: bool) {
        let (method, parameters) = if own {
            ("RequestName", (PROVIDERS[index].0, 0u32).to_variant())
        } else {
            ("ReleaseName", (PROVIDERS[index].0,).to_variant())
        };
        self.connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                method,
                Some(&parameters),
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    }

    pub(crate) fn profile(&self, index: usize, profile: &str, invalidate: bool) {
        *self.values[index].borrow_mut() = profile.into();
        let changed: BTreeMap<&str, glib::Variant> = if invalidate {
            BTreeMap::new()
        } else {
            BTreeMap::from([("ActiveProfile", profile.to_variant())])
        };
        let invalidated: Vec<&str> = if invalidate {
            vec!["ActiveProfile"]
        } else {
            vec![]
        };
        self.connection
            .emit_signal(
                None,
                PROVIDERS[index].1,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&(PROVIDERS[index].0, changed, invalidated).to_variant()),
            )
            .unwrap();
        self.connection.flush_sync(gio::Cancellable::NONE).unwrap();
    }

    pub(in crate::desktop::appearance) fn monitor(
        &self,
        changed: impl Fn(State) + 'static,
    ) -> Rc<Monitor> {
        Monitor::start(Some(&self.address), changed)
    }

    fn stop_bus(&mut self) {
        self.bus.stop();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
        self.stop_bus();
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < until,
            "power monitor transition timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "starts a private D-Bus; run alone with --test-threads=1"]
fn private_bus_profiles_restart_invalidation_and_disconnect_are_read_only() {
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let mut fixture = Fixture::new();
    let state = Rc::new(Cell::new(State::Checking));
    let output = state.clone();
    let monitor = fixture.monitor(move |value| output.set(value));
    wait_for(|| state.get() == State::Unavailable);
    assert_eq!(
        fixture.reads.get(),
        0,
        "must not auto-start absent services"
    );
    fixture.own(1, true);
    wait_for(|| state.get() == State::Normal);
    fixture.profile(1, "power-saver", false);
    wait_for(|| state.get() == State::Saver);
    fixture.profile(1, "balanced", true);
    wait_for(|| state.get() == State::Normal);
    fixture.own(0, true);
    wait_for(|| monitor.observations.get()[0] == Observation::Normal);
    fixture.profile(0, "unexpected", false);
    wait_for(|| state.get() == State::Unavailable);
    fixture.profile(0, "performance", false);
    wait_for(|| state.get() == State::Normal);
    fixture.profile(1, "power-saver", false);
    wait_for(|| state.get() == State::Saver);
    fixture.own(1, false);
    wait_for(|| state.get() == State::Normal);
    fixture.own(0, false);
    wait_for(|| state.get() == State::Unavailable);
    fixture.profile(0, "power-saver", false);
    fixture.own(0, true);
    wait_for(|| state.get() == State::Saver);
    fixture.profile(0, "balanced", false);
    wait_for(|| state.get() == State::Normal);
    let reads = fixture.reads.get();
    for _ in 0..100 {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        fixture.reads.get(),
        reads,
        "steady-state monitoring must not poll"
    );
    assert_eq!(fixture.writes.get(), 0);
    fixture.stop_bus();
    wait_for(|| state.get() == State::Unavailable);
    let weak = Rc::downgrade(&monitor);
    drop(monitor);
    assert!(weak.upgrade().is_none());
    let output = state.clone();
    state.set(State::Checking);
    let broken = Monitor::start(Some(&fixture.address), move |value| output.set(value));
    wait_for(|| state.get() == State::Unavailable);
    drop(broken);
    let calls = Rc::new(Cell::new(0));
    let output = calls.clone();
    let cancelled = Monitor::start(Some(&fixture.address), move |_| {
        output.set(output.get() + 1)
    });
    let weak = Rc::downgrade(&cancelled);
    drop(cancelled);
    assert!(weak.upgrade().is_none());
    for _ in 0..50 {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(calls.get(), 0, "destroyed monitor received a late callback");
    println!(
        "private power bus: absent, legacy, saver, invalidation, modern, unknown, conflict, owner loss/restart and bus disconnect passed; writes=0; idle polls=0"
    );
}

#[test]
fn missing_bus_address_is_conservative_without_a_timer() {
    let state = Rc::new(Cell::new(State::Checking));
    let output = state.clone();
    let monitor = Monitor::start(None, move |value| output.set(value));
    assert_eq!(state.get(), State::Unavailable);
    assert!(monitor.deadline.borrow().is_none());
    assert!(monitor.connection.borrow().is_none());
}

#[test]
#[ignore = "explicit read-only host power-profile probe; not an isolated unit test"]
fn host_power_profile_probe_does_not_change_system_policy() {
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let state = Rc::new(Cell::new(State::Checking));
    let output = state.clone();
    let monitor = Monitor::system(move |value| output.set(value));
    wait_for(|| state.get() != State::Checking);
    assert!(
        matches!(state.get(), State::Normal | State::Saver),
        "host has no readable power-profile service"
    );
    println!(
        "read-only host power profile: {:?}; no Set/HoldProfile/ReleaseProfile calls",
        state.get()
    );
    drop(monitor);
}
