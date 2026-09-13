use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use tempfile::Builder;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::fsutil::{create_dir_all_durable, read_limited, sync_directory};
use crate::model::{Delivery, MANIFEST_SCHEMA_VERSION};

pub use crate::protocol::{validate_delivery_id, validate_sha256};

const COPY_BUFFER_SIZE: usize = 64 * 1024;
const HEADER_SIZE: usize = 512;
const STAGING_DIR_NAME: &str = ".mirelay-staging-v1";
const STAGING_OWNER_FILE: &str = ".owner";
const STAGING_OWNER_CONTENT: &[u8] = b"MiRelay staging directory v1\n";
const STAGING_OWNER_TEMP_PREFIX: &str = ".mirelay-owner-";
const LIBRARY_LOCK_FILE: &str = ".mirelay.lock";

pub struct LibraryLock {
    file: File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInspection {
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
    pub extension: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAsset {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
}

pub fn lock_library(library_dir: &Path) -> Result<LibraryLock> {
    create_dir_all_durable(library_dir)?;
    let lock_path = library_dir.join(LIBRARY_LOCK_FILE);
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("library lock {} is not a regular file", lock_path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect library lock {}", lock_path.display())
            });
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(&lock_path)
        .with_context(|| format!("failed to open library lock {}", lock_path.display()))?;
    if !file.metadata()?.file_type().is_file() {
        bail!("library lock {} is not a regular file", lock_path.display());
    }
    FileExt::lock_exclusive(&file)
        .with_context(|| format!("failed to lock media library {}", library_dir.display()))?;
    sync_directory(library_dir)?;
    Ok(LibraryLock { file })
}

impl Drop for LibraryLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn cleanup_staging(library_dir: &Path) -> Result<usize> {
    create_dir_all_durable(library_dir)?;
    let staging_dir = ensure_staging_directory(library_dir)?;

