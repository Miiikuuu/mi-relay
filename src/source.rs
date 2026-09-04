use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::fsutil::{atomic_write, create_dir_all_durable, read_limited, sync_directory, unix_now};
use crate::model::{Delivery, MANIFEST_SCHEMA_VERSION};
use crate::storage::{inspect_file, validate_delivery_id, validate_delivery_metadata};

const MAX_MANIFEST_SIZE_BYTES: u64 = 64 * 1024;
const MAX_ACK_SIZE_BYTES: u64 = 16 * 1024;

pub trait DeliverySource {
    fn scan_pending(&self, max_file_size: u64) -> Result<SourceScan>;
    fn open_payload(&self, delivery: &Delivery) -> Result<Box<dyn Read + Send>>;
    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()>;

    /// Return the delay before retrying a failed full payload transfer.
    ///
    /// The default deliberately disables retries. Network-backed sources can
    /// opt in only for failures they know are transient; integrity and
    /// validation failures must remain permanent.
    fn payload_retry_delay(
        &self,
        _error: &anyhow::Error,
        _failed_attempts: u32,
    ) -> Option<Duration> {
        None
    }
}

#[derive(Debug, Default)]
pub struct SourceScan {
    pub deliveries: Vec<Delivery>,
    pub issues: Vec<SourceIssue>,
}

#[derive(Debug, Clone)]
pub struct SourceIssue {
    pub item: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct FilesystemSource {
    inbox_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AckMarker {
    id: String,
    sha256: String,
    acknowledged_at_unix: u64,
}

impl FilesystemSource {
    pub fn new(inbox_dir: PathBuf) -> Self {
        Self { inbox_dir }
    }

    pub fn inbox_dir(&self) -> &Path {
        &self.inbox_dir
    }

    pub fn enqueue(&self, input: &Path, max_file_size: u64) -> Result<Delivery> {
        create_dir_all_durable(&self.inbox_dir)?;
        let input_inspection = inspect_file(input, max_file_size)?;
        let original_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .context("mock input file name must be valid UTF-8")?
            .to_owned();

        let id = Uuid::new_v4().to_string();
        let payload_name = format!("{id}.payload");
        let payload_path = self.inbox_dir.join(&payload_name);
        let delivery = Delivery {
            schema_version: MANIFEST_SCHEMA_VERSION,
            id: id.clone(),
            original_name,
            payload: payload_name,
            size: input_inspection.size,
            sha256: input_inspection.sha256.clone(),
            media_type: input_inspection.media_type.clone(),
            created_at_unix: Some(unix_now()),
        };
        validate_delivery_metadata(&delivery, max_file_size)?;

        let input_file =
            File::open(input).with_context(|| format!("failed to open {}", input.display()))?;
        let mut temporary = NamedTempFile::new_in(&self.inbox_dir).with_context(|| {
            format!(
                "failed to create a temporary file in {}",
                self.inbox_dir.display()
            )
        })?;
        let mut limited_input = input_file.take(max_file_size.saturating_add(1));
        std::io::copy(&mut limited_input, temporary.as_file_mut())
            .with_context(|| format!("failed to copy {} into mock inbox", input.display()))?;
        temporary
            .as_file_mut()
            .sync_all()
            .with_context(|| format!("failed to sync mock payload for {}", input.display()))?;

        let copied_inspection = inspect_file(temporary.path(), max_file_size)?;
        if copied_inspection != input_inspection {
            bail!(
                "input {} changed while it was being enqueued",
                input.display()
            );
        }
        temporary
            .persist_noclobber(&payload_path)
            .map_err(|error| error.error)
            .with_context(|| {
                format!("failed to publish mock payload {}", payload_path.display())
            })?;
        sync_directory(&self.inbox_dir)?;

        let manifest_path = self.inbox_dir.join(format!("{id}.json"));
        let mut manifest = serde_json::to_vec_pretty(&delivery)
            .context("failed to serialize mock delivery manifest")?;
        manifest.push(b'\n');
        atomic_write(&manifest_path, &manifest)?;
        Ok(delivery)
    }

    fn ack_path(&self, id: &str) -> PathBuf {
        self.inbox_dir.join(".acks").join(format!("{id}.json"))
    }

    fn read_ack(&self, id: &str) -> Result<Option<AckMarker>> {
        let path = self.ack_path(id);
        let ack_dir = path.parent().context("ACK path has no parent")?;
        match fs::symlink_metadata(ack_dir) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => bail!("ACK path {} is not a real directory", ack_dir.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect ACK directory {}", ack_dir.display())
                });
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => bail!("ACK marker {} is not a regular file", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect ACK marker {}", path.display()));
            }
        }
        match read_limited(&path, MAX_ACK_SIZE_BYTES) {
            Ok(raw) => serde_json::from_slice(&raw)
                .with_context(|| format!("failed to parse ACK marker {}", path.display()))
                .map(Some),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(None)
            }
            Err(error) => {
                Err(error).with_context(|| format!("failed to read ACK marker {}", path.display()))
            }
        }
    }
}

