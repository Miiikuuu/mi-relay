//! Explicit initial consent followed by durable, one-way change delivery.
use super::{
    client::DirectoryClient,
    lock::DirectoryLock,
    receiver::{MAX_FILE, Root, load, save},
    *,
};
use crate::{
    fsutil::{create_dir_all_durable, sync_directory},
    state::{StateLock, StateStore},
    tus_client::{UploadRequest, upload_with_events},
};
use anyhow::Context;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Preview {
    pub source: Inventory,
    pub remote: DirectoryState,
    pub comparison: Comparison,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    id: String,
    entry: InventoryEntry,
    version: u64,
}
#[derive(Serialize, Deserialize)]
struct Ledger {
    schema_version: u32,
    root: PathBuf,
    device: u64,
    inode: u64,
    scope: String,
    initialized: bool,
    next_version: u64,
    files: BTreeMap<String, String>,
    pending: Vec<Pending>,
}

pub struct Sender {
    root: Root,
    state: PathBuf,
    ledger: Ledger,
    // Root ownership must end before a waiting state-lock owner is admitted.
    _root_lock: DirectoryLock,
    _lock: StateLock,
}
impl Sender {
    pub fn open(directory: &Path, state: &Path, scope: &str) -> Result<Self> {
        let root = Root::open(directory)?;
        create_dir_all_durable(state)?;
        let state_root = Root::open(state)?;
        ensure!(
            !state_root.path().starts_with(root.path())
                && !root.path().starts_with(state_root.path()),
            "Keep sender state separate from the synchronized directory."
        );
        let state = state_root.path().to_path_buf();
        let lock = StateStore::new(state.join("sender.json")).lock_exclusive()?;
        ensure!(
            !state.join("retired.json").exists(),
            "This sender has exited; create a new Folder to reconnect."
        );
        let root_lock = root.lock_sender()?;
        let metadata = fs::metadata(root.path())?;
        let ledger = match load::<Ledger>(&state.join("sender.json"))? {
            Some(ledger) => {
                ensure!(
                    ledger.schema_version == VERSION
                        && ledger.root == root.path()
                        && ledger.device == metadata.dev()
                        && ledger.inode == metadata.ino()
                        && ledger.scope == scope,
                    "Sender state belongs to another directory or Folder."
                );
                ensure!(
                    ledger.files.len() <= MAX_ENTRIES
                        && ledger.pending.len() <= MAX_ENTRIES
                        && ledger.next_version > 0,
                    "Invalid sender ledger."
                );
                for job in &ledger.pending {
                    ensure!(
                        uuid::Uuid::parse_str(&job.id)?.to_string() == job.id,
                        "Invalid staged job identity."
                    );
                    DirectoryVersion {
                        path: job.entry.path.clone(),
                        version: job.version,
                    }
                    .validate()?;
                    crate::protocol::validate_sha256(&job.entry.sha256)?;
                }
                ledger
            }
            None => Ledger {
                schema_version: VERSION,
                root: root.path().into(),
                device: metadata.dev(),
                inode: metadata.ino(),
                scope: scope.into(),
                initialized: false,
                next_version: 1,
                files: BTreeMap::new(),
                pending: Vec::new(),
            },
        };
        let sender = Self {
            root,
            state,
            ledger,
            _lock: lock,
            _root_lock: root_lock,
        };
        sender.save()?;
        Ok(sender)
    }
    fn save(&self) -> Result<()> {
        save(&self.state.join("sender.json"), &self.ledger)
    }
    pub fn preview(&self, client: &DirectoryClient) -> Result<Preview> {
        ensure!(
            !self.ledger.initialized,
            "This directory is already initialized. Use send to process changes."
        );
        let source = self.root.inventory()?;
        ensure!(
            source.entries.iter().all(|v| v.size > 0),
            "Empty files are not supported by the current transfer protocol."
        );
        let remote = client.state()?;
        let preview = Preview {
            comparison: compare_remote(&source, &remote)?,
            source,
            remote,
        };
        save(&self.state.join("preview.json"), &preview)?;
        Ok(preview)
    }
    /// Explicitly called only after the user accepted the stored preview.
    /// Revalidate both inventories and source version history before staging.
    pub fn initialize(&mut self, client: &DirectoryClient) -> Result<()> {
        ensure!(
            !self.ledger.initialized,
            "Directory is already initialized."
        );
        let preview: Preview = load(&self.state.join("preview.json"))?
            .context("Preview and review this directory before initialization.")?;
        let source = self.root.inventory()?;
        let remote = client.state()?;
        ensure!(
            source == preview.source
                && serde_json::to_value(&remote)? == serde_json::to_value(&preview.remote)?,
            "Source or receiver changed since preview. Preview again before confirming."
        );
        let comparison = compare_remote(&source, &remote)?;
        ensure!(
            source.entries.iter().all(|v| v.size > 0),
            "Empty files are not supported yet."
        );
        self.ledger.next_version = remote
            .entries
            .iter()
            .map(|e| e.version)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .context("Source version exhausted.")?;
        let changed: BTreeSet<_> = comparison
            .missing
            .iter()
            .chain(&comparison.different)
            .collect();
        // Commit the baseline atomically before any upload. Missing/different
        // paths remain untracked and will be staged by send, even after a crash.
        self.ledger.files = source
            .entries
            .iter()
            .filter(|e| !changed.contains(&e.path))
            .map(|e| (e.path.clone(), e.sha256.clone()))
            .collect();
        self.ledger.initialized = true;
        self.save()
    }
    pub fn send(&mut self, client: &DirectoryClient, token: &str, insecure: bool) -> Result<usize> {
        ensure!(
            self.ledger.initialized,
            "Review preview and confirm initialization before sending files."
        );
        let mut count = self.flush(token, insecure)?;
        let source = self.root.inventory()?;
        ensure!(
            source.entries.iter().all(|v| v.size > 0),
            "Empty files are not supported yet."
        );
        let remote = client.state()?;
        compare_remote(&source, &remote)?;
        // Adopt a higher remote floor after re-pairing, never recycle versions.
        self.ledger.next_version = self.ledger.next_version.max(
            remote
                .entries
                .iter()
                .map(|e| e.version)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        );
        for entry in source.entries {
            if self.ledger.files.get(&entry.path) == Some(&entry.sha256) {
                continue;
            }
            ensure!(
                self.ledger.files.contains_key(&entry.path)
                    || self.ledger.files.len() < MAX_ENTRIES,
                "Sender history limit reached; no source files were deleted."
            );
            let version = self.ledger.next_version;
            DirectoryVersion {
                path: entry.path.clone(),
                version,
            }
            .validate()?;
            let id = uuid::Uuid::new_v4().to_string();
            let job = self.state.join(&id);
            create_dir_all_durable(&job)?;
            let payload = job.join("payload");
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&payload)?;
            let size = std::io::copy(
                &mut self.root.read(&entry.path)?.take(MAX_FILE + 1),
                &mut output,
            )?;
            output.sync_all()?;
            sync_directory(&job)?;
            let inspection = crate::storage::inspect_file(&payload, MAX_FILE)?;
            ensure!(
                size == entry.size && inspection.sha256 == entry.sha256,
                "Source changed while staging; retry after it settles."
            );
            self.ledger.next_version += 1;
            self.ledger.pending.push(Pending { id, entry, version });
            self.save()?;
            // One bounded durable payload at a time, not an entire directory
            // copied into memory or a second unbounded outgoing queue.
            count += self.flush(token, insecure)?;
        }
        Ok(count)
    }
    fn flush(&mut self, token: &str, insecure: bool) -> Result<usize> {
        let mut count = 0;
        while let Some(pending) = self.ledger.pending.first() {
            let job = self.state.join(&pending.id);
            let payload = job.join("payload");
            let inspected = crate::storage::inspect_file(&payload, MAX_FILE)?;
            ensure!(
                inspected.sha256 == pending.entry.sha256 && inspected.size == pending.entry.size,
                "Staged source changed; refusing to resume another file."
            );
            let outcome = upload_with_events(
                UploadRequest {
                    file: payload.clone(),
                    state_file: job.join("resume.json"),
                    server_url: self.ledger.scope.clone(),
                    token: token.into(),
                    name: pending.entry.path.rsplit('/').next().map(str::to_owned),
                    directory: Some(DirectoryVersion {
                        path: pending.entry.path.clone(),
                        version: pending.version,
                    }),
                    chunk_size_bytes: 1024 * 1024,
                    max_chunks: None,
                    allow_insecure_http: insecure,
                    request_timeout_seconds: 30,
                    keep_completed_state: true,
                },
                |_| true,
            )?;
            ensure!(outcome.delivery_id.is_some(), "Upload did not complete.");
            self.ledger
                .files
                .insert(pending.entry.path.clone(), pending.entry.sha256.clone());
            self.ledger.pending.remove(0);
            self.save()?;
            // These are exclusively owned staged copies, never source files.
            for name in ["payload", "resume.json", "resume.json.lock"] {
                match fs::remove_file(job.join(name)) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
            }
            fs::remove_dir(&job)?;
            sync_directory(&self.state)?;
            count += 1;
        }
        Ok(count)
    }
}
use std::io::Read;