    let mut removed = 0;
    for entry in fs::read_dir(&staging_dir)
        .with_context(|| format!("failed to scan staging directory {}", staging_dir.display()))?
    {
        let entry = entry
            .with_context(|| format!("failed to read an entry in {}", staging_dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect staging entry {}", path.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_owned_temporary = name.starts_with("mirelay-") && name.ends_with(".part");
        if is_owned_temporary && (file_type.is_file() || file_type.is_symlink()) {
            fs::remove_file(&path).with_context(|| {
                format!("failed to remove stale staging file {}", path.display())
            })?;
            removed += 1;
        }
    }
    if removed > 0 {
        sync_directory(&staging_dir)?;
    }
    Ok(removed)
}

pub fn receive(
    delivery: &Delivery,
    mut reader: Box<dyn Read + Send>,
    library_dir: &Path,
    max_file_size: u64,
) -> Result<StoredAsset> {
    validate_delivery_metadata(delivery, max_file_size)?;

    create_dir_all_durable(library_dir)?;
    let staging_dir = ensure_staging_directory(library_dir)?;

    let mut temporary = Builder::new()
        .prefix(&format!("mirelay-{}-", delivery.id))
        .suffix(".part")
        .tempfile_in(&staging_dir)
        .with_context(|| {
            format!(
                "failed to create a staging file in {}",
                staging_dir.display()
            )
        })?;
    let inspection = copy_and_inspect(
        &mut reader,
        temporary.as_file_mut(),
        max_file_size,
        Some(delivery.size),
    )
    .with_context(|| format!("failed to receive delivery {}", delivery.id))?;

    if inspection.sha256 != delivery.sha256.to_ascii_lowercase() {
        bail!(
            "delivery {} checksum mismatch: expected {}, got {}",
            delivery.id,
            delivery.sha256,
            inspection.sha256
        );
    }
    if inspection.media_type != delivery.media_type {
        bail!(
            "delivery {} media type mismatch: manifest says {}, content is {}",
            delivery.id,
            delivery.media_type,
            inspection.media_type
        );
    }

    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to sync staged delivery {}", delivery.id))?;

    let shard_dir = library_dir.join(&inspection.sha256[..2]);
    ensure_child_directory(library_dir, &shard_dir)?;
    let destination = shard_dir.join(format!("{}.{}", inspection.sha256, inspection.extension));

    if destination.exists() {
        let existing = inspect_file(&destination, max_file_size)?;
        if existing != inspection {
            bail!(
                "content-addressed file {} exists but does not match its digest",
                destination.display()
            );
        }
        sync_directory(&shard_dir)?;
        return Ok(StoredAsset {
            path: destination,
            size: inspection.size,
            sha256: inspection.sha256,
            media_type: inspection.media_type,
        });
    }

    match temporary.persist_noclobber(&destination) {
        Ok(_) => sync_directory(&shard_dir)?,
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = inspect_file(&destination, max_file_size)?;
            if existing != inspection {
                bail!(
                    "content-addressed file {} appeared concurrently with different content",
                    destination.display()
                );
            }
            sync_directory(&shard_dir)?;
        }
        Err(error) => {
            return Err(error.error).with_context(|| {
                format!(
                    "failed to commit delivery {} to {}",
                    delivery.id,
                    destination.display()
                )
            });
        }
    }

    Ok(StoredAsset {
        path: destination,
        size: inspection.size,
        sha256: inspection.sha256,
        media_type: inspection.media_type,
    })
}

pub fn inspect_file(path: &Path, max_file_size: u64) -> Result<FileInspection> {
    let mut source = File::open(path)
        .with_context(|| format!("failed to open media file {}", path.display()))?;
    let mut sink = std::io::sink();
    copy_and_inspect(&mut source, &mut sink, max_file_size, None)
        .with_context(|| format!("failed to inspect media file {}", path.display()))
}

pub fn verify_recorded_file(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    expected_media_type: &str,
    max_file_size: u64,
) -> Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut source = options
        .open(path)
        .with_context(|| format!("failed to open stored media file {}", path.display()))?;
    if !source
        .metadata()
        .with_context(|| format!("failed to inspect stored media file {}", path.display()))?
        .file_type()
        .is_file()
    {
        bail!("stored media file {} is not a regular file", path.display());
    }
    let mut sink = std::io::sink();
    let inspection = copy_and_inspect(&mut source, &mut sink, max_file_size, None)
        .with_context(|| format!("failed to inspect stored media file {}", path.display()))?;
    if inspection.size != expected_size
        || inspection.sha256 != expected_sha256
        || inspection.media_type != expected_media_type
    {
        bail!(
            "stored file {} no longer matches its recorded metadata",
            path.display()
        );
    }
    Ok(())
}

pub fn validate_delivery_metadata(delivery: &Delivery, max_file_size: u64) -> Result<()> {
    if delivery.schema_version != MANIFEST_SCHEMA_VERSION {
        bail!(
            "delivery {} uses unsupported manifest schema {}; expected {}",
            delivery.id,
            delivery.schema_version,
            MANIFEST_SCHEMA_VERSION
        );
    }
    validate_delivery_id(&delivery.id)?;
    if !delivery.payload.is_empty() {
        validate_single_component(&delivery.payload, "payload")?;
    }
    validate_single_component(&delivery.original_name, "original_name")?;
    if delivery.size == 0 {
        bail!("delivery {} is empty", delivery.id);
    }
    if delivery.size > max_file_size {
        bail!(
            "delivery {} is {} bytes, exceeding the {} byte limit",
            delivery.id,
            delivery.size,
            max_file_size
        );
    }
    validate_sha256(&delivery.sha256)
        .with_context(|| format!("delivery {} has an invalid SHA-256 digest", delivery.id))?;
    if extension_for_media_type(&delivery.media_type).is_none() {
        bail!(
            "delivery {} uses unsupported media type {}",
            delivery.id,
            delivery.media_type
        );
    }
    Ok(())
}

pub fn content_path(library_dir: &Path, sha256: &str, media_type: &str) -> Result<PathBuf> {
    validate_sha256(sha256)?;
    let extension = extension_for_media_type(media_type)
        .with_context(|| format!("unsupported media type {media_type:?}"))?;
    Ok(library_dir
        .join(&sha256[..2])
        .join(format!("{sha256}.{extension}")))
}

/// Move a suspect content-addressed object into MiRelay's staging area.
///
/// The caller must hold the library lock. The quarantine name is recognized by
/// `cleanup_staging`, so a failed repair cannot leave an unbounded collection
/// of damaged objects behind.
pub fn quarantine_content_object(
    library_dir: &Path,
    sha256: &str,
    media_type: &str,
) -> Result<bool> {
    let path = content_path(library_dir, sha256, media_type)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect suspect object {}", path.display()));
        }
    };
    if !(metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
        bail!("suspect object {} is not a file", path.display());
    }

    let staging_dir = ensure_staging_directory(library_dir)?;
    let quarantined = staging_dir.join(format!("mirelay-repair-{}.part", uuid::Uuid::new_v4()));
    fs::rename(&path, &quarantined).with_context(|| {
        format!(
            "failed to quarantine suspect object {} as {}",
            path.display(),
            quarantined.display()
        )
    })?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    sync_directory(&staging_dir)?;
    Ok(true)
}

