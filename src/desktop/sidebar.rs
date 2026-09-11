use std::collections::HashSet;

use super::{BridgeSnapshot, BridgeView};
use crate::model::{DeliveryStatus, WallpaperStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FolderSort {
    #[default]
    NameAsc,
    NameDesc,
    Recent,
    Oldest,
}

impl FolderSort {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "name-asc" => Some(Self::NameAsc),
            "name-desc" => Some(Self::NameDesc),
            "recent" => Some(Self::Recent),
            "oldest" => Some(Self::Oldest),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::NameAsc => "name-asc",
            Self::NameDesc => "name-desc",
            Self::Recent => "recent",
            Self::Oldest => "oldest",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name (A–Z)",
            Self::NameDesc => "Name (Z–A)",
            Self::Recent => "Most recent",
            Self::Oldest => "Oldest first",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FolderFilter {
    #[default]
    All,
    Attention,
    Pending,
    NewFiles,
}

impl FolderFilter {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "all" => Some(Self::All),
            "attention" => Some(Self::Attention),
            "pending" => Some(Self::Pending),
            "new-files" => Some(Self::NewFiles),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Attention => "attention",
            Self::Pending => "pending",
            Self::NewFiles => "new-files",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All folders",
            Self::Attention => "Needs attention",
            Self::Pending => "Pending",
            Self::NewFiles => "New files",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FolderNotice {
    Attention,
    Pending,
    NewFiles,
}

impl FolderNotice {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::Pending => "Pending",
            Self::NewFiles => "New files",
        }
    }

    pub(super) fn icon_name(self) -> &'static str {
        match self {
            Self::Attention => "dialog-warning-symbolic",
            Self::Pending => "content-loading-symbolic",
            Self::NewFiles => "document-new-symbolic",
        }
    }
}

fn needs_attention(view: &BridgeView, sync_failed: bool) -> bool {
    view.error.is_some()
        || sync_failed
        || view.snapshot.as_ref().is_none_or(|snapshot| {
            snapshot.deliveries.iter().any(|record| {
                record.delivery_error.is_some()
                    || record.wallpaper_error.is_some()
                    || matches!(
                        record.wallpaper_status,
                        WallpaperStatus::Failed | WallpaperStatus::Uncertain
                    )
            })
        })
}

fn has_pending(view: &BridgeView) -> bool {
    view.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.deliveries.iter().any(|record| {
            record.delivery_status == DeliveryStatus::AckPending
                || matches!(
                    record.wallpaper_status,
                    WallpaperStatus::Pending | WallpaperStatus::Running
                )
        })
    })
}

pub(super) fn folder_notice(
    view: &BridgeView,
    unread: bool,
    sync_failed: bool,
) -> Option<FolderNotice> {
    if needs_attention(view, sync_failed) {
        Some(FolderNotice::Attention)
    } else if has_pending(view) {
        Some(FolderNotice::Pending)
    } else if unread {
        Some(FolderNotice::NewFiles)
    } else {
        None
    }
}

pub(super) fn visible_folder_ids(
    views: &[BridgeView],
    sort: FolderSort,
    filter: FolderFilter,
    query: &str,
    unread: &HashSet<String>,
    sync_failed: &HashSet<String>,
) -> Vec<String> {
    let query = query.trim().to_lowercase();
    let mut matching = views
        .iter()
        .filter(|view| {
            let id = &view.registration.id;
            view.registration.name.to_lowercase().contains(&query)
                && match filter {
                    FolderFilter::All => true,
                    FolderFilter::Attention => needs_attention(view, sync_failed.contains(id)),
                    FolderFilter::Pending => has_pending(view),
                    FolderFilter::NewFiles => unread.contains(id),
                }
        })
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        let by_name = || {
            left.registration
                .name
                .to_lowercase()
                .cmp(&right.registration.name.to_lowercase())
                .then_with(|| left.registration.name.cmp(&right.registration.name))
                .then_with(|| left.registration.id.cmp(&right.registration.id))
        };
        match sort {
            FolderSort::NameAsc => by_name(),
            FolderSort::NameDesc => by_name().reverse(),
            FolderSort::Recent => latest_received(right)
                .cmp(&latest_received(left))
                .then_with(by_name),
            FolderSort::Oldest => {
                let left_received = latest_received(left);
                let right_received = latest_received(right);
                left_received
                    .is_none()
                    .cmp(&right_received.is_none())
                    .then_with(|| left_received.cmp(&right_received))
                    .then_with(by_name)
            }
        }
    });
    matching
        .into_iter()
        .map(|view| view.registration.id.clone())
        .collect()
}

