//! Linux directory projection. No followed symlinks, no deletions, no blind
//! overwrite: replaced inodes are retained beside the file as private history.
use super::{client::DirectoryClient, lock::DirectoryLock, *};
use crate::{
    fsutil::{atomic_write, create_dir_all_durable, read_limited, sync_directory},
    source::DeliverySource,
    state::{StateLock, StateStore},
};
use anyhow::{Context, bail};
use nix::{
    errno::Errno,
    fcntl::{AtFlags, OFlag, RenameFlags, open, openat, renameat2},
    sys::stat::{Mode, SFlag, fstatat, mkdirat},
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

pub const MAX_FILE: u64 = 100 * 1024 * 1024;
const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const DIR_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

pub struct Root {
    path: PathBuf,
    file: File,
}
impl Root {
    pub fn open(path: &Path) -> Result<Self> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut file = File::from(open(Path::new("/"), DIR_FLAGS, Mode::empty())?);
        for part in path.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name) => {
                    file = File::from(openat(&file, name, DIR_FLAGS, Mode::empty())?)
                }
                _ => bail!("Use an absolute directory without dot or parent components."),
            }
        }
        Ok(Self { path, file })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn identity(&self) -> Result<(u64, u64)> {
        let metadata = self.file.metadata()?;
        Ok((metadata.dev(), metadata.ino()))
    }
    pub(super) fn lock_sender(&self) -> Result<DirectoryLock> {
        let lock = File::from(openat(
            &self.file,
            ".mirelay-sender.lock",
            OFlag::O_CREAT
                | OFlag::O_RDWR
                | OFlag::O_NOFOLLOW
                | OFlag::O_CLOEXEC
                | OFlag::O_NONBLOCK,
            Mode::from_bits_truncate(0o600),
        )?);
        ensure!(lock.metadata()?.is_file(), "Invalid sender directory lock.");
        DirectoryLock::acquire(lock).context("Another sender owns this directory.")
    }
    fn parent(&self, path: &str, create: bool) -> Result<(File, String)> {
        validate_path(path)?;
        let mut parts = path.split('/').collect::<Vec<_>>();
        let name = parts.pop().context("Missing filename")?.to_string();
        let mut parent = self.file.try_clone()?;
        for part in parts {
            if create {
                match mkdirat(&parent, part, Mode::from_bits_truncate(0o700)) {
                    Ok(()) => parent.sync_all()?,
                    Err(Errno::EEXIST) => (),
                    Err(error) => return Err(error.into()),
                }
            }
            parent = File::from(openat(&parent, part, DIR_FLAGS, Mode::empty())?);
        }
        Ok((parent, name))
    }
    pub fn read(&self, path: &str) -> Result<File> {
        let (parent, name) = self.parent(path, false)?;
        regular(&parent, &name)
    }
    pub fn inventory(&self) -> Result<Inventory> {
        let mut entries = Vec::new();
        let mut budget = ScanBudget {
            bytes: MAX_SCAN_BYTES,
            nodes: MAX_ENTRIES * 2,
            started: std::time::Instant::now(),
        };
        scan(&self.file, "", &mut entries, &mut budget)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        // Stable identity: unchanged inventory does not invalidate a preview.
        let digest = Sha256::digest(serde_json::to_vec(&entries)?);
        let id = uuid::Uuid::from_bytes(digest[..16].try_into()?).to_string();
        let inventory = Inventory { id, entries };
        inventory.validate()?;
        Ok(inventory)
    }
}

