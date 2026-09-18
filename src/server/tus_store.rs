use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::fsutil::{atomic_write, sync_directory, unix_now};
use crate::model::{Delivery, MANIFEST_SCHEMA_VERSION};
use crate::storage::{
    lock_library, validate_delivery_id, validate_delivery_metadata, verify_recorded_file,
};

use super::store::{ServerStore, validate_device_id};

const UPLOAD_RECORD_VERSION: u32 = 1;
const MAX_UPLOAD_RECORD_BYTES: u64 = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUpload {
    pub directory: Option<crate::directory::DirectoryVersion>,
    pub original_name: String,
    pub media_type: String,
    pub sha256: String,
    pub length: u64,
    pub metadata_header: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadInfo {
    pub id: String,
    pub length: u64,
    pub offset: u64,
    pub metadata_header: String,
    pub delivery_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendUploadOutcome {
    Appended(UploadInfo),
    NotFound,
    OffsetMismatch(UploadInfo),
    TooLarge(UploadInfo),
}

#[derive(Debug)]
pub(super) struct InvalidUploadContent {
    upload_id: String,
    reason: String,
}

impl fmt::Display for InvalidUploadContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "completed tus upload {} contains invalid content: {}",
            self.upload_id, self.reason
        )
    }
}

impl std::error::Error for InvalidUploadContent {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    directory: Option<crate::directory::DirectoryVersion>,
    version: u32,
    id: String,
    device_id: String,
    original_name: String,
    media_type: String,
    sha256: String,
    length: u64,
    offset: u64,
    metadata_header: String,
    created_at_unix: u64,
    delivery_id: Option<String>,
}

impl ServerStore {
    /// Caller holds uploads_dir lock. Keep the ownership record until its part
    /// is durably removed, so a crash never turns an owned part into an orphan.
    pub(super) fn purge_folder_uploads_locked(&self, device: &str) -> Result<()> {
        // Old versions could crash between part creation and ownership commit.
        // Never guess which Folder owns such bytes, or claim a complete purge.
        for entry in fs::read_dir(&self.uploads_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("part") {
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .context("Invalid upload fragment name")?;
                anyhow::ensure!(
                    self.read_upload_record(id)?.is_some(),
                    "Unowned legacy upload fragment requires administrator review before cleanup can be confirmed"
                );
            }
        }
        for entry in fs::read_dir(&self.uploads_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("Invalid upload record name")?;
            let Some(record) = self.read_upload_record(id)? else {
                continue;
            };
            if record.device_id != device {
                continue;
            }
            super::exit::remove_regular_if_present(&self.upload_part_path(id)?)?;
            sync_directory(&self.uploads_dir)?;
            super::exit::remove_regular_if_present(&path)?;
            sync_directory(&self.uploads_dir)?;
        }
        Ok(())
    }

    pub fn max_file_size(&self) -> u64 {
        self.max_file_size
    }