fn latest_received(view: &BridgeView) -> Option<u64> {
    view.snapshot.as_ref().and_then(|snapshot| {
        snapshot
            .deliveries
            .iter()
            .map(|record| record.received_at_unix)
            .max()
    })
}

pub(super) fn has_new_deliveries(previous: &BridgeSnapshot, next: &BridgeSnapshot) -> bool {
    let previous_ids: HashSet<&str> = previous
        .deliveries
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    next.deliveries
        .iter()
        .any(|record| !previous_ids.contains(record.id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_registry::BridgeRegistration;
    use crate::desktop::BridgeCounts;
    use crate::model::DeliveryRecord;
    use std::path::PathBuf;

    fn record(id: &str, received_at_unix: u64) -> DeliveryRecord {
        DeliveryRecord {
            id: id.into(),
            original_name: format!("{id}.txt"),
            media_type: "text/plain".into(),
            sha256: "0".repeat(64),
            size: 0,
            stored_path: PathBuf::from("/dev/null"),
            delivery_status: DeliveryStatus::Acknowledged,
            wallpaper_status: WallpaperStatus::NotApplicable,
            wallpaper_attempts: 0,
            delivery_error: None,
            wallpaper_error: None,
            source_created_at_unix: None,
            received_at_unix,
            acknowledged_at_unix: Some(received_at_unix),
            imported_at_unix: None,
        }
    }

    fn view(id: &str, name: &str, deliveries: Vec<DeliveryRecord>) -> BridgeView {
        BridgeView {
            registration: BridgeRegistration {
                kind: Default::default(),
                id: id.into(),
                name: name.into(),
                config_path: PathBuf::from(format!("/fixtures/{id}.toml")),
                auto_receive: false,
            },
            snapshot: Some(BridgeSnapshot {
                directory: None,
                name: name.into(),
                device_id: id.into(),
                source_label: "Local inbox".into(),
                library_dir: PathBuf::from(format!("/fixtures/{id}")),
                max_file_size_bytes: 1024,
                wallpaper_label: "Disabled".into(),
                counts: BridgeCounts {
                    total: deliveries.len(),
                    ..BridgeCounts::default()
                },
                deliveries: std::sync::Arc::new(deliveries.into()),
            }),
            error: None,
        }
    }

    fn ids(views: &[BridgeView], sort: FolderSort) -> Vec<String> {
        visible_folder_ids(
            views,
            sort,
            FolderFilter::All,
            "",
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    #[test]
    fn defaults_and_action_keys_round_trip() {
        assert_eq!(FolderSort::default(), FolderSort::NameAsc);
        assert_eq!(FolderFilter::default(), FolderFilter::All);
        for sort in [
            FolderSort::NameAsc,
            FolderSort::NameDesc,
            FolderSort::Recent,
            FolderSort::Oldest,
        ] {
            assert_eq!(FolderSort::from_key(sort.key()), Some(sort));
            assert!(!sort.label().is_empty());
        }
        for filter in [
            FolderFilter::All,
            FolderFilter::Attention,
            FolderFilter::Pending,
            FolderFilter::NewFiles,
        ] {
            assert_eq!(FolderFilter::from_key(filter.key()), Some(filter));
            assert!(!filter.label().is_empty());
        }
        assert_eq!(FolderSort::from_key("unknown"), None);
        assert_eq!(FolderFilter::from_key("unknown"), None);
        for notice in [
            FolderNotice::Attention,
            FolderNotice::Pending,
            FolderNotice::NewFiles,
        ] {
            assert!(!notice.label().is_empty());
            assert!(notice.icon_name().ends_with("-symbolic"));
        }
    }

    #[test]
    fn ordinary_folders_and_disabled_wallpaper_are_quiet() {
        assert_eq!(
            folder_notice(&view("empty", "Empty", vec![]), false, false),
            None
        );
        for status in [
            WallpaperStatus::NotApplicable,
            WallpaperStatus::NotConfigured,
            WallpaperStatus::Applied,
        ] {
            let mut delivery = record("received", 1);
            delivery.wallpaper_status = status;
            assert_eq!(
                folder_notice(&view("files", "Files", vec![delivery]), false, false),
                None
            );
        }
    }

    #[test]
    fn notices_prioritize_attention_over_pending_over_new_files() {
        let mut folder = view("files", "Files", vec![record("new", 1)]);
        assert_eq!(
            folder_notice(&folder, true, false),
            Some(FolderNotice::NewFiles)
        );
        std::sync::Arc::make_mut(&mut folder.snapshot.as_mut().unwrap().deliveries)[0]
            .delivery_status = DeliveryStatus::AckPending;
        assert_eq!(
            folder_notice(&folder, true, false),
            Some(FolderNotice::Pending)
        );
        assert_eq!(
            folder_notice(&folder, true, true),
            Some(FolderNotice::Attention)
        );
        std::sync::Arc::make_mut(&mut folder.snapshot.as_mut().unwrap().deliveries)[0]
            .delivery_error = Some("Retry needed".into());
        assert_eq!(
            folder_notice(&folder, true, false),
            Some(FolderNotice::Attention)
        );
    }

    #[test]
    fn failed_or_unavailable_config_and_wallpaper_errors_need_attention() {
        let mut folder = view("files", "Files", vec![]);
        folder.error = Some("Configuration could not be read".into());
        assert_eq!(
            folder_notice(&folder, false, false),
            Some(FolderNotice::Attention)
        );
        folder.error = None;
        folder.snapshot = None;
        assert_eq!(
            folder_notice(&folder, false, false),
            Some(FolderNotice::Attention)
        );
        for status in [WallpaperStatus::Failed, WallpaperStatus::Uncertain] {
            let mut delivery = record("wallpaper", 1);
            delivery.wallpaper_status = status;
            assert_eq!(
                folder_notice(&view("files", "Files", vec![delivery]), false, false),
                Some(FolderNotice::Attention)
            );
        }
        let mut delivery = record("wallpaper", 1);
        delivery.wallpaper_error = Some("Wallpaper command failed".into());
        assert_eq!(
            folder_notice(&view("files", "Files", vec![delivery]), false, false),
            Some(FolderNotice::Attention)
        );
    }

    #[test]
    fn pending_wallpaper_is_visible_without_acknowledgement_pending() {
        for status in [WallpaperStatus::Pending, WallpaperStatus::Running] {
            let mut delivery = record("wallpaper", 1);
            delivery.wallpaper_status = status;
            assert_eq!(
                folder_notice(&view("files", "Files", vec![delivery]), false, false),
                Some(FolderNotice::Pending)
            );
        }
    }

    #[test]
    fn name_sort_is_case_insensitive_and_deterministic_in_both_directions() {
        let views = [
            view("z", "beta", vec![]),
            view("lower", "alpha", vec![]),
            view("b", "Alpha", vec![]),
            view("a", "Alpha", vec![]),
        ];
        assert_eq!(ids(&views, FolderSort::NameAsc), ["a", "b", "lower", "z"]);
        assert_eq!(ids(&views, FolderSort::NameDesc), ["z", "lower", "b", "a"]);
        let reversed = views.into_iter().rev().collect::<Vec<_>>();
        assert_eq!(
            ids(&reversed, FolderSort::NameAsc),
            ["a", "b", "lower", "z"]
        );
    }

    #[test]
    fn time_sorts_use_latest_received_time_with_empty_or_unreadable_folders_last() {
        let mut source_newer = record("old", 10);
        source_newer.source_created_at_unix = Some(1000);
        let mut unreadable = view("unreadable", "B unreadable", vec![]);
        unreadable.snapshot = None;
        unreadable.error = Some("Configuration could not be read".into());
        let views = [
            view("empty", "A empty", vec![]),
            unreadable,
            view("old", "Older", vec![source_newer]),
            view("z", "Zulu", vec![record("new", 50)]),
            view("a", "Alpha", vec![record("newer", 50), record("older", 1)]),
            view("epoch", "Epoch", vec![record("epoch", 0)]),
        ];
        assert_eq!(
            ids(&views, FolderSort::Recent),
            ["a", "z", "old", "epoch", "empty", "unreadable"]
        );
        assert_eq!(
            ids(&views, FolderSort::Oldest),
            ["epoch", "old", "a", "z", "empty", "unreadable"]
        );
    }

    #[test]
    fn time_sort_ties_use_case_insensitive_name_then_exact_name_then_id() {
        let views = [
            view("z", "beta", vec![record("same-time", 50)]),
            view("lower", "alpha", vec![record("same-time", 50)]),
            view("b", "Alpha", vec![record("same-time", 50)]),
            view("a", "Alpha", vec![record("same-time", 50)]),
            view("empty-b", "Empty", vec![]),
            view("empty-a", "Empty", vec![]),
        ];
        for sort in [FolderSort::Recent, FolderSort::Oldest] {
            assert_eq!(
                ids(&views, sort),
                ["a", "b", "lower", "z", "empty-a", "empty-b"]
            );
        }
        let reversed = views.into_iter().rev().collect::<Vec<_>>();
        for sort in [FolderSort::Recent, FolderSort::Oldest] {
            assert_eq!(
                ids(&reversed, sort),
                ["a", "b", "lower", "z", "empty-a", "empty-b"]
            );
        }
    }

    #[test]
    fn search_is_trimmed_case_insensitive_and_name_only() {
        let views = [
            view("secret-id", "ÉTUDES 中文", vec![record("needle", 1)]),
            view("other", "Other", vec![]),
        ];
        let search = |query| {
            visible_folder_ids(
                &views,
                FolderSort::NameAsc,
                FolderFilter::All,
                query,
                &HashSet::new(),
                &HashSet::new(),
            )
        };
        assert_eq!(search("  études  "), ["secret-id"]);
        assert_eq!(search("中文"), ["secret-id"]);
        assert_eq!(search("  ").len(), 2);
        assert!(search("secret-id").is_empty());
        assert!(search("needle").is_empty());
        assert!(search("Local inbox").is_empty());
    }

    #[test]
    fn filters_are_independent_of_notice_precedence_and_combine_with_search() {
        let mut pending = record("pending", 1);
        pending.delivery_status = DeliveryStatus::AckPending;
        let mut both = pending.clone();
        both.delivery_error = Some("Retry failed".into());
        let views = [
            view("quiet", "Quiet", vec![]),
            view("pending", "Pending", vec![pending]),
            view("both", "Mixed", vec![both]),
            view("unread", "Unread", vec![record("new", 2)]),
            view("sync-failed", "Sync failed", vec![]),
        ];
        let unread = HashSet::from(["both".into(), "unread".into()]);
        let sync_failed = HashSet::from(["sync-failed".into()]);
        let filter = |filter, query| {
            visible_folder_ids(
                &views,
                FolderSort::NameAsc,
                filter,
                query,
                &unread,
                &sync_failed,
            )
        };
        assert_eq!(filter(FolderFilter::Attention, ""), ["both", "sync-failed"]);
        assert_eq!(filter(FolderFilter::Pending, ""), ["both", "pending"]);
        assert_eq!(filter(FolderFilter::NewFiles, ""), ["both", "unread"]);
        assert_eq!(filter(FolderFilter::Pending, " mixed "), ["both"]);
        assert!(filter(FolderFilter::Attention, "Unread").is_empty());
        assert!(
            visible_folder_ids(
                &[],
                FolderSort::NameAsc,
                FolderFilter::All,
                "",
                &unread,
                &sync_failed
            )
            .is_empty()
        );
    }

    #[test]
    fn new_deliveries_compare_ids_not_counts_or_status_changes() {
        let previous = view("files", "Files", vec![record("a", 1), record("b", 2)])
            .snapshot
            .unwrap();
        let mut next = previous.clone();
        std::sync::Arc::make_mut(&mut next.deliveries).reverse();
        std::sync::Arc::make_mut(&mut next.deliveries)[0].wallpaper_status =
            WallpaperStatus::Applied;
        assert!(!has_new_deliveries(&previous, &next));
        std::sync::Arc::make_mut(&mut next.deliveries).pop();
        assert!(!has_new_deliveries(&previous, &next));
        std::sync::Arc::make_mut(&mut next.deliveries).push(record("replacement", 3));
        assert_eq!(previous.deliveries.len(), next.deliveries.len());
        assert!(has_new_deliveries(&previous, &next));
        std::sync::Arc::make_mut(&mut next.deliveries).push(record("additional", 4));
        assert!(has_new_deliveries(&previous, &next));
        std::sync::Arc::make_mut(&mut next.deliveries).clear();
        assert!(!has_new_deliveries(&previous, &next));
    }
}