fn regular(parent: &File, name: &str) -> Result<File> {
    let file = File::from(openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    ensure!(
        file.metadata()?.is_file(),
        "Directory contains a non-regular file."
    );
    Ok(file)
}
fn digest(file: &mut File, limit: u64) -> Result<(String, u64)> {
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= limit,
        "File exceeds directory scan limit."
    );
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let count = file.read(&mut buf)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(size <= limit, "File grew beyond directory scan limit.");
        hash.update(&buf[..count]);
    }
    let after = file.metadata()?;
    ensure!(
        size == before.len()
            && (
                before.len(),
                before.mtime(),
                before.mtime_nsec(),
                before.ctime(),
                before.ctime_nsec()
            ) == (
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec()
            ),
        "File changed while scanning; retry after it settles."
    );
    Ok((hex::encode(hash.finalize()), size))
}
struct ScanBudget {
    bytes: u64,
    nodes: usize,
    started: std::time::Instant,
}
fn scan(
    parent: &File,
    prefix: &str,
    entries: &mut Vec<InventoryEntry>,
    budget: &mut ScanBudget,
) -> Result<()> {
    let before = parent.metadata()?;
    for entry in fs::read_dir(format!("/proc/self/fd/{}", parent.as_raw_fd()))? {
        ensure!(
            budget.nodes > 0 && budget.started.elapsed().as_secs() < 90,
            "Directory scan budget exceeded; no partial inventory was accepted."
        );
        budget.nodes -= 1;
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Directory filenames must be UTF-8."))?;
        if name.to_ascii_lowercase().starts_with(".mirelay") {
            continue;
        }
        let path = format!("{prefix}{name}");
        validate_path(&path)?;
        let stat = fstatat(parent, name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW)?;
        let kind = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT;
        if kind == SFlag::S_IFDIR {
            ensure!(
                path.split('/').count() <= 16,
                "Directory nesting exceeds 16 levels."
            );
            let child = File::from(openat(parent, name.as_str(), DIR_FLAGS, Mode::empty())?);
            scan(&child, &format!("{path}/"), entries, budget)?;
        } else {
            ensure!(
                kind == SFlag::S_IFREG && entries.len() < MAX_ENTRIES,
                "Symlink, special file or excessive directory entries."
            );
            let (sha256, size) = digest(&mut regular(parent, &name)?, MAX_FILE.min(budget.bytes))?;
            budget.bytes -= size;
            entries.push(InventoryEntry { path, sha256, size });
        }
    }
    let after = parent.metadata()?;
    ensure!(
        (
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec()
        ) == (
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec()
        ),
        "Directory changed while scanning; refresh the preview."
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Applied {
    pub version: u64,
    pub sha256: String,
    pub conflict: bool,
    pub history: Option<String>,
    pub acknowledged: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub received_at_unix: u64,
}

pub enum ReceiveEvent<'a> {
    Active(&'a DirectoryEntry, crate::sync::SyncPhase),
    Settled(&'a DirectoryEntry, &'a Applied),
}

/// A retained copy is adjacent to the original file, never an arbitrary URI.
pub fn validate_history(path: &str, history: &str) -> Result<()> {
    validate_path(path)?;
    let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
    let (history_parent, name) = history.rsplit_once('/').unwrap_or(("", history));
    let id = name
        .strip_prefix(".mirelay-history-")
        .context("Invalid retained copy name.")?;
    ensure!(
        parent == history_parent && uuid::Uuid::parse_str(id)?.to_string() == id,
        "Invalid retained copy path."
    );
    Ok(())
}

fn validate_ledger(ledger: &Ledger, root: &Root, scope: &str) -> Result<()> {
    let (device, inode) = root.identity()?;
    ensure!(
        ledger.schema_version == VERSION
            && ledger.root == root.path
            && ledger.device == device
            && ledger.inode == inode
            && ledger.scope == scope,
        "Receiver state belongs to another directory or Folder."
    );
    ensure!(
        ledger.files.len() <= MAX_ENTRIES,
        "Oversized receiver ledger."
    );
    for (path, file) in &ledger.files {
        DirectoryVersion {
            path: path.clone(),
            version: file.version,
        }
        .validate()?;
        crate::protocol::validate_sha256(&file.sha256)?;
        ensure!(
            file.size <= MAX_FILE
                && file.media_type.len() <= 255
                && !file.media_type.chars().any(char::is_control),
            "Invalid receiver file metadata."
        );
        if let Some(history) = &file.history {
            validate_history(path, history)?;
        }
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Ledger {
    schema_version: u32,
    root: PathBuf,
    device: u64,
    inode: u64,
    scope: String,
    files: BTreeMap<String, Applied>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    entry: DirectoryEntry,
    stage: String,
    history: String,
    inode: u64,
    parent_inode: u64,
    parent_device: u64,
    previous_sha: Option<String>,
}

pub struct Receiver {
    root: Root,
    state: PathBuf,
    ledger: Ledger,
    // Fields drop in declaration order: release root ownership before waking
    // a same-state waiter. DirectoryLock also releases fork-inherited copies.
    _root_lock: DirectoryLock,
    _lock: StateLock,
}
impl Receiver {
    /// Read persisted status without recovery, network access, or touching the
    /// projection. Missing state is an error: never silently rebind a Folder.
    pub fn inspect(
        directory: &Path,
        state: &Path,
        scope: &str,
    ) -> Result<BTreeMap<String, Applied>> {
        let root = Root::open(directory)?;
        Root::open(state)?;
        let _lock = StateStore::new(state.join("receiver.json")).lock_shared()?;
        let ledger = load::<Ledger>(&state.join("receiver.json"))?
            .context("Directory state is missing. Restore its state; do not reset this Folder.")?;
        validate_ledger(&ledger, &root, scope)?;
        Ok(ledger.files)
    }
    pub fn open(directory: &Path, state: &Path, scope: &str) -> Result<Self> {
        let root = Root::open(directory)?;
        create_dir_all_durable(state)?;
        let state_root = Root::open(state)?;
        ensure!(
            !state_root.path.starts_with(&root.path) && !root.path.starts_with(&state_root.path),
            "Keep directory state separate from the synchronized directory."
        );
        let state = state_root.path;
        let lock = StateStore::new(state.join("receiver.json")).lock_exclusive()?;
        let root_lock = File::from(openat(
            &root.file,
            ".mirelay-receiver.lock",
            OFlag::O_CREAT
                | OFlag::O_RDWR
                | OFlag::O_NOFOLLOW
                | OFlag::O_CLOEXEC
                | OFlag::O_NONBLOCK,
            Mode::from_bits_truncate(0o600),
        )?);
        ensure!(root_lock.metadata()?.is_file(), "Invalid directory lock.");
        let root_lock =
            DirectoryLock::acquire(root_lock).context("Another receiver owns this directory.")?;
        let metadata = root.file.metadata()?;
        let ledger = match load::<Ledger>(&state.join("receiver.json"))? {
            Some(ledger) => {
                validate_ledger(&ledger, &root, scope)?;
                ledger
            }
            None => Ledger {
                schema_version: VERSION,
                root: root.path.clone(),
                device: metadata.dev(),
                inode: metadata.ino(),
                scope: scope.into(),
                files: BTreeMap::new(),
            },
        };
        let mut receiver = Self {
            root,
            state,
            ledger,
            _lock: lock,
            _root_lock: root_lock,
        };
        receiver.save()?;
        receiver.recover()?;
        Ok(receiver)
    }
    pub fn inventory(&self) -> Result<Inventory> {
        self.root.inventory()
    }
    pub fn identity(&self) -> Result<(u64, u64)> {
        self.root.identity()
    }
    pub fn files(&self) -> &BTreeMap<String, Applied> {
        &self.ledger.files
    }
    fn save(&self) -> Result<()> {
        save(&self.state.join("receiver.json"), &self.ledger)
    }

    pub fn apply(
        &mut self,
        entry: &DirectoryEntry,
        reader: Box<dyn Read + Send>,
    ) -> Result<Applied> {
        DirectoryState {
            schema_version: VERSION,
            receiver: None,
            entries: vec![entry.clone()],
        }
        .validate()?;
        self.recover()?;
        if let Some(previous) = self.ledger.files.get(&entry.path) {
            ensure!(
                previous.version != entry.version || previous.sha256 == entry.sha256,
                "Same directory version has different content."
            );
            if previous.version >= entry.version {
                return Ok(previous.clone());
            }
        }
        ensure!(
            self.ledger.files.contains_key(&entry.path) || self.ledger.files.len() < MAX_ENTRIES,
            "Directory file limit reached."
        );
        let asset = crate::storage::receive(
            &entry.delivery(),
            reader,
            &self.state.join("objects"),
            MAX_FILE,
        )?;
        let (parent, name) = self.root.parent(&entry.path, true)?;
        if let Some(stat) = stat_optional(&parent, &name)? {
            ensure!(
                SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG,
                "Target is a symlink, directory or special file."
            );
            let mut file = regular(&parent, &name)?;
            if digest(&mut file, MAX_FILE)? == (entry.sha256.clone(), entry.size) {
                file.sync_all()?;
                parent.sync_all()?;
                let applied = Applied {
                    version: entry.version,
                    sha256: entry.sha256.clone(),
                    conflict: false,
                    history: None,
                    acknowledged: false,
                    size: entry.size,
                    media_type: entry.media_type.clone(),
                    received_at_unix: crate::fsutil::unix_now(),
                };
                self.ledger
                    .files
                    .insert(entry.path.clone(), applied.clone());
                self.save()?;
                return Ok(applied);
            }
        }
        let stage = format!(".mirelay-incoming-{}", uuid::Uuid::new_v4());
        let mut file = File::from(openat(
            &parent,
            stage.as_str(),
            OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )?);
        std::io::copy(&mut File::open(&asset.path)?, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        parent.sync_all()?;
        let metadata = parent.metadata()?;
        let journal = Journal {
            entry: entry.clone(),
            stage,
            history: format!(".mirelay-history-{}", uuid::Uuid::new_v4()),
            inode: file.metadata()?.ino(),
            parent_inode: metadata.ino(),
            parent_device: metadata.dev(),
            previous_sha: self.ledger.files.get(&entry.path).map(|v| v.sha256.clone()),
        };
        save(&self.state.join("journal.json"), &journal)?;
        self.recover()?;
        Ok(self.ledger.files[&entry.path].clone())
    }

    /// A persisted incoming inode distinguishes a completed exchange from a
    /// pre-commit crash. Unknown artifacts are never deleted during recovery.
    pub fn recover(&mut self) -> Result<()> {
        let Some(journal) = load::<Journal>(&self.state.join("journal.json"))? else {
            return Ok(());
        };
        DirectoryState {
            schema_version: VERSION,
            receiver: None,
            entries: vec![journal.entry.clone()],
        }
        .validate()?;
        for (name, prefix) in [
            (&journal.stage, ".mirelay-incoming-"),
            (&journal.history, ".mirelay-history-"),
        ] {
            let id = name
                .strip_prefix(prefix)
                .context("Invalid recovery artifact.")?;
            ensure!(
                uuid::Uuid::parse_str(id)?.to_string() == id,
                "Invalid recovery identity."
            );
        }
        let (parent, name) = self.root.parent(&journal.entry.path, false)?;
        let metadata = parent.metadata()?;
        ensure!(
            metadata.ino() == journal.parent_inode && metadata.dev() == journal.parent_device,
            "Destination parent changed during recovery."
        );
        let mut stage = stat_optional(&parent, &journal.stage)?;
        if stage.as_ref().is_some_and(|v| v.st_ino == journal.inode) {
            let (sha, size) = digest(&mut regular(&parent, &journal.stage)?, MAX_FILE)?;
            ensure!(
                sha == journal.entry.sha256 && size == journal.entry.size,
                "Incoming file changed before commit."
            );
            // Never fall back to a non-atomic overwrite on an unsupported FS.
            match renameat2(
                &parent,
                journal.stage.as_str(),
                &parent,
                name.as_str(),
                RenameFlags::RENAME_NOREPLACE,
            ) {
                Ok(()) => (),
                Err(Errno::EEXIST) => {
                    let stat = fstatat(&parent, name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW)?;
                    ensure!(
                        SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG,
                        "Target became a non-regular file."
                    );
                    renameat2(
                        &parent,
                        journal.stage.as_str(),
                        &parent,
                        name.as_str(),
                        RenameFlags::RENAME_EXCHANGE,
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
            parent.sync_all()?;
            stage = stat_optional(&parent, &journal.stage)?;
        }
        if stage.is_some() {
            renameat2(
                &parent,
                journal.stage.as_str(),
                &parent,
                journal.history.as_str(),
                RenameFlags::RENAME_NOREPLACE,
            )?;
            parent.sync_all()?;
        }
        let history = stat_optional(&parent, &journal.history)?;
        ensure!(
            history.is_some()
                || stat_optional(&parent, &name)?.is_some_and(|v| v.st_ino == journal.inode),
            "Cannot prove the directory commit completed; retained recovery state."
        );
        let conflict = if history.is_some() {
            let hash = regular(&parent, &journal.history)
                .and_then(|mut file| digest(&mut file, MAX_FILE))
                .ok()
                .map(|v| v.0);
            hash.is_none() || hash != journal.previous_sha
        } else {
            false
        };
        let history = history.map(|_| match journal.entry.path.rsplit_once('/') {
            Some((prefix, _)) => format!("{prefix}/{}", journal.history),
            None => journal.history.clone(),
        });
        self.ledger.files.insert(
            journal.entry.path.clone(),
            Applied {
                version: journal.entry.version,
                sha256: journal.entry.sha256.clone(),
                conflict,
                history,
                acknowledged: false,
                size: journal.entry.size,
                media_type: journal.entry.media_type.clone(),
                received_at_unix: crate::fsutil::unix_now(),
            },
        );
        self.save()?;
        fs::remove_file(self.state.join("journal.json"))?;
        sync_directory(&self.state)?;
        Ok(())
    }

    pub fn sync(&mut self, client: &DirectoryClient, source: &dyn DeliverySource) -> Result<usize> {
        self.sync_with_events(client, source, &|_| {})
    }

    pub fn sync_with_events(
        &mut self,
        client: &DirectoryClient,
        source: &dyn DeliverySource,
        emit: &dyn Fn(ReceiveEvent<'_>),
    ) -> Result<usize> {
        use crate::sync::SyncPhase;
        self.recover()?;
        let remote = client.state()?;
        let mut count = 0;
        for entry in &remote.entries {
            if entry.acknowledged {
                if let Some(applied) = self.ledger.files.get_mut(&entry.path)
                    && applied.version == entry.version
                    && applied.sha256 == entry.sha256
                    && !applied.acknowledged
                {
                    applied.acknowledged = true;
                    self.save()?;
                    emit(ReceiveEvent::Settled(
                        entry,
                        &self.ledger.files[&entry.path],
                    ));
                }
                continue;
            }
            if !self
                .ledger
                .files
                .get(&entry.path)
                .is_some_and(|v| v.version >= entry.version)
            {
                emit(ReceiveEvent::Active(entry, SyncPhase::Downloading));
                let mut failures = 0;
                loop {
                    match source
                        .open_payload(&entry.delivery())
                        .and_then(|reader| self.apply(entry, reader))
                    {
                        Ok(_) => break,
                        Err(error) => {
                            failures += 1;
                            match source.payload_retry_delay(&error, failures) {
                                Some(delay) => {
                                    emit(ReceiveEvent::Active(entry, SyncPhase::Retrying));
                                    std::thread::sleep(delay);
                                }
                                None => return Err(error),
                            }
                        }
                    }
                }
                count += 1;
            }
            let applied = &self.ledger.files[&entry.path];
            emit(ReceiveEvent::Active(entry, SyncPhase::Verifying));
            // A crash may leave a durable local record before the remote ACK.
            // Never acknowledge a missing/tampered projection on that retry.
            let actual = digest(&mut self.root.read(&entry.path)?, MAX_FILE)?;
            ensure!(
                actual.0 == applied.sha256,
                "Destination changed before its receipt. No ACK was sent; received bytes and recovery state are retained."
            );
            // Also covers retrying this Receiver object after an earlier local
            // ledger write failed: memory alone never authorizes remote GC.
            self.save()?;
            emit(ReceiveEvent::Active(entry, SyncPhase::Confirming));
            client.acknowledge(&DirectoryAck {
                path: entry.path.clone(),
                version: applied.version,
                sha256: applied.sha256.clone(),
                conflict: applied.conflict,
            })?;
            self.ledger.files.get_mut(&entry.path).unwrap().acknowledged = true;
            self.save()?;
            emit(ReceiveEvent::Settled(
                entry,
                &self.ledger.files[&entry.path],
            ));
        }
        client.publish(&InventoryUpdate {
            previous_id: remote.receiver.map(|v| v.id),
            inventory: self.inventory()?,
        })?;
        Ok(count)
    }
}

fn stat_optional(parent: &File, name: &str) -> Result<Option<nix::sys::stat::FileStat>> {
    match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(stat)),
        Err(Errno::ENOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
pub(super) fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match read_limited(path, MAX_BODY as u64) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
pub(super) fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(bytes.len() <= MAX_BODY, "Directory state limit exceeded.");
    atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn crash_point(phase: u8, existing: bool) {
        let tmp = tempfile::tempdir().unwrap();
        let dst = tmp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        let state = tmp.path().join("state");
        let receiver = Receiver::open(&dst, &state, "scope").unwrap();
        if existing {
            fs::write(dst.join("image.txt"), b"user's existing copy").unwrap();
        }
        let bytes = b"incoming copy";
        let entry = DirectoryEntry {
            path: "image.txt".into(),
            version: 2,
            delivery_id: uuid::Uuid::new_v4().to_string(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
            media_type: "text/plain".into(),
            acknowledged: false,
            conflict: false,
        };
        let parent = &receiver.root.file;
        let stage = format!(".mirelay-incoming-{}", uuid::Uuid::new_v4());
        let history = format!(".mirelay-history-{}", uuid::Uuid::new_v4());
        let mut staged = File::from(
            openat(
                parent,
                stage.as_str(),
                OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY,
                Mode::from_bits_truncate(0o600),
            )
            .unwrap(),
        );
        staged.write_all(bytes).unwrap();
        staged.sync_all().unwrap();
        parent.sync_all().unwrap();
        let journal = Journal {
            entry: entry.clone(),
            stage: stage.clone(),
            history: history.clone(),
            inode: staged.metadata().unwrap().ino(),
            parent_inode: parent.metadata().unwrap().ino(),
            parent_device: parent.metadata().unwrap().dev(),
            previous_sha: None,
        };
        save(&state.join("journal.json"), &journal).unwrap();
        if phase >= 1 {
            renameat2(
                parent,
                stage.as_str(),
                parent,
                "image.txt",
                if existing {
                    RenameFlags::RENAME_EXCHANGE
                } else {
                    RenameFlags::RENAME_NOREPLACE
                },
            )
            .unwrap();
        }
        if phase >= 2 && existing {
            renameat2(
                parent,
                stage.as_str(),
                parent,
                history.as_str(),
                RenameFlags::RENAME_NOREPLACE,
            )
            .unwrap();
        }
        // Re-open exactly the durable files left at each interrupted boundary.
        drop(receiver);
        let mut recovered = Receiver::open(&dst, &state, "scope").unwrap();
        assert_eq!(fs::read(dst.join("image.txt")).unwrap(), bytes);
        if existing {
            assert_eq!(
                fs::read(dst.join(history)).unwrap(),
                b"user's existing copy"
            );
            assert!(recovered.files()["image.txt"].conflict);
        }
        recovered
            .apply(&entry, Box::new(Cursor::new(bytes)))
            .unwrap();
        assert!(!state.join("journal.json").exists());
        assert_eq!(recovered.inventory().unwrap().entries.len(), 1);
    }
    #[test]
    fn recover_prepared_exchange() {
        crash_point(0, true);
    }
    #[test]
    fn recover_after_exchange_before_receipt() {
        crash_point(1, true);
    }
    #[test]
    fn recover_after_archiving_before_ledger() {
        crash_point(2, true);
    }
    #[test]
    fn recover_prepared_new_file() {
        crash_point(0, false);
    }
    #[test]
    fn recover_after_new_file_rename() {
        crash_point(1, false);
    }
}
