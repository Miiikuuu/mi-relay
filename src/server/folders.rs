//! Folder-scoped credentials and a two-party pairing handshake. No plaintext
//! credentials are stored. A sender cannot receive, acknowledge, or administer.
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::api::{ApiError, ApiState, authorize_bearer, check_protocol, run_store};
use crate::fsutil::unix_now;
use crate::protocol::{
    ClaimFolderRequest, ConfirmFolderRequest, CreateFolderRequest, FolderCreated, FolderHandshake,
    FolderInvitation, PROTOCOL_VERSION,
};

const INVITATION_SECONDS: u64 = 600;

// Bound database work before spawning blocking tasks. The permit lives INSIDE
// the task, so cancelling an HTTP request cannot release it while SQL still runs.
pub(super) async fn folder_store<T: Send + 'static>(
    state: &ApiState,
    operation: &'static str,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state
        .inner
        .folder_queries
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "setup_busy",
                "Folder verification is busy. Retry later.",
            )
        })?;
    run_store(operation, move || {
        let _permit = permit;
        work()
    })
    .await
}

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/folders", post(create))
        .route("/api/v1/pairings/claim", post(claim))
        .route("/f/{folder_id}/api/v1/handshake", get(handshake))
        .route("/f/{folder_id}/api/v1/pairing/confirm", post(confirm))
        .route("/f/{folder_id}/api/v1/pairing/renew", post(renew))
}

pub(super) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(format!("Bearer {token}").as_bytes()).into()
}

fn credential() -> String {
    // UUID v4 uses the OS CSPRNG; two UUIDs provide 244 random bits.
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub(super) fn supplied_hash(headers: &HeaderMap) -> Result<[u8; 32], ApiError> {
    if headers.get_all("authorization").iter().count() != 1 {
        return Err(ApiError::unauthorized());
    }
    let bytes = headers.get("authorization").unwrap().as_bytes();
    if bytes.len() > 4103 || !bytes.starts_with(b"Bearer ") {
        return Err(ApiError::unauthorized());
    }
    Ok(Sha256::digest(bytes).into())
}

fn same(stored: &[u8], supplied: &[u8; 32]) -> bool {
    stored.ct_eq(supplied).unwrap_u8() == 1
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    if uuid::Uuid::parse_str(id)
        .ok()
        .is_none_or(|v| v.to_string() != id)
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

fn pending() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "pairing_pending",
        "Both devices must confirm pairing before files can be transferred.",
    )
}

async fn create(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<CreateFolderRequest>,
) -> Result<Json<FolderCreated>, ApiError> {
    let supplied = supplied_hash(&headers)?;
    if state
        .inner
        .admin_hash
        .as_ref()
        .is_none_or(|hash| !same(hash, &supplied))
    {
        return Err(ApiError::unauthorized());
    }
    check_protocol(&headers)?;
    let name = request.name.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "invalid_name",
            "Folder name must contain 1–128 bytes without control characters.",
        ));
    }
    let name = name.to_owned();
    let store = state.inner.store.clone();
    let created = folder_store(&state, "create Folder", move || {
        let mut db = store.open_connection_unchecked()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0))?;
        anyhow::ensure!(count < 1000, "Folder limit reached");
        let id = uuid::Uuid::new_v4().to_string();
        let token = credential();
        let code = format!("{id}.{}", credential());
        let expires = unix_now() + INVITATION_SECONDS;
        tx.execute(
            "INSERT INTO folders(id,name,receiver_hash,invite_hash,expires) VALUES(?,?,?,?,?)",
            params![
                id,
                name,
                token_hash(&token).as_slice(),
                token_hash(&code).as_slice(),
                expires as i64
            ],
        )?;
        tx.commit()?;
        Ok(FolderCreated {
            folder_id: id,
            receiver_token: token,
            pairing_code: code,
            expires_at_unix: expires,
        })
    })
    .await?;
    Ok(Json(created))
}

async fn claim(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<ClaimFolderRequest>,
) -> Result<Json<FolderHandshake>, ApiError> {
    check_protocol(&headers)?;
    let Some((id, secret)) = request.pairing_code.split_once('.') else {
        return Err(ApiError::unauthorized());
    };
    validate_id(id)?;
    if secret.len() != 64
        || !secret.bytes().all(|b| b.is_ascii_hexdigit())
        || request.sender_token.len() != 64
        || !request.sender_token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::unauthorized());
    }
    let id = id.to_owned();
    let store = state.inner.store.clone();
    let sender_hash = token_hash(&request.sender_token);
    let invite_hash = token_hash(&request.pairing_code);
    let claimed = folder_store(&state, "claim Folder invitation", move || {
        let mut db = store.open_connection_unchecked()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT invite_hash,expires,sender_hash FROM folders WHERE id=?",
                [&id],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored, expires, previous)) = row else {
            return Ok(false);
        };
        if !same(&stored, &invite_hash) {
            return Ok(false);
        }
        // Same-credential retries are safe, including a lost response. A claimed
        // invitation can never replace an existing sender.
        if let Some(previous) = previous {
            return Ok(same(&previous, &sender_hash));
        }
        if expires <= 0 || unix_now() >= expires as u64 {
            return Ok(false);
        }
        tx.execute(
            "UPDATE folders SET sender_hash=? WHERE id=?",
            params![sender_hash.as_slice(), id],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await?;
    if !claimed {
        return Err(ApiError::unauthorized());
    }
    let (id, _) = request.pairing_code.split_once('.').unwrap();
    Ok(Json(identity(&state, id.to_owned(), sender_hash).await?))
}