    pub fn validate_new_upload(&self, upload: &NewUpload) -> Result<()> {
        if let Some(directory) = &upload.directory {
            directory.validate()?;
            anyhow::ensure!(
                directory.path.rsplit('/').next() == Some(upload.original_name.as_str()),
                "Directory filename and path disagree."
            );
        }
        let delivery = Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: "00000000-0000-4000-8000-000000000000".to_owned(),
            original_name: upload.original_name.clone(),
            payload: String::new(),
            size: upload.length,
            sha256: upload.sha256.clone(),
            media_type: upload.media_type.clone(),
            created_at_unix: Some(0),
        };
        validate_delivery_metadata(&delivery, self.max_file_size)?;
        if upload.metadata_header.is_empty()
            || upload.metadata_header.len() > MAX_UPLOAD_RECORD_BYTES as usize
            || upload.metadata_header.chars().any(char::is_control)
        {
            bail!("upload metadata header is empty, too long, or contains control characters");
        }
        Ok(())
    }

    pub fn create_upload(&self, device_id: &str, upload: NewUpload) -> Result<UploadInfo> {
        validate_device_id(device_id)?;
        self.validate_new_upload(&upload)?;
        if upload.directory.is_some() {
            self.require_directory_receiver(device_id)?;
        }
        let _upload_lock = lock_library(&self.uploads_dir)?;
        super::exit::require_device_open(&self.open_connection_unchecked()?, device_id)?;

        for _ in 0..16 {
            let id = uuid::Uuid::new_v4().to_string();
            let record_path = self.upload_record_path(&id)?;
            let part_path = self.upload_part_path(&id)?;
            if record_path.exists() || part_path.exists() {
                continue;
            }
            // Persist ownership BEFORE creating any payload bytes. The endpoint
            // does not expose this id until both writes are durable.
            let record = UploadRecord {
                directory: upload.directory.clone(),
                version: UPLOAD_RECORD_VERSION,
                id,
                device_id: device_id.to_owned(),
                original_name: upload.original_name.clone(),
                media_type: upload.media_type.clone(),
                sha256: upload.sha256.clone(),
                length: upload.length,
                offset: 0,
                metadata_header: upload.metadata_header.clone(),
                created_at_unix: unix_now(),
                delivery_id: None,
            };
            self.write_upload_record(&record)?;
            let mut options = OpenOptions::new();
            options.create_new(true).read(true).write(true);
            #[cfg(unix)]
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            let part = match options.open(&part_path) {
                Ok(part) => part,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create tus upload data {}", part_path.display())
                    });
                }
            };
            part.sync_all().with_context(|| {
                format!("failed to sync new tus upload data {}", part_path.display())
            })?;
            sync_directory(&self.uploads_dir)?;

            return Ok(record.info());
        }
        bail!("failed to allocate a unique tus upload id");
    }

    pub fn get_upload(&self, device_id: &str, upload_id: &str) -> Result<Option<UploadInfo>> {
        validate_device_id(device_id)?;
        validate_upload_id(upload_id)?;
        let _upload_lock = lock_library(&self.uploads_dir)?;
        super::exit::require_device_open(&self.open_connection_unchecked()?, device_id)?;
        let Some(mut record) = self.read_upload_record(upload_id)? else {
            return Ok(None);
        };
        if record.device_id != device_id {
            return Ok(None);
        }
        self.reconcile_upload_data(&record)?;
        self.finalize_upload(&mut record)?;
        Ok(Some(record.info()))
    }

    pub fn append_upload(
        &self,
        device_id: &str,
        upload_id: &str,
        expected_offset: u64,
        mut chunk: File,
        chunk_length: u64,
    ) -> Result<AppendUploadOutcome> {
        validate_device_id(device_id)?;
        validate_upload_id(upload_id)?;
        let _upload_lock = lock_library(&self.uploads_dir)?;
        super::exit::require_device_open(&self.open_connection_unchecked()?, device_id)?;
        let Some(mut record) = self.read_upload_record(upload_id)? else {
            return Ok(AppendUploadOutcome::NotFound);
        };
        if record.device_id != device_id {
            return Ok(AppendUploadOutcome::NotFound);
        }
        self.reconcile_upload_data(&record)?;
        self.finalize_upload(&mut record)?;

        if expected_offset != record.offset {
            return Ok(AppendUploadOutcome::OffsetMismatch(record.info()));
        }
        let remaining = record.length.saturating_sub(record.offset);
        if chunk_length > remaining {
            return Ok(AppendUploadOutcome::TooLarge(record.info()));
        }
        if record.delivery_id.is_some() {
            return Ok(AppendUploadOutcome::Appended(record.info()));
        }

        if chunk_length != 0 {
            let actual_chunk_length = chunk
                .metadata()
                .context("failed to inspect buffered tus chunk")?
                .len();
            if actual_chunk_length != chunk_length {
                bail!("buffered tus chunk has size {actual_chunk_length}, expected {chunk_length}");
            }
            chunk
                .seek(SeekFrom::Start(0))
                .context("failed to rewind buffered tus chunk")?;
            let part_path = self.upload_part_path(&record.id)?;
            let mut part = open_upload_part(&part_path)?;
            part.seek(SeekFrom::Start(record.offset)).with_context(|| {
                format!("failed to seek tus upload data {}", part_path.display())
            })?;
            let copied = std::io::copy(&mut chunk, &mut part).with_context(|| {
                format!("failed to append tus upload data {}", part_path.display())
            })?;
            if copied != chunk_length {
                bail!("copied {copied} tus bytes, expected {chunk_length}");
            }
            part.sync_all().with_context(|| {
                format!("failed to sync tus upload data {}", part_path.display())
            })?;
        }

        record.offset = record
            .offset
            .checked_add(chunk_length)
            .context("tus upload offset overflow")?;
        self.write_upload_record(&record)?;
        self.finalize_upload(&mut record)?;
        Ok(AppendUploadOutcome::Appended(record.info()))
    }

    fn finalize_upload(&self, record: &mut UploadRecord) -> Result<()> {
        if record.offset != record.length || record.delivery_id.is_some() {
            return Ok(());
        }
        let part_path = self.upload_part_path(&record.id)?;
        if let Err(error) = verify_recorded_file(
            &part_path,
            record.length,
            &record.sha256,
            &record.media_type,
            self.max_file_size,
        ) {
            let reason = format!("{error:#}");
            self.discard_upload(record)?;
            return Err(InvalidUploadContent {
                upload_id: record.id.clone(),
                reason,
            }
            .into());
        }
        let delivery = self.enqueue_directory_with_id(
            &record.device_id,
            &part_path,
            record.original_name.clone(),
            record.id.clone(),
            record.created_at_unix,
            record.directory.as_ref(),
        )?;
        record.delivery_id = Some(delivery.id);
        self.write_upload_record(record)?;
        if record.directory.is_some() {
            self.garbage_collect_digest(&record.sha256)?;
        }

        match fs::remove_file(&part_path) {
            Ok(()) => {
                sync_directory(&self.uploads_dir)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "server non-fatal cleanup error: failed to remove completed tus data {}: {}",
                part_path.display(),
                crate::cli::terminal_safe(&error.to_string())
            ),
        }
        Ok(())
    }

    fn discard_upload(&self, record: &UploadRecord) -> Result<()> {
        let record_path = self.upload_record_path(&record.id)?;
        match fs::remove_file(&record_path) {
            Ok(()) => sync_directory(&self.uploads_dir)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to discard invalid tus record {}",
                        record_path.display()
                    )
                });
            }
        }

        let part_path = self.upload_part_path(&record.id)?;
        match fs::remove_file(&part_path) {
            Ok(()) => sync_directory(&self.uploads_dir)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to discard invalid tus data {}", part_path.display())
                });
            }
        }
        Ok(())
    }

    fn reconcile_upload_data(&self, record: &UploadRecord) -> Result<()> {
        if record.delivery_id.is_some() {
            return Ok(());
        }
        let part_path = self.upload_part_path(&record.id)?;
        let part = open_upload_part(&part_path)?;
        let actual_length = part
            .metadata()
            .with_context(|| format!("failed to inspect tus upload data {}", part_path.display()))?
            .len();
        if actual_length < record.offset {
            bail!(
                "tus upload {} contains {actual_length} bytes but records offset {}",
                record.id,
                record.offset
            );
        }
        if actual_length > record.offset {
            part.set_len(record.offset).with_context(|| {
                format!(
                    "failed to roll back uncommitted tus bytes in {}",
                    part_path.display()
                )
            })?;
            part.sync_all().with_context(|| {
                format!(
                    "failed to sync repaired tus upload data {}",
                    part_path.display()
                )
            })?;
        }
        Ok(())
    }

    fn read_upload_record(&self, upload_id: &str) -> Result<Option<UploadRecord>> {
        let path = self.upload_record_path(upload_id)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to open tus record {}", path.display()));
            }
        };
        if !file
            .metadata()
            .with_context(|| format!("failed to inspect tus record {}", path.display()))?
            .file_type()
            .is_file()
        {
            bail!("tus record {} is not a regular file", path.display());
        }
        let mut bytes = Vec::new();
        file.take(MAX_UPLOAD_RECORD_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read tus record {}", path.display()))?;
        if bytes.len() as u64 > MAX_UPLOAD_RECORD_BYTES {
            bail!("tus record {} exceeds its safety limit", path.display());
        }
        let record: UploadRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse tus record {}", path.display()))?;
        self.validate_upload_record(upload_id, &record)?;
        Ok(Some(record))
    }

    fn write_upload_record(&self, record: &UploadRecord) -> Result<()> {
        self.validate_upload_record(&record.id, record)?;
        let path = self.upload_record_path(&record.id)?;
        let bytes = serde_json::to_vec(record).context("failed to serialize tus upload record")?;
        if bytes.len() as u64 > MAX_UPLOAD_RECORD_BYTES {
            bail!("serialized tus record exceeds its safety limit");
        }
        atomic_write(&path, &bytes)
            .with_context(|| format!("failed to persist tus record {}", path.display()))
    }

    fn validate_upload_record(&self, upload_id: &str, record: &UploadRecord) -> Result<()> {
        validate_upload_id(upload_id)?;
        if record.version != UPLOAD_RECORD_VERSION || record.id != upload_id {
            bail!("tus record identity or version is invalid");
        }
        validate_device_id(&record.device_id)?;
        if record.offset > record.length {
            bail!("tus record offset exceeds its upload length");
        }
        let upload = NewUpload {
            directory: record.directory.clone(),
            original_name: record.original_name.clone(),
            media_type: record.media_type.clone(),
            sha256: record.sha256.clone(),
            length: record.length,
            metadata_header: record.metadata_header.clone(),
        };
        self.validate_new_upload(&upload)?;
        if record
            .delivery_id
            .as_deref()
            .is_some_and(|id| id != record.id)
        {
            bail!("tus record contains an inconsistent delivery id");
        }
        Ok(())
    }

    fn upload_record_path(&self, upload_id: &str) -> Result<PathBuf> {
        validate_upload_id(upload_id)?;
        Ok(self.uploads_dir.join(format!("{upload_id}.json")))
    }

    fn upload_part_path(&self, upload_id: &str) -> Result<PathBuf> {
        validate_upload_id(upload_id)?;
        Ok(self.uploads_dir.join(format!("{upload_id}.part")))
    }
}

