use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use crate::fsutil::{create_dir_all_durable, sync_directory, unix_now};
use crate::model::{Delivery, MANIFEST_SCHEMA_VERSION};
use crate::storage::{
    cleanup_staging, extension_for_media_type, inspect_file, is_supported_content_extension,
    lock_library, receive, validate_delivery_id, validate_delivery_metadata, verify_recorded_file,
};

pub use crate::storage::validate_sha256;

const DATABASE_NAME: &str = "mirelay-server.sqlite3";
const CONTENT_DIR_NAME: &str = "content";
const UPLOADS_DIR_NAME: &str = "uploads";
const SERVER_SCHEMA_VERSION: i64 = 1;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct ServerStore {
    pub(super) data_dir: PathBuf,
    pub(super) database_path: PathBuf,
    pub(super) content_dir: PathBuf,
    pub(super) uploads_dir: PathBuf,
    pub(super) max_file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDelivery {
    pub sequence: i64,
    pub id: String,
    pub original_name: String,
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
    pub created_at_unix: u64,
}

#[derive(Debug)]
pub struct PendingPage {
    pub deliveries: Vec<StoredDelivery>,
    pub snapshot_sequence: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgeOutcome {
    Acknowledged,
    AlreadyAcknowledged,
    NotFound,
    DigestConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerStats {
    pub pending: u64,
    pub acknowledged: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub referenced_objects: u64,
    pub healthy_objects: u64,
    pub missing_objects: u64,
    pub corrupt_objects: u64,
    pub removed_unreferenced_objects: u64,
    pub stale_staging_files_removed: u64,
    pub unexpected_entries: u64,
}

#[derive(Debug)]
struct DbDelivery {
    sequence: i64,
    id: String,
    original_name: String,
    size: i64,
    sha256: String,
    media_type: String,
    created_at_unix: i64,
}

impl ServerStore {
    pub fn new(data_dir: PathBuf, max_file_size: u64) -> Result<Self> {
        if data_dir.as_os_str().is_empty() || !data_dir.is_absolute() {
            bail!("server data directory must be an absolute path");
        }
        if max_file_size == 0 || max_file_size > i64::MAX as u64 {
            bail!(
                "server maximum file size must be between 1 and {}",
                i64::MAX
            );
        }
        Ok(Self {
            database_path: data_dir.join(DATABASE_NAME),
            content_dir: data_dir.join(CONTENT_DIR_NAME),
            uploads_dir: data_dir.join(UPLOADS_DIR_NAME),
            data_dir,
            max_file_size,
        })
    }

    pub fn initialize(&self) -> Result<()> {
        create_dir_all_durable(&self.data_dir)?;
        create_dir_all_durable(&self.content_dir)?;
        create_dir_all_durable(&self.uploads_dir)?;
        harden_directory_permissions(&self.data_dir)?;
        harden_directory_permissions(&self.content_dir)?;
        harden_directory_permissions(&self.uploads_dir)?;

        let mut connection = self.open_connection_unchecked()?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable SQLite WAL mode")?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("failed to read server database schema version")?;
        match version {
            0 => {
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .context("failed to begin server database initialization")?;
                transaction
                    .execute_batch(
                        "
                        CREATE TABLE IF NOT EXISTS deliveries (
                            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                            device_id TEXT NOT NULL,
                            delivery_id TEXT NOT NULL,
                            original_name TEXT NOT NULL,
                            size_bytes INTEGER NOT NULL CHECK (size_bytes > 0),
                            sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
                            media_type TEXT NOT NULL,
                            created_at_unix INTEGER NOT NULL CHECK (created_at_unix >= 0),
                            acknowledged_at_unix INTEGER,
                            UNIQUE (device_id, delivery_id)
                        );
                        CREATE INDEX IF NOT EXISTS deliveries_pending
                            ON deliveries (device_id, acknowledged_at_unix, sequence);
                        CREATE INDEX IF NOT EXISTS deliveries_digest
                            ON deliveries (sha256, acknowledged_at_unix);
                        PRAGMA user_version = 1;
                        ",
                    )
                    .context("failed to create server database schema")?;
                transaction
                    .commit()
                    .context("failed to commit server database schema")?;
            }
            SERVER_SCHEMA_VERSION => {}
            other => bail!(
                "unsupported server database schema version {other}; expected {SERVER_SCHEMA_VERSION}"
            ),
        }
        harden_database_permissions(&self.database_path)?;
        Ok(())
    }

    pub fn health_check(&self) -> Result<()> {
        let connection = self.connection()?;
        let value: i64 = connection
            .query_row("SELECT 1", [], |row| row.get(0))
            .context("server database health query failed")?;
        if value != 1 {
            bail!("server database returned an invalid health result");
        }
        Ok(())
    }

    pub fn enqueue(
        &self,
        device_id: &str,
        input: &Path,
        original_name: String,
    ) -> Result<StoredDelivery> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at_unix = unix_now();
        self.enqueue_with_id(device_id, input, original_name, id, created_at_unix)
    }

    pub(super) fn enqueue_with_id(
        &self,
        device_id: &str,
        input: &Path,
        original_name: String,
        id: String,
        created_at_unix: u64,
    ) -> Result<StoredDelivery> {
        validate_device_id(device_id)?;
        let inspection = inspect_file(input, self.max_file_size)?;
        let delivery = Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: id.clone(),
            original_name: original_name.clone(),
            payload: String::new(),
            size: inspection.size,
            sha256: inspection.sha256.clone(),
            media_type: inspection.media_type.clone(),
            created_at_unix: Some(created_at_unix),
        };
        validate_delivery_metadata(&delivery, self.max_file_size)?;

        let _content_lock = lock_library(&self.content_dir)?;
        if let Some(existing) = self.get_delivery(device_id, &id)? {
            if existing.original_name != original_name
                || existing.size != inspection.size
                || existing.sha256 != inspection.sha256
                || existing.media_type != inspection.media_type
                || existing.created_at_unix != created_at_unix
            {
                bail!("delivery {id} already exists with different metadata");
            }
            return Ok(existing);
        }
        let input_file = File::open(input)
            .with_context(|| format!("failed to open media file {}", input.display()))?;
        let stored = receive(
            &delivery,
            Box::new(input_file),
            &self.content_dir,
            self.max_file_size,
        )?;
        if stored.size != inspection.size
            || stored.sha256 != inspection.sha256
            || stored.media_type != inspection.media_type
        {
            bail!("stored server content does not match inspected input");
        }

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin enqueue transaction")?;
        transaction
            .execute(
                "
                INSERT INTO deliveries (
                    device_id, delivery_id, original_name, size_bytes, sha256,
                    media_type, created_at_unix, acknowledged_at_unix
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
                ",
                params![
                    device_id,
                    id,
                    original_name,
                    i64::try_from(inspection.size).context("media size does not fit SQLite")?,
                    inspection.sha256,
                    inspection.media_type,
                    i64::try_from(created_at_unix).context("timestamp does not fit SQLite")?,
                ],
            )
            .context("failed to insert delivery")?;
        let sequence = transaction.last_insert_rowid();
        transaction
            .commit()
            .context("failed to commit enqueued delivery")?;

        Ok(StoredDelivery {
            sequence,
            id,
            original_name,
            size: inspection.size,
            sha256: inspection.sha256,
            media_type: inspection.media_type,
            created_at_unix,
        })
    }

    pub fn list_pending(
        &self,
        device_id: &str,
        after_sequence: i64,
        snapshot_sequence: Option<i64>,
        limit: u32,
    ) -> Result<PendingPage> {
        validate_device_id(device_id)?;
        if after_sequence < 0 {
            bail!("pagination sequence must not be negative");
        }
        if limit == 0 || limit > 100 {
            bail!("pagination limit must be between 1 and 100");
        }
        let connection = self.connection()?;
        let snapshot_sequence = match snapshot_sequence {
            Some(snapshot) if snapshot >= after_sequence => snapshot,
            Some(_) => bail!("pagination cursor is internally inconsistent"),
            None => connection
                .query_row(
                    "
                    SELECT COALESCE(MAX(sequence), 0)
                    FROM deliveries
                    WHERE device_id = ?1 AND acknowledged_at_unix IS NULL
                    ",
                    [device_id],
                    |row| row.get(0),
                )
                .context("failed to create pending-delivery snapshot")?,
        };

        let mut statement = connection
            .prepare(
                "
                SELECT sequence, delivery_id, original_name, size_bytes, sha256,
                       media_type, created_at_unix
                FROM deliveries
                WHERE device_id = ?1
                  AND acknowledged_at_unix IS NULL
                  AND sequence > ?2
                  AND sequence <= ?3
                ORDER BY sequence ASC
                LIMIT ?4
                ",
            )
            .context("failed to prepare pending-delivery query")?;
        let rows = statement
            .query_map(
                params![
                    device_id,
                    after_sequence,
                    snapshot_sequence,
                    i64::from(limit) + 1,
                ],
                db_delivery_from_row,
            )
            .context("failed to query pending deliveries")?;
        let mut deliveries = Vec::with_capacity(limit as usize + 1);
        for row in rows {
            deliveries.push(validate_db_delivery(row?, self.max_file_size)?);
        }
        let has_more = deliveries.len() > limit as usize;
        if has_more {
            deliveries.truncate(limit as usize);
        }
        Ok(PendingPage {
            deliveries,
            snapshot_sequence,
            has_more,
        })
    }

    pub fn get_delivery(
        &self,
        device_id: &str,
        delivery_id: &str,
    ) -> Result<Option<StoredDelivery>> {
        validate_device_id(device_id)?;
        validate_delivery_id(delivery_id)?;
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "
                SELECT sequence, delivery_id, original_name, size_bytes, sha256,
                       media_type, created_at_unix
                FROM deliveries
                WHERE device_id = ?1 AND delivery_id = ?2
                ",
                params![device_id, delivery_id],
                db_delivery_from_row,
            )
            .optional()
            .context("failed to query delivery")?;
        row.map(|row| validate_db_delivery(row, self.max_file_size))
            .transpose()
    }

    pub fn acknowledge(
        &self,
        device_id: &str,
        delivery_id: &str,
        sha256: &str,
    ) -> Result<AcknowledgeOutcome> {
        validate_device_id(device_id)?;
        validate_delivery_id(delivery_id)?;
        validate_sha256(sha256)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin ACK transaction")?;
        let existing = transaction
            .query_row(
                "
                SELECT sha256, acknowledged_at_unix
                FROM deliveries
                WHERE device_id = ?1 AND delivery_id = ?2
                ",
                params![device_id, delivery_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()
            .context("failed to query delivery for ACK")?;
        let Some((expected_sha256, acknowledged_at)) = existing else {
            return Ok(AcknowledgeOutcome::NotFound);
        };
        if expected_sha256 != sha256 {
            return Ok(AcknowledgeOutcome::DigestConflict);
        }
        if acknowledged_at.is_some() {
            return Ok(AcknowledgeOutcome::AlreadyAcknowledged);
        }
        transaction
            .execute(
                "
                UPDATE deliveries
                SET acknowledged_at_unix = ?3
                WHERE device_id = ?1 AND delivery_id = ?2
                ",
                params![
                    device_id,
                    delivery_id,
                    i64::try_from(unix_now()).context("timestamp does not fit SQLite")?,
                ],
            )
            .context("failed to persist delivery ACK")?;
        transaction
            .commit()
            .context("failed to commit delivery ACK")?;
        Ok(AcknowledgeOutcome::Acknowledged)
    }

    pub fn open_content(&self, delivery: &StoredDelivery) -> Result<File> {
        let path = self.content_path(delivery)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options
            .open(&path)
            .with_context(|| format!("failed to open staged content {}", path.display()))?;
        let metadata = file
            .metadata()
            .with_context(|| format!("failed to inspect staged content {}", path.display()))?;
        if !metadata.file_type().is_file() {
            bail!("staged content {} is not a regular file", path.display());
        }
        if metadata.len() != delivery.size {
            bail!(
                "staged content {} has size {}, expected {}",
                path.display(),
                metadata.len(),
                delivery.size
            );
        }
        Ok(file)
    }

    pub fn garbage_collect_digest(&self, sha256: &str) -> Result<bool> {
        validate_sha256(sha256)?;
        let _content_lock = lock_library(&self.content_dir)?;
        let connection = self.connection()?;
        let pending: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM deliveries
                WHERE sha256 = ?1 AND acknowledged_at_unix IS NULL
                ",
                [sha256],
                |row| row.get(0),
            )
            .context("failed to check content references")?;
        if pending != 0 {
            return Ok(false);
        }
        let media_type = connection
            .query_row(
                "SELECT media_type FROM deliveries WHERE sha256 = ?1 LIMIT 1",
                [sha256],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to determine acknowledged content type")?;
        let Some(media_type) = media_type else {
            return Ok(false);
        };
        let extension = extension_for_media_type(&media_type)
            .with_context(|| format!("unsupported stored media type {media_type:?}"))?;
        let path = self
            .content_dir
            .join(&sha256[..2])
            .join(format!("{sha256}.{extension}"));
        match fs::remove_file(&path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    sync_directory(parent)?;
                }
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| {
                format!("failed to remove acknowledged content {}", path.display())
            }),
        }
    }

    pub fn reconcile_content(&self) -> Result<ReconcileReport> {
        let _content_lock = lock_library(&self.content_dir)?;
        let mut report = ReconcileReport {
            stale_staging_files_removed: u64::try_from(cleanup_staging(&self.content_dir)?)
                .context("staging cleanup count does not fit u64")?,
            ..Default::default()
        };
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "
                SELECT sequence, delivery_id, original_name, size_bytes, sha256,
                       media_type, created_at_unix
                FROM deliveries
                WHERE acknowledged_at_unix IS NULL
                ORDER BY sequence ASC
                ",
            )
            .context("failed to prepare content reconciliation query")?;
        let rows = statement
            .query_map([], db_delivery_from_row)
            .context("failed to query content references")?;
        let mut referenced = BTreeMap::<PathBuf, StoredDelivery>::new();
        for row in rows {
            let delivery = validate_db_delivery(row?, self.max_file_size)?;
            let path = self.content_path(&delivery)?;
            if let Some(previous) = referenced.get(&path) {
                if previous.sha256 != delivery.sha256
                    || previous.size != delivery.size
                    || previous.media_type != delivery.media_type
                {
                    bail!(
                        "pending deliveries {} and {} contain inconsistent metadata for {}",
                        previous.id,
                        delivery.id,
                        path.display()
                    );
                }
            } else {
                referenced.insert(path, delivery);
            }
        }

        report.referenced_objects =
            u64::try_from(referenced.len()).context("content reference count does not fit u64")?;
        for (path, delivery) in &referenced {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    if verify_recorded_file(
                        path,
                        delivery.size,
                        &delivery.sha256,
                        &delivery.media_type,
                        self.max_file_size,
                    )
                    .is_ok()
                    {
                        report.healthy_objects += 1;
                    } else {
                        report.corrupt_objects += 1;
                    }
                }
                Ok(_) => report.corrupt_objects += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    report.missing_objects += 1;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to inspect referenced content {}", path.display())
                    });
                }
            }
        }

        for entry in fs::read_dir(&self.content_dir).with_context(|| {
            format!(
                "failed to scan server content directory {}",
                self.content_dir.display()
            )
        })? {
            let entry = entry.context("failed to read server content directory entry")?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".mirelay.lock" || name == ".mirelay-staging-v1" {
                continue;
            }
            let file_type = entry.file_type().with_context(|| {
                format!("failed to inspect content entry {}", entry.path().display())
            })?;
            if !file_type.is_dir() || !is_content_shard_name(&name) {
                report.unexpected_entries += 1;
                continue;
            }

            let shard_path = entry.path();
            let mut removed_from_shard = false;
            for object in fs::read_dir(&shard_path)
                .with_context(|| format!("failed to scan content shard {}", shard_path.display()))?
            {
                let object = object.with_context(|| {
                    format!("failed to read content shard {}", shard_path.display())
                })?;
                let object_type = object.file_type().with_context(|| {
                    format!(
                        "failed to inspect content object {}",
                        object.path().display()
                    )
                })?;
                let object_name = object.file_name();
                let object_name = object_name.to_string_lossy();
                if !is_owned_content_object_name(&name, &object_name)
                    || !(object_type.is_file() || object_type.is_symlink())
                {
                    report.unexpected_entries += 1;
                    continue;
                }
                let object_path = object.path();
                if referenced.contains_key(&object_path) {
                    continue;
                }
                fs::remove_file(&object_path).with_context(|| {
                    format!(
                        "failed to remove unreferenced content object {}",
                        object_path.display()
                    )
                })?;
                report.removed_unreferenced_objects += 1;
                removed_from_shard = true;
            }
            if removed_from_shard {
                sync_directory(&shard_path)?;
            }
        }
        Ok(report)
    }

    pub fn stats(&self, device_id: &str) -> Result<ServerStats> {
        validate_device_id(device_id)?;
        let connection = self.connection()?;
        let (pending, acknowledged): (i64, i64) = connection
            .query_row(
                "
                SELECT
                    COALESCE(SUM(CASE WHEN acknowledged_at_unix IS NULL THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN acknowledged_at_unix IS NOT NULL THEN 1 ELSE 0 END), 0)
                FROM deliveries
                WHERE device_id = ?1
                ",
                [device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .context("failed to query server statistics")?;
        Ok(ServerStats {
            pending: u64::try_from(pending).context("invalid pending delivery count")?,
            acknowledged: u64::try_from(acknowledged)
                .context("invalid acknowledged delivery count")?,
        })
    }

    fn content_path(&self, delivery: &StoredDelivery) -> Result<PathBuf> {
        let manifest = Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: delivery.id.clone(),
            original_name: delivery.original_name.clone(),
            payload: String::new(),
            size: delivery.size,
            sha256: delivery.sha256.clone(),
            media_type: delivery.media_type.clone(),
            created_at_unix: Some(delivery.created_at_unix),
        };
        validate_delivery_metadata(&manifest, self.max_file_size)?;
        let extension = extension_for_media_type(&delivery.media_type)
            .context("stored delivery uses an unsupported media type")?;
        Ok(self
            .content_dir
            .join(&delivery.sha256[..2])
            .join(format!("{}.{}", delivery.sha256, extension)))
    }

    fn connection(&self) -> Result<Connection> {
        let connection = self.open_connection_unchecked()?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("failed to read server database schema version")?;
        if version != SERVER_SCHEMA_VERSION {
            bail!(
                "unsupported server database schema version {version}; expected {SERVER_SCHEMA_VERSION}"
            );
        }
        Ok(connection)
    }

    fn open_connection_unchecked(&self) -> Result<Connection> {
        let connection = Connection::open_with_flags(
            &self.database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| {
            format!(
                "failed to open server database {}",
                self.database_path.display()
            )
        })?;
        connection
            .busy_timeout(SQLITE_BUSY_TIMEOUT)
            .context("failed to configure SQLite busy timeout")?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable SQLite foreign keys")?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .context("failed to configure SQLite durability")?;
        connection
            .pragma_update(None, "trusted_schema", "OFF")
            .context("failed to disable SQLite trusted schema")?;
        Ok(connection)
    }
}

