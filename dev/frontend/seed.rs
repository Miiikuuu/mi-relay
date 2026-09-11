//! Local-only UI samples, deliberately separate from normal MiRelay data.
use std::fs::{self, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use fs2::FileExt;
use mirelay::bridge_registry::{BridgeRegistration, BridgeRegistry, BridgeRegistryStore};
use mirelay::config::{Config, InitOverrides, ServerConfig};
use mirelay::fsutil::{atomic_write, create_dir_all_durable};
use mirelay::model::{DeliveryStatus, WallpaperStatus};
use mirelay::source::FilesystemSource;
use mirelay::state::StateStore;
use mirelay::sync::sync_once;

// A valid 1x1 PNG; visual placeholders, not artwork or external downloads.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";
const FOLDERS: &[(&str, &str)] = &[
    ("documents", "Documents"),
    ("illustrations", "Illustrations"),
    ("empty", "Empty Folder"),
    (
        "long-name",
        "Design references and research — a deliberately long Folder name",
    ),
];

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = args.next().context("usage: frontend-fixture PATH")?;
    let directory = args.next();
    ensure!(
        directory
            .as_deref()
            .is_none_or(|value| value == "--directory"),
        "expected optional --directory"
    );
    ensure!(args.next().is_none(), "unexpected fixture argument");
    prepare(Path::new(&root), directory.is_some())
}

fn prepare(root: &Path, directory: bool) -> Result<()> {
    create_dir_all_durable(root)?;
    let root = root.canonicalize()?;
    // Serialize initializers; never reset an existing workspace or partial seed.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".seed.lock"))?;
    FileExt::lock_exclusive(&lock)?;
    let registry_store = BridgeRegistryStore::new(root.join("folders.toml"));
    if registry_store.path().try_exists()? {
        registry_store.load()?;
        println!("Using existing frontend workspace: {}", root.display());
        return Ok(());
    }
    for entry in fs::read_dir(&root)? {
        if entry?.file_name() != ".seed.lock" {
            bail!(
                "refusing to overwrite non-empty workspace {}; use --workspace with a new name",
                root.display()
            );
        }
    }

    create_dir_all_durable(&root.join("screenshots"))?;
    let mut registry = BridgeRegistry::default();
    for &(id, name) in FOLDERS {
        let folder_root = root.join("folders").join(id);
        let config = Config::defaults(InitOverrides {
            data_dir: Some(folder_root.clone()),
            ..Default::default()
        })?;
        config.ensure_directories()?;
        let config_path = folder_root.join("config.toml");
        config.save(&config_path, false)?;
        seed_folder(&config, &folder_root, id)?;
        registry.add(BridgeRegistration {
            kind: if id == "illustrations" {
                mirelay::bridge_registry::FolderKind::Photos
            } else {
                Default::default()
            },
            id: id.into(),
            name: name.into(),
            config_path,
            auto_receive: false,
        })?;
    }
    registry.select("documents")?;
    if directory {
        seed_directory(&root, &mut registry)?;
    }
    BridgeRegistryStore::new(root.join("empty.toml")).update(|_| Ok(()))?;
    registry_store.update(|current| {
        *current = registry;
        Ok(())
    })?;
    println!("Created local-only frontend workspace: {}", root.display());
    Ok(())
}

