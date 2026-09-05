use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use crate::model::{DeliveryRecord, DeliveryStatus, WallpaperStatus};
use crate::sync::SyncPhase;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileActivity {
    pub id: String,
    pub original_name: String,
    pub media_type: String,
    pub size: u64,
    pub phase: Option<SyncPhase>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Entry<S, L> {
    Saved(S),
    Live(L),
}

pub(super) type FileEntry = Entry<DeliveryRecord, FileActivity>;
type FileRef<'a> = Entry<&'a DeliveryRecord, &'a FileActivity>;

impl<S: Borrow<DeliveryRecord>, L: Borrow<FileActivity>> Entry<S, L> {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Saved(record) => &record.borrow().id,
            Self::Live(activity) => &activity.borrow().id,
        }
    }

    pub(super) fn name(&self) -> &str {
        match self {
            Self::Saved(record) => &record.borrow().original_name,
            Self::Live(activity) => &activity.borrow().original_name,
        }
    }

    pub(super) fn media_type(&self) -> &str {
        match self {
            Self::Saved(record) => &record.borrow().media_type,
            Self::Live(activity) => &activity.borrow().media_type,
        }
    }

    pub(super) fn size(&self) -> u64 {
        match self {
            Self::Saved(record) => record.borrow().size,
            Self::Live(activity) => activity.borrow().size,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        matches!(self, Self::Live(activity) if activity.borrow().phase.is_some())
    }

    pub(super) fn needs_attention(&self) -> bool {
        match self {
            Self::Saved(record) => {
                let record = record.borrow();
                record.delivery_error.is_some()
                    || record.wallpaper_error.is_some()
                    || matches!(
                        record.wallpaper_status,
                        WallpaperStatus::Failed | WallpaperStatus::Uncertain
                    )
            }
            Self::Live(activity) => activity.borrow().error.is_some(),
        }
    }

    pub(super) fn is_pending(&self) -> bool {
        match self {
            Self::Saved(record) => {
                let record = record.borrow();
                record.delivery_status == DeliveryStatus::AckPending
                    || matches!(
                        record.wallpaper_status,
                        WallpaperStatus::Pending | WallpaperStatus::Running
                    )
            }
            Self::Live(activity) => activity.borrow().phase.is_some(),
        }
    }

    pub(super) fn is_complete(&self) -> bool {
        matches!(self, Self::Saved(record)
            if record.borrow().delivery_status == DeliveryStatus::Acknowledged
                && matches!(record.borrow().wallpaper_status,
                    WallpaperStatus::NotApplicable
                        | WallpaperStatus::NotConfigured
                        | WallpaperStatus::Applied)
                && !self.needs_attention())
    }

    fn received_at(&self) -> u64 {
        match self {
            Self::Saved(record) => record.borrow().received_at_unix,
            Self::Live(_) => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FileSort {
    #[default]
    Newest,
    Oldest,
    NameAsc,
    NameDesc,
    Largest,
    Smallest,
}

impl FileSort {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "newest" => Some(Self::Newest),
            "oldest" => Some(Self::Oldest),
            "name-asc" => Some(Self::NameAsc),
            "name-desc" => Some(Self::NameDesc),
            "largest" => Some(Self::Largest),
            "smallest" => Some(Self::Smallest),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::NameAsc => "name-asc",
            Self::NameDesc => "name-desc",
            Self::Largest => "largest",
            Self::Smallest => "smallest",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
            Self::NameAsc => "Name (A–Z)",
            Self::NameDesc => "Name (Z–A)",
            Self::Largest => "Largest first",
            Self::Smallest => "Smallest first",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FileFilter {
    #[default]
    All,
    InProgress,
    Attention,
    Completed,
}

impl FileFilter {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "all" => Some(Self::All),
            "in-progress" => Some(Self::InProgress),
            "attention" => Some(Self::Attention),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::InProgress => "in-progress",
            Self::Attention => "attention",
            Self::Completed => "completed",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All statuses",
            Self::InProgress => "In progress",
            Self::Attention => "Needs attention",
            Self::Completed => "Completed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FileKind {
    #[default]
    All,
    Images,
    Documents,
    Audio,
    Video,
    Archives,
    Other,
}

impl FileKind {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "all" => Some(Self::All),
            "images" => Some(Self::Images),
            "documents" => Some(Self::Documents),
            "audio" => Some(Self::Audio),
            "video" => Some(Self::Video),
            "archives" => Some(Self::Archives),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Images => "images",
            Self::Documents => "documents",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Archives => "archives",
            Self::Other => "other",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All types",
            Self::Images => "Images",
            Self::Documents => "Documents",
            Self::Audio => "Audio",
            Self::Video => "Video",
            Self::Archives => "Archives",
            Self::Other => "Other",
        }
    }

    fn of(media_type: &str) -> Self {
        let mime = media_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.starts_with("image/") {
            Self::Images
        } else if mime.starts_with("text/")
            || matches!(
                mime.as_str(),
                "application/pdf"
                    | "application/json"
                    | "application/xml"
                    | "application/rtf"
                    | "application/msword"
                    | "application/vnd.ms-excel"
                    | "application/vnd.ms-powerpoint"
            )
            || mime.starts_with("application/vnd.openxmlformats-officedocument.")
            || mime.starts_with("application/vnd.oasis.opendocument.")
        {
            Self::Documents
        } else if mime.starts_with("audio/") || mime == "application/ogg" {
            Self::Audio
        } else if mime.starts_with("video/") {
            Self::Video
        } else if matches!(
            mime.as_str(),
            "application/zip"
                | "application/x-zip-compressed"
                | "application/x-7z-compressed"
                | "application/vnd.rar"
                | "application/x-rar-compressed"
                | "application/gzip"
                | "application/x-gzip"
                | "application/x-tar"
                | "application/x-bzip"
                | "application/x-bzip2"
                | "application/x-xz"
                | "application/zstd"
        ) {
            Self::Archives
        } else {
            Self::Other
        }
    }
}

struct Candidate<'a> {
    entry: FileRef<'a>,
    name_key: Cow<'a, str>,
}

#[derive(Debug, Clone)]
struct Prepared {
    names: Vec<String>,
    kinds: Vec<FileKind>,
    positions: HashMap<String, usize>,
    unique: Vec<usize>,
}

impl Prepared {
    fn new(records: &[DeliveryRecord]) -> Self {
        let mut positions = HashMap::with_capacity(records.len());
        let mut names = Vec::with_capacity(records.len());
        let mut kinds = Vec::with_capacity(records.len());
        for (position, record) in records.iter().enumerate() {
            positions.insert(record.id.clone(), position);
            names.push(record.original_name.to_lowercase());
            kinds.push(FileKind::of(&record.media_type));
        }
        let mut unique: Vec<_> = positions.values().copied().collect();
        unique.sort_unstable();
        Self {
            names,
            kinds,
            positions,
            unique,
        }
    }
}

/// Immutable snapshots share this history through Arc. Loading prepares its
/// search keys on the worker; view changes only borrow those keys. A real file
/// settlement updates a single indexed record rather than invalidating history.
#[derive(Debug, Clone)]
pub(super) struct FileHistory {
    records: Vec<DeliveryRecord>,
    prepared: OnceLock<Prepared>,
}

impl From<Vec<DeliveryRecord>> for FileHistory {
    fn from(records: Vec<DeliveryRecord>) -> Self {
        let prepared = OnceLock::from(Prepared::new(&records));
        Self { records, prepared }
    }
}

impl Deref for FileHistory {
    type Target = Vec<DeliveryRecord>;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

impl DerefMut for FileHistory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Arbitrary edits (including fixture mutations) cannot leave stale keys.
        self.prepared.take();
        &mut self.records
    }
}

