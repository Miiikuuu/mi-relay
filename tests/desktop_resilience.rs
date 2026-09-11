#![cfg(all(feature = "desktop", target_os = "linux"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use mirelay::bridge_registry::{BridgeRegistration, BridgeRegistryStore};
use mirelay::config::{Config, InitOverrides, ServerConfig};
use mirelay::source::FilesystemSource;
use mirelay::state::StateStore;
use mirelay::sync::sync_once;

const DESKTOP: &str = env!("CARGO_BIN_EXE_mirelay-desktop");

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    root: tempfile::TempDir,
    registry: PathBuf,
    protected: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("folders.toml");
        let store = BridgeRegistryStore::new(registry.clone());
        let mut protected = vec![registry.clone()];
        for id in ["one", "two"] {
            let data = root.path().join(id);
            let config = Config::defaults(InitOverrides {
                data_dir: Some(data.clone()),
                ..Default::default()
            })
            .unwrap();
            config.ensure_directories().unwrap();
            let path = data.join("config.toml");
            config.save(&path, false).unwrap();
            let input = data.join("sample.txt");
            fs::write(&input, format!("isolated GUI resilience fixture {id}\n")).unwrap();
            let ServerConfig::Filesystem { inbox_dir } = &config.server else {
                unreachable!()
            };
            let source = FilesystemSource::new(inbox_dir.clone());
            source
                .enqueue(&input, config.limits.max_file_size_bytes)
                .unwrap();
            assert!(sync_once(&config, &source).unwrap().failures.is_empty());
            let state = StateStore::new(config.storage.state_file.clone())
                .load()
                .unwrap();
            protected.extend([path.clone(), config.storage.state_file.clone()]);
            protected.extend(
                state
                    .deliveries
                    .into_values()
                    .map(|record| record.stored_path),
            );
            store
                .update(|registry| {
                    registry.add(BridgeRegistration {
                        kind: Default::default(),
                        id: id.into(),
                        name: id.into(),
                        config_path: path,
                        auto_receive: false,
                    })
                })
                .unwrap();
        }
        store.update(|registry| registry.select("one")).unwrap();
        Self {
            root,
            registry,
            protected,
        }
    }

    fn bytes(&self) -> Vec<Vec<u8>> {
        self.protected
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect()
    }

    fn command(&self) -> Command {
        let mut command = Command::new(DESKTOP);
        command
            .arg("--new-instance")
            .arg("--registry")
            .arg(&self.registry)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn checked(&self, args: &[&str]) -> std::process::Output {
        let mut command = self.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        assert_cmd::Command::from_std(command)
            .args(args)
            .timeout(Duration::from_secs(20))
            .output()
            .unwrap()
    }
}

#[test]
fn invalid_desktop_arguments_fail_before_creating_files() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["--new-instance"],
        vec!["--file-smoke-test"],
        vec!["--sidebar-smoke-test"],
        vec!["--screenshot-settings"],
        vec!["--screenshot-file-activity"],
        vec!["--screenshot-width", "819", "--screenshot", "x.png"],
        vec!["--screenshot-width", "2401", "--screenshot", "x.png"],
        vec![
            "--screenshot-sidebar-menu",
            "bogus",
            "--screenshot",
            "x.png",
        ],
        vec!["--screenshot-file-filter", "bogus", "--screenshot", "x.png"],
        vec![
            "--registry",
            "folders.toml",
            "--file-smoke-test",
            "--screenshot",
            "x.png",
        ],
        vec![
            "--registry",
            "folders.toml",
            "--file-smoke-test",
            "--automation-smoke-test",
        ],
        vec![
            "--registry",
            "folders.toml",
            "--sidebar-smoke-test",
            "--file-smoke-test",
        ],
        vec!["--unknown-option"],
    ] {
        assert_cmd::Command::new(DESKTOP)
            .current_dir(root.path())
            .args(&args)
            .timeout(Duration::from_secs(5))
            .assert()
            .code(2);
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            0,
            "invalid arguments wrote files: {args:?}"
        );
    }
}