impl DeliverySource for FilesystemSource {
    fn scan_pending(&self, max_file_size: u64) -> Result<SourceScan> {
        create_dir_all_durable(&self.inbox_dir)?;
        let mut scan = SourceScan::default();
        let entries = fs::read_dir(&self.inbox_dir)
            .with_context(|| format!("failed to scan mock inbox {}", self.inbox_dir.display()))?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan.issues.push(SourceIssue {
                        item: self.inbox_dir.display().to_string(),
                        message: format!("failed to read an inbox directory entry: {error}"),
                    });
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan.issues.push(SourceIssue {
                        item: path.display().to_string(),
                        message: format!("failed to inspect inbox entry: {error}"),
                    });
                    continue;
                }
            };
            let is_manifest = path.extension().is_some_and(|ext| ext == "json");
            if file_type.is_symlink() && is_manifest {
                scan.issues.push(SourceIssue {
                    item: path.display().to_string(),
                    message: "manifest symlinks are not allowed".into(),
                });
            } else if file_type.is_file() && is_manifest {
                paths.push(path);
            }
        }
        paths.sort();

        let mut by_id: BTreeMap<String, (PathBuf, Delivery)> = BTreeMap::new();
        let mut conflicts = BTreeSet::new();

        for path in paths {
            let item = path.display().to_string();
            let result = (|| -> Result<Delivery> {
                let raw = read_limited(&path, MAX_MANIFEST_SIZE_BYTES)
                    .with_context(|| format!("failed to read manifest {}", path.display()))?;
                let delivery: Delivery = serde_json::from_slice(&raw)
                    .with_context(|| format!("failed to parse manifest {}", path.display()))?;
                validate_delivery_metadata(&delivery, max_file_size)?;
                if delivery.payload.is_empty() {
                    bail!("filesystem delivery {} is missing payload", delivery.id);
                }
                Ok(delivery)
            })();

            let delivery = match result {
                Ok(delivery) => delivery,
                Err(error) => {
                    scan.issues.push(SourceIssue {
                        item,
                        message: format!("{error:#}"),
                    });
                    continue;
                }
            };

            match self.read_ack(&delivery.id) {
                Ok(Some(marker))
                    if marker.id == delivery.id && marker.sha256 == delivery.sha256 =>
                {
                    continue;
                }
                Ok(Some(marker)) => {
                    scan.issues.push(SourceIssue {
                        item,
                        message: format!(
                            "ACK marker conflicts with delivery {} (marker id={}, digest={})",
                            delivery.id, marker.id, marker.sha256
                        ),
                    });
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    scan.issues.push(SourceIssue {
                        item,
                        message: format!("{error:#}"),
                    });
                    continue;
                }
            }

            if let Some((previous_path, previous)) = by_id.get(&delivery.id) {
                conflicts.insert(delivery.id.clone());
                scan.issues.push(SourceIssue {
                    item,
                    message: format!(
                        "duplicate delivery id {} also appears in {} (digests {} and {})",
                        delivery.id,
                        previous_path.display(),
                        previous.sha256,
                        delivery.sha256
                    ),
                });
            } else {
                by_id.insert(delivery.id.clone(), (path, delivery));
            }
        }

        for id in conflicts {
            by_id.remove(&id);
        }
        scan.deliveries = by_id.into_values().map(|(_, delivery)| delivery).collect();
        scan.deliveries.sort_by(|left, right| {
            left.created_at_unix
                .unwrap_or(0)
                .cmp(&right.created_at_unix.unwrap_or(0))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(scan)
    }

    fn open_payload(&self, delivery: &Delivery) -> Result<Box<dyn Read + Send>> {
        validate_delivery_id(&delivery.id)?;
        let inbox = fs::canonicalize(&self.inbox_dir).with_context(|| {
            format!("failed to resolve mock inbox {}", self.inbox_dir.display())
        })?;
        let requested = self.inbox_dir.join(&delivery.payload);
        let resolved = fs::canonicalize(&requested).with_context(|| {
            format!(
                "failed to resolve payload {} for delivery {}",
                requested.display(),
                delivery.id
            )
        })?;
        if resolved.parent() != Some(inbox.as_path()) {
            bail!(
                "payload for delivery {} resolves outside the mock inbox",
                delivery.id
            );
        }
        let file = open_regular_payload(&resolved).with_context(|| {
            format!(
                "failed to open payload {} for delivery {}",
                resolved.display(),
                delivery.id
            )
        })?;
        Ok(Box::new(file))
    }

    fn acknowledge(&self, id: &str, sha256: &str) -> Result<()> {
        validate_delivery_id(id)?;
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("cannot acknowledge delivery {id}: invalid SHA-256 digest");
        }
        let ack_path = self.ack_path(id);
        let ack_dir = ack_path.parent().context("ACK path has no parent")?;
        if let Some(marker) = self.read_ack(id)? {
            if marker.id == id && marker.sha256 == sha256 {
                return sync_directory(ack_dir);
            }
            bail!("existing ACK marker for delivery {id} has conflicting metadata");
        }

        let marker = AckMarker {
            id: id.to_owned(),
            sha256: sha256.to_owned(),
            acknowledged_at_unix: unix_now(),
        };
        create_dir_all_durable(&self.inbox_dir)?;
        let ack_dir_existed = match fs::symlink_metadata(ack_dir) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() {
                    bail!("ACK path {} is not a real directory", ack_dir.display());
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect ACK directory {}", ack_dir.display())
                });
            }
        };
        if !ack_dir_existed {
            match fs::create_dir(ack_dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create ACK directory {}", ack_dir.display())
                    });
                }
            }
            let metadata = fs::symlink_metadata(ack_dir).with_context(|| {
                format!("failed to inspect ACK directory {}", ack_dir.display())
            })?;
            if !metadata.file_type().is_dir() {
                bail!("new ACK path {} is not a real directory", ack_dir.display());
            }
            sync_directory(&self.inbox_dir)?;
        }
        let mut bytes =
            serde_json::to_vec_pretty(&marker).context("failed to serialize ACK marker")?;
        bytes.push(b'\n');

        let mut temporary = NamedTempFile::new_in(ack_dir)
            .with_context(|| format!("failed to create temporary ACK in {}", ack_dir.display()))?;
        temporary
            .write_all(&bytes)
            .context("failed to write temporary ACK marker")?;
        temporary
            .as_file_mut()
            .sync_all()
            .context("failed to sync temporary ACK marker")?;
        match temporary.persist_noclobber(&ack_path) {
            Ok(_) => sync_directory(ack_dir),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                let marker = self
                    .read_ack(id)?
                    .context("ACK marker disappeared during a concurrent write")?;
                if marker.id == id && marker.sha256 == sha256 {
                    sync_directory(ack_dir)
                } else {
                    bail!("concurrent ACK marker for delivery {id} has conflicting metadata")
                }
            }
            Err(error) => Err(error.error)
                .with_context(|| format!("failed to publish ACK marker {}", ack_path.display())),
        }
    }
}