impl FileHistory {
    pub(super) fn upsert(&mut self, record: DeliveryRecord) -> bool {
        self.prepared.get_or_init(|| Prepared::new(&self.records));
        let prepared = self.prepared.get_mut().expect("index was initialized");
        let name = record.original_name.to_lowercase();
        let kind = FileKind::of(&record.media_type);
        if let Some(&index) = prepared.positions.get(&record.id) {
            self.records[index] = record;
            prepared.names[index] = name;
            prepared.kinds[index] = kind;
            false
        } else {
            let index = self.records.len();
            prepared.positions.insert(record.id.clone(), index);
            prepared.unique.push(index);
            prepared.names.push(name);
            prepared.kinds.push(kind);
            self.records.push(record);
            true
        }
    }

    pub(super) fn page(
        &self,
        activity: &[FileActivity],
        sort: FileSort,
        filter: FileFilter,
        kind: FileKind,
        query: &str,
        limit: usize,
    ) -> FilePage {
        select_page(
            &self.records,
            self.prepared.get_or_init(|| Prepared::new(&self.records)),
            activity,
            Query {
                sort,
                filter,
                kind,
                text: query,
                limit,
            },
        )
    }
}

struct Query<'a> {
    sort: FileSort,
    filter: FileFilter,
    kind: FileKind,
    text: &'a str,
    limit: usize,
}