#[test]
#[ignore = "requires a graphical session; kills only isolated child instances"]
fn desktop_sigkill_at_four_startup_timings_preserves_files_and_reopens() {
    let fixture = Fixture::new();
    let before = fixture.bytes();
    for delay_ms in [0, 30, 150, 500] {
        let mut child = TestChild(fixture.command().spawn().unwrap());
        thread::sleep(Duration::from_millis(delay_ms));
        assert!(child.0.try_wait().unwrap().is_none());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert_eq!(
            fixture.bytes(),
            before,
            "startup termination damaged data at {delay_ms} ms"
        );
    }
    for mode in ["--file-smoke-test", "--sidebar-smoke-test"] {
        assert!(
            fixture.checked(&[mode]).status.success(),
            "restart smoke failed: {mode}"
        );
        assert_eq!(fixture.bytes(), before);
    }
}

fn process_sample(pid: u32) -> (u64, u64) {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    let ticks = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let rss = status
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .unwrap();
    (ticks, rss)
}

#[test]
#[ignore = "requires a graphical session; idle CPU/RSS diagnostic, not a hardware-independent budget"]
fn desktop_idle_cpu_and_memory_sample() {
    let fixture = Fixture::new();
    let mut child = TestChild(fixture.command().spawn().unwrap());
    thread::sleep(Duration::from_secs(2));
    assert!(child.0.try_wait().unwrap().is_none());
    let ticks_per_second = String::from_utf8(
        Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse::<f64>()
    .unwrap();
    let before = process_sample(child.0.id());
    let started = Instant::now();
    thread::sleep(Duration::from_secs(8));
    let after = process_sample(child.0.id());
    println!(
        "QA_IDLE {}",
        serde_json::json!({
            "seconds": started.elapsed().as_secs_f64(),
            "cpu_percent_one_core": (after.0 - before.0) as f64 / ticks_per_second / started.elapsed().as_secs_f64() * 100.0,
            "rss_before_kib": before.1, "rss_after_kib": after.1,
            "scope": "idle debug GTK window, two Folders, no active transfers"
        })
    );
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
#[ignore = "requires a graphical session; validates broken-config isolation"]
fn corrupt_folder_config_does_not_crash_desktop_or_get_overwritten() {
    let fixture = Fixture::new();
    let bad_config = fixture.root.path().join("one/config.toml");
    fs::write(&bad_config, b"this is not valid TOML [{\n").unwrap();
    let before = fixture.bytes();
    let screenshot = fixture.root.path().join("broken-folder.png");
    let output = fixture.checked(&["--screenshot", screenshot.to_str().unwrap()]);
    assert!(output.status.success());
    assert!(fs::metadata(screenshot).unwrap().len() > 0);
    assert_eq!(fixture.bytes(), before);
}

#[test]
#[ignore = "GUI negative diagnostic: screenshot I/O failure should produce a failing exit code"]
fn screenshot_write_failure_must_not_report_success() {
    let fixture = Fixture::new();
    let missing_parent = fixture.root.path().join("missing/output.png");
    let output = fixture.checked(&["--screenshot", missing_parent.to_str().unwrap()]);
    assert!(!Path::new(&missing_parent).exists());
    assert!(
        output.status.code().is_some(),
        "screenshot I/O failure crashed the process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "screenshot creation failed but the process reported success"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to capture desktop window"));
    // An existing directory is also a write error, not a destination to remove.
    let output = fixture.checked(&["--screenshot", fixture.root.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(fixture.root.path().is_dir());
    // Successful texture saving still works after releasing the renderer.
    let valid = fixture.root.path().join("after-error.png");
    assert!(
        fixture
            .checked(&["--screenshot", valid.to_str().unwrap()])
            .status
            .success()
    );
    assert!(fs::read(valid).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
}