fn validate_single_component(value: &str, field: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 255
        || value.contains('\0')
        || value.chars().any(char::is_control)
        || path.file_name().and_then(|name| name.to_str()) != Some(value)
    {
        bail!("{field} must be a single UTF-8 file name, got {value:?}");
    }
    Ok(())
}

fn copy_and_inspect(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    max_file_size: u64,
    expected_size: Option<u64>,
) -> Result<FileInspection> {
    let mut hasher = Sha256::new();
    let mut header = Vec::with_capacity(HEADER_SIZE);
    let mut text_probe = Utf8TextProbe::default();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];

    loop {
        let read = reader
            .read(&mut buffer)
            .context("failed while reading media bytes")?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .context("media size overflow")?;
        if size > max_file_size {
            bail!("media exceeds the configured {max_file_size} byte limit");
        }
        if let Some(expected) = expected_size
            && size > expected
        {
            bail!("media is larger than its declared {expected} byte size");
        }

        if header.len() < HEADER_SIZE {
            let remaining = HEADER_SIZE - header.len();
            header.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        hasher.update(&buffer[..read]);
        text_probe.observe(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .context("failed while writing media bytes")?;
    }

    if let Some(expected) = expected_size
        && size != expected
    {
        bail!("media size mismatch: expected {expected} bytes, got {size}");
    }
    if size == 0 {
        bail!("media file is empty");
    }

    let (media_type, extension) = detect_media_type(&header, text_probe.is_plain_text());
    Ok(FileInspection {
        size,
        sha256: hex::encode(hasher.finalize()),
        media_type: media_type.to_owned(),
        extension,
    })
}

fn detect_media_type(header: &[u8], is_plain_text: bool) -> (&'static str, &'static str) {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        return ("image/png", "png");
    }
    if header.starts_with(&[0xff, 0xd8, 0xff]) {
        return ("image/jpeg", "jpg");
    }
    if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        return ("image/gif", "gif");
    }
    if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        return ("image/webp", "webp");
    }
    if header.len() >= 12
        && &header[4..8] == b"ftyp"
        && (&header[8..12] == b"avif" || &header[8..12] == b"avis")
    {
        return ("image/avif", "avif");
    }
    if header.starts_with(b"%PDF-") {
        return ("application/pdf", "pdf");
    }
    if header.starts_with(b"PK\x03\x04")
        || header.starts_with(b"PK\x05\x06")
        || header.starts_with(b"PK\x07\x08")
    {
        return ("application/zip", "zip");
    }
    if header.starts_with(&[0x1f, 0x8b]) {
        return ("application/gzip", "gz");
    }
    if header.starts_with(&[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]) {
        return ("application/x-7z-compressed", "7z");
    }
    if header.starts_with(b"Rar!\x1a\x07\x00") || header.starts_with(b"Rar!\x1a\x07\x01\x00") {
        return ("application/vnd.rar", "rar");
    }
    if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        return ("audio/wav", "wav");
    }
    if header.starts_with(b"fLaC") {
        return ("audio/flac", "flac");
    }
    if header.starts_with(b"OggS") {
        return ("application/ogg", "ogg");
    }
    if header.starts_with(b"ID3")
        || (header.len() >= 2 && header[0] == 0xff && header[1] & 0xe0 == 0xe0)
    {
        return ("audio/mpeg", "mp3");
    }
    if header.len() >= 12 && &header[4..8] == b"ftyp" {
        return ("video/mp4", "mp4");
    }
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return ("video/webm", "webm");
    }
    if is_plain_text {
        return ("text/plain", "txt");
    }
    ("application/octet-stream", "bin")
}