pub(super) async fn identity(
    state: &ApiState,
    id: String,
    hash: [u8; 32],
) -> Result<FolderHandshake, ApiError> {
    validate_id(&id)?;
    let store = state.inner.store.clone();
    let result = folder_store(state, "verify Folder credentials", move || {
        let db = store.open_connection_unchecked()?;
        let row = db
            .query_row(
                "SELECT name,receiver_hash,sender_hash,ready FROM folders WHERE id=?",
                [&id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((name, receiver, sender, ready)) = row else {
            return Ok(None);
        };
        let role = if same(&receiver, &hash) {
            "receiver"
        } else if sender.as_ref().is_some_and(|v| same(v, &hash)) {
            "sender"
        } else {
            return Ok(None);
        };
        // Both devices display the same comparison code. Confirm binds to this
        // exact sender, so a stale dialog cannot confirm a replacement claimant.
        let verification = sender.as_ref().map(|v| hex::encode(&v[..6]));
        let status = if ready {
            "ready"
        } else if sender.is_some() {
            "awaiting_confirmation"
        } else {
            "awaiting_peer"
        };
        Ok(Some(FolderHandshake {
            schema_version: PROTOCOL_VERSION,
            folder_id: id,
            name,
            role: role.into(),
            state: status.into(),
            verification,
            max_file_size_bytes: store.max_file_size(),
        }))
    })
    .await?;
    result.ok_or_else(ApiError::unauthorized)
}

async fn handshake(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<FolderHandshake>, ApiError> {
    let hash = supplied_hash(&headers)?;
    check_protocol(&headers)?;
    Ok(Json(identity(&state, id, hash).await?))
}

async fn confirm(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ConfirmFolderRequest>,
) -> Result<Json<FolderHandshake>, ApiError> {
    let hash = supplied_hash(&headers)?;
    check_protocol(&headers)?;
    let info = identity(&state, id.clone(), hash).await?;
    if info.role != "receiver" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_role",
            "Receiver confirmation is required.",
        ));
    }
    if info.verification.as_deref() != Some(request.verification.as_str()) {
        return Err(pending());
    }
    let store = state.inner.store.clone();
    let confirmed = folder_store(&state, "confirm Folder pairing", move || {
        let db = store.open_connection_unchecked()?;
        Ok(db.execute(
            "UPDATE folders SET ready=1 WHERE id=? AND lower(substr(hex(sender_hash),1,12))=?",
            params![id, request.verification],
        )? == 1)
    })
    .await?;
    if !confirmed {
        return Err(pending());
    }
    Ok(Json(identity(&state, info.folder_id, hash).await?))
}

async fn renew(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<FolderInvitation>, ApiError> {
    let hash = supplied_hash(&headers)?;
    check_protocol(&headers)?;
    let info = identity(&state, id.clone(), hash).await?;
    if info.role != "receiver" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_role",
            "Only the receiver can replace an invitation.",
        ));
    }
    let code = format!("{id}.{}", credential());
    let expires = unix_now() + INVITATION_SECONDS;
    let stored = token_hash(&code);
    let store = state.inner.store.clone();
    folder_store(&state, "replace Folder invitation", move || {
        store.open_connection_unchecked()?.execute(
            "UPDATE folders SET invite_hash=?,expires=?,sender_hash=NULL,ready=0 WHERE id=?",
            params![stored.as_slice(), expires as i64, id],
        )?;
        Ok(())
    })
    .await?;
    Ok(Json(FolderInvitation {
        pairing_code: code,
        expires_at_unix: expires,
    }))
}

/// The scoped URL selects an identity, NEVER authority. Both the token and its
/// role must match that identity. Legacy credentials cannot cross into Folders.
pub(super) async fn authorize_transfer(
    state: &ApiState,
    headers: &HeaderMap,
    uri: &Uri,
    role: &str,
) -> Result<String, ApiError> {
    if let Some(path) = uri.path().strip_prefix("/f/") {
        let id = path.split('/').next().unwrap_or("");
        let info = identity(state, id.to_owned(), supplied_hash(headers)?).await?;
        if info.role != role {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "wrong_role",
                "This credential cannot perform that operation.",
            ));
        }
        if info.state != "ready" {
            return Err(pending());
        }
        Ok(format!("folder_{}", info.folder_id))
    } else {
        authorize_bearer(state, headers)?;
        Ok(state.inner.device_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn setup_database_work_has_a_hard_concurrency_bound() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::server::ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let state = ApiState::new(store, "linux".into(), "legacy").unwrap();
        let occupied = state
            .inner
            .folder_queries
            .clone()
            .acquire_many_owned(32)
            .await
            .unwrap();
        let error = folder_store(&state, "saturation test", || Ok(()))
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.into_response().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(occupied);
        assert!(
            folder_store(&state, "available test", || Ok(()))
                .await
                .is_ok()
        );
    }
}
