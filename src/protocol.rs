use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const PROTOCOL_HEADER: &str = "mirelay-protocol-version";

/// A Folder is a server identity, not a local path or display name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderHandshake {
    pub schema_version: u32,
    pub folder_id: String,
    pub name: String,
    pub role: String,
    pub state: String,
    pub verification: Option<String>,
    pub max_file_size_bytes: u64,
}

/// Minimal authenticated tombstone; retained so offline peers and lost replies
/// can finish cleanup without regaining transfer authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderExitStatus {
    pub schema_version: u32,
    pub folder_id: String,
    pub role: String,
    pub requested: bool,
    pub server_cleaned: bool,
    pub sender_cleaned: bool,
    pub receiver_cleaned: bool,
}

impl FolderExitStatus {
    pub fn complete(&self) -> bool {
        self.requested && self.server_cleaned && self.sender_cleaned && self.receiver_cleaned
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFolderRequest {
    pub name: String,
}

// Deliberately no Debug: these responses contain one-time credentials.
#[derive(Serialize, Deserialize)]
pub struct FolderCreated {
    pub folder_id: String,
    pub receiver_token: String,
    pub pairing_code: String,
    pub expires_at_unix: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimFolderRequest {
    pub pairing_code: String,
    pub sender_token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmFolderRequest {
    pub verification: String,
}

#[derive(Serialize, Deserialize)]
pub struct FolderInvitation {
    pub pairing_code: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct DeliveryIndex<T> {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryDescriptor {
    pub delivery_id: String,
    pub original_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeRequest {
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemDetails {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub fn validate_delivery_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("delivery id must be 1-128 ASCII letters, digits, '-' or '_': {id:?}");
    }
    Ok(())
}

pub fn validate_sha256(sha256: &str) -> Result<()> {
    if sha256.len() != 64
        || sha256 != sha256.to_ascii_lowercase()
        || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("SHA-256 must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}