pub fn extension_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/avif" => Some("avif"),
        "text/plain" => Some("txt"),
        "application/pdf" => Some("pdf"),
        "application/zip" => Some("zip"),
        "application/gzip" => Some("gz"),
        "application/x-7z-compressed" => Some("7z"),
        "application/vnd.rar" => Some("rar"),
        "application/octet-stream" => Some("bin"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" => Some("wav"),
        "audio/flac" => Some("flac"),
        "application/ogg" => Some("ogg"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        _ => None,
    }
}

pub fn is_image_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
}

pub fn is_supported_content_extension(extension: &str) -> bool {
    matches!(
        extension,
        "png"
            | "jpg"
            | "gif"
            | "webp"
            | "avif"
            | "txt"
            | "pdf"
            | "zip"
            | "gz"
            | "7z"
            | "rar"
            | "bin"
            | "mp3"
            | "wav"
            | "flac"
            | "ogg"
            | "mp4"
            | "webm"
    )
}

#[derive(Debug)]
struct Utf8TextProbe {
    valid: bool,
    incomplete: Vec<u8>,
}

impl Default for Utf8TextProbe {
    fn default() -> Self {
        Self {
            valid: true,
            incomplete: Vec::with_capacity(4),
        }
    }
}

impl Utf8TextProbe {
    fn observe(&mut self, bytes: &[u8]) {
        if !self.valid {
            return;
        }
        let mut combined = std::mem::take(&mut self.incomplete);
        combined.extend_from_slice(bytes);
        match std::str::from_utf8(&combined) {
            Ok(text) => {
                self.valid = text.chars().all(is_plain_text_character);
            }
            Err(error) if error.error_len().is_none() => {
                let valid_up_to = error.valid_up_to();
                let (prefix, incomplete) = combined.split_at(valid_up_to);
                self.valid = std::str::from_utf8(prefix)
                    .is_ok_and(|text| text.chars().all(is_plain_text_character))
                    && incomplete.len() <= 3;
                if self.valid {
                    self.incomplete.extend_from_slice(incomplete);
                }
            }
            Err(_) => self.valid = false,
        }
    }

    fn is_plain_text(&self) -> bool {
        self.valid && self.incomplete.is_empty()
    }
}

fn is_plain_text_character(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\t') || !character.is_control()
}

fn ensure_staging_directory(library_dir: &Path) -> Result<PathBuf> {
    let staging_dir = library_dir.join(STAGING_DIR_NAME);
    ensure_child_directory(library_dir, &staging_dir)?;
    let owner_path = staging_dir.join(STAGING_OWNER_FILE);

    match fs::symlink_metadata(&owner_path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!(
                    "staging ownership marker {} is not a regular file",
                    owner_path.display()
                );
            }
            let contents = read_limited(&owner_path, 128)?;
            if contents != STAGING_OWNER_CONTENT {
                bail!(
                    "staging directory {} is not owned by this MiRelay format",
                    staging_dir.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut removed_owner_temps = false;
            for entry in fs::read_dir(&staging_dir).with_context(|| {
                format!(
                    "failed to inspect staging directory {}",
                    staging_dir.display()
                )
            })? {
                let entry = entry.with_context(|| {
                    format!(
                        "failed to inspect staging directory {}",
                        staging_dir.display()
                    )
                })?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(STAGING_OWNER_TEMP_PREFIX) && name.ends_with(".tmp") {
                    let file_type = entry.file_type().with_context(|| {
                        format!("failed to inspect owner temp {}", entry.path().display())
                    })?;
                    if file_type.is_file() || file_type.is_symlink() {
                        fs::remove_file(entry.path()).with_context(|| {
                            format!("failed to remove owner temp {}", entry.path().display())
                        })?;
                        removed_owner_temps = true;
                        continue;
                    }
                }
                bail!(
                    "refusing to claim non-empty staging directory {}",
                    staging_dir.display()
                );
            }
            if removed_owner_temps {
                sync_directory(&staging_dir)?;
            }
            create_staging_owner(&staging_dir, &owner_path)?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect staging ownership marker {}",
                    owner_path.display()
                )
            });
        }
    }
    Ok(staging_dir)
}

