use super::{
    ServerStore,
    api::{ApiError, ApiState, check_protocol},
    folders::{folder_store, identity, supplied_hash},
};
use crate::directory::*;
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

#[derive(Debug)]
pub(super) struct DirectoryConflict;
impl std::fmt::Display for DirectoryConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Initialize the receiver or refresh the conflicting directory version.")
    }
}
impl std::error::Error for DirectoryConflict {}

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/f/{folder_id}/api/v1/directory", get(state))
        .route("/f/{folder_id}/api/v1/directory/index", put(index))
        .route("/f/{folder_id}/api/v1/directory/ack", post(ack))
        .layer(DefaultBodyLimit::max(MAX_BODY))
}

async fn authorize(
    state: &ApiState,
    headers: &HeaderMap,
    id: String,
    role: Option<&str>,
) -> Result<String, ApiError> {
    let info = identity(state, id, supplied_hash(headers)?).await?;
    super::folders::require_connected(&info)?;
    check_protocol(headers)?;
    if role.is_some_and(|role| role != info.role) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_role",
            "This Folder credential cannot perform that operation.",
        ));
    }
    if info.state != "ready" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "pairing_pending",
            "Confirm pairing before directory initialization.",
        ));
    }
    Ok(format!("folder_{}", info.folder_id))
}
async fn state(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DirectoryState>, ApiError> {
    let device = authorize(&state, &headers, id, None).await?;
    let store = state.inner.store.clone();
    Ok(Json(
        folder_store(&state, "read directory", move || {
            store.directory_state(&device)
        })
        .await?,
    ))
}
async fn index(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(update): Json<InventoryUpdate>,
) -> Result<Json<Inventory>, ApiError> {
    let device = authorize(&state, &headers, id, Some("receiver")).await?;
    update.inventory.validate().map_err(|_| {
        ApiError::bad_request(
            "invalid_index",
            "Invalid, ambiguous or oversized directory inventory.",
        )
    })?;
    let store = state.inner.store.clone();
    let result = folder_store(&state, "publish directory inventory", move || {
        store.publish_directory(&device, &update)
    })
    .await?;
    result.map(Json).ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "stale_index",
            "The receiver inventory changed; refresh before publishing.",
        )
    })
}
async fn ack(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(ack): Json<DirectoryAck>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let device = authorize(&state, &headers, id, Some("receiver")).await?;
    DirectoryVersion {
        path: ack.path.clone(),
        version: ack.version,
    }
    .validate()
    .and_then(|_| crate::protocol::validate_sha256(&ack.sha256))
    .map_err(|_| ApiError::bad_request("invalid_receipt", "Invalid directory receipt."))?;
    let store = state.inner.store.clone();
    let accepted = folder_store(&state, "acknowledge directory version", move || {
        store.ack_directory(&device, &ack)
    })
    .await?;
    if !accepted {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "receipt_mismatch",
            "Directory version or digest did not match.",
        ));
    }
    Ok(Json(serde_json::json!({"acknowledged":true})))
}

