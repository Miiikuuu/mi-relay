use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const STATE_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub schema_version: u32,
    pub id: String,
    pub original_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub payload: String,
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
    #[serde(default)]
    pub created_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    AckPending,
    Acknowledged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperStatus {
    Pending,
    NotApplicable,
    NotConfigured,
    Running,
    Applied,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRecord {
    pub id: String,
    pub original_name: String,
    pub media_type: String,
    pub sha256: String,
    pub size: u64,
    pub stored_path: PathBuf,
    pub delivery_status: DeliveryStatus,
    pub wallpaper_status: WallpaperStatus,
    pub wallpaper_attempts: u32,
    #[serde(default)]
    pub delivery_error: Option<String>,
    #[serde(default)]
    pub wallpaper_error: Option<String>,
    #[serde(default)]
    pub source_created_at_unix: Option<u64>,
    pub received_at_unix: u64,
    #[serde(default)]
    pub acknowledged_at_unix: Option<u64>,
    #[serde(default)]
    pub imported_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppState {
    pub schema_version: u32,
    pub deliveries: BTreeMap<String, DeliveryRecord>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            deliveries: BTreeMap::new(),
        }
    }
}
