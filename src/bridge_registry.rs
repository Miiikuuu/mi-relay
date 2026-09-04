use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use crate::config::default_config_path;
use crate::fsutil::{atomic_write, create_dir_all_durable, read_limited};

pub const BRIDGE_REGISTRY_SCHEMA_VERSION: u32 = 1;
const MAX_BRIDGES: usize = 256;
const MAX_REGISTRY_SIZE_BYTES: u64 = 1024 * 1024;
const MAX_BRIDGE_NAME_CHARS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeRegistration {
    pub id: String,
    pub name: String,
    pub config_path: PathBuf,
    #[serde(default)]
    pub auto_receive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeRegistry {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_bridge_id: Option<String>,
    #[serde(default)]
    pub bridges: Vec<BridgeRegistration>,
}

impl Default for BridgeRegistry {
    fn default() -> Self {
        Self {
            schema_version: BRIDGE_REGISTRY_SCHEMA_VERSION,
            selected_bridge_id: None,
            bridges: Vec::new(),
        }
    }
}

impl BridgeRegistry {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != BRIDGE_REGISTRY_SCHEMA_VERSION {
            bail!(
                "unsupported Folder registry schema version {}; expected {}",
                self.schema_version,
                BRIDGE_REGISTRY_SCHEMA_VERSION
            );
        }
        if self.bridges.len() > MAX_BRIDGES {
            bail!("Folder registry cannot contain more than {MAX_BRIDGES} folders");
        }

        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for bridge in &self.bridges {
            validate_id(&bridge.id)?;
            validate_name(&bridge.name)?;
            validate_config_path(&bridge.config_path)?;
            if !ids.insert(bridge.id.as_str()) {
                bail!("duplicate Folder ID {:?}", bridge.id);
            }
            if !paths.insert(bridge.config_path.as_path()) {
                bail!(
                    "multiple Folders reference configuration {}",
                    bridge.config_path.display()
                );
            }
        }

        if let Some(selected) = &self.selected_bridge_id
            && !ids.contains(selected.as_str())
        {
            bail!("selected Folder {:?} is not registered", selected);
        }
        Ok(())
    }

    pub fn add(&mut self, bridge: BridgeRegistration) -> Result<()> {
        let previous_selection = self.selected_bridge_id.clone();
        self.bridges.push(bridge);
        self.selected_bridge_id = self.bridges.last().map(|bridge| bridge.id.clone());
        if let Err(error) = self.validate() {
            self.bridges.pop();
            self.selected_bridge_id = previous_selection;
            return Err(error);
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<BridgeRegistration> {
        let index = self
            .bridges
            .iter()
            .position(|bridge| bridge.id == id)
            .with_context(|| format!("Folder {id:?} is not registered"))?;
        let removed = self.bridges.remove(index);
        if self.selected_bridge_id.as_deref() == Some(id) {
            self.selected_bridge_id = self
                .bridges
                .get(index.min(self.bridges.len().saturating_sub(1)))
                .map(|bridge| bridge.id.clone());
        }
        self.validate()?;
        Ok(removed)
    }

    pub fn rename(&mut self, id: &str, name: String) -> Result<()> {
        validate_name(&name)?;
        let bridge = self
            .bridges
            .iter_mut()
            .find(|bridge| bridge.id == id)
            .with_context(|| format!("Folder {id:?} is not registered"))?;
        let previous = std::mem::replace(&mut bridge.name, name);
        if let Err(error) = self.validate() {
            let bridge = self
                .bridges
                .iter_mut()
                .find(|bridge| bridge.id == id)
                .expect("renamed Folder must remain present");
            bridge.name = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn set_auto_receive(&mut self, id: &str, enabled: bool) -> Result<()> {
        let bridge = self
            .bridges
            .iter_mut()
            .find(|bridge| bridge.id == id)
            .with_context(|| format!("Folder {id:?} is not registered"))?;
        bridge.auto_receive = enabled;
        Ok(())
    }

    pub fn select(&mut self, id: &str) -> Result<()> {
        if !self.bridges.iter().any(|bridge| bridge.id == id) {
            bail!("Folder {id:?} is not registered");
        }
        self.selected_bridge_id = Some(id.to_owned());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BridgeRegistryStore {
    path: PathBuf,
    lock_path: PathBuf,
}

impl BridgeRegistryStore {
    pub fn new(path: PathBuf) -> Self {
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".lock");
        Self {
            path,
            lock_path: PathBuf::from(lock_name),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<BridgeRegistry> {
        let lock = self.open_lock_file()?;
        FileExt::lock_shared(&lock).with_context(|| {
            format!(
                "failed to acquire Folder registry lock {}",
                self.lock_path.display()
            )
        })?;
        self.load_unlocked()
    }

    pub fn update<T>(&self, update: impl FnOnce(&mut BridgeRegistry) -> Result<T>) -> Result<T> {
        let lock = self.open_lock_file()?;
        FileExt::lock_exclusive(&lock).with_context(|| {
            format!(
                "failed to acquire exclusive Folder registry lock {}",
                self.lock_path.display()
            )
        })?;
        let mut registry = self.load_unlocked()?;
        let result = update(&mut registry)?;
        registry.validate()?;
        self.save_unlocked(&registry)?;
        Ok(result)
    }

    fn load_unlocked(&self) -> Result<BridgeRegistry> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    bail!(
                        "Folder registry {} is not a regular file",
                        self.path.display()
                    );
                }
                #[cfg(unix)]
                if metadata.nlink() != 1 {
                    bail!(
                        "Folder registry {} has multiple hard links",
                        self.path.display()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BridgeRegistry::default());
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect Folder registry {}", self.path.display())
                });
            }
        }

        let raw = read_limited(&self.path, MAX_REGISTRY_SIZE_BYTES)
            .with_context(|| format!("failed to read Folder registry {}", self.path.display()))?;
        let raw = String::from_utf8(raw)
            .with_context(|| format!("Folder registry {} is not UTF-8", self.path.display()))?;
        let registry: BridgeRegistry = toml::from_str(&raw)
            .with_context(|| format!("failed to parse Folder registry {}", self.path.display()))?;
        registry.validate()?;
        Ok(registry)
    }

    fn save_unlocked(&self, registry: &BridgeRegistry) -> Result<()> {
        registry.validate()?;
        let mut raw = String::from("# MiRelay Linux desktop Folder registry.\n");
        raw.push_str(&toml::to_string_pretty(registry).context("failed to serialize registry")?);
        if raw.len() as u64 > MAX_REGISTRY_SIZE_BYTES {
            bail!(
                "refusing to grow Folder registry beyond {} bytes",
                MAX_REGISTRY_SIZE_BYTES
            );
        }
        atomic_write(&self.path, raw.as_bytes())
            .with_context(|| format!("failed to save Folder registry {}", self.path.display()))
    }

    fn open_lock_file(&self) -> Result<File> {
        if let Some(parent) = self.lock_path.parent() {
            create_dir_all_durable(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options.open(&self.lock_path).with_context(|| {
            format!(
                "failed to open Folder registry lock {}",
                self.lock_path.display()
            )
        })?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            bail!(
                "Folder registry lock {} is not a regular file",
                self.lock_path.display()
            );
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            bail!(
                "Folder registry lock {} has multiple hard links",
                self.lock_path.display()
            );
        }
        Ok(file)
    }
}

pub fn default_registry_path() -> Result<PathBuf> {
    let config_path = default_config_path()?;
    let parent = config_path
        .parent()
        .context("default config path has no parent")?;
    Ok(parent.join("bridges.toml"))
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("Folder ID must contain 1-128 ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name != name.trim()
        || name.chars().count() > MAX_BRIDGE_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        bail!("Folder name must be trimmed, non-empty, and at most 128 characters");
    }
    Ok(())
}

fn validate_config_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!(
            "Folder configuration path must be absolute: {}",
            path.display()
        );
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!(
            "Folder configuration path must not contain '.' or '..': {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(root: &Path, id: &str, name: &str) -> BridgeRegistration {
        BridgeRegistration {
            id: id.to_owned(),
            name: name.to_owned(),
            config_path: root.join(format!("{id}.toml")),
            auto_receive: false,
        }
    }

    #[test]
    fn missing_registry_loads_empty_and_updates_atomically() {
        let root = tempfile::tempdir().unwrap();
        let store = BridgeRegistryStore::new(root.path().join("config/bridges.toml"));
        assert_eq!(store.load().unwrap(), BridgeRegistry::default());

        store
            .update(|registry| registry.add(bridge(root.path(), "one", "Illustration")))
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.bridges.len(), 1);
        assert_eq!(loaded.selected_bridge_id.as_deref(), Some("one"));
    }

    #[test]
    fn automatic_receive_flag_is_backward_compatible_and_persistent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("folders.toml");
        let config_path = root.path().join("one.toml");
        fs::write(
            &path,
            format!(
                "schema_version = 1\nselected_bridge_id = \"one\"\n\n[[bridges]]\nid = \"one\"\nname = \"One\"\nconfig_path = {:?}\n",
                config_path.to_string_lossy()
            ),
        )
        .unwrap();
        let store = BridgeRegistryStore::new(path.clone());

        let legacy = store.load().unwrap();
        assert!(!legacy.bridges[0].auto_receive);
        store
            .update(|registry| registry.set_auto_receive("one", true))
            .unwrap();

        assert!(store.load().unwrap().bridges[0].auto_receive);
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("auto_receive = true")
        );
    }

    #[test]
    fn duplicate_ids_and_config_paths_are_rejected_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        let mut registry = BridgeRegistry::default();
        let original = bridge(root.path(), "one", "First");
        registry.add(original.clone()).unwrap();

        let mut duplicate_id = bridge(root.path(), "one", "Second");
        duplicate_id.config_path = root.path().join("second.toml");
        assert!(registry.add(duplicate_id).is_err());
        assert_eq!(registry.bridges, vec![original.clone()]);

        let duplicate_path = BridgeRegistration {
            id: "two".to_owned(),
            name: "Second".to_owned(),
            config_path: original.config_path.clone(),
            auto_receive: false,
        };
        assert!(registry.add(duplicate_path).is_err());
        assert_eq!(registry.bridges, vec![original]);
    }

    #[test]
    fn corrupt_registry_is_never_overwritten_by_update() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bridges.toml");
        fs::write(&path, b"not valid = [toml\n").unwrap();
        let before = fs::read(&path).unwrap();
        let store = BridgeRegistryStore::new(path.clone());

        assert!(store.update(|_| Ok(())).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn removal_selects_a_surviving_bridge_and_keeps_files_untouched() {
        let root = tempfile::tempdir().unwrap();
        let first = bridge(root.path(), "one", "First");
        let second = bridge(root.path(), "two", "Second");
        fs::write(&first.config_path, b"first").unwrap();
        let mut registry = BridgeRegistry::default();
        registry.add(first.clone()).unwrap();
        registry.add(second.clone()).unwrap();

        let removed = registry.remove("two").unwrap();
        assert_eq!(removed, second);
        assert_eq!(registry.selected_bridge_id.as_deref(), Some("one"));
        assert_eq!(fs::read(first.config_path).unwrap(), b"first");
    }

    #[test]
    fn invalid_selection_and_path_traversal_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut registry = BridgeRegistry::default();
        assert!(registry.select("missing").is_err());
        assert!(
            registry
                .add(BridgeRegistration {
                    id: "one".to_owned(),
                    name: "One".to_owned(),
                    config_path: root.path().join("folder/../one.toml"),
                    auto_receive: false,
                })
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn registry_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target.toml");
        fs::write(&target, "schema_version = 1\nbridges = []\n").unwrap();
        let link = root.path().join("bridges.toml");
        symlink(&target, &link).unwrap();
        let store = BridgeRegistryStore::new(link);
        assert!(store.load().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn registry_hard_link_is_rejected_without_replacing_either_name() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bridges.toml");
        let alias = root.path().join("alias.toml");
        let store = BridgeRegistryStore::new(path.clone());
        store
            .update(|registry| registry.add(bridge(root.path(), "one", "One")))
            .unwrap();
        fs::hard_link(&path, &alias).unwrap();
        let before = fs::read(&path).unwrap();

        assert!(store.load().is_err());
        assert!(store.update(|_| Ok(())).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(fs::read(alias).unwrap(), before);
    }
}