impl ServerStore {
    pub fn directory_state(&self, device: &str) -> Result<DirectoryState> {
        let mut db = self.open_connection_unchecked()?;
        let tx = db.transaction()?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT inventory FROM directory_indexes WHERE device_id=?",
                [device],
                |r| r.get(0),
            )
            .optional()?;
        let receiver = raw.map(|raw| serde_json::from_str(&raw)).transpose()?;
        let mut stmt = tx.prepare("SELECT v.path,v.version,d.delivery_id,d.sha256,d.size_bytes,d.media_type,d.acknowledged_at_unix IS NOT NULL,v.conflict
            FROM directory_versions v JOIN deliveries d ON d.device_id=v.device_id AND d.delivery_id=v.delivery_id
            WHERE v.device_id=? AND NOT EXISTS (SELECT 1 FROM directory_versions later WHERE later.device_id=v.device_id AND later.path=v.path AND later.version>v.version)
            ORDER BY v.path LIMIT 5001")?;
        let entries = stmt
            .query_map([device], |r| {
                Ok(DirectoryEntry {
                    path: r.get(0)?,
                    version: r.get::<_, i64>(1)? as u64,
                    delivery_id: r.get(2)?,
                    sha256: r.get(3)?,
                    size: r.get::<_, i64>(4)? as u64,
                    media_type: r.get(5)?,
                    acknowledged: r.get(6)?,
                    conflict: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let state = DirectoryState {
            schema_version: VERSION,
            receiver,
            entries,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn publish_directory(
        &self,
        device: &str,
        update: &InventoryUpdate,
    ) -> Result<Option<Inventory>> {
        ensure!(
            device.starts_with("folder_"),
            "Directory sync requires a paired Folder."
        );
        update.inventory.validate()?;
        let mut db = self.open_connection_unchecked()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT inventory FROM directory_indexes WHERE device_id=?",
                [device],
                |r| r.get(0),
            )
            .optional()?;
        let previous: Option<Inventory> = raw.map(|raw| serde_json::from_str(&raw)).transpose()?;
        if previous.as_ref() == Some(&update.inventory) {
            return Ok(previous);
        }
        if previous.as_ref().map(|v| &v.id) != update.previous_id.as_ref()
            || previous
                .as_ref()
                .is_some_and(|v| v.id == update.inventory.id)
        {
            return Ok(None);
        }
        tx.execute("INSERT INTO directory_indexes(device_id,inventory) VALUES(?,?) ON CONFLICT(device_id) DO UPDATE SET inventory=excluded.inventory",
            params![device,serde_json::to_string(&update.inventory)?])?;
        tx.commit()?;
        Ok(Some(update.inventory.clone()))
    }

    pub(super) fn require_directory_receiver(&self, device: &str) -> Result<()> {
        if !device.starts_with("folder_") {
            return Err(DirectoryConflict.into());
        }
        let db = self.open_connection_unchecked()?;
        if !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM directory_indexes WHERE device_id=?)",
            [device],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(DirectoryConflict.into());
        }
        Ok(())
    }

    pub(super) fn check_directory_replay(
        &self,
        device: &str,
        id: &str,
        expected: Option<&DirectoryVersion>,
    ) -> Result<()> {
        let db = self.open_connection_unchecked()?;
        let actual = db
            .query_row(
                "SELECT path,version FROM directory_versions WHERE device_id=? AND delivery_id=?",
                [device, id],
                |r| {
                    Ok(DirectoryVersion {
                        path: r.get(0)?,
                        version: r.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()?;
        ensure!(
            actual.as_ref() == expected,
            "Directory replay metadata differs."
        );
        Ok(())
    }

    pub(super) fn insert_directory_version(
        &self,
        tx: &Transaction<'_>,
        device: &str,
        id: &str,
        version: &DirectoryVersion,
    ) -> Result<()> {
        version.validate()?;
        ensure!(
            device.starts_with("folder_"),
            "Directory versions require paired Folders."
        );
        let existing: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM directory_versions WHERE device_id=? AND path=?)",
            params![device, version.path],
            |r| r.get(0),
        )?;
        if !existing {
            let count: i64 = tx.query_row(
                "SELECT COUNT(DISTINCT path) FROM directory_versions WHERE device_id=?",
                [device],
                |r| r.get(0),
            )?;
            ensure!(count < MAX_ENTRIES as i64, "Directory path limit reached.");
        }
        let sequence = version.version as i64;
        let collision: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM directory_versions WHERE device_id=?1 AND ((path=?2 AND version=?3) OR substr(path,1,length(?2)+1)=?2||'/' OR substr(?2,1,length(path)+1)=path||'/'))", params![device,version.path,sequence], |r|r.get(0))?;
        if collision {
            return Err(DirectoryConflict.into());
        }
        tx.execute(
            "INSERT INTO directory_versions(device_id,delivery_id,path,version) VALUES(?,?,?,?)",
            params![device, id, version.path, sequence],
        )?;
        // A slow old upload cannot resurrect a version already superseded and ACKed.
        tx.execute("UPDATE deliveries SET acknowledged_at_unix=? WHERE device_id=? AND delivery_id=? AND EXISTS (
            SELECT 1 FROM directory_versions v JOIN deliveries d ON d.device_id=v.device_id AND d.delivery_id=v.delivery_id
            WHERE v.device_id=? AND v.path=? AND v.version>? AND d.acknowledged_at_unix IS NOT NULL)",
            params![crate::fsutil::unix_now() as i64,device,id,device,version.path,sequence])?;
        Ok(())
    }

    pub(super) fn is_directory_delivery(&self, device: &str, id: &str) -> Result<bool> {
        Ok(self.open_connection_unchecked()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM directory_versions WHERE device_id=? AND delivery_id=?)",
            [device, id],
            |r| r.get(0),
        )?)
    }

    pub fn ack_directory(&self, device: &str, ack: &DirectoryAck) -> Result<bool> {
        DirectoryVersion {
            path: ack.path.clone(),
            version: ack.version,
        }
        .validate()?;
        crate::protocol::validate_sha256(&ack.sha256)?;
        let mut db = self.open_connection_unchecked()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = ack.version as i64;
        let matches: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM directory_versions v JOIN deliveries d ON d.device_id=v.device_id AND d.delivery_id=v.delivery_id WHERE v.device_id=? AND v.path=? AND v.version=? AND d.sha256=?)", params![device,ack.path,sequence,ack.sha256], |r| r.get(0))?;
        if !matches {
            return Ok(false);
        }
        let hashes = {
            let mut stmt = tx.prepare("SELECT DISTINCT d.sha256 FROM deliveries d JOIN directory_versions v ON d.device_id=v.device_id AND d.delivery_id=v.delivery_id WHERE v.device_id=? AND v.path=? AND v.version<=?")?;
            stmt.query_map(params![device, ack.path, sequence], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute("UPDATE deliveries SET acknowledged_at_unix=COALESCE(acknowledged_at_unix,?) WHERE device_id=? AND delivery_id IN (SELECT delivery_id FROM directory_versions WHERE device_id=? AND path=? AND version<=?)", params![crate::fsutil::unix_now() as i64,device,device,ack.path,sequence])?;
        tx.execute(
            "UPDATE directory_versions SET conflict=MAX(conflict,?) WHERE device_id=? AND path=? AND version=?",
            params![ack.conflict, device, ack.path, sequence],
        )?;
        tx.commit()?;
        for hash in hashes {
            self.garbage_collect_digest(&hash)?;
        }
        Ok(true)
    }
}
