//! Resumable Folder retirement. Transfer revocation precedes all destructive
//! work. Upload -> content -> SQLite is the same lock order as tus finalization.
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use std::{fs, path::Path as FsPath};
use subtle::ConstantTimeEq;

use super::{
    ApiState, ServerStore,
    api::{ApiError, check_protocol},
    folders::{folder_store, identity, supplied_hash},
};
use crate::{
    fsutil::sync_directory,
    protocol::{FolderExitStatus, PROTOCOL_VERSION},
    storage::{extension_for_media_type, lock_library, validate_sha256},
};

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/f/{folder_id}/api/v1/pairing/exit",
            get(status).post(start),
        )
        .route("/f/{folder_id}/api/v1/pairing/exit/ack", post(ack))
}

pub(super) fn require_device_open(db: &Connection, device: &str) -> Result<()> {
    if let Some(id) = device.strip_prefix("folder_") {
        let exiting: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_exits WHERE folder_id=?)",
            [id],
            |r| r.get(0),
        )?;
        ensure!(
            !exiting,
            "Folder exit has begun; no new transfer data may be written."
        );
    }
    Ok(())
}

fn authorized(db: &Connection, id: &str, hash: &[u8; 32]) -> Result<bool> {
    let row = db
        .query_row(
            "SELECT receiver_hash,sender_hash FROM folders WHERE id=?",
            [id],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()?;
    Ok(row.is_some_and(|(receiver, sender)| {
        receiver.ct_eq(hash).unwrap_u8() == 1
            || sender.is_some_and(|value| value.ct_eq(hash).unwrap_u8() == 1)
    }))
}

pub(super) fn remove_regular_if_present(path: &FsPath) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file(),
                "Refusing non-regular cleanup target: {}",
                path.display()
            );
            fs::remove_file(path)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

