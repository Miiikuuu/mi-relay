use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    create_dir_all_durable(parent)?;

    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create a temporary file in {}", parent.display()))?;
    temporary
        .write_all(bytes)
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    sync_directory(parent)?;
    Ok(())
}

pub fn create_dir_all_durable(path: &Path) -> Result<()> {
    let resolved = if path.as_os_str().is_empty() {
        std::env::current_dir().context("failed to determine the current directory")?
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine the current directory")?
            .join(path)
    };
    let mut missing = Vec::new();
    let mut cursor = resolved.as_path();

    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_dir()
                    || (metadata.file_type().is_symlink() && cursor.is_dir())
                {
                    break;
                }
                bail!("{} exists but is not a directory", cursor.display());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().with_context(|| {
                    format!("cannot find an existing parent for {}", resolved.display())
                })?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect directory {}", cursor.display()));
            }
        }
    }

    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&directory).with_context(|| {
                    format!("failed to inspect raced directory {}", directory.display())
                })?;
                if !metadata.file_type().is_dir() {
                    bail!("raced path {} is not a real directory", directory.display());
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create directory {}", directory.display())
                });
            }
        }
        let parent = directory
            .parent()
            .with_context(|| format!("created directory {} has no parent", directory.display()))?;
        sync_directory(parent)?;
    }
    Ok(())
}

pub fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{} is not a regular file", path.display());
    }
    let capacity = metadata.len().min(max_bytes) as usize;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        bail!(
            "{} exceeds the {} byte safety limit",
            path.display(),
            max_bytes
        );
    }
    Ok(bytes)
}

pub fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open directory {} for syncing", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync directory {}", path.display()))
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_read_accepts_regular_files_and_enforces_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data");
        fs::write(&path, b"abcdef").unwrap();

        assert_eq!(read_limited(&path, 6).unwrap(), b"abcdef");
        assert!(read_limited(&path, 5).is_err());
        assert!(read_limited(root.path(), 100).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn limited_read_rejects_a_unix_socket_without_blocking() {
        use std::os::unix::net::UnixListener;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("socket");
        let _listener = UnixListener::bind(&path).unwrap();

        assert!(read_limited(&path, 100).is_err());
    }
}