fn create_staging_owner(staging_dir: &Path, owner_path: &Path) -> Result<()> {
    let mut temporary = Builder::new()
        .prefix(STAGING_OWNER_TEMP_PREFIX)
        .suffix(".tmp")
        .tempfile_in(staging_dir)
        .with_context(|| {
            format!(
                "failed to create staging ownership marker in {}",
                staging_dir.display()
            )
        })?;
    temporary
        .write_all(STAGING_OWNER_CONTENT)
        .context("failed to write staging ownership marker")?;
    temporary
        .as_file_mut()
        .sync_all()
        .context("failed to sync staging ownership marker")?;
    match temporary.persist_noclobber(owner_path) {
        Ok(_) => sync_directory(staging_dir),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let contents = read_limited(owner_path, 128)?;
            if contents == STAGING_OWNER_CONTENT {
                Ok(())
            } else {
                bail!(
                    "concurrent staging ownership marker {} has unexpected contents",
                    owner_path.display()
                )
            }
        }
        Err(error) => Err(error.error).with_context(|| {
            format!(
                "failed to publish staging ownership marker {}",
                owner_path.display()
            )
        }),
    }
}

fn ensure_child_directory(parent: &Path, child: &Path) -> Result<()> {
    let existed = match fs::symlink_metadata(child) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                bail!(
                    "internal path {} exists but is not a real directory",
                    child.display()
                );
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect directory {}", child.display()));
        }
    };

    if !existed {
        fs::create_dir(child)
            .with_context(|| format!("failed to create directory {}", child.display()))?;
        let metadata = fs::symlink_metadata(child)
            .with_context(|| format!("failed to inspect directory {}", child.display()))?;
        if !metadata.file_type().is_dir() {
            bail!(
                "new internal path {} is not a real directory",
                child.display()
            );
        }
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use super::*;

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"test payload");
        bytes
    }

    fn delivery_for(bytes: &[u8]) -> Delivery {
        Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: "delivery-1".into(),
            original_name: "wallpaper.png".into(),
            payload: "delivery-1.payload".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            media_type: "image/png".into(),
            created_at_unix: None,
        }
    }

    #[test]
    fn receive_verifies_and_commits_by_content_hash() {
        let root = tempdir().unwrap();
        let bytes = png_bytes();
        let delivery = delivery_for(&bytes);

        let stored = receive(
            &delivery,
            Box::new(Cursor::new(bytes.clone())),
            root.path(),
            1024,
        )
        .unwrap();

        assert_eq!(fs::read(&stored.path).unwrap(), bytes);
        assert!(stored.path.ends_with(format!("{}.png", delivery.sha256)));
    }

    #[test]
    fn receive_rejects_checksum_mismatch_without_committing() {
        let root = tempdir().unwrap();
        let bytes = png_bytes();
        let mut delivery = delivery_for(&bytes);
        delivery.sha256 = "0".repeat(64);

        assert!(receive(&delivery, Box::new(Cursor::new(bytes)), root.path(), 1024).is_err());
        assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
            let entry = entry.unwrap();
            entry.file_name() == STAGING_DIR_NAME
        }));
    }

    #[test]
    fn receive_commits_text_and_unknown_binary_with_canonical_extensions() {
        let root = tempdir().unwrap();
        let cases: [(&[u8], &str, &str); 2] = [
            (b"hello from MiRelay\n", "text/plain", "txt"),
            (
                b"\x00\x01\x02\xffunknown binary payload",
                "application/octet-stream",
                "bin",
            ),
        ];

        for (index, (bytes, media_type, extension)) in cases.into_iter().enumerate() {
            let delivery = Delivery {
                schema_version: MANIFEST_SCHEMA_VERSION,
                id: format!("delivery-{index}"),
                original_name: format!("file-{index}"),
                payload: format!("delivery-{index}.payload"),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
                media_type: media_type.to_owned(),
                created_at_unix: None,
            };
            let stored = receive(
                &delivery,
                Box::new(Cursor::new(bytes.to_vec())),
                root.path(),
                1024,
            )
            .unwrap();
            assert_eq!(stored.media_type, media_type);
            assert!(
                stored
                    .path
                    .ends_with(format!("{}.{extension}", delivery.sha256))
            );
            assert_eq!(fs::read(stored.path).unwrap(), bytes);
        }
    }

    #[test]
    fn content_detection_covers_documents_archives_audio_and_video() {
        let cases: &[(&[u8], &str, &str)] = &[
            (b"%PDF-1.7\n", "application/pdf", "pdf"),
            (b"PK\x03\x04archive", "application/zip", "zip"),
            (b"PK\x05\x06empty zip", "application/zip", "zip"),
            (b"\x1f\x8bgzip", "application/gzip", "gz"),
            (
                b"\x37\x7a\xbc\xaf\x27\x1carchive",
                "application/x-7z-compressed",
                "7z",
            ),
            (b"Rar!\x1a\x07\x00archive", "application/vnd.rar", "rar"),
            (b"ID3\x04\x00\x00audio", "audio/mpeg", "mp3"),
            (b"RIFF\x10\x00\x00\x00WAVEaudio", "audio/wav", "wav"),
            (b"fLaCaudio", "audio/flac", "flac"),
            (b"OggSaudio", "application/ogg", "ogg"),
            (b"\x00\x00\x00\x18ftypisomvideo", "video/mp4", "mp4"),
            (b"\x1a\x45\xdf\xa3webm", "video/webm", "webm"),
        ];
        for (bytes, media_type, extension) in cases {
            assert_eq!(detect_media_type(bytes, false), (*media_type, *extension));
        }
        assert_eq!(
            detect_media_type(b"ordinary UTF-8 text", true),
            ("text/plain", "txt")
        );
        assert_eq!(
            detect_media_type(b"\x00\xffunrecognized", false),
            ("application/octet-stream", "bin")
        );
    }

    #[test]
    fn text_detection_handles_split_utf8_and_rejects_binary_controls() {
        let encoded = "跨设备".as_bytes();
        let mut text = Utf8TextProbe::default();
        text.observe(&encoded[..2]);
        text.observe(&encoded[2..5]);
        text.observe(&encoded[5..]);
        assert!(text.is_plain_text());

        let mut binary = Utf8TextProbe::default();
        binary.observe(b"looks like text\x00but contains NUL");
        assert!(!binary.is_plain_text());
    }

    #[test]
    fn rejects_path_traversal_in_manifest() {
        let bytes = png_bytes();
        let mut delivery = delivery_for(&bytes);
        delivery.payload = "../secret".into();
        assert!(validate_delivery_metadata(&delivery, 1024).is_err());
    }

    #[test]
    fn cleanup_removes_crash_leftovers_without_touching_committed_files() {
        let root = tempdir().unwrap();
        let staging = ensure_staging_directory(root.path()).unwrap();
        fs::write(staging.join("mirelay-old.part"), b"partial").unwrap();
        fs::write(staging.join("unrelated.txt"), b"keep").unwrap();
        fs::write(root.path().join("committed.png"), png_bytes()).unwrap();

        assert_eq!(cleanup_staging(root.path()).unwrap(), 1);
        assert!(staging.join(STAGING_OWNER_FILE).exists());
        assert!(staging.join("unrelated.txt").exists());
        assert!(root.path().join("committed.png").exists());
    }
}