fn db_delivery_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbDelivery> {
    Ok(DbDelivery {
        sequence: row.get(0)?,
        id: row.get(1)?,
        original_name: row.get(2)?,
        size: row.get(3)?,
        sha256: row.get(4)?,
        media_type: row.get(5)?,
        created_at_unix: row.get(6)?,
    })
}

fn validate_db_delivery(row: DbDelivery, max_file_size: u64) -> Result<StoredDelivery> {
    if row.sequence <= 0 {
        bail!("server database contains an invalid delivery sequence");
    }
    let delivery = StoredDelivery {
        sequence: row.sequence,
        id: row.id,
        original_name: row.original_name,
        size: u64::try_from(row.size).context("server database contains a negative media size")?,
        sha256: row.sha256,
        media_type: row.media_type,
        created_at_unix: u64::try_from(row.created_at_unix)
            .context("server database contains a negative timestamp")?,
    };
    let manifest = Delivery {
        schema_version: MANIFEST_SCHEMA_VERSION,
        id: delivery.id.clone(),
        original_name: delivery.original_name.clone(),
        payload: String::new(),
        size: delivery.size,
        sha256: delivery.sha256.clone(),
        media_type: delivery.media_type.clone(),
        created_at_unix: Some(delivery.created_at_unix),
    };
    validate_delivery_metadata(&manifest, max_file_size)?;
    Ok(delivery)
}

