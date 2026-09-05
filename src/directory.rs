//! Opt-in, one-way directory synchronization. Legacy deliveries remain separate.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod client;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod receiver;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod sender;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Comparison {
    pub identical: usize,
    pub missing: Vec<String>,
    pub different: Vec<String>,
    pub destination_only: usize,
}
/// Path AND digest are the identity: identical bytes in different locations
/// are distinct files, not duplicate transfers that may be silently omitted.
pub fn compare(source: &Inventory, receiver: &Inventory) -> Result<Comparison> {
    source.validate()?;
    receiver.validate()?;
    let targets: std::collections::BTreeMap<_, _> = receiver
        .entries
        .iter()
        .map(|v| (v.path.as_str(), v))
        .collect();
    let mut result = Comparison {
        identical: 0,
        missing: Vec::new(),
        different: Vec::new(),
        destination_only: 0,
    };
    for entry in &source.entries {
        match targets.get(entry.path.as_str()) {
            None => result.missing.push(entry.path.clone()),
            Some(other) if other.sha256 == entry.sha256 && other.size == entry.size => {
                result.identical += 1
            }
            Some(_) => result.different.push(entry.path.clone()),
        }
    }
    let paths: BTreeSet<_> = source.entries.iter().map(|v| v.path.as_str()).collect();
    result.destination_only = targets
        .keys()
        .filter(|path| !paths.contains(**path))
        .count();
    // Reject file/directory collisions before confirmation or payload staging.
    let all: BTreeSet<_> = paths
        .union(&targets.keys().copied().collect())
        .copied()
        .collect();
    for path in &all {
        for (offset, _) in path.match_indices('/') {
            ensure!(
                !all.contains(&path[..offset]),
                "Source and destination disagree about a file/directory path. Resolve this before initialization."
            );
        }
    }
    Ok(result)
}
pub const VERSION: u32 = 1;
/// Include already-in-flight versions in the preview so an old pending upload
/// cannot later undo an apparently identical initialization baseline.
pub fn compare_remote(source: &Inventory, remote: &DirectoryState) -> Result<Comparison> {
    remote.validate()?;
    let receiver = remote.receiver.as_ref().ok_or_else(|| {
        anyhow::anyhow!("Run the Linux directory receiver once to publish its inventory first.")
    })?;
    let mut expected: std::collections::BTreeMap<_, _> = receiver
        .entries
        .iter()
        .cloned()
        .map(|e| (e.path.clone(), e))
        .collect();
    for pending in remote.entries.iter().filter(|e| !e.acknowledged) {
        expected.insert(
            pending.path.clone(),
            InventoryEntry {
                path: pending.path.clone(),
                sha256: pending.sha256.clone(),
                size: pending.size,
            },
        );
    }
    compare(
        source,
        &Inventory {
            id: receiver.id.clone(),
            entries: expected.into_values().collect(),
        },
    )
}
pub const MAX_ENTRIES: usize = 5_000;
pub const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectoryVersion {
    pub path: String,
    pub version: u64,
}
impl DirectoryVersion {
    pub fn validate(&self) -> Result<()> {
        validate_path(&self.path)?;
        ensure!(
            self.version > 0 && self.version < i64::MAX as u64,
            "Invalid directory version."
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventoryEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub id: String,
    pub entries: Vec<InventoryEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryUpdate {
    pub previous_id: Option<String>,
    pub inventory: Inventory,
}
impl Inventory {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            uuid::Uuid::parse_str(&self.id)?.to_string() == self.id,
            "Invalid inventory identity."
        );
        ensure!(
            self.entries.len() <= MAX_ENTRIES,
            "Directory exceeds 5,000 files."
        );
        let mut paths = BTreeSet::<&str>::new();
        for entry in &self.entries {
            validate_path(&entry.path)?;
            crate::protocol::validate_sha256(&entry.sha256)?;
            ensure!(
                entry.size <= i64::MAX as u64 && paths.insert(&entry.path),
                "Invalid or duplicate directory entry."
            );
        }
        // A file may not also be another entry's parent.
        for path in &paths {
            for (offset, _) in path.match_indices('/') {
                ensure!(
                    !paths.contains(&path[..offset]),
                    "File/directory path collision."
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub path: String,
    pub version: u64,
    pub delivery_id: String,
    pub sha256: String,
    pub size: u64,
    pub media_type: String,
    pub acknowledged: bool,
    pub conflict: bool,
}
impl DirectoryEntry {
    pub fn delivery(&self) -> crate::model::Delivery {
        crate::model::Delivery {
            schema_version: crate::model::MANIFEST_SCHEMA_VERSION,
            id: self.delivery_id.clone(),
            original_name: self.path.rsplit('/').next().unwrap_or_default().into(),
            payload: String::new(),
            size: self.size,
            sha256: self.sha256.clone(),
            media_type: self.media_type.clone(),
            created_at_unix: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryState {
    pub schema_version: u32,
    pub receiver: Option<Inventory>,
    pub entries: Vec<DirectoryEntry>,
}
impl DirectoryState {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == VERSION && self.entries.len() <= MAX_ENTRIES,
            "Unsupported or oversized directory state."
        );
        if let Some(inventory) = &self.receiver {
            inventory.validate()?;
        }
        let mut paths = BTreeSet::new();
        for entry in &self.entries {
            DirectoryVersion {
                path: entry.path.clone(),
                version: entry.version,
            }
            .validate()?;
            crate::protocol::validate_delivery_id(&entry.delivery_id)?;
            crate::storage::validate_delivery_metadata(&entry.delivery(), 100 * 1024 * 1024)?;
            ensure!(paths.insert(&entry.path), "Duplicate source path.");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryAck {
    pub path: String,
    pub version: u64,
    pub sha256: String,
    pub conflict: bool,
}

/// Portable path components, not URL paths or SAF document IDs. Never normalize
/// traversal into an apparently safe path or silently rename a source component.
pub fn validate_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 1024 && !path.contains('\\'),
        "Invalid relative path."
    );
    let parts: Vec<_> = path.split('/').collect();
    ensure!(parts.len() <= 17, "Directory nesting exceeds 16 levels.");
    for part in parts {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && !part.chars().any(char::is_control)
                && !part.to_ascii_lowercase().starts_with(".mirelay")
                && !part.contains(':'),
            "Unsafe or reserved directory component."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_never_silently_normalized() {
        for path in [
            "",
            "/root",
            "a//b",
            "a/../b",
            "./a",
            "a\\b",
            "C:/x",
            "a/\0b",
            ".mirelay.lock",
            "a/.MIRELAY-history",
        ] {
            assert!(validate_path(path).is_err(), "{path:?}");
        }
        assert!(validate_path("Pixiv/画师/ original.png").is_ok());
        assert!(validate_path(&"x/".repeat(18)).is_err());
    }
}