impl Candidate<'_> {
    fn compare(&self, right: &Self, sort: FileSort) -> Ordering {
        let left = self;
        let names = || {
            left.name_key
                .cmp(&right.name_key)
                .then_with(|| left.entry.name().cmp(right.entry.name()))
        };
        right
            .entry
            .is_active()
            .cmp(&left.entry.is_active())
            .then_with(|| {
                match sort {
                    FileSort::Newest => right.entry.received_at().cmp(&left.entry.received_at()),
                    FileSort::Oldest => left.entry.received_at().cmp(&right.entry.received_at()),
                    FileSort::NameAsc => names(),
                    FileSort::NameDesc => names().reverse(),
                    FileSort::Largest => right.entry.size().cmp(&left.entry.size()),
                    FileSort::Smallest => left.entry.size().cmp(&right.entry.size()),
                }
                .then_with(names)
                .then_with(|| left.entry.id().cmp(right.entry.id()))
            })
    }

    fn into_owned(self) -> FileEntry {
        match self.entry {
            FileRef::Saved(record) => FileEntry::Saved(record.clone()),
            FileRef::Live(activity) => FileEntry::Live(activity.clone()),
        }
    }
}

pub(super) struct FilePage {
    pub entries: Vec<FileEntry>,
    pub total: usize,
}

pub(super) fn visible_files(
    records: &[DeliveryRecord],
    activity: &[FileActivity],
    sort: FileSort,
    filter: FileFilter,
    kind: FileKind,
    query: &str,
) -> Vec<FileEntry> {
    file_page(records, activity, sort, filter, kind, query, usize::MAX).entries
}

pub(super) fn file_page(
    records: &[DeliveryRecord],
    activity: &[FileActivity],
    sort: FileSort,
    filter: FileFilter,
    kind: FileKind,
    query: &str,
    limit: usize,
) -> FilePage {
    select_page(
        records,
        &Prepared::new(records),
        activity,
        Query {
            sort,
            filter,
            kind,
            text: query,
            limit,
        },
    )
}