fn seed_directory(root: &Path, registry: &mut BridgeRegistry) -> Result<()> {
    use mirelay::directory::{DirectoryEntry, receiver::Receiver};
    use sha2::{Digest, Sha256};
    let data = root.join("folders/directory-sync");
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(data.clone()),
        ..Default::default()
    })?;
    config.ensure_directories()?;
    config.directory_sync = true;
    let url = format!("http://127.0.0.1:9/f/{}", uuid::Uuid::new_v4());
    config.server = serde_json::from_value(
        serde_json::json!({"kind":"http","base_url":url,"token_env":"MIRELAY_PREVIEW_UNUSED_TOKEN","allow_insecure_http":true}),
    )?;
    let path = data.join("config.toml");
    config.save(&path, false)?;
    create_dir_all_durable(&config.storage.library_dir.join("Documents"))?;
    atomic_write(
        &config.storage.library_dir.join("Documents/notes.txt"),
        b"Local copy to keep",
    )?;
    let state = mirelay::config::directory_state_dir(&config);
    let mut receiver = Receiver::open(&config.storage.library_dir, &state, &url)?;
    for (index, (name, mime, bytes)) in [
        ("Pixiv/original.png", "image/png", STANDARD.decode(PNG)?),
        (
            "Documents/notes.txt",
            "text/plain",
            b"Updated from source".to_vec(),
        ),
        (
            "Archive/received.bin",
            "application/octet-stream",
            vec![0, 255, 1, 0, 128],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        receiver.apply(
            &DirectoryEntry {
                path: name.into(),
                version: index as u64 + 1,
                delivery_id: uuid::Uuid::new_v4().to_string(),
                sha256: hex::encode(Sha256::digest(&bytes)),
                size: bytes.len() as u64,
                media_type: mime.into(),
                acknowledged: false,
                conflict: false,
            },
            Box::new(std::io::Cursor::new(bytes)),
        )?;
    }
    drop(receiver);
    // Display fixtures only. No service is listening or contacted, and auto is
    // disabled. Deliberately retain one unacknowledged record for UI coverage.
    let ledger = state.join("receiver.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&ledger)?)?;
    value["files"]["Pixiv/original.png"]["acknowledged"] = true.into();
    value["files"]["Documents/notes.txt"]["acknowledged"] = true.into();
    atomic_write(&ledger, &serde_json::to_vec_pretty(&value)?)?;
    registry.add(BridgeRegistration {
        kind: Default::default(),
        id: "directory-sync".into(),
        name: "Shared files".into(),
        config_path: path,
        auto_receive: false,
    })?;
    Ok(())
}

fn seed_folder(config: &Config, root: &Path, id: &str) -> Result<()> {
    let ServerConfig::Filesystem { inbox_dir } = &config.server else {
        bail!("preview fixtures must use a local filesystem source");
    };
    let source = FilesystemSource::new(inbox_dir.clone());
    let samples: Vec<(&str, Vec<u8>)> = match id {
        "documents" => vec![
            (
                "Getting started.md",
                b"# MiRelay preview\nLocal UI sample.\n".to_vec(),
            ),
            (
                "Transfer notes.csv",
                b"file,status\nnotes,received\n".to_vec(),
            ),
            (
                "Awaiting acknowledgement.txt",
                b"Synthetic pending ACK display state.\n".to_vec(),
            ),
            ("Retry example.bin", vec![0, 1, 2, 255]),
        ],
        "illustrations" => vec![
            (
                "Wallpaper sample.png",
                include_bytes!("../../assets/brand/MiRelay-brand-kit-v1/icons/png/mirelay-512.png")
                    .to_vec(),
            ),
            ("Wallpaper failure.png", STANDARD.decode(PNG)?),
        ],
        "long-name" => vec![(
            "Research notes — 跨设备传输 — a deliberately long filename for layout testing.md",
            b"# Layout stress test\nCheck truncation, tooltips and narrow windows.\n".to_vec(),
        )],
        _ => vec![],
    };
    create_dir_all_durable(&root.join("samples"))?;
    for (name, bytes) in &samples {
        let path = root.join("samples").join(name);
        atomic_write(&path, bytes)?;
        source.enqueue(&path, config.limits.max_file_size_bytes)?;
    }
    // Reuse real hashing, storage and acknowledgement. Only UI statuses below are mocked.
    let summary = sync_once(config, &source)?;
    ensure!(
        summary.failures.is_empty(),
        "fixture receive failed: {:?}",
        summary.failures
    );
    let store = StateStore::new(config.storage.state_file.clone());
    let _guard = store.lock_exclusive()?;
    let mut state = store.load()?;
    for record in state.deliveries.values_mut() {
        let index = samples
            .iter()
            .position(|(name, _)| *name == record.original_name)
            .unwrap();
        // Stable ordering and timestamps make screenshots easier to compare.
        record.source_created_at_unix = Some(1_783_000_000 + index as u64 * 60);
        record.received_at_unix = 1_783_000_001 + index as u64 * 60;
        record.acknowledged_at_unix = Some(record.received_at_unix + 1);
        match record.original_name.as_str() {
            "Awaiting acknowledgement.txt" | "Retry example.bin" => {
                record.delivery_status = DeliveryStatus::AckPending;
                record.acknowledged_at_unix = None;
                if record.original_name == "Retry example.bin" {
                    record.delivery_error = Some(
                        "Preview: acknowledgement timed out. Receive retries this local sample."
                            .into(),
                    );
                }
            }
            "Wallpaper failure.png" => {
                record.wallpaper_status = WallpaperStatus::Failed;
                record.wallpaper_attempts = 1;
                record.wallpaper_error = Some(
                    "Preview: wallpaper command failed. No command was actually executed.".into(),
                );
            }
            _ => {}
        }
    }
    store.save(&state)?;
    // Leave one genuine incoming file so Receive also exercises the normal local workflow.
    if id == "documents" {
        let path = root.join("samples/New arrival.txt");
        atomic_write(
            &path,
            b"This file was waiting in the local preview inbox.\n",
        )?;
        source.enqueue(&path, config.limits.max_file_size_bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirelay::storage::inspect_file;
    use std::collections::HashSet;

    #[test]
    fn sample_png_is_decodable() -> Result<()> {
        let png = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(STANDARD.decode(PNG)?))?;
        assert_eq!((png.width(), png.height()), (1, 1));
        Ok(())
    }

    #[test]
    fn fixtures_are_isolated_valid_and_reusable() -> Result<()> {
        let root = tempfile::tempdir()?;
        prepare(root.path(), false)?;
        let registry_path = root.path().join("folders.toml");
        let store = BridgeRegistryStore::new(registry_path.clone());
        let registry = store.load()?;
        assert_eq!(registry.bridges.len(), 4);
        assert_eq!(registry.selected_bridge_id.as_deref(), Some("documents"));
        let mut devices = HashSet::new();
        let mut files = 0;
        for folder in &registry.bridges {
            assert!(!folder.auto_receive);
            let config = Config::load(&folder.config_path)?;
            assert!(devices.insert(config.device_id.clone()));
            assert!(config.storage.library_dir.starts_with(root.path()));
            assert!(config.wallpaper.command.is_empty());
            let ServerConfig::Filesystem { inbox_dir } = &config.server else {
                panic!("network source")
            };
            assert!(inbox_dir.starts_with(root.path()));
            let state = StateStore::new(config.storage.state_file.clone()).load()?;
            for record in state.deliveries.values() {
                let actual = inspect_file(&record.stored_path, config.limits.max_file_size_bytes)?;
                assert_eq!(record.sha256, actual.sha256);
                assert_eq!(record.size, actual.size);
                files += 1;
            }
            if folder.id == "documents" {
                assert_eq!(state.deliveries.len(), 4);
                assert_eq!(
                    state
                        .deliveries
                        .values()
                        .filter(|r| r.delivery_status == DeliveryStatus::AckPending)
                        .count(),
                    2
                );
                let summary = sync_once(&config, &FilesystemSource::new(inbox_dir.clone()))?;
                assert!(summary.failures.is_empty());
                assert_eq!(summary.received, 1);
                assert_eq!(summary.acknowledged, 3);
            }
            if folder.id == "empty" {
                assert!(state.deliveries.is_empty());
            }
        }
        assert_eq!(files, 7);
        // Re-launch must preserve edits, including an intentionally emptied Folder list.
        store.update(|registry| {
            *registry = BridgeRegistry::default();
            Ok(())
        })?;
        let before = fs::read(&registry_path)?;
        let state_path = root.path().join("folders/documents/state.json");
        let state_before = fs::read(&state_path)?;
        prepare(root.path(), false)?;
        assert_eq!(before, fs::read(registry_path)?);
        assert_eq!(state_before, fs::read(state_path)?);
        assert!(
            BridgeRegistryStore::new(root.path().join("empty.toml"))
                .load()?
                .bridges
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn partial_or_unrelated_workspace_is_not_overwritten() -> Result<()> {
        let root = tempfile::tempdir()?;
        let existing = root.path().join("keep.txt");
        atomic_write(&existing, b"keep me")?;
        assert!(
            prepare(root.path(), false)
                .unwrap_err()
                .to_string()
                .contains("refusing to overwrite")
        );
        assert_eq!(fs::read(existing)?, b"keep me");
        assert!(!root.path().join("folders.toml").exists());
        Ok(())
    }
}