fn open_regular_payload(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);

    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        bail!("payload {} is not a regular file", path.display());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use tempfile::tempdir;

    use super::*;

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"mock source test");
        bytes
    }

    #[test]
    fn enqueue_scan_open_and_ack_is_idempotent() {
        let root = tempdir().unwrap();
        let inbox = root.path().join("inbox");
        let input = root.path().join("input.png");
        fs::write(&input, png_bytes()).unwrap();
        let source = FilesystemSource::new(inbox);

        let delivery = source.enqueue(&input, 1024).unwrap();
        let scan = source.scan_pending(1024).unwrap();
        assert_eq!(scan.deliveries, vec![delivery.clone()]);

        let mut payload = Vec::new();
        source
            .open_payload(&delivery)
            .unwrap()
            .read_to_end(&mut payload)
            .unwrap();
        assert_eq!(payload, png_bytes());

        source.acknowledge(&delivery.id, &delivery.sha256).unwrap();
        source.acknowledge(&delivery.id, &delivery.sha256).unwrap();
        assert!(source.scan_pending(1024).unwrap().deliveries.is_empty());
    }

    #[test]
    fn malformed_manifest_does_not_block_valid_deliveries() {
        let root = tempdir().unwrap();
        let inbox = root.path().join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        fs::write(inbox.join("bad.json"), b"not json").unwrap();
        let input = root.path().join("input.png");
        fs::write(&input, png_bytes()).unwrap();
        let source = FilesystemSource::new(inbox);
        source.enqueue(&input, 1024).unwrap();

        let scan = source.scan_pending(1024).unwrap();
        assert_eq!(scan.deliveries.len(), 1);
        assert_eq!(scan.issues.len(), 1);
    }

    #[test]
    fn oversized_manifest_does_not_block_valid_deliveries() {
        let root = tempdir().unwrap();
        let inbox = root.path().join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        fs::write(
            inbox.join("huge.json"),
            vec![b' '; (MAX_MANIFEST_SIZE_BYTES + 1) as usize],
        )
        .unwrap();
        let input = root.path().join("input.png");
        fs::write(&input, png_bytes()).unwrap();
        let source = FilesystemSource::new(inbox);
        source.enqueue(&input, 1024).unwrap();

        let scan = source.scan_pending(1024).unwrap();
        assert_eq!(scan.deliveries.len(), 1);
        assert_eq!(scan.issues.len(), 1);
        assert!(scan.issues[0].message.contains("safety limit"));
    }

    #[test]
    fn duplicate_delivery_id_is_quarantined() {
        let root = tempdir().unwrap();
        let inbox = root.path().join("inbox");
        let input = root.path().join("input.png");
        fs::write(&input, png_bytes()).unwrap();
        let source = FilesystemSource::new(inbox.clone());
        let delivery = source.enqueue(&input, 1024).unwrap();
        fs::copy(
            inbox.join(format!("{}.json", delivery.id)),
            inbox.join("duplicate.json"),
        )
        .unwrap();

        let scan = source.scan_pending(1024).unwrap();
        assert!(scan.deliveries.is_empty());
        assert_eq!(scan.issues.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn payload_symlink_cannot_escape_the_inbox() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let inbox = root.path().join("inbox");
        let input = root.path().join("input.png");
        fs::write(&input, png_bytes()).unwrap();
        let source = FilesystemSource::new(inbox.clone());
        let delivery = source.enqueue(&input, 1024).unwrap();
        let payload = inbox.join(&delivery.payload);
        fs::remove_file(&payload).unwrap();
        symlink(&input, &payload).unwrap();

        assert!(source.open_payload(&delivery).is_err());
    }
}