pub fn validate_device_id(device_id: &str) -> Result<()> {
    if device_id.is_empty()
        || device_id.len() > 128
        || !device_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("device id must be 1-128 ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn is_content_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_owned_content_object_name(shard: &str, name: &str) -> bool {
    let Some((sha256, extension)) = name.rsplit_once('.') else {
        return false;
    };
    validate_sha256(sha256).is_ok()
        && sha256.starts_with(shard)
        && is_supported_content_extension(extension)
}

#[cfg(unix)]
fn harden_directory_permissions(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to restrict permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn harden_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_database_permissions(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn harden_database_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"MiRelay server store test");
        bytes
    }

    #[test]
    fn queue_snapshot_and_ack_are_persistent_and_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        fs::write(&image, png_bytes()).unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();

        let delivery = store.enqueue("linux", &image, "image.png".into()).unwrap();
        let first = store.list_pending("linux", 0, None, 50).unwrap();
        assert_eq!(first.deliveries.as_slice(), std::slice::from_ref(&delivery));
        assert!(!first.has_more);

        assert_eq!(
            store
                .acknowledge("linux", &delivery.id, &delivery.sha256)
                .unwrap(),
            AcknowledgeOutcome::Acknowledged
        );
        assert_eq!(
            store
                .acknowledge("linux", &delivery.id, &delivery.sha256)
                .unwrap(),
            AcknowledgeOutcome::AlreadyAcknowledged
        );
        assert_eq!(
            store
                .acknowledge("linux", &delivery.id, &"0".repeat(64))
                .unwrap(),
            AcknowledgeOutcome::DigestConflict
        );
        assert!(
            store
                .list_pending("linux", 0, None, 50)
                .unwrap()
                .deliveries
                .is_empty()
        );
        assert_eq!(
            store.stats("linux").unwrap(),
            ServerStats {
                pending: 0,
                acknowledged: 1
            }
        );
    }

    #[test]
    fn snapshot_excludes_deliveries_enqueued_after_the_first_page() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        fs::write(&image, png_bytes()).unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let first_delivery = store.enqueue("linux", &image, "one.png".into()).unwrap();
        let second_delivery = store.enqueue("linux", &image, "two.png".into()).unwrap();

        let first_page = store.list_pending("linux", 0, None, 1).unwrap();
        assert_eq!(first_page.deliveries, [first_delivery]);
        assert!(first_page.has_more);
        let late_delivery = store.enqueue("linux", &image, "late.png".into()).unwrap();
        let second_page = store
            .list_pending(
                "linux",
                first_page.deliveries[0].sequence,
                Some(first_page.snapshot_sequence),
                1,
            )
            .unwrap();
        assert_eq!(second_page.deliveries, [second_delivery]);
        assert!(!second_page.has_more);
        assert!(late_delivery.sequence > first_page.snapshot_sequence);
    }

    #[test]
    fn reconciliation_removes_owned_orphans_and_reports_reference_damage() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        fs::write(&image, png_bytes()).unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let delivery = store.enqueue("linux", &image, "image.png".into()).unwrap();

        let initial = store.reconcile_content().unwrap();
        assert_eq!(initial.referenced_objects, 1);
        assert_eq!(initial.healthy_objects, 1);

        let orphan_digest = format!("aa{}", "0".repeat(62));
        let orphan_shard = store.content_dir.join("aa");
        fs::create_dir_all(&orphan_shard).unwrap();
        fs::write(
            orphan_shard.join(format!("{orphan_digest}.png")),
            png_bytes(),
        )
        .unwrap();
        fs::write(
            store
                .content_dir
                .join(".mirelay-staging-v1")
                .join("mirelay-crash.part"),
            b"partial",
        )
        .unwrap();
        let unexpected = store.content_dir.join("operator-notes.txt");
        fs::write(&unexpected, b"keep me").unwrap();

        let cleaned = store.reconcile_content().unwrap();
        assert_eq!(cleaned.healthy_objects, 1);
        assert_eq!(cleaned.removed_unreferenced_objects, 1);
        assert_eq!(cleaned.stale_staging_files_removed, 1);
        assert_eq!(cleaned.unexpected_entries, 1);
        assert!(unexpected.exists());

        let content_path = store.content_path(&delivery).unwrap();
        fs::write(&content_path, b"broken").unwrap();
        let corrupt = store.reconcile_content().unwrap();
        assert_eq!(corrupt.healthy_objects, 0);
        assert_eq!(corrupt.corrupt_objects, 1);

        fs::remove_file(content_path).unwrap();
        let missing = store.reconcile_content().unwrap();
        assert_eq!(missing.corrupt_objects, 0);
        assert_eq!(missing.missing_objects, 1);
    }
}
