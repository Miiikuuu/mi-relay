use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fs2::FileExt;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::fsutil::{atomic_write, create_dir_all_durable, read_limited};
use crate::model::{AppState, STATE_SCHEMA_VERSION};

const MAX_STATE_SIZE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
    lock_path: PathBuf,
}

pub struct StateLock {
    file: File,
}

impl StateStore {
    pub fn new(path: PathBuf) -> Self {
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".lock");
        let lock_path = PathBuf::from(lock_name);
        Self { path, lock_path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_exclusive(&self) -> Result<StateLock> {
        let file = self.open_lock_file()?;
        FileExt::lock_exclusive(&file).with_context(|| {
            format!(
                "failed to acquire exclusive state lock {}",
                self.lock_path.display()
            )
        })?;
        Ok(StateLock { file })
    }

    pub fn lock_shared(&self) -> Result<StateLock> {
        let file = self.open_lock_file()?;
        FileExt::lock_shared(&file).with_context(|| {
            format!(
                "failed to acquire shared state lock {}",
                self.lock_path.display()
            )
        })?;
        Ok(StateLock { file })
    }

    pub fn load(&self) -> Result<AppState> {
        let raw = match read_limited(&self.path, MAX_STATE_SIZE_BYTES) {
            Ok(raw) => raw,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(AppState::default());
            }
            Err(error) => {
                return Err(error).context(format!("failed to read state {}", self.path.display()));
            }
        };
        let state: AppState = serde_json::from_slice(&raw)
            .with_context(|| format!("failed to parse state {}", self.path.display()))?;
        if state.schema_version != STATE_SCHEMA_VERSION {
            bail!(
                "unsupported state schema version {} in {}; expected {}",
                state.schema_version,
                self.path.display(),
                STATE_SCHEMA_VERSION
            );
        }
        for (id, record) in &state.deliveries {
            if id != &record.id {
                bail!(
                    "state {} maps key {:?} to delivery {:?}",
                    self.path.display(),
                    id,
                    record.id
                );
            }
        }
        Ok(state)
    }

    pub fn save(&self, state: &AppState) -> Result<()> {
        if state.schema_version != STATE_SCHEMA_VERSION {
            bail!("refusing to save an unsupported state schema");
        }
        let mut raw = serde_json::to_vec_pretty(state).context("failed to serialize state")?;
        raw.push(b'\n');
        if raw.len() as u64 > MAX_STATE_SIZE_BYTES {
            bail!(
                "refusing to grow state beyond the {} byte safety limit",
                MAX_STATE_SIZE_BYTES
            );
        }
        atomic_write(&self.path, &raw)
            .with_context(|| format!("failed to persist state {}", self.path.display()))
    }

    fn open_lock_file(&self) -> Result<File> {
        if let Some(parent) = self.lock_path.parent() {
            create_dir_all_durable(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options
            .open(&self.lock_path)
            .with_context(|| format!("failed to open state lock {}", self.lock_path.display()))?;
        if !file.metadata()?.file_type().is_file() {
            bail!(
                "state lock {} is not a regular file",
                self.lock_path.display()
            );
        }
        Ok(file)
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn missing_state_loads_as_empty_and_round_trips() {
        let root = tempdir().unwrap();
        let store = StateStore::new(root.path().join("state.json"));
        let _guard = store.lock_exclusive().unwrap();
        let state = store.load().unwrap();
        assert!(state.deliveries.is_empty());
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap(), state);
    }
}