impl ServerStore {
    pub(super) fn resume_exits(&self) -> Result<()> {
        let db = self.open_connection_unchecked()?;
        let mut stmt = db.prepare("SELECT folder_id FROM folder_exits WHERE server_cleaned=0")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            if let Err(error) = self.clean_exit(&id, None) {
                eprintln!(
                    "Folder exit {id} remains pending: {}",
                    crate::cli::terminal_safe(&error.to_string())
                );
            }
        }
        Ok(())
    }
    pub(super) fn exit_status(&self, id: &str, role: &str) -> Result<FolderExitStatus> {
        let row = self.open_connection_unchecked()?.query_row(
            "SELECT server_cleaned,sender_cleaned,receiver_cleaned FROM folder_exits WHERE folder_id=?", [id],
            |r| Ok((r.get::<_, bool>(0)?, r.get::<_, bool>(1)?, r.get::<_, bool>(2)?))).optional()?;
        let (server_cleaned, sender_cleaned, receiver_cleaned) = row.unwrap_or_default();
        Ok(FolderExitStatus {
            schema_version: PROTOCOL_VERSION,
            folder_id: id.into(),
            role: role.into(),
            requested: row.is_some(),
            server_cleaned,
            sender_cleaned,
            receiver_cleaned,
        })
    }

    pub(super) fn clean_exit(&self, id: &str, credential: Option<[u8; 32]>) -> Result<bool> {
        let device = format!("folder_{id}");
        {
            let mut db = self.open_connection_unchecked()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(hash) = credential {
                if !authorized(&tx, id, &hash)? {
                    return Ok(false);
                }
            } else {
                ensure!(
                    tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM folder_exits WHERE folder_id=?)",
                        [id],
                        |r| r.get::<_, bool>(0)
                    )?,
                    "Only a previously requested exit may be resumed internally"
                );
            }
            ensure!(
                tx.execute(
                    "UPDATE folders SET disconnected=1,ready=0,expires=0 WHERE id=?",
                    [id]
                )? == 1,
                "Folder no longer exists"
            );
            tx.execute("INSERT OR IGNORE INTO folder_exits(folder_id,sender_cleaned) SELECT id,sender_hash IS NULL FROM folders WHERE id=?", [id])?;
            tx.commit()?;
        }
        // Also drains requests admitted before the durable revocation above.
        let _uploads = lock_library(&self.uploads_dir)?;
        let _content = lock_library(&self.content_dir)?;
        self.purge_folder_uploads_locked(&device)?;
        let mut db = self.open_connection_unchecked()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let objects = {
            let mut stmt =
                tx.prepare("SELECT DISTINCT sha256,media_type FROM deliveries WHERE device_id=?")?;
            stmt.query_map([&device], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (hash, media) in objects {
            validate_sha256(&hash)?;
            let shared: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE sha256=? AND device_id<>? AND acknowledged_at_unix IS NULL)", [&hash, &device], |r| r.get(0))?;
            if shared {
                continue;
            }
            let extension =
                extension_for_media_type(&media).context("Invalid stored content type")?;
            let shard = self.content_dir.join(&hash[..2]);
            match fs::symlink_metadata(&shard) {
                Ok(meta) => ensure!(meta.is_dir(), "Refusing non-directory content shard"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            }
            remove_regular_if_present(&shard.join(format!("{hash}.{extension}")))?;
            sync_directory(&shard)?;
        }
        // Metadata remains until payload deletion succeeds. A crash rolls this
        // transaction back and leaves enough ownership information to retry.
        tx.execute(
            "DELETE FROM directory_versions WHERE device_id=?",
            [&device],
        )?;
        tx.execute("DELETE FROM directory_indexes WHERE device_id=?", [&device])?;
        tx.execute("DELETE FROM deliveries WHERE device_id=?", [&device])?;
        tx.execute(
            "UPDATE folders SET name='Retired Folder',invite_hash=zeroblob(32) WHERE id=?",
            [id],
        )?;
        tx.execute(
            "UPDATE folder_exits SET server_cleaned=1 WHERE folder_id=?",
            [id],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

enum ExitOutcome {
    Unauthorized,
    CleanupPending,
    Status(FolderExitStatus),
}

async fn dispatch(
    state: ApiState,
    id: String,
    headers: HeaderMap,
    operation: u8,
) -> Result<Json<FolderExitStatus>, ApiError> {
    check_protocol(&headers)?;
    let hash = supplied_hash(&headers)?;
    let info = identity(&state, id.clone(), hash).await?;
    let store = state.inner.store.clone();
    let result = folder_store(&state, "Folder clean exit", move || {
        if operation == 1 && !store.clean_exit(&id, Some(hash))? {
            return Ok(ExitOutcome::Unauthorized);
        }
        if operation == 2 {
            let mut db = store.open_connection_unchecked()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !authorized(&tx, &id, &hash)? {
                return Ok(ExitOutcome::Unauthorized);
            }
            let column = if info.role == "sender" {
                "sender_cleaned"
            } else {
                "receiver_cleaned"
            };
            if tx.execute(
                &format!(
                    "UPDATE folder_exits SET {column}=1 WHERE folder_id=? AND server_cleaned=1"
                ),
                [&id],
            )? != 1
            {
                // A valid but premature receipt is a state conflict, not an
                // internal failure. Keep this check inside the write transaction.
                return Ok(ExitOutcome::CleanupPending);
            }
            tx.commit()?;
        }
        store.exit_status(&id, &info.role).map(ExitOutcome::Status)
    })
    .await?;
    match result {
        ExitOutcome::Status(status) => Ok(Json(status)),
        ExitOutcome::Unauthorized => Err(ApiError::unauthorized()),
        ExitOutcome::CleanupPending => Err(ApiError::new(
            StatusCode::CONFLICT,
            "exit_cleanup_pending",
            "Server cleanup must finish before acknowledging local cleanup.",
        )),
    }
}
async fn status(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<FolderExitStatus>, ApiError> {
    dispatch(s, id, h, 0).await
}
async fn start(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<FolderExitStatus>, ApiError> {
    dispatch(s, id, h, 1).await
}
async fn ack(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<FolderExitStatus>, ApiError> {
    dispatch(s, id, h, 2).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revocation_rechecks_credential_inside_the_write_transaction() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let old = super::super::folders::token_hash("old-sender");
        let new = super::super::folders::token_hash("new-sender");
        let receiver = super::super::folders::token_hash("receiver");
        let db = store.open_connection_unchecked().unwrap();
        db.execute("INSERT INTO folders(id,name,receiver_hash,sender_hash,invite_hash,expires) VALUES(?,'test',?,?,zeroblob(32),0)", rusqlite::params![id,receiver.as_slice(),old.as_slice()]).unwrap();
        assert!(authorized(&db, &id, &old).unwrap());
        db.execute(
            "UPDATE folders SET sender_hash=? WHERE id=?",
            rusqlite::params![new.as_slice(), id],
        )
        .unwrap();
        assert!(!store.clean_exit(&id, Some(old)).unwrap());
        assert!(!store.exit_status(&id, "sender").unwrap().requested);
        assert!(store.clean_exit(&id, Some(new)).unwrap());
        assert!(store.exit_status(&id, "sender").unwrap().server_cleaned);
    }
}