fn select_page<'a>(
    records: &'a [DeliveryRecord],
    prepared: &'a Prepared,
    activity: &'a [FileActivity],
    query: Query<'_>,
) -> FilePage {
    // Only the small live overlay is indexed on each frame. Mask saved records
    // by position, avoiding a string-hash lookup for every history row.
    let mut live_entries = HashMap::with_capacity(activity.len());
    let mut overridden = vec![false; records.len()];
    for live in activity {
        live_entries.insert(live.id.as_str(), live);
        if let Some(&index) = prepared.positions.get(&live.id) {
            overridden[index] = true;
        }
    }
    let text = query.text.trim().to_lowercase();
    let mut visible = Vec::with_capacity(prepared.unique.len() + live_entries.len());
    let mut consider = |entry: FileRef<'a>, name_key: Cow<'a, str>, kind: FileKind| {
        let status_matches = match query.filter {
            FileFilter::All => true,
            FileFilter::InProgress => entry.is_pending(),
            FileFilter::Attention => entry.needs_attention(),
            FileFilter::Completed => entry.is_complete(),
        };
        if !status_matches || (query.kind != FileKind::All && query.kind != kind) {
            return;
        }
        if name_key.contains(&text) {
            visible.push(Candidate { entry, name_key });
        }
    };
    for &index in &prepared.unique {
        if !overridden[index] {
            consider(
                FileRef::Saved(&records[index]),
                Cow::Borrowed(&prepared.names[index]),
                prepared.kinds[index],
            );
        }
    }
    for live in live_entries.into_values() {
        consider(
            FileRef::Live(live),
            Cow::Owned(live.original_name.to_lowercase()),
            FileKind::of(&live.media_type),
        );
    }
    let total = visible.len();
    let limit = query.limit.min(total);
    if limit == 0 {
        visible.clear();
    } else if limit < total {
        // O(n) partition plus O(page log page), not a full-history sort.
        visible.select_nth_unstable_by(limit, |left, right| left.compare(right, query.sort));
        visible.truncate(limit);
    }
    visible.sort_unstable_by(|left, right| left.compare(right, query.sort));
    FilePage {
        entries: visible.into_iter().map(Candidate::into_owned).collect(),
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SORTS: [FileSort; 6] = [
        FileSort::Newest,
        FileSort::Oldest,
        FileSort::NameAsc,
        FileSort::NameDesc,
        FileSort::Largest,
        FileSort::Smallest,
    ];

    fn saved(id: &str, name: &str) -> DeliveryRecord {
        DeliveryRecord {
            id: id.into(),
            original_name: name.into(),
            media_type: "text/plain".into(),
            sha256: "a".repeat(64),
            size: 128,
            stored_path: format!("/tmp/{id}").into(),
            delivery_status: DeliveryStatus::Acknowledged,
            wallpaper_status: WallpaperStatus::NotApplicable,
            wallpaper_attempts: 0,
            delivery_error: None,
            wallpaper_error: None,
            source_created_at_unix: None,
            received_at_unix: 100,
            acknowledged_at_unix: Some(101),
            imported_at_unix: None,
        }
    }

    fn live(id: &str, name: &str) -> FileActivity {
        FileActivity {
            id: id.into(),
            original_name: name.into(),
            media_type: "text/plain".into(),
            size: 64,
            phase: Some(SyncPhase::Downloading),
            error: None,
        }
    }

    fn all(
        records: &[DeliveryRecord],
        activity: &[FileActivity],
        sort: FileSort,
    ) -> Vec<FileEntry> {
        visible_files(records, activity, sort, FileFilter::All, FileKind::All, "")
    }

    // Independent full-sort oracle: preserve the old ordering and overlay rules
    // while the production path now partitions only the requested prefix.
    fn reference(
        records: &[DeliveryRecord],
        activity: &[FileActivity],
        sort: FileSort,
        filter: FileFilter,
        kind: FileKind,
        query: &str,
    ) -> Vec<FileEntry> {
        let mut entries = std::collections::BTreeMap::new();
        for record in records {
            entries.insert(record.id.clone(), FileEntry::Saved(record.clone()));
        }
        for live in activity {
            entries.insert(live.id.clone(), FileEntry::Live(live.clone()));
        }
        let query = query.trim().to_lowercase();
        let mut entries: Vec<_> = entries
            .into_values()
            .filter(|entry| {
                let status = match filter {
                    FileFilter::All => true,
                    FileFilter::InProgress => entry.is_pending(),
                    FileFilter::Attention => entry.needs_attention(),
                    FileFilter::Completed => entry.is_complete(),
                };
                status
                    && (kind == FileKind::All || kind == FileKind::of(entry.media_type()))
                    && entry.name().to_lowercase().contains(&query)
            })
            .collect();
        entries.sort_by(|left, right| {
            let name = || {
                left.name()
                    .to_lowercase()
                    .cmp(&right.name().to_lowercase())
                    .then_with(|| left.name().cmp(right.name()))
            };
            right.is_active().cmp(&left.is_active()).then_with(|| {
                match sort {
                    FileSort::Newest => right.received_at().cmp(&left.received_at()),
                    FileSort::Oldest => left.received_at().cmp(&right.received_at()),
                    FileSort::NameAsc => name(),
                    FileSort::NameDesc => name().reverse(),
                    FileSort::Largest => right.size().cmp(&left.size()),
                    FileSort::Smallest => left.size().cmp(&right.size()),
                }
                .then_with(name)
                .then_with(|| left.id().cmp(right.id()))
            })
        });
        entries
    }

    #[test]
    fn paginated_queries_match_full_sort_oracle_for_all_directions_and_filters() {
        let mut records: Vec<_> = (0..257)
            .map(|index| {
                let mut record = saved(
                    &format!("id-{index:03}"),
                    ["Été.txt", "alpha.txt", "Alpha.txt", "中文.txt"][index % 4],
                );
                record.size = (index % 7) as u64;
                record.received_at_unix = (index % 11) as u64;
                if index % 5 == 0 {
                    record.delivery_status = DeliveryStatus::AckPending;
                }
                if index % 7 == 0 {
                    record.delivery_error = Some("retry".into());
                }
                record
            })
            .collect();
        records.push(saved("id-002", "last saved duplicate wins.txt"));
        let mut activities = vec![
            live("id-003", "overrides saved.txt"),
            live("new", "Été live.txt"),
        ];
        let mut failed = live("new", "last live duplicate wins.txt");
        failed.phase = None;
        failed.error = Some("failed".into());
        activities.push(failed);
        let history = FileHistory::from(records.clone());
        for sort in SORTS {
            for filter in [
                FileFilter::All,
                FileFilter::InProgress,
                FileFilter::Attention,
                FileFilter::Completed,
            ] {
                for kind in [FileKind::All, FileKind::Images] {
                    for query in ["", "  ÉTÉ  ", "no matches"] {
                        let expected = reference(&records, &activities, sort, filter, kind, query);
                        for limit in [0, 1, 7, 100, 257, usize::MAX] {
                            let page =
                                file_page(&records, &activities, sort, filter, kind, query, limit);
                            let indexed =
                                history.page(&activities, sort, filter, kind, query, limit);
                            assert_eq!(indexed.entries, page.entries);
                            assert_eq!(indexed.total, page.total);
                            assert_eq!(page.total, expected.len());
                            assert_eq!(
                                page.entries,
                                expected[..limit.min(expected.len())],
                                "{sort:?}/{filter:?}/{kind:?}/{query}/{limit}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn prepared_history_updates_names_types_and_ids_without_stale_filters() {
        let mut history =
            FileHistory::from(vec![saved("same", "old.txt"), saved("same", "last.txt")]);
        let unique = history.prepared.get().unwrap().unique.as_ptr();
        let mut changed = saved("same", "NEW.png");
        changed.media_type = "image/png".into();
        assert!(!history.upsert(changed));
        assert_eq!(history.prepared.get().unwrap().unique.as_ptr(), unique);
        let page = history.page(
            &[],
            FileSort::NameAsc,
            FileFilter::All,
            FileKind::Images,
            " new ",
            100,
        );
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].name(), "NEW.png");
        assert!(
            history
                .page(
                    &[],
                    FileSort::NameAsc,
                    FileFilter::All,
                    FileKind::All,
                    "last",
                    100
                )
                .entries
                .is_empty()
        );
        assert!(history.upsert(saved("added", "added.txt")));
        assert_eq!(
            history
                .page(
                    &[],
                    FileSort::NameAsc,
                    FileFilter::All,
                    FileKind::All,
                    "",
                    100
                )
                .total,
            2
        );
        // DerefMut is an escape hatch for fixtures/bulk edits; it invalidates keys.
        history[2].id = "renamed-id".into();
        history[2].original_name = "renamed.txt".into();
        assert!(history.prepared.get().is_none());
        assert_eq!(
            history
                .page(
                    &[],
                    FileSort::NameAsc,
                    FileFilter::All,
                    FileKind::All,
                    "renamed",
                    100
                )
                .entries[0]
                .id(),
            "renamed-id"
        );
        history.clear();
        assert_eq!(
            history
                .page(
                    &[],
                    FileSort::Newest,
                    FileFilter::All,
                    FileKind::All,
                    "",
                    100
                )
                .total,
            0
        );
    }

    #[test]
    fn active_files_are_first_in_every_sort_with_no_twelve_row_truncation() {
        let mut records: Vec<_> = (0..24)
            .map(|i| saved(&format!("id-{i}"), &format!("file-{i}.txt")))
            .collect();
        records[0].delivery_status = DeliveryStatus::AckPending;
        records[1].wallpaper_status = WallpaperStatus::Running;
        let activities = vec![live("id-23", "z-last.txt"), live("new", "a-first.txt")];
        for sort in SORTS {
            let entries = all(&records, &activities, sort);
            assert_eq!(entries.len(), 25);
            assert!(entries[..2].iter().all(FileEntry::is_active));
            assert!(entries[2..].iter().all(|entry| !entry.is_active()));
            assert_eq!(
                entries.iter().filter(|entry| entry.id() == "id-23").count(),
                1
            );
        }
    }

    #[test]
    fn name_status_and_type_filters_intersect_with_unicode_search() {
        let mut failed = saved("failed", "ÉTÉ 插画.png");
        failed.media_type = "image/png".into();
        failed.wallpaper_status = WallpaperStatus::Failed;
        let mut done = failed.clone();
        done.id = "done".into();
        done.wallpaper_status = WallpaperStatus::Applied;
        let mut document = failed.clone();
        document.id = "document".into();
        document.media_type = "text/plain".into();
        let records = [failed, done, document];
        let entries = visible_files(
            &records,
            &[],
            FileSort::Newest,
            FileFilter::Attention,
            FileKind::Images,
            "  été 插画  ",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id(), "failed");
        assert!(
            visible_files(
                &records,
                &[],
                FileSort::Newest,
                FileFilter::Completed,
                FileKind::Images,
                "missing"
            )
            .is_empty()
        );
    }

    #[test]
    fn completed_files_are_quiet_including_images_without_wallpaper_command() {
        for status in [
            WallpaperStatus::NotApplicable,
            WallpaperStatus::NotConfigured,
            WallpaperStatus::Applied,
        ] {
            let mut record = saved("done", "image.png");
            record.media_type = "image/png".into();
            record.wallpaper_status = status;
            let entry = FileEntry::Saved(record);
            assert!(entry.is_complete());
            assert!(!entry.is_pending());
            assert!(!entry.is_active());
            assert!(!entry.needs_attention());
        }
    }

    #[test]
    fn pending_saved_files_are_not_active_and_errors_are_not_complete() {
        let mut record = saved("pending", "notes.txt");
        record.delivery_status = DeliveryStatus::AckPending;
        let pending = FileEntry::Saved(record.clone());
        assert!(pending.is_pending());
        assert!(!pending.is_active());
        assert!(!pending.is_complete());
        record.delivery_status = DeliveryStatus::Acknowledged;
        for wallpaper in [WallpaperStatus::Failed, WallpaperStatus::Uncertain] {
            record.wallpaper_status = wallpaper;
            let entry = FileEntry::Saved(record.clone());
            assert!(entry.needs_attention());
            assert!(!entry.is_complete());
        }
        record.wallpaper_status = WallpaperStatus::NotApplicable;
        record.delivery_error = Some("ACK rejected".into());
        assert!(FileEntry::Saved(record.clone()).needs_attention());
        assert!(!FileEntry::Saved(record.clone()).is_complete());
        record.delivery_error = None;
        record.wallpaper_error = Some("Wallpaper hook failed".into());
        assert!(FileEntry::Saved(record.clone()).needs_attention());
        assert!(!FileEntry::Saved(record).is_complete());
    }

    #[test]
    fn progress_filter_includes_pending_work_but_completed_filter_excludes_it() {
        let mut ack = saved("ack", "confirm.txt");
        ack.delivery_status = DeliveryStatus::AckPending;
        let mut pending = saved("pending", "pending.png");
        pending.wallpaper_status = WallpaperStatus::Pending;
        let mut running = saved("running", "running.png");
        running.wallpaper_status = WallpaperStatus::Running;
        let records = [ack, pending, running, saved("complete", "done.txt")];
        let activity = [live("active", "new.txt")];
        let progress = visible_files(
            &records,
            &activity,
            FileSort::Newest,
            FileFilter::InProgress,
            FileKind::All,
            "",
        );
        assert_eq!(progress.len(), 4);
        assert_eq!(progress[0].id(), "active");
        assert_eq!(progress.iter().filter(|entry| entry.is_active()).count(), 1);
        let completed = visible_files(
            &records,
            &activity,
            FileSort::Newest,
            FileFilter::Completed,
            FileKind::All,
            "",
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id(), "complete");
    }

    #[test]
    fn live_recovery_overrides_stale_error_and_stops_being_active_on_failure() {
        let mut record = saved("retry", "notes.txt");
        record.delivery_error = Some("Disconnected".into());
        let mut activity = live("retry", "notes.txt");
        activity.phase = Some(SyncPhase::Retrying);
        let recovered = all(&[record.clone()], &[activity.clone()], FileSort::Newest);
        assert!(recovered[0].is_active());
        assert!(!recovered[0].needs_attention());
        activity.phase = None;
        activity.error = Some("Retry limit reached".into());
        let failed = all(&[record], &[activity], FileSort::Newest);
        assert!(!failed[0].is_active());
        assert!(!failed[0].is_pending());
        assert!(failed[0].needs_attention());
        assert!(!failed[0].is_complete());
    }

    #[test]
    fn mime_categories_cover_documents_media_archives_and_unknown_files() {
        for (mime, kind) in [
            (" image/svg+xml ; charset=utf-8", FileKind::Images),
            ("IMAGE/PNG", FileKind::Images),
            ("text/csv", FileKind::Documents),
            ("application/pdf", FileKind::Documents),
            ("application/json", FileKind::Documents),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                FileKind::Documents,
            ),
            ("audio/flac", FileKind::Audio),
            ("application/ogg", FileKind::Audio),
            ("video/mp4", FileKind::Video),
            ("video/ogg", FileKind::Video),
            ("application/zip", FileKind::Archives),
            ("application/x-7z-compressed", FileKind::Archives),
            ("application/vnd.rar", FileKind::Archives),
            ("application/gzip", FileKind::Archives),
            ("application/x-tar", FileKind::Archives),
            ("application/x-bzip2", FileKind::Archives),
            ("application/x-xz", FileKind::Archives),
            ("application/zstd", FileKind::Archives),
            ("application/octet-stream", FileKind::Other),
            ("", FileKind::Other),
        ] {
            assert_eq!(FileKind::of(mime), kind, "{mime}");
        }
    }

    #[test]
    fn saved_sort_orders_use_time_name_and_size_with_deterministic_ties() {
        let mut a = saved("a", "Alpha.txt");
        a.received_at_unix = 1;
        a.size = 300;
        let mut b = saved("b", "Beta.txt");
        b.received_at_unix = 2;
        b.size = 100;
        let records = [b, a];
        for (sort, first) in [
            (FileSort::Newest, "b"),
            (FileSort::Oldest, "a"),
            (FileSort::NameAsc, "a"),
            (FileSort::NameDesc, "b"),
            (FileSort::Largest, "a"),
            (FileSort::Smallest, "b"),
        ] {
            assert_eq!(all(&records, &[], sort)[0].id(), first);
        }
        let identical = [saved("b", "same.txt"), saved("a", "same.txt")];
        for sort in SORTS {
            assert_eq!(all(&identical, &[], sort)[0].id(), "a");
        }
    }

    #[test]
    fn enum_keys_round_trip_and_unknown_keys_do_not_change_defaults() {
        assert_eq!(FileSort::default(), FileSort::Newest);
        assert_eq!(FileFilter::default(), FileFilter::All);
        assert_eq!(FileKind::default(), FileKind::All);
        for value in SORTS {
            assert_eq!(FileSort::from_key(value.key()), Some(value));
            assert!(!value.label().is_empty());
        }
        for value in [
            FileFilter::All,
            FileFilter::InProgress,
            FileFilter::Attention,
            FileFilter::Completed,
        ] {
            assert_eq!(FileFilter::from_key(value.key()), Some(value));
            assert!(!value.label().is_empty());
        }
        for value in [
            FileKind::All,
            FileKind::Images,
            FileKind::Documents,
            FileKind::Audio,
            FileKind::Video,
            FileKind::Archives,
            FileKind::Other,
        ] {
            assert_eq!(FileKind::from_key(value.key()), Some(value));
            assert!(!value.label().is_empty());
        }
        for invalid in ["", "unknown", "ALL", " newest "] {
            assert_eq!(FileSort::from_key(invalid), None);
            assert_eq!(FileFilter::from_key(invalid), None);
            assert_eq!(FileKind::from_key(invalid), None);
        }
    }
}
