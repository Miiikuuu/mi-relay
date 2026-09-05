use std::fs;

use mirelay::config::{Config, InitOverrides, ServerConfig};
use mirelay::model::AppState;
use mirelay::source::{DeliverySource, FilesystemSource};
use mirelay::state::StateStore;
use mirelay::sync::sync_once;

struct Fixture {
    _root: tempfile::TempDir,
    config: Config,
    source: FilesystemSource,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.storage.state_file = root.path().join("state/state.json");
        config.ensure_directories().unwrap();
        let ServerConfig::Filesystem { inbox_dir } = &config.server else {
            unreachable!()
        };
        let source = FilesystemSource::new(inbox_dir.clone());
        let input = root.path().join("input.txt");
        fs::write(&input, b"input guard fixture\n").unwrap();
        source
            .enqueue(&input, config.limits.max_file_size_bytes)
            .unwrap();
        let store = StateStore::new(config.storage.state_file.clone());
        let _lock = store.lock_exclusive().unwrap();
        store.save(&AppState::default()).unwrap();
        Self {
            _root: root,
            config,
            source,
        }
    }

    fn assert_unacknowledged(&self) {
        assert!(!self.source.inbox_dir().join(".acks").exists());
        assert_eq!(
            self.source
                .scan_pending(self.config.limits.max_file_size_bytes)
                .unwrap()
                .deliveries
                .len(),
            1
        );
    }
}

#[test]
fn malformed_or_unsupported_state_is_preserved_without_acknowledging() {
    let fixture = Fixture::new();
    for bytes in [
        b"{broken json".as_slice(),
        br#"{"schema_version":999,"deliveries":{}}"#,
        br#"{"schema_version":1,"deliveries":{},"unknown_field":true}"#,
        b"\xff\xfe\x00",
        br#"{"schema_version":1,"deliveries":[]}"#,
    ] {
        fs::write(&fixture.config.storage.state_file, bytes).unwrap();
        assert!(sync_once(&fixture.config, &fixture.source).is_err());
        assert_eq!(fs::read(&fixture.config.storage.state_file).unwrap(), bytes);
        fixture.assert_unacknowledged();
    }
}

#[test]
fn oversized_state_is_rejected_without_rewriting_it() {
    let fixture = Fixture::new();
    let size = 64 * 1024 * 1024 + 1;
    fs::File::options()
        .write(true)
        .open(&fixture.config.storage.state_file)
        .unwrap()
        .set_len(size)
        .unwrap();
    assert!(sync_once(&fixture.config, &fixture.source).is_err());
    assert_eq!(
        fs::metadata(&fixture.config.storage.state_file)
            .unwrap()
            .len(),
        size
    );
    fixture.assert_unacknowledged();
}

#[test]
fn oversized_config_and_invalid_file_limit_fail_before_receiving() {
    let mut fixture = Fixture::new();
    let path = fixture._root.path().join("oversized.toml");
    fs::File::create(&path)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    assert!(Config::load(&path).is_err());
    fixture.config.limits.max_file_size_bytes = 0;
    assert!(sync_once(&fixture.config, &fixture.source).is_err());
    assert!(!fixture.source.inbox_dir().join(".acks").exists());
}

#[cfg(unix)]
#[test]
fn symlinked_state_lock_is_rejected_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    fixture.config.storage.state_file = fixture._root.path().join("linked-state.json");
    let target = fixture._root.path().join("unrelated-user-file");
    fs::write(&target, b"must stay unchanged").unwrap();
    symlink(&target, fixture._root.path().join("linked-state.json.lock")).unwrap();
    assert!(sync_once(&fixture.config, &fixture.source).is_err());
    assert_eq!(fs::read(target).unwrap(), b"must stay unchanged");
    fixture.assert_unacknowledged();
}

#[cfg(unix)]
struct RestorePermissions(std::path::PathBuf, fs::Permissions);

#[cfg(unix)]
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, self.1.clone());
    }
}

#[cfg(unix)]
#[test]
fn unwritable_state_directory_never_acknowledges_and_can_recover() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let fixture = Fixture::new();
    if fs::metadata(&fixture.config.storage.state_file)
        .unwrap()
        .uid()
        == 0
    {
        eprintln!("permission test skipped: root bypasses ordinary mode-bit restrictions");
        return;
    }
    let directory = fixture.config.storage.state_file.parent().unwrap();
    let before = fs::read(&fixture.config.storage.state_file).unwrap();
    let restore = RestorePermissions(
        directory.into(),
        fs::metadata(directory).unwrap().permissions(),
    );
    fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
    let result = sync_once(&fixture.config, &fixture.source);
    assert!(result.is_err() || result.is_ok_and(|summary| !summary.failures.is_empty()));
    assert_eq!(
        fs::read(&fixture.config.storage.state_file).unwrap(),
        before
    );
    fixture.assert_unacknowledged();
    drop(restore);
    let resumed = sync_once(&fixture.config, &fixture.source).unwrap();
    assert!(resumed.failures.is_empty());
    assert_eq!(resumed.acknowledged, 1);
}