impl UploadRecord {
    fn info(&self) -> UploadInfo {
        UploadInfo {
            id: self.id.clone(),
            length: self.length,
            offset: self.offset,
            metadata_header: self.metadata_header.clone(),
            delivery_id: self.delivery_id.clone(),
        }
    }
}

fn validate_upload_id(upload_id: &str) -> Result<()> {
    validate_delivery_id(upload_id)?;
    let parsed = uuid::Uuid::parse_str(upload_id).context("tus upload id is not a UUID")?;
    if parsed.to_string() != upload_id {
        bail!("tus upload id is not in canonical form");
    }
    Ok(())
}

fn open_upload_part(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(path)
        .with_context(|| format!("failed to open tus upload data {}", path.display()))?;
    if !file
        .metadata()
        .with_context(|| format!("failed to inspect tus upload data {}", path.display()))?
        .file_type()
        .is_file()
    {
        bail!("tus upload data {} is not a regular file", path.display());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use sha2::{Digest, Sha256};

    use super::*;

    fn upload(bytes: &[u8]) -> NewUpload {
        NewUpload {
            directory: None,
            original_name: "test.png".to_owned(),
            media_type: "image/png".to_owned(),
            sha256: hex::encode(Sha256::digest(bytes)),
            length: bytes.len() as u64,
            metadata_header: "filename dGVzdC5wbmc=,media_type aW1hZ2UvcG5n,sha256 ignored"
                .to_owned(),
        }
    }

    #[test]
    fn upload_resumes_and_becomes_one_idempotent_delivery() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nresumable upload";
        let created = store.create_upload("linux", upload(bytes)).unwrap();

        let mut first = tempfile::tempfile().unwrap();
        first.write_all(&bytes[..9]).unwrap();
        let appended = store
            .append_upload("linux", &created.id, 0, first, 9)
            .unwrap();
        assert_eq!(
            appended,
            AppendUploadOutcome::Appended(UploadInfo {
                offset: 9,
                ..created.clone()
            })
        );

        let conflict = store
            .append_upload("linux", &created.id, 0, tempfile::tempfile().unwrap(), 0)
            .unwrap();
        assert!(matches!(conflict, AppendUploadOutcome::OffsetMismatch(_)));

        let mut second = tempfile::tempfile().unwrap();
        second.write_all(&bytes[9..]).unwrap();
        let completed = store
            .append_upload("linux", &created.id, 9, second, (bytes.len() - 9) as u64)
            .unwrap();
        let AppendUploadOutcome::Appended(completed) = completed else {
            panic!("expected a completed upload")
        };
        assert_eq!(completed.offset, completed.length);
        assert_eq!(completed.delivery_id.as_deref(), Some(created.id.as_str()));
        assert_eq!(
            store
                .list_pending("linux", 0, None, 50)
                .unwrap()
                .deliveries
                .len(),
            1
        );
        assert_eq!(
            store.get_upload("linux", &created.id).unwrap().unwrap(),
            completed
        );
        assert_eq!(
            store
                .list_pending("linux", 0, None, 50)
                .unwrap()
                .deliveries
                .len(),
            1
        );
    }

    #[test]
    fn uncommitted_tail_is_truncated_to_recorded_offset() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\ncrash repair";
        let created = store.create_upload("linux", upload(bytes)).unwrap();
        let path = store.upload_part_path(&created.id).unwrap();
        fs::write(&path, b"uncommitted bytes").unwrap();

        let repaired = store.get_upload("linux", &created.id).unwrap().unwrap();
        assert_eq!(repaired.offset, 0);
        assert_eq!(fs::metadata(path).unwrap().len(), 0);
    }

    #[test]
    fn invalid_completed_content_is_rejected_and_discarded() {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::new(root.path().join("server"), 1024).unwrap();
        store.initialize().unwrap();
        let bytes = b"plain text pretending to be PNG";
        let created = store.create_upload("linux", upload(bytes)).unwrap();
        let record_path = store.upload_record_path(&created.id).unwrap();
        let part_path = store.upload_part_path(&created.id).unwrap();
        let mut chunk = tempfile::tempfile().unwrap();
        chunk.write_all(bytes).unwrap();

        let error = store
            .append_upload("linux", &created.id, 0, chunk, bytes.len() as u64)
            .unwrap_err();
        assert!(
            error
                .chain()
                .any(|cause| cause.is::<InvalidUploadContent>())
        );
        assert!(!record_path.exists());
        assert!(!part_path.exists());
        assert!(store.get_upload("linux", &created.id).unwrap().is_none());
        assert_eq!(store.stats("linux").unwrap().pending, 0);
    }
}
