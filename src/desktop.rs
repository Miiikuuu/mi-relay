use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context, Result, bail};
use clap::Parser;
use gtk::{gio, glib};
use uuid::Uuid;

use crate::bridge_registry::{
    BridgeRegistration, BridgeRegistry, BridgeRegistryStore, default_registry_path,
};
use crate::client::source_for;
use crate::config::{
    Config, DEFAULT_HTTP_PAGE_SIZE, DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS,
    DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS, DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
    DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS, DEFAULT_HTTP_TOKEN_ENV, InitOverrides, ServerConfig,
    default_config_path, default_data_dir,
};
use crate::model::{DeliveryRecord, DeliveryStatus, WallpaperStatus};
use crate::state::StateStore;
use crate::sync::{SyncEvent, SyncPhase, SyncSummary, status_counts, sync_once_with_events};

mod brand;
mod directory_panel;
#[cfg(test)]
mod directory_tests;
mod file_smoke;
mod files;
mod loading;
mod pairing_panel;
mod photos;
mod sidebar;
mod sorting;
mod stress_smoke;
use files::{FileActivity, FileEntry, FileFilter, FileHistory, FileKind, FileSort, visible_files};
use loading::loading_ring;
use sidebar::{FolderFilter, FolderSort, folder_notice, has_new_deliveries, visible_folder_ids};
use sorting::{SortField, sort_button};

const APPLICATION_ID: &str = "io.mirelay.Desktop";
const AUTO_RECEIVE_INTERVAL_SECONDS: u64 = 60;
const FILE_PAGE_SIZE: usize = 100;
const FILE_KINDS: [FileKind; 7] = [
    FileKind::All,
    FileKind::Images,
    FileKind::Documents,
    FileKind::Audio,
    FileKind::Video,
    FileKind::Archives,
    FileKind::Other,
];

#[derive(Clone)]
struct FileViewOptions {
    sort: FileSort,
    filter: FileFilter,
    kind: FileKind,
    query: String,
    limit: usize,
}

impl Default for FileViewOptions {
    fn default() -> Self {
        Self {
            sort: FileSort::default(),
            filter: FileFilter::default(),
            kind: FileKind::default(),
            query: String::new(),
            limit: FILE_PAGE_SIZE,
        }
    }
}

const DESKTOP_CSS: &str = r#"
.mirelay .brand-button {
  background: #ffffff;
  padding: 2px;
  border-radius: 8px;
}

.mirelay .brand-button:hover {
  box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.25);
}

.mirelay .brand-canvas {
  background: #ffffff;
}

.mirelay button.suggested-action,
.mirelay menubutton.suggested-action > button,
.mirelay switch:checked {
  background: @relay_accent;
  color: @relay_on_accent;
  box-shadow: inset 0 0 0 1px @relay_accent_border;
}

.mirelay button.suggested-action:hover,
.mirelay menubutton.suggested-action > button:hover {
  background: @relay_accent_hover;
}

.mirelay button.suggested-action:disabled {
  background: alpha(@window_fg_color, 0.10);
  color: alpha(@window_fg_color, 0.40);
  box-shadow: none;
}

.mirelay button,
.mirelay entry {
  border-radius: 6px;
}

.mirelay :focus-visible {
  outline-color: alpha(@window_fg_color, 0.65);
}

.mirelay button.suggested-action:focus-visible,
.mirelay menubutton.suggested-action > button:focus-visible,
.mirelay .bridge-list row:selected:focus-visible,
.mirelay switch:checked:focus-visible {
  outline-color: @relay_on_accent;
}

.workspace {
  background: @window_bg_color;
}

.bridge-sidebar {
  background: @view_bg_color;
  border-right: 1px solid alpha(@window_fg_color, 0.10);
}

.sidebar-heading {
  padding: 16px 14px 10px;
}

.sidebar-title {
  font-size: 14px;
  font-weight: 600;
}

.secondary-text,
.property-title {
  color: alpha(@window_fg_color, 0.68);
}

.bridge-list {
  background: transparent;
  padding: 0 8px 8px;
}

.bridge-list row {
  border-radius: 6px;
  margin: 2px 0;
}

.bridge-list row:hover {
  background: alpha(@window_fg_color, 0.055);
}

.bridge-list row:selected {
  background: @relay_accent;
  color: @relay_on_accent;
  box-shadow: inset 0 0 0 1px @relay_accent_border;
}

.bridge-list row:selected .secondary-text {
  color: alpha(@relay_on_accent, 0.75);
}

.bridge-list row:selected .folder-notice {
  color: @relay_on_accent;
}

.bridge-row {
  padding: 8px 10px;
}

.bridge-icon {
  padding: 2px;
}

.bridge-name,
.delivery-name,
.activity-title {
  font-weight: 600;
}

.folder-notice {
  color: alpha(@window_fg_color, 0.70);
}

.sidebar-heading button,
.activity-header button {
  min-width: 28px;
  min-height: 28px;
  padding: 0;
}

.folder-filter {
  padding: 8px;
}

.folder-filter checkbutton {
  padding: 6px 4px;
}

.sort-options {
  padding: 4px;
  min-width: 184px;
}

.sort-field {
  padding: 2px 2px 2px 6px;
}

.sort-options button.sort-direction {
  min-width: 28px;
  min-height: 28px;
  padding: 0;
  border-radius: 5px;
  color: alpha(@window_fg_color, 0.50);
  box-shadow: none;
}

.sort-options button.sort-direction:hover {
  background: alpha(@window_fg_color, 0.07);
  color: @window_fg_color;
}

.sort-options button.sort-direction:checked {
  background: @relay_accent;
  color: @relay_on_accent;
  box-shadow: inset 0 0 0 1px @relay_accent_border;
}

.sort-options button.sort-direction:checked:focus-visible {
  outline-color: @relay_on_accent;
}

.sidebar-no-results {
  padding: 20px 12px;
}

.bridge-page {
  padding: 24px;
}

.bridge-title {
  font-size: 23px;
  font-weight: 700;
  letter-spacing: -0.4px;
}

.bridge-hero-icon {
  background: alpha(@window_fg_color, 0.055);
  color: @window_fg_color;
  border-radius: 8px;
  padding: 8px;
}

.bridge-status {
  font-weight: 600;
  color: @window_fg_color;
}

.state-success {
  color: alpha(@window_fg_color, 0.75);
}

.state-pending {
  color: @warning_color;
}

.state-error {
  color: @error_color;
}

.property-group {
  background: @card_bg_color;
  border: 1px solid alpha(@window_fg_color, 0.09);
  border-radius: 8px;
}

.property-row {
  padding: 9px 12px;
  border-bottom: 1px solid alpha(@window_fg_color, 0.075);
}

.property-row:last-child {
  border-bottom: none;
}

.property-title {
  font-size: 12px;
  font-weight: 400;
}

.property-value {
  font-size: 13px;
}

.activity-header {
  margin-top: 4px;
}

.activity-title {
  font-size: 14px;
}

.activity-list {
  background: @card_bg_color;
  border: 1px solid alpha(@window_fg_color, 0.09);
  border-radius: 8px;
}

.activity-list row {
  border-bottom: 1px solid alpha(@window_fg_color, 0.075);
}

.activity-list row:last-child {
  border-bottom: none;
}

.delivery-row {
  padding: 8px 12px;
}

.mime-icon {
  color: alpha(@window_fg_color, 0.75);
  padding: 4px;
}

.activity-empty {
  padding: 18px 16px;
  border: 1px solid alpha(@window_fg_color, 0.09);
  border-radius: 8px;
}

.row-state {
  font-size: 12px;
  font-weight: 600;
}

.error-banner {
  background: alpha(#e01b24, 0.12);
  border-bottom: 1px solid alpha(#e01b24, 0.24);
  padding: 10px 16px;
}

.settings-page {
  padding: 16px 20px;
}

.settings-page row {
  min-height: 48px;
}
"#;

#[derive(Debug, Parser)]
#[command(
    name = "mirelay-desktop",
    version,
    about = "Native Linux desktop frontend for MiRelay"
)]
struct DesktopArgs {
    /// Import or select a MiRelay Folder configuration file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a specific desktop Folder registry.
    #[arg(long, value_name = "PATH")]
    registry: Option<PathBuf>,

    /// Open an independent window using an explicit Folder registry.
    #[arg(long, requires = "registry")]
    new_instance: bool,

    /// Open the window briefly and exit. Used by automated smoke tests.
    #[arg(long, hide = true)]
    smoke_test: bool,

    /// Save a rendered window for visual regression checks and exit.
    #[arg(long, value_name = "PATH", hide = true)]
    screenshot: Option<PathBuf>,

    /// Capture Folder Settings instead of the main window.
    #[arg(long, requires = "screenshot", hide = true)]
    screenshot_settings: bool,

    /// Set the main window width for visual regression checks.
    #[arg(long, requires = "screenshot", value_parser = clap::value_parser!(i32).range(820..=2400), hide = true)]
    screenshot_width: Option<i32>,

    /// Capture an open sidebar menu for visual checks.
    #[arg(long, requires = "screenshot", conflicts_with = "screenshot_settings", value_parser = ["sort", "filter", "file-sort", "file-filter"], hide = true)]
    screenshot_sidebar_menu: Option<String>,

    /// Apply a sidebar filter before capture.
    #[arg(long, requires = "screenshot", value_parser = ["all", "attention", "pending", "new-files"], hide = true)]
    screenshot_folder_filter: Option<String>,

    /// Apply a file filter before capture.
    #[arg(long, requires = "screenshot", value_parser = ["all", "in-progress", "attention", "completed"], hide = true)]
    screenshot_file_filter: Option<String>,

    /// Preview a synthetic active file; never initiates a real transfer.
    #[arg(long, requires = "screenshot", hide = true)]
    screenshot_file_activity: bool,

    /// Exercise sidebar controls in an isolated graphical test window and exit.
    #[arg(long, requires = "registry", conflicts_with_all = ["smoke_test", "screenshot", "automation_smoke_test"], hide = true)]
    sidebar_smoke_test: bool,

    /// Exercise file controls and live updates using a disposable registry.
    #[arg(long, requires = "registry", conflicts_with_all = ["smoke_test", "sidebar_smoke_test", "screenshot", "automation_smoke_test"], hide = true)]
    file_smoke_test: bool,

    /// Run automatic receive on a short interval and exit. Used by integration smoke tests.
    #[arg(long, hide = true)]
    automation_smoke_test: bool,
}

#[derive(Clone)]
struct ScreenshotOptions {
    path: PathBuf,
    settings: bool,
    width: Option<i32>,
    sidebar_menu: Option<String>,
    folder_filter: Option<String>,
    file_filter: Option<String>,
    file_activity: bool,
    failure: Rc<RefCell<Option<String>>>,
}

struct SmokeChecks {
    sidebar: bool,
    files: bool,
    failure: Rc<RefCell<Option<String>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BridgeCounts {
    total: usize,
    ack_pending: usize,
    acknowledged: usize,
    wallpaper_pending: usize,
    wallpaper_attention: usize,
}

#[derive(Debug, Clone)]
struct BridgeSnapshot {
    name: String,
    device_id: String,
    source_label: String,
    library_dir: PathBuf,
    max_file_size_bytes: u64,
    wallpaper_label: String,
    counts: BridgeCounts,
    deliveries: Arc<FileHistory>,
    directory: Option<Arc<directory_panel::DirectorySnapshot>>,
}

#[derive(Debug, Clone)]
struct BridgeView {
    registration: BridgeRegistration,
    snapshot: Option<BridgeSnapshot>,
    error: Option<String>,
}

#[derive(Debug)]
struct RegistrySnapshot {
    bridges: Vec<BridgeView>,
    selected_bridge_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DesktopPaths {
    registry: BridgeRegistryStore,
    bridge_config_dir: PathBuf,
    bridge_data_dir: PathBuf,
    bootstrap_config: Option<PathBuf>,
    bootstrap_only_if_registry_missing: bool,
    bootstrap_pending: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct BridgeResources {
    library_dir: PathBuf,
    state_file: PathBuf,
    device_id: String,
    filesystem_inbox: Option<PathBuf>,
    http_endpoint: Option<String>,
    directory_state: Option<PathBuf>,
}

impl BridgeResources {
    fn from_config(config: &Config) -> Self {
        Self {
            library_dir: comparable_path(&config.storage.library_dir),
            state_file: comparable_path(&config.storage.state_file),
            device_id: config.device_id.clone(),
            directory_state: config
                .directory_sync
                .then(|| comparable_path(&crate::config::directory_state_dir(config))),
            filesystem_inbox: match &config.server {
                ServerConfig::Filesystem { inbox_dir } => Some(comparable_path(inbox_dir)),
                ServerConfig::Http { .. } => None,
            },
            http_endpoint: match &config.server {
                ServerConfig::Http { base_url, .. } => Some(normalized_http_endpoint(base_url)),
                ServerConfig::Filesystem { .. } => None,
            },
        }
    }
}

#[derive(Debug)]
struct SyncResult {
    summary: SyncSummary,
    snapshot: BridgeSnapshot,
}

#[derive(Debug)]
struct SyncRequest {
    bridge_id: String,
    bridge_name: String,
    config_path: PathBuf,
    token: Option<String>,
}

#[derive(Debug)]
struct SyncOutcome {
    bridge_id: String,
    bridge_name: String,
    result: std::result::Result<SyncResult, String>,
}

enum WorkerMessage {
    FileProgress {
        bridge_id: String,
        event: SyncEvent,
    },
    Loaded(std::result::Result<RegistrySnapshot, String>),
    Synced {
        automatic: bool,
        outcomes: Vec<SyncOutcome>,
    },
}

#[derive(Clone)]
struct Widgets {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    refresh_button: gtk::Button,
    sync_button: gtk::Button,
    sync_indicator: gtk::Stack,
    sync_label: gtk::Label,
    add_bridge_button: gtk::Button,
    settings_button: gtk::Button,
    open_library_button: gtk::Button,
    error_revealer: gtk::Revealer,
    error_label: gtk::Label,
    toast_overlay: adw::ToastOverlay,
    bridge_list: gtk::ListBox,
    sidebar_results: gtk::Stack,
    folder_sort_button: gtk::MenuButton,
    folder_filter_button: gtk::MenuButton,
    folder_search: gtk::SearchEntry,
    summary_label: gtk::Label,
    content_stack: gtk::Stack,
    empty_page: adw::StatusPage,
    empty_action: gtk::Button,
    bridge_error_page: adw::StatusPage,
    bridge_error_action: gtk::Button,
    bridge_name: gtk::Label,
    folder_category: gtk::DropDown,
    bridge_icon: gtk::Image,
    bridge_path: gtk::Label,
    bridge_state_icon: gtk::Image,
    bridge_state: gtk::Label,
    source_value: gtk::Label,
    folder_value: gtk::Label,
    device_value: gtk::Label,
    size_limit_value: gtk::Label,
    auto_receive_value: gtk::Label,
    wallpaper_value: gtk::Label,
    activity_stack: gtk::Stack,
    activity_empty: gtk::Label,
    delivery_list: gtk::ListBox,
    photo_grid: gtk::FlowBox,
    file_sort_button: gtk::MenuButton,
    file_filter_button: gtk::MenuButton,
    file_search: gtk::SearchEntry,
    file_kind: gtk::DropDown,
    file_show_more: gtk::Button,
}

struct DesktopUi {
    paths: DesktopPaths,
    bridges: RefCell<Vec<BridgeView>>,
    bridge_ids: RefCell<Vec<String>>,
    selected_bridge_id: RefCell<Option<String>>,
    folder_sort: Cell<FolderSort>,
    folder_filter: Cell<FolderFilter>,
    unread_folders: RefCell<HashSet<String>>,
    sync_failed_folders: RefCell<HashSet<String>>,
    rendering_list: Cell<bool>,
    file_views: RefCell<HashMap<String, FileViewOptions>>,
    file_activity: RefCell<HashMap<String, HashMap<String, FileActivity>>>,
    rendering_file_controls: Cell<bool>,
    processing_worker_batch: Cell<bool>,
    file_render_pending: Cell<bool>,
    rendered_files: RefCell<Vec<FileEntry>>,
    rendered_context: RefCell<Option<(String, crate::bridge_registry::FolderKind)>>,
    tokens: RefCell<HashMap<String, String>>,
    busy: Cell<bool>,
    editor_open: Cell<bool>,
    sender: mpsc::Sender<WorkerMessage>,
    widgets: Widgets,
}

fn resolve_desktop_paths(
    config: Option<PathBuf>,
    registry: Option<PathBuf>,
) -> Result<DesktopPaths> {
    let explicit_config = config.map(absolute_path).transpose()?;
    let has_explicit_config = explicit_config.is_some();
    let default_registry = default_registry_path()?;
    let registry_path = match registry {
        Some(path) => absolute_path(path)?,
        None => explicit_config
            .as_ref()
            .and_then(|path| path.parent())
            .map(|parent| parent.join("bridges.toml"))
            .unwrap_or_else(|| default_registry.clone()),
    };
    let registry_parent = registry_path
        .parent()
        .context("Folder registry path has no parent")?
        .to_path_buf();
    let uses_default_registry = registry_path == default_registry;
    let bootstrap_config = match explicit_config {
        Some(path) => Some(path),
        None if uses_default_registry => Some(default_config_path()?),
        None => None,
    };
    let bridge_data_dir = if uses_default_registry {
        default_data_dir()?.join("bridges")
    } else {
        registry_parent.join("bridge-data")
    };

    Ok(DesktopPaths {
        registry: BridgeRegistryStore::new(registry_path),
        bridge_config_dir: registry_parent.join("bridges"),
        bridge_data_dir,
        bootstrap_config,
        bootstrap_only_if_registry_missing: !has_explicit_config,
        bootstrap_pending: Arc::new(AtomicBool::new(true)),
    })
}

fn load_registry_snapshot(paths: &DesktopPaths) -> Result<RegistrySnapshot> {
    maybe_import_bootstrap_config(paths)?;
    let registry = paths.registry.load()?;
    let mut bridges = Vec::with_capacity(registry.bridges.len());
    let mut resources = Vec::with_capacity(registry.bridges.len());

    for registration in registry.bridges {
        match load_snapshot_and_resources(&registration.config_path) {
            Ok((mut snapshot, bridge_resources)) => {
                snapshot.name.clone_from(&registration.name);
                let index = bridges.len();
                bridges.push(BridgeView {
                    registration,
                    snapshot: Some(snapshot),
                    error: None,
                });
                resources.push((index, bridge_resources));
            }
            Err(error) => bridges.push(BridgeView {
                registration,
                snapshot: None,
                error: Some(format!("{error:#}")),
            }),
        }
    }
    mark_resource_conflicts(&mut bridges, &resources);

    Ok(RegistrySnapshot {
        bridges,
        selected_bridge_id: registry.selected_bridge_id,
    })
}

fn maybe_import_bootstrap_config(paths: &DesktopPaths) -> Result<()> {
    if !paths.bootstrap_pending.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let registry_exists = match std::fs::symlink_metadata(paths.registry.path()) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            paths.bootstrap_pending.store(true, Ordering::Release);
            return Err(error).with_context(|| {
                format!(
                    "Unable to inspect Folder registry {}",
                    paths.registry.path().display()
                )
            });
        }
    };
    if paths.bootstrap_only_if_registry_missing && registry_exists {
        return Ok(());
    }
    if let Err(error) = import_bootstrap_config(paths) {
        paths.bootstrap_pending.store(true, Ordering::Release);
        return Err(error);
    }
    Ok(())
}

fn import_bootstrap_config(paths: &DesktopPaths) -> Result<()> {
    let Some(config_path) = &paths.bootstrap_config else {
        return Ok(());
    };
    match std::fs::symlink_metadata(config_path) {
        Ok(metadata) if !metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            bail!(
                "Folder configuration {} is not a regular file",
                config_path.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Unable to inspect Folder configuration {}",
                    config_path.display()
                )
            });
        }
    }
    let config = load_config_for_read(config_path)?;
    let resources = BridgeResources::from_config(&config);
    let name = bridge_name_for_path(&config.storage.library_dir);
    let config_path = absolute_path(config_path.clone())?;

    paths.registry.update(|registry| {
        if let Some(existing) = registry
            .bridges
            .iter()
            .find(|bridge| bridge.config_path == config_path)
            .map(|bridge| bridge.id.clone())
        {
            registry.select(&existing)?;
            return Ok(());
        }
        ensure_resources_unique(registry, &resources, None)?;
        registry.add(BridgeRegistration {
            kind: Default::default(),
            id: Uuid::new_v4().to_string(),
            name,
            config_path,
            auto_receive: false,
        })
    })
}

fn ensure_resources_unique(
    registry: &BridgeRegistry,
    candidate: &BridgeResources,
    except_id: Option<&str>,
) -> Result<()> {
    if let Some(problem) = self_resource_problem(candidate) {
        bail!("Folder path conflict: {problem}");
    }
    for bridge in registry
        .bridges
        .iter()
        .filter(|bridge| Some(bridge.id.as_str()) != except_id)
    {
        let config = load_config_for_read(&bridge.config_path).with_context(|| {
            format!(
                "Cannot verify resources used by Folder {:?}; repair or remove it first",
                bridge.name
            )
        })?;
        let existing = BridgeResources::from_config(&config);
        if let Some(problem) = resource_problem(candidate, &existing) {
            bail!("Conflicts with Folder {:?}: {problem}", bridge.name);
        }
    }
    Ok(())
}

fn mark_resource_conflicts(bridges: &mut [BridgeView], resources: &[(usize, BridgeResources)]) {
    for (index, resource) in resources {
        if let Some(problem) = self_resource_problem(resource) {
            append_bridge_error(
                &mut bridges[*index],
                format!("Folder path conflict: {problem}"),
            );
        }
    }
    for left in 0..resources.len() {
        for right in (left + 1)..resources.len() {
            let (left_index, left_resources) = &resources[left];
            let (right_index, right_resources) = &resources[right];
            if let Some(problem) = resource_problem(left_resources, right_resources) {
                let left_name = bridges[*left_index].registration.name.clone();
                let right_name = bridges[*right_index].registration.name.clone();
                append_bridge_error(
                    &mut bridges[*left_index],
                    format!("Conflicts with Folder {right_name:?}: {problem}"),
                );
                append_bridge_error(
                    &mut bridges[*right_index],
                    format!("Conflicts with Folder {left_name:?}: {problem}"),
                );
            }
        }
    }
}

fn append_bridge_error(bridge: &mut BridgeView, error: String) {
    match &mut bridge.error {
        Some(existing) => {
            existing.push('\n');
            existing.push_str(&error);
        }
        None => bridge.error = Some(error),
    }
}

fn self_resource_problem(resources: &BridgeResources) -> Option<String> {
    if resources
        .directory_state
        .as_ref()
        .is_some_and(|state| paths_overlap(state, &resources.library_dir))
    {
        return Some("Directory state overlaps its managed folder".into());
    }
    if resources.state_file.starts_with(&resources.library_dir) {
        return Some(format!(
            "State file {} is inside the managed folder",
            resources.state_file.display()
        ));
    }
    let inbox = resources.filesystem_inbox.as_ref()?;
    if paths_overlap(&resources.library_dir, inbox) {
        return Some(format!(
            "Managed folder {} overlaps local inbox {}",
            resources.library_dir.display(),
            inbox.display()
        ));
    }
    if resources.state_file.starts_with(inbox) {
        return Some(format!(
            "State file {} is inside the local inbox",
            resources.state_file.display()
        ));
    }
    None
}

fn resource_problem(left: &BridgeResources, right: &BridgeResources) -> Option<String> {
    for (owner, other) in [(left, right), (right, left)] {
        if let Some(state) = &owner.directory_state
            && (paths_overlap(state, &other.library_dir)
                || other.state_file.starts_with(state)
                || other
                    .directory_state
                    .as_ref()
                    .is_some_and(|next| paths_overlap(state, next))
                || other
                    .filesystem_inbox
                    .as_ref()
                    .is_some_and(|inbox| paths_overlap(state, inbox)))
        {
            return Some("One Folder's directory state overlaps another Folder's resources".into());
        }
    }
    if paths_overlap(&left.library_dir, &right.library_dir) {
        return Some(format!(
            "Managed folders {} and {} overlap",
            left.library_dir.display(),
            right.library_dir.display()
        ));
    }
    if left.state_file == right.state_file {
        return Some(format!("Shared state file {}", left.state_file.display()));
    }
    if left.state_file.starts_with(&right.library_dir)
        || right.state_file.starts_with(&left.library_dir)
    {
        return Some("One Folder's state file is inside another managed folder".to_owned());
    }
    if left.device_id == right.device_id {
        return Some(format!("Shared device ID {:?}", left.device_id));
    }
    if left.http_endpoint.is_some() && left.http_endpoint == right.http_endpoint {
        return Some(format!(
            "Shared HTTP receive queue {}",
            left.http_endpoint.as_deref().unwrap_or_default()
        ));
    }
    if let (Some(left_inbox), Some(right_inbox)) = (&left.filesystem_inbox, &right.filesystem_inbox)
        && paths_overlap(left_inbox, right_inbox)
    {
        return Some(format!(
            "Local inboxes {} and {} overlap",
            left_inbox.display(),
            right_inbox.display()
        ));
    }
    if left
        .filesystem_inbox
        .as_ref()
        .is_some_and(|inbox| paths_overlap(inbox, &right.library_dir))
        || right
            .filesystem_inbox
            .as_ref()
            .is_some_and(|inbox| paths_overlap(inbox, &left.library_dir))
    {
        return Some("One Folder's local inbox overlaps another managed folder".to_owned());
    }
    if left
        .filesystem_inbox
        .as_ref()
        .is_some_and(|inbox| right.state_file.starts_with(inbox))
        || right
            .filesystem_inbox
            .as_ref()
            .is_some_and(|inbox| left.state_file.starts_with(inbox))
    {
        return Some("One Folder's state file is inside another local inbox".to_owned());
    }
    None
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn comparable_path(path: &Path) -> PathBuf {
    let normalized = normalize_absolute_path(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(canonical) = std::fs::canonicalize(&normalized) {
        return canonical;
    }

    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(cursor) {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        let (Some(name), Some(parent)) = (cursor.file_name(), cursor.parent()) else {
            return normalized;
        };
        missing.push(name.to_os_string());
        cursor = parent;
    }
}

fn normalized_http_endpoint(base_url: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(base_url) else {
        return base_url.to_owned();
    };
    if !url.path().ends_with('/') {
        let mut path = url.path().to_owned();
        path.push('/');
        url.set_path(&path);
    }
    url.to_string()
}

pub fn run() -> Result<()> {
    let DesktopArgs {
        config,
        registry,
        new_instance,
        smoke_test,
        screenshot,
        screenshot_settings,
        screenshot_width,
        screenshot_sidebar_menu,
        screenshot_folder_filter,
        screenshot_file_filter,
        screenshot_file_activity,
        sidebar_smoke_test,
        file_smoke_test,
        automation_smoke_test,
    } = DesktopArgs::parse();
    let paths = resolve_desktop_paths(config, registry)?;
    brand::register_resources()?;
    let screenshot_failure = Rc::new(RefCell::new(None));
    let screenshot_requested = screenshot.is_some();
    let screenshot = screenshot.map(|path| ScreenshotOptions {
        path,
        settings: screenshot_settings,
        width: screenshot_width,
        sidebar_menu: screenshot_sidebar_menu,
        folder_filter: screenshot_folder_filter,
        file_filter: screenshot_file_filter,
        file_activity: screenshot_file_activity,
        failure: Rc::clone(&screenshot_failure),
    });

    let flags = if new_instance
        || smoke_test
        || sidebar_smoke_test
        || file_smoke_test
        || screenshot.is_some()
        || automation_smoke_test
    {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(flags)
        .build();
    let diagnostic_failure = Rc::clone(&screenshot_failure);
    application.connect_activate(move |application| {
        install_css();
        brand::install_icons();
        build_window(
            application,
            paths.clone(),
            smoke_test,
            screenshot.clone(),
            automation_smoke_test,
            SmokeChecks {
                sidebar: sidebar_smoke_test,
                files: file_smoke_test,
                failure: Rc::clone(&diagnostic_failure),
            },
        );
    });
    let exit_code = application.run_with_args(&["mirelay-desktop"]);
    if let Some(error) = screenshot_failure.borrow_mut().take() {
        if screenshot_requested {
            bail!("failed to capture desktop window: {error}");
        }
        bail!("desktop diagnostic failed: {error}");
    }
    if exit_code != glib::ExitCode::SUCCESS {
        bail!(
            "desktop application exited with status {}",
            exit_code.value()
        );
    }
    Ok(())
}

fn build_window(
    application: &adw::Application,
    paths: DesktopPaths,
    smoke_test: bool,
    screenshot: Option<ScreenshotOptions>,
    automation_smoke_test: bool,
    checks: SmokeChecks,
) {
    let SmokeChecks {
        sidebar: sidebar_smoke_test,
        files: file_smoke_test,
        failure: diagnostic_failure,
    } = checks;
    let (widgets, error_close) = build_widgets(application);
    let (sender, receiver) = mpsc::channel();
    let ui = Rc::new(DesktopUi {
        paths,
        bridges: RefCell::new(Vec::new()),
        bridge_ids: RefCell::new(Vec::new()),
        selected_bridge_id: RefCell::new(None),
        folder_sort: Cell::new(FolderSort::default()),
        folder_filter: Cell::new(FolderFilter::default()),
        unread_folders: RefCell::new(HashSet::new()),
        sync_failed_folders: RefCell::new(HashSet::new()),
        rendering_list: Cell::new(false),
        file_views: RefCell::new(HashMap::new()),
        file_activity: RefCell::new(HashMap::new()),
        rendering_file_controls: Cell::new(false),
        processing_worker_batch: Cell::new(false),
        file_render_pending: Cell::new(false),
        rendered_files: RefCell::new(Vec::new()),
        rendered_context: RefCell::new(None),
        tokens: RefCell::new(HashMap::new()),
        busy: Cell::new(false),
        editor_open: Cell::new(false),
        sender,
        widgets,
    });

    {
        let weak = Rc::downgrade(&ui);
        ui.widgets
            .folder_category
            .connect_selected_notify(move |choice| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let Some(bridge) = ui.selected_bridge() else {
                    return;
                };
                let kind = if choice.selected() == 1 {
                    crate::bridge_registry::FolderKind::Photos
                } else {
                    crate::bridge_registry::FolderKind::General
                };
                if bridge.registration.kind == kind {
                    return;
                }
                // Presentation can be changed offline without touching config, tokens or Auto.
                if let Err(error) = ui
                    .paths
                    .registry
                    .update(|registry| registry.set_kind(&bridge.registration.id, kind))
                {
                    choice.set_selected(u32::from(!bridge.registration.kind.is_general()));
                    ui.widgets
                        .toast_overlay
                        .add_toast(adw::Toast::new(&safe_ui_message(&error.to_string(), 300)));
                    return;
                }
                if let Some(view) = ui
                    .bridges
                    .borrow_mut()
                    .iter_mut()
                    .find(|view| view.registration.id == bridge.registration.id)
                {
                    view.registration.kind = kind;
                }
                ui.rendered_files.borrow_mut().clear();
                clear_list_box(&ui.widgets.delivery_list);
                ui.render_bridge_list();
                ui.render_current_bridge();
            });
    }

    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.refresh_button.clone();
        button.connect_clicked(move |_| ui.start_refresh());
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.sync_button.clone();
        button.connect_clicked(move |_| ui.start_sync());
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.settings_button.clone();
        button.connect_clicked(move |_| ui.show_selected_bridge_editor());
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.add_bridge_button.clone();
        button.connect_clicked(move |_| {
            show_bridge_editor(&ui, BridgeEditorMode::New);
        });
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.empty_action.clone();
        button.connect_clicked(move |_| {
            show_bridge_editor(&ui, BridgeEditorMode::New);
        });
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.bridge_error_action.clone();
        button.connect_clicked(move |_| ui.show_selected_bridge_editor());
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.open_library_button.clone();
        button.connect_clicked(move |_| ui.open_library());
    }
    {
        let ui = Rc::clone(&ui);
        let list = ui.widgets.bridge_list.clone();
        list.connect_row_selected(move |_, row| {
            if !ui.rendering_list.get()
                && let Some(row) = row
            {
                ui.select_bridge_at(row.index());
            }
        });
    }
    {
        let revealer = ui.widgets.error_revealer.clone();
        error_close.connect_clicked(move |_| revealer.set_reveal_child(false));
    }

    let receive_action = gio::SimpleAction::new("receive", None);
    {
        let ui = Rc::clone(&ui);
        receive_action.connect_activate(move |_, _| {
            if ui
                .selected_bridge()
                .is_some_and(|view| view.error.is_none())
            {
                ui.start_sync();
            } else {
                ui.show_selected_bridge_editor();
            }
        });
    }
    ui.widgets.window.add_action(&receive_action);
    application.set_accels_for_action("win.receive", &["<Primary>r"]);

    let settings_action = gio::SimpleAction::new("settings", None);
    {
        let ui = Rc::clone(&ui);
        settings_action.connect_activate(move |_, _| ui.show_selected_bridge_editor());
    }
    ui.widgets.window.add_action(&settings_action);
    application.set_accels_for_action("win.settings", &["<Primary>comma"]);

    install_folder_actions(&ui, application);
    install_file_actions(&ui);

    let weak_ui = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_millis(75), move || {
        let Some(ui) = weak_ui.upgrade() else {
            return glib::ControlFlow::Break;
        };
        // Keep bulk transfers from starving GTK input/animation. Render once per batch,
        // not once for every download/verification/ACK event in the queue.
        let started = std::time::Instant::now();
        ui.processing_worker_batch.set(true);
        for _ in 0..64 {
            let Ok(message) = receiver.try_recv() else {
                break;
            };
            ui.handle_worker_message(message);
            if started.elapsed() >= Duration::from_millis(8) {
                break;
            }
        }
        ui.processing_worker_batch.set(false);
        ui.flush_file_render();
        glib::ControlFlow::Continue
    });

    if !smoke_test && !sidebar_smoke_test && !file_smoke_test && screenshot.is_none() {
        let weak_ui = Rc::downgrade(&ui);
        let interval = if automation_smoke_test {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(AUTO_RECEIVE_INTERVAL_SECONDS)
        };
        glib::timeout_add_local(interval, move || {
            let Some(ui) = weak_ui.upgrade() else {
                return glib::ControlFlow::Break;
            };
            ui.start_auto_sync();
            glib::ControlFlow::Continue
        });
    }

    if let Some(width) = screenshot.as_ref().and_then(|options| options.width) {
        ui.widgets.window.set_default_size(width, 680);
    }
    ui.widgets.window.present();
    ui.start_load_all();

    if sidebar_smoke_test || file_smoke_test {
        let application = application.clone();
        let mut attempts = 0;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            attempts += 1;
            if ui.busy.get() && attempts < 100 {
                return glib::ControlFlow::Continue;
            }
            let result = if ui.busy.get() {
                Err(anyhow::anyhow!("Desktop diagnostic load timed out"))
            } else if file_smoke_test {
                file_smoke::run(&ui)
            } else {
                sidebar_smoke_checks(&ui)
            };
            match result {
                Ok(()) => println!(
                    "{} interaction checks passed",
                    if file_smoke_test { "File" } else { "Sidebar" }
                ),
                Err(error) => {
                    diagnostic_failure
                        .replace(Some(format!("Desktop interaction check failed: {error:#}")));
                }
            }
            application.quit();
            glib::ControlFlow::Break
        });
        return;
    }

    if let Some(options) = screenshot {
        let application = application.clone();
        glib::timeout_add_local_once(Duration::from_millis(900), move || {
            if options.file_activity
                && let Some(bridge_id) = ui.selected_bridge_id.borrow().clone()
            {
                ui.handle_worker_message(WorkerMessage::FileProgress {
                    bridge_id,
                    event: SyncEvent::FileActive {
                        id: "preview-active-file".into(),
                        original_name: "Design references.zip".into(),
                        media_type: "application/zip".into(),
                        size: 16 * 1024 * 1024,
                        phase: SyncPhase::Downloading,
                    },
                });
            }
            if let Some(filter) = &options.file_filter {
                ui.widgets
                    .window
                    .lookup_action("file-filter")
                    .unwrap()
                    .activate(Some(&filter.to_variant()));
            }
            if let Some(filter) = &options.folder_filter {
                ui.widgets
                    .window
                    .lookup_action("folder-filter")
                    .unwrap()
                    .activate(Some(&filter.to_variant()));
            }
            match options.sidebar_menu.as_deref() {
                Some("sort") => ui.widgets.folder_sort_button.popup(),
                Some("filter") => ui.widgets.folder_filter_button.popup(),
                Some("file-sort") => ui.widgets.file_sort_button.popup(),
                Some("file-filter") => ui.widgets.file_filter_button.popup(),
                _ => {}
            }
            // Popovers use a separate native surface and are absent from a window snapshot.
            let capture: gtk::Widget = if options.settings {
                show_bridge_editor(
                    &ui,
                    ui.selected_bridge_id
                        .borrow()
                        .clone()
                        .map(BridgeEditorMode::Edit)
                        .unwrap_or(BridgeEditorMode::New),
                )
                .expect("the loaded Folder must have an editor")
                .upcast()
            } else if let Some(menu) = options.sidebar_menu.as_deref() {
                match menu {
                    "sort" => ui.widgets.folder_sort_button.popover(),
                    "file-sort" => ui.widgets.file_sort_button.popover(),
                    "file-filter" => ui.widgets.file_filter_button.popover(),
                    _ => ui.widgets.folder_filter_button.popover(),
                }
                .expect("the sidebar menu must have a popover")
                .upcast()
            } else {
                ui.widgets.window.clone().upcast()
            };
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Err(error) = capture_widget(&capture, &options.path) {
                    options.failure.replace(Some(format!("{error:#}")));
                }
                application.quit();
            });
        });
    } else if smoke_test {
        let application = application.clone();
        glib::timeout_add_local_once(Duration::from_millis(450), move || application.quit());
    } else if automation_smoke_test {
        let application = application.clone();
        glib::timeout_add_local_once(Duration::from_secs(5), move || application.quit());
    }
}

fn build_widgets(application: &adw::Application) -> (Widgets, gtk::Button) {
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("MiRelay")
        .default_width(1000)
        .default_height(680)
        .width_request(820)
        .height_request(560)
        .build();
    window.add_css_class("mirelay");

    let title = adw::WindowTitle::new("MiRelay", "Folder");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    header.pack_start(&brand::about_button(&window));

    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh folders")
        .build();
    refresh_button.add_css_class("flat");
    let settings_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Folder settings")
        .build();
    settings_button.add_css_class("flat");

    let sync_spinner = loading_ring(16);
    let sync_icon = gtk::Image::from_icon_name("folder-download-symbolic");
    sync_icon.set_pixel_size(16);
    let sync_indicator = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    sync_indicator.add_named(&sync_icon, Some("icon"));
    sync_indicator.add_named(&sync_spinner, Some("spinner"));
    sync_indicator.set_visible_child_name("icon");
    let sync_label = gtk::Label::new(Some("Receive"));
    let sync_content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    sync_content.append(&sync_indicator);
    sync_content.append(&sync_label);
    let sync_button = gtk::Button::builder()
        .child(&sync_content)
        .tooltip_text("Receive, verify, and acknowledge new files")
        .build();
    sync_button.add_css_class("suggested-action");

    header.pack_end(&settings_button);
    header.pack_end(&refresh_button);
    header.pack_end(&sync_button);

    let error_label = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    let error_close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Close")
        .build();
    error_close.add_css_class("flat");
    let error_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    error_box.add_css_class("error-banner");
    error_box.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
    error_box.append(&error_label);
    error_box.append(&error_close);
    let error_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&error_box)
        .build();

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.add_css_class("bridge-sidebar");
    sidebar.set_width_request(216);

    let sidebar_heading = gtk::Box::new(gtk::Orientation::Vertical, 5);
    sidebar_heading.add_css_class("sidebar-heading");
    let heading_line = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let sidebar_title = gtk::Label::builder()
        .label("Folder")
        .xalign(0.0)
        .hexpand(true)
        .build();
    sidebar_title.add_css_class("sidebar-title");
    let folder_sort_button = sort_button(
        "win.folder-sort",
        "Sort Folders",
        &[
            SortField {
                label: "Name",
                directions: [
                    (FolderSort::NameAsc.key(), "Ascending (A–Z)"),
                    (FolderSort::NameDesc.key(), "Descending (Z–A)"),
                ],
            },
            SortField {
                label: "Last received",
                directions: [
                    (FolderSort::Oldest.key(), "Ascending (oldest first)"),
                    (FolderSort::Recent.key(), "Descending (newest first)"),
                ],
            },
        ],
    );

    let folder_search = gtk::SearchEntry::builder()
        .placeholder_text("Filter by name…")
        .width_request(220)
        .build();
    let filter_content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    filter_content.add_css_class("folder-filter");
    filter_content.append(&folder_search);
    filter_content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let mut group: Option<gtk::CheckButton> = None;
    for filter in [
        FolderFilter::All,
        FolderFilter::Attention,
        FolderFilter::Pending,
        FolderFilter::NewFiles,
    ] {
        let choice = gtk::CheckButton::with_label(filter.label());
        choice.set_group(group.as_ref());
        choice.set_action_name(Some("win.folder-filter"));
        choice.set_action_target_value(Some(&filter.key().to_variant()));
        if group.is_none() {
            group = Some(choice.clone());
        }
        filter_content.append(&choice);
    }
    let clear_filters = gtk::Button::with_label("Clear Filters");
    clear_filters.set_action_name(Some("win.clear-folder-filters"));
    clear_filters.add_css_class("flat");
    filter_content.append(&clear_filters);
    let filter_popover = gtk::Popover::builder().child(&filter_content).build();
    let folder_filter_button = gtk::MenuButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Filter Folders")
        .popover(&filter_popover)
        .build();
    folder_filter_button.add_css_class("flat");
    let add_bridge_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Folder")
        .build();
    add_bridge_button.add_css_class("flat");
    heading_line.append(&sidebar_title);
    heading_line.append(&folder_sort_button);
    heading_line.append(&folder_filter_button);
    heading_line.append(&add_bridge_button);
    sidebar_heading.append(&heading_line);
    sidebar.append(&sidebar_heading);

    let bridge_list = gtk::ListBox::new();
    bridge_list.add_css_class("bridge-list");
    bridge_list.set_selection_mode(gtk::SelectionMode::Single);
    let bridge_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&bridge_list)
        .build();
    let no_results = gtk::Box::new(gtk::Orientation::Vertical, 10);
    no_results.add_css_class("sidebar-no-results");
    let no_results_label = gtk::Label::new(Some("No matching Folders"));
    no_results_label.add_css_class("secondary-text");
    no_results.append(&no_results_label);
    let clear_filters = gtk::Button::with_label("Clear Filters");
    clear_filters.add_css_class("flat");
    clear_filters.set_action_name(Some("win.clear-folder-filters"));
    no_results.append(&clear_filters);
    let sidebar_results = gtk::Stack::builder().vexpand(true).build();
    sidebar_results.add_named(&bridge_scroll, Some("list"));
    sidebar_results.add_named(&no_results, Some("empty"));
    sidebar.append(&sidebar_results);

    let empty_action = gtk::Button::with_label("Add Folder");
    empty_action.add_css_class("suggested-action");
    empty_action.set_halign(gtk::Align::Center);
    let empty_page = adw::StatusPage::builder()
        .icon_name("folder-new-symbolic")
        .title("No folders yet")
        .description("Add a folder and connect it to your MiRelay server.")
        .child(&empty_action)
        .build();
    empty_page.set_vexpand(true);

    let bridge_icon = gtk::Image::from_icon_name("folder-symbolic");
    bridge_icon.set_pixel_size(24);
    bridge_icon.add_css_class("bridge-hero-icon");
    bridge_icon.set_valign(gtk::Align::Start);
    let bridge_name = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    bridge_name.add_css_class("bridge-title");
    let bridge_path = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .hexpand(true)
        .build();
    bridge_path.add_css_class("secondary-text");
    let bridge_titles = gtk::Box::new(gtk::Orientation::Vertical, 3);
    bridge_titles.set_hexpand(true);
    let folder_category = gtk::DropDown::from_strings(&["General", "Photos"]);
    folder_category.set_halign(gtk::Align::Start);
    folder_category.set_widget_name("folder-category");
    folder_category.set_tooltip_text(Some(
        "Folder type · changes the view, not which files are received",
    ));
    folder_category.update_property(&[gtk::accessible::Property::Label("Folder type")]);
    bridge_titles.append(&folder_category);
    bridge_titles.append(&bridge_name);
    bridge_titles.append(&bridge_path);
    let open_library_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Open folder")
        .sensitive(false)
        .valign(gtk::Align::Center)
        .build();
    let bridge_heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bridge_heading.append(&bridge_icon);
    bridge_heading.append(&bridge_titles);
    bridge_heading.append(&open_library_button);

    let bridge_state_icon = gtk::Image::from_icon_name("network-transmit-receive-symbolic");
    bridge_state_icon.set_pixel_size(16);
    bridge_state_icon.add_css_class("state-success");
    let bridge_state = gtk::Label::builder().label("Ready").xalign(0.0).build();
    bridge_state.add_css_class("bridge-status");
    let summary_label = gtk::Label::builder()
        .label("Loading…")
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    summary_label.add_css_class("secondary-text");
    let state_line = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    state_line.append(&bridge_state_icon);
    state_line.append(&bridge_state);
    state_line.append(&gtk::Label::new(Some("·")));
    state_line.append(&summary_label);

    let (source_property, source_value) = property_row("Source");
    let (folder_property, folder_value) = property_row("Folder Path");
    folder_value.set_selectable(true);
    folder_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let (device_property, device_value) = property_row("Device ID");
    device_value.set_selectable(true);
    device_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let (limit_property, size_limit_value) = property_row("File Size Limit");
    let (auto_receive_property, auto_receive_value) = property_row("Automatic Receive");
    let (wallpaper_property, wallpaper_value) = property_row("Wallpaper Workflow");
    let property_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    property_group.add_css_class("property-group");
    property_group.append(&source_property);
    property_group.append(&folder_property);
    property_group.append(&device_property);
    property_group.append(&limit_property);
    property_group.append(&auto_receive_property);
    property_group.append(&wallpaper_property);

    let activity_title = gtk::Label::builder()
        .label("Files")
        .xalign(0.0)
        .hexpand(true)
        .build();
    activity_title.add_css_class("activity-title");
    let activity_header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    activity_header.add_css_class("activity-header");
    activity_header.append(&activity_title);

    let file_sort_button = sort_button(
        "win.file-sort",
        "Sort Files · active transfers always first",
        &[
            SortField {
                label: "Name",
                directions: [
                    (FileSort::NameAsc.key(), "Ascending (A–Z)"),
                    (FileSort::NameDesc.key(), "Descending (Z–A)"),
                ],
            },
            SortField {
                label: "Date received",
                directions: [
                    (FileSort::Oldest.key(), "Ascending (oldest first)"),
                    (FileSort::Newest.key(), "Descending (newest first)"),
                ],
            },
            SortField {
                label: "Size",
                directions: [
                    (FileSort::Smallest.key(), "Ascending (smallest first)"),
                    (FileSort::Largest.key(), "Descending (largest first)"),
                ],
            },
        ],
    );
    let file_search = gtk::SearchEntry::builder()
        .placeholder_text("Filter files by name…")
        .width_request(230)
        .build();
    let file_filter_content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    file_filter_content.add_css_class("folder-filter");
    file_filter_content.append(&file_search);
    let file_kind = gtk::DropDown::from_strings(&FILE_KINDS.map(|kind| kind.label()));
    file_kind.set_tooltip_text(Some("File type"));
    file_kind.update_property(&[gtk::accessible::Property::Label("File type")]);
    file_filter_content.append(&file_kind);
    file_filter_content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let mut group: Option<gtk::CheckButton> = None;
    for filter in [
        FileFilter::All,
        FileFilter::InProgress,
        FileFilter::Attention,
        FileFilter::Completed,
    ] {
        let choice = gtk::CheckButton::with_label(filter.label());
        choice.set_group(group.as_ref());
        choice.set_action_name(Some("win.file-filter"));
        choice.set_action_target_value(Some(&filter.key().to_variant()));
        if group.is_none() {
            group = Some(choice.clone());
        }
        file_filter_content.append(&choice);
    }
    let clear_files = gtk::Button::with_label("Clear Filters");
    clear_files.add_css_class("flat");
    clear_files.set_action_name(Some("win.clear-file-filters"));
    file_filter_content.append(&clear_files);
    let file_filter_button = gtk::MenuButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Filter Files")
        .popover(&gtk::Popover::builder().child(&file_filter_content).build())
        .build();
    file_filter_button.add_css_class("flat");
    activity_header.append(&file_sort_button);
    activity_header.append(&file_filter_button);

    let delivery_list = gtk::ListBox::new();
    delivery_list.add_css_class("activity-list");
    delivery_list.set_selection_mode(gtk::SelectionMode::None);
    let photo_grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(8)
        .row_spacing(8)
        .build();
    photo_grid.set_widget_name("photo-grid");
    let files_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    files_content.append(&delivery_list);
    files_content.append(&photo_grid);
    let activity_empty = gtk::Label::builder()
        .label("Received files will appear here.")
        .xalign(0.0)
        .wrap(true)
        .build();
    activity_empty.add_css_class("secondary-text");
    let activity_empty_title = gtk::Label::builder()
        .label("No transfers yet")
        .xalign(0.0)
        .build();
    activity_empty_title.add_css_class("activity-title");
    let activity_empty_text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    activity_empty_text.set_hexpand(true);
    activity_empty_text.append(&activity_empty_title);
    activity_empty_text.append(&activity_empty);
    let activity_empty_icon = gtk::Image::from_icon_name("folder-download-symbolic");
    activity_empty_icon.set_pixel_size(24);
    activity_empty_icon.add_css_class("secondary-text");
    let activity_empty_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    activity_empty_box.add_css_class("activity-empty");
    activity_empty_box.append(&activity_empty_icon);
    activity_empty_box.append(&activity_empty_text);
    let activity_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vhomogeneous(false)
        .build();
    activity_stack.add_named(&activity_empty_box, Some("empty"));
    activity_stack.add_named(&files_content, Some("list"));
    let file_no_results = gtk::Box::new(gtk::Orientation::Vertical, 8);
    file_no_results.add_css_class("activity-empty");
    file_no_results.append(&gtk::Label::new(Some("No matching files")));
    let clear_files = gtk::Button::with_label("Clear Filters");
    clear_files.add_css_class("flat");
    clear_files.set_action_name(Some("win.clear-file-filters"));
    file_no_results.append(&clear_files);
    activity_stack.add_named(&file_no_results, Some("no-results"));
    activity_stack.set_visible_child_name("empty");
    let file_show_more = gtk::Button::with_label("Show More");
    file_show_more.add_css_class("flat");
    file_show_more.set_action_name(Some("win.more-files"));
    file_show_more.set_visible(false);

    let bridge_page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    bridge_page.add_css_class("bridge-page");
    bridge_page.append(&bridge_heading);
    bridge_page.append(&state_line);
    bridge_page.append(&property_group);
    bridge_page.append(&activity_header);
    bridge_page.append(&activity_stack);
    bridge_page.append(&file_show_more);

    let bridge_clamp = adw::Clamp::new();
    bridge_clamp.set_maximum_size(860);
    bridge_clamp.set_tightening_threshold(600);
    bridge_clamp.set_child(Some(&bridge_page));
    let bridge_detail_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&bridge_clamp)
        .build();

    let bridge_error_action = gtk::Button::with_label("Manage Folder");
    bridge_error_action.add_css_class("suggested-action");
    bridge_error_action.set_halign(gtk::Align::Center);
    let bridge_error_page = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title("Folder needs attention")
        .description("This Folder cannot be loaded. Other folders remain available.")
        .child(&bridge_error_action)
        .build();
    bridge_error_page.set_vexpand(true);

    let content_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .vexpand(true)
        .build();
    content_stack.add_named(&empty_page, Some("empty"));
    content_stack.add_named(&bridge_detail_scroll, Some("bridge"));
    content_stack.add_named(&bridge_error_page, Some("bridge-error"));
    content_stack.set_visible_child_name("empty");

    let workspace = gtk::Paned::new(gtk::Orientation::Horizontal);
    workspace.add_css_class("workspace");
    workspace.set_start_child(Some(&sidebar));
    workspace.set_end_child(Some(&content_stack));
    workspace.set_position(232);
    workspace.set_resize_start_child(false);
    workspace.set_shrink_start_child(false);
    workspace.set_shrink_end_child(false);
    workspace.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&error_revealer);
    root.append(&workspace);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&root));
    window.set_content(Some(&toast_overlay));

    (
        Widgets {
            window,
            title,
            refresh_button,
            sync_button,
            sync_indicator,
            sync_label,
            add_bridge_button,
            settings_button,
            open_library_button,
            error_revealer,
            error_label,
            toast_overlay,
            bridge_list,
            sidebar_results,
            folder_sort_button,
            folder_filter_button,
            folder_search,
            summary_label,
            content_stack,
            empty_page,
            empty_action,
            bridge_error_page,
            bridge_error_action,
            bridge_name,
            folder_category,
            bridge_icon,
            bridge_path,
            bridge_state_icon,
            bridge_state,
            source_value,
            folder_value,
            device_value,
            size_limit_value,
            auto_receive_value,
            wallpaper_value,
            activity_stack,
            activity_empty,
            delivery_list,
            photo_grid,
            file_sort_button,
            file_filter_button,
            file_search,
            file_kind,
            file_show_more,
        },
        error_close,
    )
}

// Run only through --sidebar-smoke-test with a disposable registry (no network work).
fn sidebar_smoke_checks(ui: &DesktopUi) -> Result<()> {
    use anyhow::ensure;
    let activate = |name: &str, target: Option<&str>| {
        ui.widgets
            .window
            .lookup_action(name)
            .unwrap()
            .activate(target.map(|value| value.to_variant()).as_ref());
    };
    let original = ui.selected_bridge_id.borrow().clone();
    let original_registry = ui.paths.registry.load()?;
    let original_views = ui.bridges.borrow().clone();
    ensure!(
        ui.bridges.borrow().len() >= 2,
        "provide at least two preview Folders"
    );
    ensure!(
        ui.unread_folders.borrow().is_empty(),
        "initial records should not be marked new"
    );
    file_smoke::check_sort_controls(
        &ui.widgets.folder_sort_button,
        "win.folder-sort",
        &[
            ("Name", ["name-asc", "name-desc"]),
            ("Last received", ["oldest", "recent"]),
        ],
    )?;
    file_smoke::check_sort_focus(&ui.widgets.folder_sort_button, "name-asc")?;
    let ascending = ui.bridge_ids.borrow().clone();
    file_smoke::click_sort_direction(&ui.widgets.folder_sort_button, "name-desc")?;
    ensure!(
        ui.bridge_ids.borrow().iter().eq(ascending.iter().rev()),
        "descending sort did not reorder the GTK list"
    );
    for sort in ["oldest", "recent", "name-asc", "name-desc"] {
        file_smoke::click_sort_direction(&ui.widgets.folder_sort_button, sort)?;
        ensure!(
            ui.folder_sort.get().key() == sort,
            "sort triangle did not update the Folder model"
        );
        ensure!(
            ui.selected_bridge_id.borrow().as_ref() == original.as_ref(),
            "sort triangle changed the current Folder"
        );
    }
    activate("folder-sort", Some("recent"));
    file_smoke::check_sort_focus(&ui.widgets.folder_sort_button, "recent")?;
    ensure!(
        ui.selected_bridge_id.borrow().as_ref() == original.as_ref(),
        "sorting changed selection"
    );

    ui.widgets
        .folder_search
        .set_text("no-such-folder-for-smoke-check");
    ensure!(
        ui.bridge_ids.borrow().is_empty(),
        "name filter did not apply"
    );
    ensure!(
        ui.widgets.sidebar_results.visible_child_name().as_deref() == Some("empty"),
        "missing no-results state"
    );
    ensure!(
        ui.selected_bridge_id.borrow().as_ref() == original.as_ref(),
        "filtering hid the current detail"
    );
    ensure!(
        ui.paths.registry.load()? == original_registry,
        "view controls wrote to registry"
    );
    activate("clear-folder-filters", None);
    for filter in ["attention", "pending", "new-files"] {
        activate("folder-filter", Some(filter));
        ensure!(
            ui.selected_bridge_id.borrow().as_ref() == original.as_ref(),
            "status filtering changed selection"
        );
    }
    ensure!(
        ui.bridge_ids.borrow().is_empty(),
        "no unread folders expected initially"
    );
    activate("clear-folder-filters", None);

    // Simulate a new delivery on a non-selected Folder without touching transfer state.
    let target = ui
        .bridges
        .borrow()
        .iter()
        .find(|view| {
            Some(&view.registration.id) != original.as_ref()
                && view
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| !snapshot.deliveries.is_empty())
        })
        .cloned()
        .context("preview needs a non-selected Folder with a sample record")?;
    let mut next = target.snapshot.clone().unwrap();
    let mut record = next.deliveries[0].clone();
    record.id = "sidebar-smoke-new-delivery".into();
    Arc::make_mut(&mut next.deliveries).push(record);
    ui.update_bridge_snapshot(&target.registration.id, next);
    ensure!(
        ui.unread_folders.borrow().contains(&target.registration.id),
        "new delivery was not marked unread"
    );
    activate("folder-filter", Some("new-files"));
    ensure!(
        *ui.bridge_ids.borrow() == vec![target.registration.id.clone()],
        "unread filter did not find updated Folder"
    );
    // Use a real GTK selection event, not just a direct call to the handler.
    ui.widgets
        .bridge_list
        .select_row(ui.widgets.bridge_list.row_at_index(0).as_ref());
    ensure!(
        ui.selected_bridge_id.borrow().as_deref() == Some(&target.registration.id),
        "filtered row opened wrong Folder"
    );
    ensure!(
        !ui.unread_folders.borrow().contains(&target.registration.id),
        "opening Folder did not clear unread notice"
    );
    ensure!(
        ui.bridge_ids.borrow().is_empty(),
        "read Folder stayed in new-files filter"
    );
    ui.update_bridge_snapshot(&target.registration.id, target.snapshot.unwrap());
    activate("clear-folder-filters", None);
    activate("folder-sort", Some("name-asc"));
    let mut refreshed = original_views.clone();
    if let Some(snapshot) = refreshed
        .iter_mut()
        .find(|view| Some(&view.registration.id) == original.as_ref())
        .and_then(|view| view.snapshot.as_mut())
        && let Some(mut record) = snapshot.deliveries.first().cloned()
    {
        record.id = "sidebar-smoke-refresh-delivery".into();
        Arc::make_mut(&mut snapshot.deliveries).push(record);
    }
    ui.handle_worker_message(WorkerMessage::Loaded(Ok(RegistrySnapshot {
        bridges: refreshed,
        selected_bridge_id: original.clone(),
    })));
    ensure!(
        ui.unread_folders.borrow().is_empty(),
        "registry-selected Folder retained its unread marker"
    );
    // Restore the model, then exercise real selection to restore the on-disk registry.
    ui.bridges.replace(original_views);
    ui.selected_bridge_id.replace(None);
    if let Some(original) = original {
        let index = ui
            .bridge_ids
            .borrow()
            .iter()
            .position(|id| id == &original)
            .unwrap();
        ui.select_bridge_at(index as i32);
    }
    ensure!(
        ui.paths.registry.load()? == original_registry,
        "test did not restore selection"
    );
    Ok(())
}

fn install_file_actions(ui: &Rc<DesktopUi>) {
    let sort = gio::SimpleAction::new_stateful(
        "file-sort",
        Some(glib::VariantTy::STRING),
        &FileSort::default().key().to_variant(),
    );
    {
        let ui = Rc::clone(ui);
        sort.connect_activate(move |_, value| {
            if let Some(sort) = value
                .and_then(|value| value.str())
                .and_then(FileSort::from_key)
            {
                ui.change_file_view(true, |view| view.sort = sort);
            }
        });
    }
    ui.widgets.window.add_action(&sort);
    let filter = gio::SimpleAction::new_stateful(
        "file-filter",
        Some(glib::VariantTy::STRING),
        &FileFilter::default().key().to_variant(),
    );
    {
        let ui = Rc::clone(ui);
        filter.connect_activate(move |_, value| {
            if let Some(filter) = value
                .and_then(|value| value.str())
                .and_then(FileFilter::from_key)
            {
                ui.change_file_view(true, |view| view.filter = filter);
            }
        });
    }
    ui.widgets.window.add_action(&filter);
    let kind = gio::SimpleAction::new_stateful(
        "file-kind",
        Some(glib::VariantTy::STRING),
        &FileKind::default().key().to_variant(),
    );
    {
        let ui = Rc::clone(ui);
        kind.connect_activate(move |_, value| {
            if let Some(kind) = value
                .and_then(|value| value.str())
                .and_then(FileKind::from_key)
            {
                ui.change_file_view(true, |view| view.kind = kind);
            }
        });
    }
    ui.widgets.window.add_action(&kind);
    {
        let ui = Rc::clone(ui);
        let dropdown = ui.widgets.file_kind.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            if !ui.rendering_file_controls.get()
                && let Some(kind) = FILE_KINDS.get(dropdown.selected() as usize)
            {
                ui.widgets
                    .window
                    .lookup_action("file-kind")
                    .unwrap()
                    .activate(Some(&kind.key().to_variant()));
            }
        });
    }
    {
        let ui = Rc::clone(ui);
        let search = ui.widgets.file_search.clone();
        search.connect_changed(move |search| {
            if !ui.rendering_file_controls.get() {
                ui.change_file_view(true, |view| view.query = search.text().to_string());
            }
        });
    }
    let clear = gio::SimpleAction::new("clear-file-filters", None);
    {
        let ui = Rc::clone(ui);
        clear.connect_activate(move |_, _| {
            ui.change_file_view(true, |view| {
                view.query.clear();
                view.filter = FileFilter::All;
                view.kind = FileKind::All;
            })
        });
    }
    ui.widgets.window.add_action(&clear);
    let more = gio::SimpleAction::new("more-files", None);
    {
        let ui = Rc::clone(ui);
        more.connect_activate(move |_, _| {
            ui.change_file_view(false, |view| {
                view.limit = view.limit.saturating_add(FILE_PAGE_SIZE)
            })
        });
    }
    ui.widgets.window.add_action(&more);
}

fn install_folder_actions(ui: &Rc<DesktopUi>, application: &adw::Application) {
    let sort_action = gio::SimpleAction::new_stateful(
        "folder-sort",
        Some(glib::VariantTy::STRING),
        &FolderSort::default().key().to_variant(),
    );
    {
        let ui = Rc::clone(ui);
        sort_action.connect_activate(move |action, parameter| {
            let Some(sort) = parameter
                .and_then(|value| value.str())
                .and_then(FolderSort::from_key)
            else {
                return;
            };
            action.set_state(&sort.key().to_variant());
            ui.folder_sort.set(sort);
            ui.render_bridge_list();
        });
    }
    ui.widgets.window.add_action(&sort_action);

    let filter_action = gio::SimpleAction::new_stateful(
        "folder-filter",
        Some(glib::VariantTy::STRING),
        &FolderFilter::default().key().to_variant(),
    );
    {
        let ui = Rc::clone(ui);
        filter_action.connect_activate(move |action, parameter| {
            let Some(filter) = parameter
                .and_then(|value| value.str())
                .and_then(FolderFilter::from_key)
            else {
                return;
            };
            action.set_state(&filter.key().to_variant());
            ui.folder_filter.set(filter);
            ui.render_bridge_list();
        });
    }
    ui.widgets.window.add_action(&filter_action);
    let clear_action = gio::SimpleAction::new("clear-folder-filters", None);
    {
        let ui = Rc::clone(ui);
        clear_action.connect_activate(move |_, _| {
            ui.widgets.folder_search.set_text("");
            filter_action.activate(Some(&FolderFilter::All.key().to_variant()));
        });
    }
    ui.widgets.window.add_action(&clear_action);
    {
        let ui = Rc::clone(ui);
        let search = ui.widgets.folder_search.clone();
        search.connect_changed(move |_| ui.render_bridge_list());
    }
    let search_action = gio::SimpleAction::new("find-folder", None);
    {
        let ui = Rc::clone(ui);
        search_action.connect_activate(move |_, _| {
            ui.widgets.folder_filter_button.popup();
            ui.widgets.folder_search.grab_focus();
        });
    }
    ui.widgets.window.add_action(&search_action);
    application.set_accels_for_action("win.find-folder", &["<Primary>f"]);
}

impl DesktopUi {
    fn start_load_all(&self) {
        if !self.begin_task("Loading…") {
            return;
        }
        let paths = self.paths.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = load_registry_snapshot(&paths).map_err(|error| format!("{error:#}"));
            let _ = sender.send(WorkerMessage::Loaded(result));
        });
    }

    fn start_refresh(&self) {
        self.start_load_all();
    }

    fn start_sync(&self) {
        let Some(bridge) = self.selected_bridge() else {
            self.show_error("Add a Folder first.");
            return;
        };
        if let Some(error) = &bridge.error {
            self.show_error(error);
            return;
        }
        let request = self.sync_request(&bridge);
        self.start_sync_requests(vec![request], false, "Receiving…");
    }

    fn start_auto_sync(&self) {
        if self.busy.get() || self.editor_open.get() {
            return;
        }
        let tokens = self.tokens.borrow();
        let requests = automatic_sync_requests(&self.bridges.borrow(), &tokens);
        drop(tokens);
        if requests.is_empty() {
            return;
        }
        self.start_sync_requests(requests, true, "Checking…");
    }

    fn sync_request(&self, bridge: &BridgeView) -> SyncRequest {
        SyncRequest {
            bridge_id: bridge.registration.id.clone(),
            bridge_name: bridge.registration.name.clone(),
            config_path: bridge.registration.config_path.clone(),
            token: self.tokens.borrow().get(&bridge.registration.id).cloned(),
        }
    }

    fn start_sync_requests(&self, requests: Vec<SyncRequest>, automatic: bool, label: &str) {
        if !self.begin_task(label) {
            return;
        }
        let registry = self.paths.registry.clone();
        let sender = self.sender.clone();
        for request in &requests {
            self.file_activity.borrow_mut().remove(&request.bridge_id);
        }
        std::thread::spawn(move || {
            let outcomes =
                execute_sync_requests_with_events(&registry, requests, &|bridge_id, event| {
                    let _ = sender.send(WorkerMessage::FileProgress {
                        bridge_id: bridge_id.to_owned(),
                        event,
                    });
                });
            let _ = sender.send(WorkerMessage::Synced {
                automatic,
                outcomes,
            });
        });
    }

    fn begin_task(&self, label: &str) -> bool {
        if self.busy.replace(true) {
            return false;
        }
        self.widgets.refresh_button.set_sensitive(false);
        self.widgets.sync_button.set_sensitive(false);
        self.widgets.add_bridge_button.set_sensitive(false);
        self.widgets.settings_button.set_sensitive(false);
        self.widgets.folder_category.set_sensitive(false);
        self.widgets.sync_label.set_label(label);
        self.widgets
            .sync_indicator
            .set_visible_child_name("spinner");
        true
    }

    fn finish_task(&self) {
        self.busy.set(false);
        self.widgets.sync_label.set_label("Receive");
        self.widgets.sync_indicator.set_visible_child_name("icon");
        self.update_action_sensitivity();
    }

    fn handle_worker_message(&self, message: WorkerMessage) {
        if let WorkerMessage::FileProgress { bridge_id, event } = message {
            self.handle_file_progress(&bridge_id, event);
            return;
        }
        self.finish_task();
        match message {
            WorkerMessage::FileProgress { .. } => {
                unreachable!("progress handled without completing task")
            }
            WorkerMessage::Loaded(Ok(snapshot)) => {
                // Initial load is the baseline, not a wave of "new file" notifications.
                for next in &snapshot.bridges {
                    if let Some(previous) = self
                        .bridges
                        .borrow()
                        .iter()
                        .find(|view| view.registration.id == next.registration.id)
                        && let (Some(previous), Some(next_snapshot)) =
                            (&previous.snapshot, &next.snapshot)
                    {
                        self.note_new_deliveries(&next.registration.id, previous, next_snapshot);
                    }
                }
                let retained: HashSet<_> = snapshot
                    .bridges
                    .iter()
                    .map(|view| view.registration.id.clone())
                    .collect();
                self.unread_folders
                    .borrow_mut()
                    .retain(|id| retained.contains(id));
                self.sync_failed_folders
                    .borrow_mut()
                    .retain(|id| retained.contains(id));
                self.file_views
                    .borrow_mut()
                    .retain(|id, _| retained.contains(id));
                // Refresh establishes the persisted truth and dismisses transient failures.
                self.file_activity.borrow_mut().clear();
                self.bridges.replace(snapshot.bridges);
                self.selected_bridge_id.replace(snapshot.selected_bridge_id);
                self.render_bridge_list();
                self.hide_error();
            }
            WorkerMessage::Loaded(Err(error)) => {
                if self.bridges.borrow().is_empty() {
                    self.render_empty_state();
                }
                self.show_error(&error);
            }
            WorkerMessage::Synced {
                automatic,
                outcomes,
            } => self.handle_sync_outcomes(automatic, outcomes),
        }
        self.update_action_sensitivity();
    }

    fn handle_sync_outcomes(&self, automatic: bool, outcomes: Vec<SyncOutcome>) {
        let checked = outcomes.len();
        let mut received = 0;
        let mut acknowledged = 0;
        let mut failures = Vec::new();

        for outcome in outcomes {
            // Fatal errors can exit before a matching FileSettled event. Never leave a spinner running.
            if let Some(activity) = self.file_activity.borrow_mut().get_mut(&outcome.bridge_id) {
                for file in activity.values_mut().filter(|file| file.phase.is_some()) {
                    file.phase = None;
                    file.error = Some(
                        outcome
                            .result
                            .as_ref()
                            .err()
                            .cloned()
                            .or_else(|| {
                                outcome.result.as_ref().ok().and_then(|result| {
                                    result
                                        .summary
                                        .failures
                                        .first()
                                        .map(|failure| failure.message.clone())
                                })
                            })
                            .unwrap_or_else(|| {
                                "Transfer ended without a final file status. Refresh to check."
                                    .into()
                            }),
                    );
                }
            }
            match outcome.result {
                Ok(result) => {
                    if result.summary.failures.is_empty() {
                        self.sync_failed_folders
                            .borrow_mut()
                            .remove(&outcome.bridge_id);
                    } else {
                        self.sync_failed_folders
                            .borrow_mut()
                            .insert(outcome.bridge_id.clone());
                    }
                    received += result.summary.received;
                    acknowledged += result.summary.acknowledged;
                    failures.extend(result.summary.failures.into_iter().map(|failure| {
                        format!(
                            "{} · {}: {}",
                            outcome.bridge_name, failure.item, failure.message
                        )
                    }));
                    self.update_bridge_snapshot(&outcome.bridge_id, result.snapshot);
                }
                Err(error) => {
                    self.sync_failed_folders
                        .borrow_mut()
                        .insert(outcome.bridge_id);
                    failures.push(format!("{}: {error}", outcome.bridge_name));
                }
            }
        }
        self.render_bridge_list();

        if !automatic || received > 0 || acknowledged > 0 {
            let message = if automatic {
                let folder_word = if checked == 1 { "Folder" } else { "Folders" };
                format!(
                    "Auto Receive: {received} new, {acknowledged} acknowledged across {checked} {folder_word}"
                )
            } else {
                format!("Received: {received} new, {acknowledged} acknowledged")
            };
            self.widgets
                .toast_overlay
                .add_toast(adw::Toast::new(&message));
        }

        if failures.is_empty() {
            self.hide_error();
        } else {
            let total = failures.len();
            let details = failures.into_iter().take(8).collect::<Vec<_>>().join("\n");
            self.show_error(&format!(
                "{} completed with {total} issue(s):\n{details}",
                if automatic { "Auto Receive" } else { "Receive" }
            ));
        }
    }

    fn selected_bridge(&self) -> Option<BridgeView> {
        let selected = self.selected_bridge_id.borrow();
        self.bridges
            .borrow()
            .iter()
            .find(|bridge| Some(bridge.registration.id.as_str()) == selected.as_deref())
            .cloned()
    }

    fn show_selected_bridge_editor(self: &Rc<Self>) {
        match self.selected_bridge_id.borrow().clone() {
            Some(id) => show_bridge_editor(self, BridgeEditorMode::Edit(id)),
            None => show_bridge_editor(self, BridgeEditorMode::New),
        };
    }

    fn select_bridge_at(&self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some(bridge_id) = self.bridge_ids.borrow().get(index).cloned() else {
            return;
        };
        if self.selected_bridge_id.borrow().as_deref() == Some(&bridge_id) {
            self.render_current_bridge();
            return;
        }
        match self
            .paths
            .registry
            .update(|registry| registry.select(&bridge_id))
        {
            Ok(()) => {
                let was_unread = self.unread_folders.borrow_mut().remove(&bridge_id);
                self.selected_bridge_id.replace(Some(bridge_id));
                // Preserve row identity/focus during ordinary keyboard navigation.
                if was_unread {
                    self.render_bridge_list();
                    if let Some(row) = self.widgets.bridge_list.selected_row() {
                        row.grab_focus();
                    }
                } else {
                    self.render_current_bridge();
                }
                self.hide_error();
            }
            Err(error) => {
                self.show_error(&format!("Could not save Folder selection: {error:#}"));
                self.select_current_row();
            }
        }
    }

    fn update_bridge_snapshot(&self, bridge_id: &str, mut snapshot: BridgeSnapshot) {
        // Settled events update shared list records before the final directory
        // snapshot brings its retained-copy metadata. Rebuild visible rows once.
        if snapshot.directory.is_some()
            && self.selected_bridge_id.borrow().as_deref() == Some(bridge_id)
        {
            self.rendered_files.borrow_mut().clear();
        }
        if let Some(bridge) = self
            .bridges
            .borrow_mut()
            .iter_mut()
            .find(|bridge| bridge.registration.id == bridge_id)
        {
            if let Some(previous) = &bridge.snapshot {
                self.note_new_deliveries(bridge_id, previous, &snapshot);
            }
            snapshot.name.clone_from(&bridge.registration.name);
            bridge.snapshot = Some(snapshot);
            bridge.error = None;
        }
    }

    fn note_new_deliveries(
        &self,
        bridge_id: &str,
        previous: &BridgeSnapshot,
        next: &BridgeSnapshot,
    ) {
        if self.selected_bridge_id.borrow().as_deref() != Some(bridge_id)
            && has_new_deliveries(previous, next)
        {
            self.unread_folders
                .borrow_mut()
                .insert(bridge_id.to_owned());
        }
    }

    fn render_bridge_list(&self) {
        // Rebuilding or reselecting rows emits GTK selection signals synchronously.
        if self.rendering_list.replace(true) {
            return;
        }
        if self.selected_bridge().is_none() {
            let fallback = self
                .bridges
                .borrow()
                .first()
                .map(|view| view.registration.id.clone());
            self.selected_bridge_id.replace(fallback);
        }
        // Also acknowledge a Folder selected by a registry refresh or removal fallback.
        if let Some(selected) = self.selected_bridge_id.borrow().as_ref() {
            self.unread_folders.borrow_mut().remove(selected);
        }
        clear_list_box(&self.widgets.bridge_list);
        let bridges = self.bridges.borrow();
        let query = self.widgets.folder_search.text();
        let unread = self.unread_folders.borrow();
        let failed = self.sync_failed_folders.borrow();
        let ids = visible_folder_ids(
            &bridges,
            self.folder_sort.get(),
            self.folder_filter.get(),
            &query,
            &unread,
            &failed,
        );
        let no_matches = !bridges.is_empty() && ids.is_empty();
        for id in &ids {
            if let Some(bridge) = bridges.iter().find(|view| &view.registration.id == id) {
                self.widgets.bridge_list.append(&bridge_row(
                    bridge,
                    unread.contains(id),
                    failed.contains(id),
                ));
            }
        }
        self.bridge_ids.replace(ids);
        drop(unread);
        drop(failed);
        drop(bridges);
        self.widgets
            .sidebar_results
            .set_visible_child_name(if no_matches { "empty" } else { "list" });
        let active = self.folder_filter.get() != FolderFilter::All || !query.trim().is_empty();
        if active {
            self.widgets
                .folder_filter_button
                .add_css_class("suggested-action");
        } else {
            self.widgets
                .folder_filter_button
                .remove_css_class("suggested-action");
        }
        self.widgets
            .folder_filter_button
            .set_tooltip_text(Some(&format!(
                "Filter Folders · {}{}",
                self.folder_filter.get().label(),
                if query.trim().is_empty() {
                    String::new()
                } else {
                    format!(" · {}", query.trim())
                }
            )));
        self.widgets
            .folder_sort_button
            .set_tooltip_text(Some(&format!(
                "Sort Folders · {}",
                self.folder_sort.get().label()
            )));
        if self.bridges.borrow().is_empty() {
            self.selected_bridge_id.replace(None);
            self.rendering_list.set(false);
            self.render_empty_state();
            return;
        }
        self.select_current_row();
        self.rendering_list.set(false);
        self.render_current_bridge();
    }

    fn select_current_row(&self) {
        let selected = self.selected_bridge_id.borrow();
        let Some(index) = self
            .bridge_ids
            .borrow()
            .iter()
            .position(|id| Some(id.as_str()) == selected.as_deref())
        else {
            return;
        };
        drop(selected);
        if let Ok(index) = i32::try_from(index)
            && let Some(row) = self.widgets.bridge_list.row_at_index(index)
        {
            self.widgets.bridge_list.select_row(Some(&row));
        }
    }

    fn render_empty_state(&self) {
        self.widgets.title.set_subtitle("Folder");
        clear_list_box(&self.widgets.delivery_list);
        self.rendered_files.borrow_mut().clear();
        self.widgets.empty_page.set_title("No folders yet");
        self.widgets
            .empty_page
            .set_description(Some("Add a folder and connect it to your MiRelay server."));
        self.widgets
            .empty_page
            .set_icon_name(Some("folder-new-symbolic"));
        self.widgets.empty_action.set_label("Add Folder");
        self.widgets.empty_action.set_visible(true);
        self.widgets.content_stack.set_visible_child_name("empty");
        self.widgets.activity_stack.set_visible_child_name("empty");
        self.update_action_sensitivity();
    }

    fn render_current_bridge(&self) {
        let Some(bridge) = self.selected_bridge() else {
            self.render_empty_state();
            return;
        };
        if let Some(error) = &bridge.error {
            self.widgets
                .bridge_error_page
                .set_title(&format!("{} needs attention", bridge.registration.name));
            self.widgets
                .bridge_error_page
                .set_description(Some(&format!(
                    "Configuration: {}\n\n{}",
                    bridge.registration.config_path.display(),
                    safe_ui_message(error, 1800)
                )));
            self.widgets
                .content_stack
                .set_visible_child_name("bridge-error");
        } else if let Some(snapshot) = &bridge.snapshot {
            self.render_snapshot(snapshot, bridge.registration.auto_receive);
        } else {
            self.widgets
                .bridge_error_page
                .set_title(&format!("{} is not loaded", bridge.registration.name));
            self.widgets
                .bridge_error_page
                .set_description(Some("Refresh this Folder's status."));
            self.widgets
                .content_stack
                .set_visible_child_name("bridge-error");
        }
        self.update_action_sensitivity();
    }

    fn render_snapshot(&self, snapshot: &BridgeSnapshot, auto_receive: bool) {
        let kind = self
            .selected_bridge()
            .map(|view| view.registration.kind)
            .unwrap_or_default();
        self.widgets
            .folder_category
            .set_selected(u32::from(!kind.is_general()));
        self.widgets
            .bridge_icon
            .set_icon_name(Some(kind.icon_name()));
        self.widgets.title.set_subtitle("Folder");
        self.widgets.bridge_name.set_label(&snapshot.name);
        self.widgets
            .bridge_name
            .set_tooltip_text(Some(&snapshot.name));
        let folder = snapshot.library_dir.to_string_lossy();
        self.widgets.bridge_path.set_label(&folder);
        self.widgets.bridge_path.set_tooltip_text(Some(&folder));
        self.widgets.bridge_state.set_label("Ready");
        self.widgets
            .bridge_state_icon
            .set_icon_name(Some("network-transmit-receive-symbolic"));
        self.widgets.source_value.set_label(&snapshot.source_label);
        self.widgets
            .source_value
            .set_tooltip_text(Some(&snapshot.source_label));
        self.widgets.folder_value.set_label(&folder);
        self.widgets.folder_value.set_tooltip_text(Some(&folder));
        self.widgets.device_value.set_label(&snapshot.device_id);
        self.widgets
            .device_value
            .set_tooltip_text(Some(&snapshot.device_id));
        self.widgets
            .size_limit_value
            .set_label(&format_bytes(snapshot.max_file_size_bytes));
        let auto_receive_label = if auto_receive {
            format!("Every {AUTO_RECEIVE_INTERVAL_SECONDS} seconds")
        } else {
            "Off".to_owned()
        };
        self.widgets
            .auto_receive_value
            .set_label(&auto_receive_label);
        self.widgets
            .wallpaper_value
            .set_label(&snapshot.wallpaper_label);
        self.widgets
            .wallpaper_value
            .set_tooltip_text(Some(&snapshot.wallpaper_label));
        self.widgets.summary_label.set_label(
            &snapshot
                .directory
                .as_ref()
                .map(|directory| {
                    let conflicts = directory
                        .files
                        .values()
                        .filter(|file| file.conflict)
                        .count();
                    format!(
                        "Directory sync · {} paths · {} pending · {conflicts} conflicts",
                        snapshot.counts.total, snapshot.counts.ack_pending
                    )
                })
                .unwrap_or_else(|| summary_text(&snapshot.counts)),
        );
        self.widgets
            .summary_label
            .set_tooltip_text(Some(&summary_text(&snapshot.counts)));
        self.render_file_list(&snapshot.deliveries);
        self.widgets.content_stack.set_visible_child_name("bridge");
    }

    fn change_file_view(&self, reset_limit: bool, change: impl FnOnce(&mut FileViewOptions)) {
        let Some(id) = self.selected_bridge_id.borrow().clone() else {
            return;
        };
        {
            let mut views = self.file_views.borrow_mut();
            let view = views.entry(id).or_default();
            if reset_limit {
                view.limit = FILE_PAGE_SIZE;
            }
            change(view);
        }
        if let Some(snapshot) = self.selected_bridge().and_then(|view| view.snapshot) {
            self.render_file_list(&snapshot.deliveries);
        }
    }

    fn render_file_list(&self, records: &FileHistory) {
        self.file_render_pending.set(false);
        let Some(id) = self.selected_bridge_id.borrow().clone() else {
            return;
        };
        let context = (
            id.clone(),
            self.selected_bridge()
                .map(|bridge| bridge.registration.kind)
                .unwrap_or_default(),
        );
        if self.rendered_context.borrow().as_ref() != Some(&context) {
            clear_list_box(&self.widgets.delivery_list);
            while let Some(child) = self.widgets.photo_grid.child_at_index(0) {
                self.widgets.photo_grid.remove(&child);
            }
            self.rendered_files.borrow_mut().clear();
            self.rendered_context.replace(Some(context));
        }
        let view = self
            .file_views
            .borrow()
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let activity = self
            .file_activity
            .borrow()
            .get(&id)
            .map(|files| files.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.widgets.bridge_state.set_label(
            activity
                .iter()
                .find_map(|file| file.phase)
                .map(phase_label)
                .unwrap_or("Ready"),
        );
        let page = records.page(
            &activity,
            view.sort,
            view.filter,
            view.kind,
            &view.query,
            view.limit,
        );
        let directory = self
            .selected_bridge()
            .and_then(|bridge| bridge.snapshot)
            .and_then(|snapshot| {
                snapshot
                    .directory
                    .map(|directory| (snapshot.library_dir, directory))
            });
        if *self.rendered_files.borrow() != page.entries {
            let photos = self
                .selected_bridge()
                .is_some_and(|bridge| !bridge.registration.kind.is_general());
            if photos {
                clear_list_box(&self.widgets.delivery_list);
                let gallery_changed = self
                    .rendered_files
                    .borrow()
                    .iter()
                    .filter(|entry| photos::eligible(entry))
                    .take(photos::MAX_TILES)
                    .ne(page
                        .entries
                        .iter()
                        .filter(|entry| photos::eligible(entry))
                        .take(photos::MAX_TILES));
                if gallery_changed {
                    while let Some(child) = self.widgets.photo_grid.child_at_index(0) {
                        self.widgets.photo_grid.remove(&child);
                    }
                }
                let root = self
                    .selected_bridge()
                    .and_then(|bridge| bridge.snapshot)
                    .map(|snapshot| snapshot.library_dir);
                let mut tiles = 0;
                for entry in &page.entries {
                    if tiles < photos::MAX_TILES
                        && photos::eligible(entry)
                        && let Some(root) = &root
                    {
                        if gallery_changed {
                            self.widgets
                                .photo_grid
                                .insert(&photos::tile(entry, root, &self.widgets.window), -1);
                        }
                        tiles += 1;
                    } else {
                        let file = directory
                            .as_ref()
                            .and_then(|(_, directory)| directory.files.get(entry.name()))
                            .filter(|file| {
                                entry.id() == format!("directory:{}:{}", file.version, entry.name())
                            });
                        self.widgets
                            .delivery_list
                            .append(&delivery_row_with_directory(
                                entry,
                                file.and_then(|file| {
                                    root.as_ref().map(|root| (root.as_path(), file))
                                }),
                            ));
                    }
                }
            } else {
                while let Some(child) = self.widgets.photo_grid.child_at_index(0) {
                    self.widgets.photo_grid.remove(&child);
                }
                let previous = self.rendered_files.borrow();
                for (index, entry) in page.entries.iter().enumerate() {
                    if previous.get(index) == Some(entry) {
                        continue;
                    }
                    if let Some(row) = self.widgets.delivery_list.row_at_index(index as i32) {
                        self.widgets.delivery_list.remove(&row);
                    }
                    let row = if let Some((root, directory)) = &directory {
                        let file = directory.files.get(entry.name()).filter(|file| {
                            entry.id() == format!("directory:{}:{}", file.version, entry.name())
                        });
                        delivery_row_with_directory(entry, file.map(|file| (root.as_path(), file)))
                    } else {
                        delivery_row(entry)
                    };
                    self.widgets.delivery_list.insert(&row, index as i32);
                }
                while let Some(row) = self
                    .widgets
                    .delivery_list
                    .row_at_index(page.entries.len() as i32)
                {
                    self.widgets.delivery_list.remove(&row);
                }
                drop(previous);
            }
            self.rendered_files.replace(page.entries);
        }
        self.widgets
            .file_show_more
            .set_visible(page.total > view.limit);
        if page.total == 0 {
            if records.is_empty() && activity.is_empty() {
                self.widgets
                    .activity_empty
                    .set_label(if directory.is_some() { "Initialize the source on Android, then select Receive. Existing Linux-only files stay unchanged." } else { "Select Receive to check for new files." });
                self.widgets.activity_stack.set_visible_child_name("empty");
            } else {
                self.widgets
                    .activity_stack
                    .set_visible_child_name("no-results");
            }
        } else {
            self.widgets.activity_stack.set_visible_child_name("list");
        }
        self.rendering_file_controls.set(true);
        for (action, key) in [
            ("file-sort", view.sort.key()),
            ("file-filter", view.filter.key()),
            ("file-kind", view.kind.key()),
        ] {
            if let Some(action) = self
                .widgets
                .window
                .lookup_action(action)
                .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
            {
                action.set_state(&key.to_variant());
            }
        }
        if self.widgets.file_search.text().as_str() != view.query {
            self.widgets.file_search.set_text(&view.query);
        }
        self.widgets.file_kind.set_selected(
            FILE_KINDS
                .iter()
                .position(|kind| kind == &view.kind)
                .unwrap() as u32,
        );
        self.rendering_file_controls.set(false);
        let filtered = view.filter != FileFilter::All
            || view.kind != FileKind::All
            || !view.query.trim().is_empty();
        if filtered {
            self.widgets
                .file_filter_button
                .add_css_class("suggested-action");
        } else {
            self.widgets
                .file_filter_button
                .remove_css_class("suggested-action");
        }
        self.widgets
            .file_filter_button
            .set_tooltip_text(Some(&format!(
                "Filter Files · {} · {}",
                view.filter.label(),
                view.kind.label()
            )));
        self.widgets
            .file_sort_button
            .set_tooltip_text(Some(&format!(
                "Sort Files · {} · active transfers first",
                view.sort.label()
            )));
    }

    fn handle_file_progress(&self, bridge_id: &str, event: SyncEvent) {
        if !self
            .bridges
            .borrow()
            .iter()
            .any(|view| view.registration.id == bridge_id)
        {
            return;
        }
        match event {
            SyncEvent::FileActive {
                id,
                original_name,
                media_type,
                size,
                phase,
            } => {
                let next = FileActivity {
                    id,
                    original_name,
                    media_type,
                    size,
                    phase: Some(phase),
                    error: None,
                };
                let mut activity = self.file_activity.borrow_mut();
                let files = activity.entry(bridge_id.to_owned()).or_default();
                if files.get(&next.id) == Some(&next) {
                    return;
                }
                files.insert(next.id.clone(), next);
            }
            SyncEvent::FileSettled { id, record, error } => {
                let previous = self
                    .file_activity
                    .borrow_mut()
                    .entry(bridge_id.to_owned())
                    .or_default()
                    .remove(&id);
                if let Some(error) = error {
                    let activity = previous.or_else(|| {
                        record.as_ref().map(|record| FileActivity {
                            id: id.clone(),
                            original_name: record.original_name.clone(),
                            media_type: record.media_type.clone(),
                            size: record.size,
                            phase: None,
                            error: None,
                        })
                    });
                    if let Some(mut activity) = activity {
                        activity.phase = None;
                        activity.error = Some(error);
                        self.file_activity
                            .borrow_mut()
                            .entry(bridge_id.to_owned())
                            .or_default()
                            .insert(id.clone(), activity);
                    }
                }
                if let Some(record) = record
                    && let Some(snapshot) = self
                        .bridges
                        .borrow_mut()
                        .iter_mut()
                        .find(|view| view.registration.id == bridge_id)
                        .and_then(|view| view.snapshot.as_mut())
                    && Arc::make_mut(&mut snapshot.deliveries).upsert(*record)
                    && self.selected_bridge_id.borrow().as_deref() != Some(bridge_id)
                {
                    self.unread_folders
                        .borrow_mut()
                        .insert(bridge_id.to_owned());
                }
            }
        }
        if self.selected_bridge_id.borrow().as_deref() == Some(bridge_id) {
            self.file_render_pending.set(true);
            if !self.processing_worker_batch.get() {
                self.flush_file_render();
            }
        }
    }

    fn flush_file_render(&self) {
        if self.file_render_pending.replace(false)
            && let Some(snapshot) = self.selected_bridge().and_then(|view| view.snapshot)
        {
            self.render_file_list(&snapshot.deliveries);
        }
    }

    fn show_error(&self, message: &str) {
        self.widgets
            .error_label
            .set_label(&safe_ui_message(message, 4000));
        self.widgets.error_revealer.set_reveal_child(true);
    }

    fn hide_error(&self) {
        self.widgets.error_revealer.set_reveal_child(false);
    }

    fn open_library(&self) {
        let Some(path) = self
            .selected_bridge()
            .and_then(|bridge| bridge.snapshot)
            .map(|snapshot| snapshot.library_dir)
        else {
            return;
        };
        let uri = gio::File::for_path(path).uri();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            self.show_error(&format!("Could not open folder: {error}"));
        }
    }

    fn update_action_sensitivity(&self) {
        let idle = !self.busy.get();
        let valid = self
            .selected_bridge()
            .is_some_and(|bridge| bridge.error.is_none() && bridge.snapshot.is_some());
        self.widgets.refresh_button.set_sensitive(idle);
        self.widgets.add_bridge_button.set_sensitive(idle);
        self.widgets.settings_button.set_sensitive(idle);
        self.widgets.folder_category.set_sensitive(idle && valid);
        self.widgets.sync_button.set_sensitive(idle && valid);
        self.widgets.open_library_button.set_sensitive(valid);
    }
}

#[derive(Debug, Clone)]
enum BridgeEditorMode {
    New,
    Edit(String),
}

fn show_bridge_editor(ui: &Rc<DesktopUi>, mode: BridgeEditorMode) -> Option<adw::Window> {
    if ui.busy.get() || ui.editor_open.get() {
        return None;
    }
    let registration = match &mode {
        BridgeEditorMode::New => None,
        BridgeEditorMode::Edit(id) => ui
            .bridges
            .borrow()
            .iter()
            .find(|bridge| bridge.registration.id == *id)
            .map(|bridge| bridge.registration.clone()),
    };
    if matches!(mode, BridgeEditorMode::Edit(_)) && registration.is_none() {
        ui.show_error("The selected Folder no longer exists. Refresh and try again.");
        return None;
    }
    let (existing, config_error) = match registration.as_ref() {
        Some(bridge) => match load_editable_config(&bridge.config_path) {
            Ok(config) => (Some(config), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        },
        None => (None, None),
    };
    let is_new_bridge = registration.is_none();
    let proposed_library_dir = existing
        .as_ref()
        .map(|config| config.storage.library_dir.clone())
        .unwrap_or_default();
    let proposed_name = registration
        .as_ref()
        .map(|bridge| bridge.name.clone())
        .unwrap_or_default();
    let existing_auto_receive = registration
        .as_ref()
        .is_some_and(|bridge| bridge.auto_receive);
    let (existing_url, existing_insecure, token_env) = match existing.as_ref().map(|c| &c.server) {
        Some(ServerConfig::Http {
            base_url,
            allow_insecure_http,
            token_env,
            ..
        }) => (base_url.clone(), *allow_insecure_http, token_env.clone()),
        _ => (String::new(), false, DEFAULT_HTTP_TOKEN_ENV.to_owned()),
    };

    let dialog_title = if is_new_bridge {
        "Add Folder"
    } else {
        "Folder Settings"
    };
    let dialog = adw::Window::builder()
        .transient_for(&ui.widgets.window)
        .modal(true)
        .title(dialog_title)
        .default_width(620)
        .default_height(560)
        .build();
    dialog.add_css_class("mirelay");
    ui.editor_open.set(true);
    let weak_ui = Rc::downgrade(ui);
    dialog.connect_hide(move |_| {
        if let Some(ui) = weak_ui.upgrade() {
            ui.editor_open.set(false);
        }
    });
    let dialog_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let setup_task = pairing_panel::SetupTask::new(&dialog, &dialog_root);
    {
        let weak_ui = Rc::downgrade(ui);
        let task = setup_task.clone();
        dialog.connect_close_request(move |_| {
            if !task.is_busy()
                && let Some(ui) = weak_ui.upgrade()
            {
                ui.editor_open.set(false);
            }
            glib::Propagation::Proceed
        });
    }
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(dialog_title, "");
    header.set_title_widget(Some(&title));
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_sensitive(is_new_bridge || existing.is_some());
    header.pack_start(&cancel);
    let remove = gtk::Button::with_label("Remove");
    remove.add_css_class("destructive-action");
    remove.set_visible(!is_new_bridge);
    header.pack_start(&remove);
    header.pack_end(&save);
    dialog_root.append(&header);

    let form = gtk::Box::new(gtk::Orientation::Vertical, 14);
    form.add_css_class("settings-page");

    let folder_group = adw::PreferencesGroup::builder().title("Folder").build();
    let name_entry = gtk::Entry::builder()
        .text(proposed_name)
        .placeholder_text("Uses the folder name when empty")
        .width_chars(18)
        .max_width_chars(24)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    let name_row = adw::ActionRow::builder().title("Display Name").build();
    name_row.add_suffix(&name_entry);
    folder_group.add(&name_row);
    let folder_entry = gtk::Entry::builder()
        .text(proposed_library_dir.to_string_lossy())
        .editable(is_new_bridge)
        .width_chars(18)
        .max_width_chars(24)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    folder_entry.set_widget_name("folder-path");
    let folder_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Choose folder")
        .sensitive(is_new_bridge)
        .valign(gtk::Align::Center)
        .build();
    let folder_row = adw::ActionRow::builder()
        .title("Folder Path")
        .subtitle(if is_new_bridge {
            "Received files"
        } else {
            "Fixed after creation"
        })
        .build();
    folder_row.add_suffix(&folder_entry);
    folder_row.add_suffix(&folder_button);
    folder_group.add(&folder_row);

    let directory_switch = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(
            existing
                .as_ref()
                .is_some_and(|config| config.directory_sync),
        )
        .sensitive(is_new_bridge)
        .build();
    directory_switch.set_widget_name("directory-mode");
    let mode_row = adw::ActionRow::builder().title("Directory sync")
        .subtitle("Android → Linux. Preserve names and subfolders; keep replaced copies. Mode is fixed after creation.")
        .subtitle_lines(3).activatable_widget(&directory_switch).build();
    mode_row.add_suffix(&directory_switch);
    folder_group.add(&mode_row);
    {
        let save = save.clone();
        directory_switch.connect_active_notify(move |toggle| {
            save.set_label(if toggle.is_active() && is_new_bridge {
                "Review directory…"
            } else {
                "Save"
            })
        });
    }

    let connection_group = adw::PreferencesGroup::builder().title("Connection").build();
    let url_entry = gtk::Entry::builder()
        .text(existing_url)
        .placeholder_text("https://relay.example.com")
        .width_chars(18)
        .max_width_chars(24)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    url_entry.set_input_purpose(gtk::InputPurpose::Url);
    url_entry.set_editable(
        !existing
            .as_ref()
            .is_some_and(|config| config.directory_sync),
    );
    let url_row = adw::ActionRow::builder().title("Server URL").build();
    url_row.add_suffix(&url_entry);
    let token_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Optional")
        .show_peek_icon(true)
        .width_chars(18)
        .max_width_chars(24)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    if let Some(token) = registration
        .as_ref()
        .and_then(|bridge| ui.tokens.borrow().get(&bridge.id).cloned())
    {
        token_entry.set_text(&token);
    }
    let token_row = adw::ActionRow::builder()
        .title("Session Token")
        .subtitle("Optional for this session")
        .build();
    token_entry.set_tooltip_text(Some(&format!("Leave empty to read {token_env}")));
    token_row.add_suffix(&token_entry);
    let insecure = gtk::Switch::builder().valign(gtk::Align::Center).build();
    insecure.set_active(existing_insecure);
    let insecure_row = adw::ActionRow::builder()
        .title("Allow Insecure HTTP")
        .subtitle("Only for trusted local networks")
        .activatable_widget(&insecure)
        .build();
    insecure_row.add_suffix(&insecure);
    connection_group.add(&url_row);
    connection_group.add(&token_row);
    connection_group.add(&insecure_row);

    let automation_group = adw::PreferencesGroup::builder().title("Automation").build();
    let auto_receive = gtk::Switch::builder().valign(gtk::Align::Center).build();
    auto_receive.set_active(existing_auto_receive);
    let auto_receive_row = adw::ActionRow::builder()
        .title("Automatically Receive")
        .subtitle(format!(
            "Check every {AUTO_RECEIVE_INTERVAL_SECONDS} seconds while MiRelay is open"
        ))
        .activatable_widget(&auto_receive)
        .build();
    auto_receive_row.add_suffix(&auto_receive);
    automation_group.add(&auto_receive_row);

    let privacy_group = adw::PreferencesGroup::builder().title("Privacy").build();
    let privacy_row = adw::ActionRow::builder()
        .title("Credentials are not stored on disk")
        .subtitle("The token stays in memory for this session. Other settings are stored locally.")
        .subtitle_lines(3)
        .build();
    privacy_row.add_prefix(&gtk::Image::from_icon_name("security-high-symbolic"));
    privacy_group.add(&privacy_row);

    let form_error = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    form_error.add_css_class("error");
    if let Some(error) = config_error {
        form_error.set_label(&format!(
            "This configuration cannot be read safely, so overwriting is disabled. Repair the original file or remove this Folder:\n{}",
            safe_ui_message(&error, 1600)
        ));
        form_error.set_visible(true);
    }

    form.append(&folder_group);
    form.append(&connection_group);
    form.append(&pairing_panel::panel(
        &dialog,
        &setup_task,
        &name_entry,
        &folder_entry,
        &url_entry,
        &token_entry,
        &insecure,
        is_new_bridge,
    ));
    form.append(&automation_group);
    form.append(&privacy_group);
    form.append(&form_error);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(560);
    clamp.set_child(Some(&form));
    let form_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    dialog_root.append(&form_scroll);
    dialog.set_content(Some(&dialog_root));

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    if let Some(registration) = registration.clone() {
        let dialog = dialog.clone();
        let ui = Rc::clone(ui);
        remove.connect_clicked(move |_| {
            confirm_remove_bridge(&ui, &dialog, registration.clone());
        });
    }
    {
        let dialog = dialog.clone();
        let folder_entry = folder_entry.clone();
        folder_button.connect_clicked(move |_| {
            let chooser = gtk::FileChooserNative::builder()
                .title("Choose Folder")
                .transient_for(&dialog)
                .action(gtk::FileChooserAction::SelectFolder)
                .accept_label("Select")
                .cancel_label("Cancel")
                .build();
            let folder_entry = folder_entry.clone();
            chooser.connect_response(move |chooser, response| {
                if response == gtk::ResponseType::Accept
                    && let Some(path) = chooser.file().and_then(|file| file.path())
                {
                    folder_entry.set_text(&path.to_string_lossy());
                }
            });
            chooser.show();
        });
    }
    {
        let dialog = dialog.clone();
        let ui = Rc::clone(ui);
        let mode = mode.clone();
        save.connect_clicked(move |_| {
            let server_url = url_entry.text().trim().to_owned();
            if server_url.is_empty() {
                form_error.set_label("Enter a server URL.");
                form_error.set_visible(true);
                return;
            }
            let library_dir = folder_entry.text().trim().to_owned();
            if library_dir.is_empty() {
                form_error.set_label("Choose a folder to manage.");
                form_error.set_visible(true);
                return;
            }
            let library_dir = PathBuf::from(library_dir);
            let requested_name = name_entry.text().trim().to_owned();
            let bridge_name = if requested_name.is_empty() {
                bridge_name_for_path(&library_dir)
            } else {
                requested_name
            };
            let allow_insecure = insecure.is_active();
            let automatic = auto_receive.is_active();
            let token = token_entry.text().trim().to_owned();
            let verification_token = if token.is_empty() {
                std::env::var(&token_env).unwrap_or_default()
            } else {
                token.clone()
            };
            let (dialog, ui, mode, form_error) =
                (dialog.clone(), ui.clone(), mode.clone(), form_error.clone());
            let checked_url = server_url.clone();
            form_error.set_label("Checking connection and Folder permission…");
            form_error.set_visible(true);
            if directory_switch.is_active() && is_new_bridge {
                let paths = ui.paths.clone();
                let secret = verification_token.clone();
                let (ui, dialog, task, error) = (
                    ui.clone(),
                    dialog.clone(),
                    setup_task.clone(),
                    form_error.clone(),
                );
                setup_task.run(
                    move || {
                        directory_panel::prepare(
                            &paths,
                            bridge_name,
                            library_dir,
                            &server_url,
                            allow_insecure,
                            automatic,
                            &secret,
                        )
                    },
                    move |result| match result {
                        Ok(prepared) => {
                            error.set_visible(false);
                            directory_panel::review(
                                &ui,
                                &dialog,
                                &task,
                                prepared,
                                verification_token,
                                &error,
                            );
                        }
                        Err(failure) => {
                            error.set_label(&safe_ui_message(&format!("{failure:#}"), 1800));
                            error.set_visible(true);
                        }
                    },
                );
                return;
            }
            setup_task.run(
                move || {
                    crate::pairing::PairingClient::new(
                        &checked_url,
                        &verification_token,
                        allow_insecure,
                    )?
                    .verify_receiver()
                },
                move |checked| {
                    if let Err(error) = checked {
                        form_error.set_label(&safe_ui_message(&error.to_string(), 1000));
                        return;
                    }
                    let result = match &mode {
                        BridgeEditorMode::New => create_bridge(
                            &ui.paths,
                            bridge_name,
                            library_dir,
                            &server_url,
                            allow_insecure,
                            automatic,
                        ),
                        BridgeEditorMode::Edit(id) => update_bridge(
                            &ui.paths,
                            id,
                            bridge_name,
                            &server_url,
                            allow_insecure,
                            automatic,
                        ),
                    };
                    match result {
                        Ok(bridge_id) => {
                            if token.is_empty() {
                                ui.tokens.borrow_mut().remove(&bridge_id);
                            } else {
                                ui.tokens.borrow_mut().insert(bridge_id, token);
                            }
                            dialog.close();
                            ui.start_load_all();
                            ui.widgets
                                .toast_overlay
                                .add_toast(adw::Toast::new(if is_new_bridge {
                                    "Folder added"
                                } else {
                                    "Folder settings saved"
                                }));
                        }
                        Err(error) => {
                            form_error.set_label(&safe_ui_message(&format!("{error:#}"), 2000));
                            form_error.set_visible(true);
                        }
                    }
                },
            );
        });
    }
    dialog.present();
    Some(dialog)
}

fn create_bridge(
    paths: &DesktopPaths,
    name: String,
    library_dir: PathBuf,
    base_url: &str,
    allow_insecure_http: bool,
    auto_receive: bool,
) -> Result<String> {
    let bridge_id = Uuid::new_v4().to_string();
    let config_path = paths.bridge_config_dir.join(format!("{bridge_id}.toml"));
    let data_dir = paths.bridge_data_dir.join(&bridge_id);
    let mut config = Config::defaults(InitOverrides {
        data_dir: Some(data_dir),
        library_dir: Some(library_dir),
        ..Default::default()
    })?;
    set_http_server(&mut config, base_url, allow_insecure_http);
    config.validate()?;
    let resources = BridgeResources::from_config(&config);
    let registration = BridgeRegistration {
        kind: Default::default(),
        id: bridge_id.clone(),
        name,
        config_path: config_path.clone(),
        auto_receive,
    };

    // Fail before touching the selected library when the registry is corrupt or
    // another Folder already owns any of these resources.
    let registry = paths.registry.load()?;
    ensure_resources_unique(&registry, &resources, None)?;
    let mut validated_registry = registry.clone();
    validated_registry.add(registration.clone())?;
    if config_path.exists() {
        bail!(
            "Refusing to overwrite existing Folder configuration {}",
            config_path.display()
        );
    }
    persist_config(&config, &config_path, false, true)?;

    paths.registry.update(|registry| {
        ensure_resources_unique(registry, &resources, None)?;
        registry.add(registration)
    })?;
    Ok(bridge_id)
}

fn update_bridge(
    paths: &DesktopPaths,
    bridge_id: &str,
    name: String,
    base_url: &str,
    allow_insecure_http: bool,
    auto_receive: bool,
) -> Result<String> {
    paths.registry.update(|registry| {
        // Validate all registry-side changes before touching the independently
        // stored config, while excluding concurrent desktop registry edits.
        let mut preview = registry.clone();
        preview.rename(bridge_id, name.clone())?;
        preview.set_auto_receive(bridge_id, auto_receive)?;
        let registration = registry
            .bridges
            .iter()
            .find(|bridge| bridge.id == bridge_id)
            .context("The selected Folder no longer exists")?
            .clone();
        let mut config = load_editable_config(&registration.config_path).with_context(|| {
            format!(
                "Folder configuration {} cannot be edited safely; repair or remove the Folder first",
                registration.config_path.display()
            )
        })?;
        if config.directory_sync {
            let ServerConfig::Http { base_url: previous, .. } = &config.server else { bail!("Invalid directory Folder."); };
            anyhow::ensure!(previous == base_url, "A directory Folder's server cannot be changed. Create a new paired Folder instead.");
        }
        set_http_server(&mut config, base_url, allow_insecure_http);
        config.validate()?;
        ensure_resources_unique(
            registry,
            &BridgeResources::from_config(&config),
            Some(bridge_id),
        )?;
        persist_config(&config, &registration.config_path, true, false)?;
        registry.rename(bridge_id, name.clone())?;
        registry.set_auto_receive(bridge_id, auto_receive)?;
        registry.select(bridge_id)
    })?;
    Ok(bridge_id.to_owned())
}

fn load_editable_config(path: &Path) -> Result<Config> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Unable to inspect Folder configuration {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "Folder configuration {} is not a regular file that can be edited safely",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            bail!(
                "Folder configuration {} has multiple hard links; refusing atomic replacement",
                path.display()
            );
        }
    }
    Config::load(path)
}

fn load_config_for_read(path: &Path) -> Result<Config> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Unable to inspect Folder configuration {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "Folder configuration {} is not a regular file",
            path.display()
        );
    }
    Config::load(path)
}

fn confirm_remove_bridge(
    ui: &Rc<DesktopUi>,
    editor: &adw::Window,
    registration: BridgeRegistration,
) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(editor)
        .modal(true)
        .message_type(gtk::MessageType::Warning)
        .buttons(gtk::ButtonsType::Cancel)
        .text(format!("Remove “{}”?", registration.name))
        .secondary_text(
            "This only removes the Folder from MiRelay. Its configuration, state, and received files remain.",
        )
        .build();
    dialog.add_button("Remove", gtk::ResponseType::Accept);
    let ui = Rc::clone(ui);
    let editor = editor.clone();
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            match ui
                .paths
                .registry
                .update(|registry| registry.remove(&registration.id).map(drop))
            {
                Ok(()) => {
                    ui.tokens.borrow_mut().remove(&registration.id);
                    editor.close();
                    ui.widgets
                        .toast_overlay
                        .add_toast(adw::Toast::new("Folder removed. Files remain in place."));
                    ui.start_load_all();
                }
                Err(error) => ui.show_error(&format!("Could not remove Folder: {error:#}")),
            }
        }
        dialog.close();
    });
    dialog.present();
}

#[cfg(test)]
fn ensure_bridge_registered(
    registry: &BridgeRegistryStore,
    bridge_id: &str,
    config_path: &Path,
) -> Result<()> {
    load_registered_config(registry, bridge_id, config_path).map(drop)
}

fn load_registered_config(
    registry: &BridgeRegistryStore,
    bridge_id: &str,
    config_path: &Path,
) -> Result<Config> {
    let current = registry.load()?;
    let bridge = current
        .bridges
        .iter()
        .find(|bridge| bridge.id == bridge_id)
        .with_context(|| format!("Folder {bridge_id:?} was removed; refresh the list"))?;
    if bridge.config_path != config_path {
        bail!("Folder {bridge_id:?} has a different configuration path; refresh the list");
    }
    let config = load_config_for_read(config_path)?;
    ensure_resources_unique(
        &current,
        &BridgeResources::from_config(&config),
        Some(bridge_id),
    )?;
    Ok(config)
}

#[cfg(test)]
fn sync_registered_bridge(
    registry: &BridgeRegistryStore,
    bridge_id: &str,
    config_path: &Path,
    token: Option<&str>,
) -> Result<SyncResult> {
    sync_registered_bridge_with_events(registry, bridge_id, config_path, token, &|_| {})
}

fn sync_registered_bridge_with_events(
    registry: &BridgeRegistryStore,
    bridge_id: &str,
    config_path: &Path,
    token: Option<&str>,
    emit: &dyn Fn(SyncEvent),
) -> Result<SyncResult> {
    let config = load_registered_config(registry, bridge_id, config_path)?;
    if config.directory_sync {
        return directory_panel::receive(config, token, emit);
    }
    let source = source_for(&config, token)?;
    let summary = sync_once_with_events(&config, source.as_ref(), emit)?;
    let snapshot = snapshot_from_config(config)?;
    Ok(SyncResult { summary, snapshot })
}

#[cfg(test)]
fn execute_sync_requests(
    registry: &BridgeRegistryStore,
    requests: Vec<SyncRequest>,
) -> Vec<SyncOutcome> {
    execute_sync_requests_with_events(registry, requests, &|_, _| {})
}

fn execute_sync_requests_with_events(
    registry: &BridgeRegistryStore,
    requests: Vec<SyncRequest>,
    emit: &dyn Fn(&str, SyncEvent),
) -> Vec<SyncOutcome> {
    requests
        .into_iter()
        .map(|request| {
            let result = sync_registered_bridge_with_events(
                registry,
                &request.bridge_id,
                &request.config_path,
                request.token.as_deref(),
                &|event| emit(&request.bridge_id, event),
            )
            .map_err(|error| format!("{error:#}"));
            SyncOutcome {
                bridge_id: request.bridge_id,
                bridge_name: request.bridge_name,
                result,
            }
        })
        .collect()
}

fn automatic_sync_requests(
    bridges: &[BridgeView],
    tokens: &HashMap<String, String>,
) -> Vec<SyncRequest> {
    bridges
        .iter()
        .filter(|bridge| {
            bridge.registration.auto_receive && bridge.error.is_none() && bridge.snapshot.is_some()
        })
        .map(|bridge| SyncRequest {
            bridge_id: bridge.registration.id.clone(),
            bridge_name: bridge.registration.name.clone(),
            config_path: bridge.registration.config_path.clone(),
            token: tokens.get(&bridge.registration.id).cloned(),
        })
        .collect()
}

#[cfg(test)]
fn load_snapshot(config_path: &Path) -> Result<BridgeSnapshot> {
    load_snapshot_and_resources(config_path).map(|(snapshot, _)| snapshot)
}

fn load_snapshot_and_resources(config_path: &Path) -> Result<(BridgeSnapshot, BridgeResources)> {
    if !config_path.exists() {
        bail!("This Folder is not configured. Open Folder Settings.");
    }
    let config = load_config_for_read(config_path)?;
    let resources = BridgeResources::from_config(&config);
    let snapshot = snapshot_from_config(config)?;
    Ok((snapshot, resources))
}

fn snapshot_from_config(config: Config) -> Result<BridgeSnapshot> {
    if config.directory_sync {
        return directory_panel::snapshot(config);
    }
    let store = StateStore::new(config.storage.state_file.clone());
    let state = {
        let _lock = store.lock_shared()?;
        store.load()?
    };
    let raw_counts = status_counts(&state);
    let counts = BridgeCounts {
        total: raw_counts["total"],
        ack_pending: raw_counts["ack_pending"],
        acknowledged: raw_counts["acknowledged"],
        wallpaper_pending: raw_counts["wallpaper_pending"],
        wallpaper_attention: raw_counts["wallpaper_failed"]
            + raw_counts["wallpaper_uncertain"]
            + raw_counts["wallpaper_not_configured"],
    };
    let source_label = match &config.server {
        ServerConfig::Filesystem { inbox_dir } => {
            format!("Local inbox · {}", inbox_dir.display())
        }
        ServerConfig::Http { base_url, .. } => format!("HTTP · {base_url}"),
    };
    let name = bridge_name_for_path(&config.storage.library_dir);
    let wallpaper_label = if config.wallpaper.command.is_empty() {
        "Disabled; images are only saved to the folder".to_owned()
    } else {
        format!(
            "Enabled · {} second timeout",
            config.wallpaper.timeout_seconds
        )
    };
    let mut deliveries = state.deliveries.into_values().collect::<Vec<_>>();
    deliveries.sort_by(|left, right| {
        right
            .received_at_unix
            .cmp(&left.received_at_unix)
            .then_with(|| right.id.cmp(&left.id))
    });

    Ok(BridgeSnapshot {
        name,
        device_id: config.device_id,
        source_label,
        library_dir: config.storage.library_dir,
        max_file_size_bytes: config.limits.max_file_size_bytes,
        wallpaper_label,
        counts,
        deliveries: Arc::new(deliveries.into()),
        directory: None,
    })
}

#[cfg(test)]
fn sync_configured(config_path: &Path, token: Option<&str>) -> Result<SyncResult> {
    if !config_path.exists() {
        bail!("This Folder is not configured. Open Folder Settings first.");
    }
    let config = load_config_for_read(config_path)?;
    let source = source_for(&config, token)?;
    let summary = crate::sync::sync_once(&config, source.as_ref())?;
    let snapshot = load_snapshot(config_path)?;
    Ok(SyncResult { summary, snapshot })
}

#[cfg(test)]
fn configure_http_with_overrides(
    config_path: &Path,
    base_url: &str,
    allow_insecure_http: bool,
    init_overrides: InitOverrides,
) -> Result<()> {
    let initialize_state = !config_path.exists();
    let mut config = if !initialize_state {
        Config::load(config_path).with_context(|| {
            format!(
                "Existing Folder configuration {} cannot be read; repair or move it before continuing",
                config_path.display()
            )
        })?
    } else {
        Config::defaults(init_overrides)?
    };

    set_http_server(&mut config, base_url, allow_insecure_http);
    persist_config(&config, config_path, true, initialize_state)
}

fn set_http_server(config: &mut Config, base_url: &str, allow_insecure_http: bool) {
    match &mut config.server {
        ServerConfig::Http {
            base_url: current_url,
            allow_insecure_http: current_insecure,
            ..
        } => {
            *current_url = base_url.to_owned();
            *current_insecure = allow_insecure_http;
        }
        ServerConfig::Filesystem { .. } => {
            config.server = ServerConfig::Http {
                base_url: base_url.to_owned(),
                token_env: DEFAULT_HTTP_TOKEN_ENV.to_owned(),
                request_timeout_seconds: DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS,
                page_size: DEFAULT_HTTP_PAGE_SIZE,
                retry_max_attempts: DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
                retry_base_delay_milliseconds: DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS,
                retry_max_delay_milliseconds: DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS,
                allow_insecure_http,
            };
        }
    }
}

fn persist_config(
    config: &Config,
    config_path: &Path,
    force: bool,
    initialize_state: bool,
) -> Result<()> {
    config.validate()?;
    config.ensure_directories()?;
    config.save(config_path, force)?;

    if initialize_state {
        let store = StateStore::new(config.storage.state_file.clone());
        let _lock = store.lock_exclusive()?;
        if !store.path().exists() {
            store.save(&Default::default())?;
        }
    }
    Ok(())
}

fn delivery_row(entry: &FileEntry) -> gtk::ListBoxRow {
    delivery_row_with_directory(entry, None)
}

fn delivery_row_with_directory(
    entry: &FileEntry,
    directory: Option<(&Path, &crate::directory::receiver::Applied)>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let tooltip = match entry {
        FileEntry::Saved(record) => format!(
            "{}\n{}\n{}",
            record.original_name,
            record.stored_path.display(),
            record
                .delivery_error
                .as_deref()
                .or(record.wallpaper_error.as_deref())
                .unwrap_or(if entry.is_complete() {
                    "Completed"
                } else {
                    delivery_state(record).1
                })
        ),
        FileEntry::Live(activity) => format!(
            "{}\n{}",
            activity.original_name,
            activity
                .error
                .as_deref()
                .unwrap_or_else(|| activity.phase.map(phase_label).unwrap_or("Waiting"))
        ),
    };
    row.set_tooltip_text(Some(&safe_ui_message(&tooltip, 1600)));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.add_css_class("delivery-row");
    let icon = gtk::Image::from_icon_name(icon_for_media_type(entry.media_type()));
    icon.set_pixel_size(16);
    icon.add_css_class("mime-icon");
    content.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(entry.name())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("delivery-name");
    name.set_tooltip_text(Some(entry.name()));
    let details_text = match entry {
        FileEntry::Saved(record) => format!(
            "{}  ·  {}",
            format_bytes(record.size),
            format_timestamp(record.received_at_unix)
        ),
        FileEntry::Live(_) => format_bytes(entry.size()),
    };
    let details = gtk::Label::builder()
        .label(details_text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    details.add_css_class("secondary-text");
    text.append(&name);
    text.append(&details);
    content.append(&text);
    if let Some((root, file)) = directory {
        details.set_label(&format!(
            "{}  ·  Version {}",
            format_bytes(entry.size()),
            file.version
        ));
        if file.received_at_unix == 0 {
            details.set_label(&format!("Version {} · Legacy record", file.version));
        }
        if let Some(button) = directory_panel::history_button(root, entry.name(), file) {
            content.append(&button);
        }
    }

    // A completed file needs no permanent Received label, checkmark, or animation.
    if entry.is_complete() {
        row.set_child(Some(&content));
        return row;
    }
    let (state_icon, state_text, state_class) = match entry {
        FileEntry::Saved(_) if directory.is_some_and(|(_, file)| !file.acknowledged) => (
            "emblem-synchronizing-symbolic",
            "Awaiting receipt",
            "state-pending",
        ),
        FileEntry::Saved(_) if directory.is_some_and(|(_, file)| file.conflict) => (
            "dialog-warning-symbolic",
            "Conflict copy kept",
            "state-pending",
        ),
        FileEntry::Saved(record) => delivery_state(record),
        FileEntry::Live(activity) => match activity.phase {
            Some(phase) => (
                "content-loading-symbolic",
                phase_label(phase),
                "secondary-text",
            ),
            None if activity.error.is_some() => {
                ("dialog-warning-symbolic", "Needs attention", "state-error")
            }
            None => ("content-loading-symbolic", "Waiting", "state-pending"),
        },
    };
    let state_label = gtk::Label::new(Some(state_text));
    state_label.set_max_width_chars(18);
    state_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    state_label.set_tooltip_text(Some(state_text));
    state_label.add_css_class("row-state");
    state_label.add_css_class(state_class);
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    state.set_valign(gtk::Align::Center);
    if entry.is_active() {
        state.append(&loading_ring(16));
    } else {
        let state_image = gtk::Image::from_icon_name(state_icon);
        state_image.set_pixel_size(16);
        state_image.add_css_class(state_class);
        state.append(&state_image);
    }
    state.append(&state_label);
    content.append(&state);

    row.set_child(Some(&content));
    row
}

fn phase_label(phase: SyncPhase) -> &'static str {
    match phase {
        SyncPhase::Downloading => "Downloading",
        SyncPhase::Verifying => "Verifying",
        SyncPhase::Confirming => "Confirming",
        SyncPhase::Retrying => "Retrying",
        SyncPhase::ApplyingWallpaper => "Applying wallpaper",
    }
}

fn bridge_row(bridge: &BridgeView, unread: bool, sync_failed: bool) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(true);
    row.set_selectable(true);
    let notice = folder_notice(bridge, unread, sync_failed);
    row.set_tooltip_text(Some(&format!(
        "{}{}\n{}",
        bridge.registration.name,
        notice
            .map(|notice| format!(" · {}", notice.label()))
            .unwrap_or_default(),
        bridge.registration.config_path.display()
    )));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.add_css_class("bridge-row");
    let icon = gtk::Image::from_icon_name(bridge.registration.kind.icon_name());
    icon.set_pixel_size(16);
    icon.add_css_class("bridge-icon");
    content.append(&icon);

    let name = gtk::Label::builder()
        .label(&bridge.registration.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("bridge-name");
    content.append(&name);
    if let Some(notice) = notice {
        let indicator = gtk::Image::from_icon_name(notice.icon_name());
        indicator.set_pixel_size(16);
        indicator.set_tooltip_text(Some(notice.label()));
        indicator.add_css_class("folder-notice");
        indicator.update_property(&[gtk::accessible::Property::Label(notice.label())]);
        content.append(&indicator);
    }
    row.set_child(Some(&content));
    row
}

fn property_row(title: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("property-row");
    let title = gtk::Label::builder().label(title).xalign(0.0).build();
    title.add_css_class("property-title");
    title.set_width_request(136);
    let value = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    value.add_css_class("property-value");
    row.append(&title);
    row.append(&value);
    (row, value)
}

fn summary_text(counts: &BridgeCounts) -> String {
    let files = format!(
        "{} file{}",
        counts.total,
        if counts.total == 1 { "" } else { "s" }
    );
    if counts.total == 0 {
        return files;
    }
    if counts.ack_pending > 0 {
        return format!("{files} · {} awaiting acknowledgement", counts.ack_pending);
    }
    if counts.wallpaper_attention > 0 {
        return format!("{files} · {} need attention", counts.wallpaper_attention);
    }
    if counts.wallpaper_pending > 0 {
        return format!("{files} · {} in progress", counts.wallpaper_pending);
    }
    files
}

fn delivery_state(record: &DeliveryRecord) -> (&'static str, &'static str, &'static str) {
    if record.delivery_error.is_some()
        || record.wallpaper_error.is_some()
        || matches!(
            record.wallpaper_status,
            WallpaperStatus::Failed | WallpaperStatus::Uncertain
        )
    {
        return ("dialog-warning-symbolic", "Needs attention", "state-error");
    }
    if record.delivery_status == DeliveryStatus::AckPending {
        return (
            "emblem-synchronizing-symbolic",
            "Awaiting acknowledgement",
            "state-pending",
        );
    }
    match record.wallpaper_status {
        WallpaperStatus::Pending | WallpaperStatus::Running => (
            "emblem-synchronizing-symbolic",
            "In progress",
            "state-pending",
        ),
        WallpaperStatus::Applied => ("emblem-ok-symbolic", "Wallpaper applied", "state-success"),
        _ => ("emblem-ok-symbolic", "Received", "state-success"),
    }
}

fn bridge_name_for_path(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Root".to_owned())
}

fn icon_for_media_type(media_type: &str) -> &'static str {
    if media_type.starts_with("image/") {
        "image-x-generic-symbolic"
    } else if media_type.starts_with("text/") || media_type == "application/pdf" {
        "text-x-generic-symbolic"
    } else if media_type.starts_with("audio/") || media_type == "application/ogg" {
        "audio-x-generic-symbolic"
    } else if media_type.starts_with("video/") {
        "video-x-generic-symbolic"
    } else if matches!(
        media_type,
        "application/zip"
            | "application/gzip"
            | "application/x-7z-compressed"
            | "application/vnd.rar"
    ) {
        "package-x-generic-symbolic"
    } else {
        "application-x-generic-symbolic"
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn format_timestamp(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
        .and_then(|date| date.format("%Y-%m-%d %H:%M").ok())
        .map(|date| date.to_string())
        .unwrap_or_else(|| format!("Unix {timestamp}"))
}

fn clear_list_box(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn capture_widget(widget: &gtk::Widget, path: &Path) -> Result<()> {
    let width = widget.width();
    let height = widget.height();
    if width <= 0 || height <= 0 {
        bail!("widget has not been laid out yet");
    }
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot
        .to_node()
        .context("window snapshot produced no render node")?;
    let renderer = widget
        .native()
        .and_then(|native| native.renderer())
        .context("widget has no native renderer")?;
    // Use the window's renderer without taking ownership of its lifecycle.
    // Creating/unrealizing a second Cairo renderer can invalidate texture caches
    // used by later snapshots of the same widget (e.g. after a theme change).
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(path)
        .with_context(|| format!("failed to save {}", path.display()))
}

fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    let style = adw::StyleManager::default();
    load_desktop_css(&provider, style.is_dark());
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    style.connect_dark_notify(move |style| load_desktop_css(&provider, style.is_dark()));
}

fn load_desktop_css(provider: &gtk::CssProvider, dark: bool) {
    // Keep the primary controls black in both themes; the dark theme needs a
    // visible edge and lighter text accents for links and selection feedback.
    let (border, text_accent) = if dark {
        ("#555555", "#f0f0f0")
    } else {
        ("#181818", "#181818")
    };
    // Newer libadwaita themes consume CSS variables instead of named colors.
    // Keep those scoped to our windows; older GTK must not parse this syntax.
    let variables = if gtk::major_version() > 4 || gtk::minor_version() >= 16 {
        format!(
            ".mirelay {{ --accent-bg-color: #181818; --accent-fg-color: #ffffff; \
             --accent-color: {text_accent}; }}"
        )
    } else {
        String::new()
    };
    provider.load_from_data(&format!(
        "@define-color relay_accent #181818;\n\
         @define-color relay_on_accent #ffffff;\n\
         @define-color relay_accent_hover #303030;\n\
         @define-color relay_accent_border {border};\n\
         @define-color accent_bg_color #181818;\n\
         @define-color accent_fg_color #ffffff;\n\
         @define-color accent_color {text_accent};\n\
         {variables}\n{DESKTOP_CSS}"
    ));
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .context("failed to determine the current directory")?
            .join(path)
    };
    normalize_absolute_path(&absolute)
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf> {
    use std::path::Component;

    if !path.is_absolute() {
        bail!("Path must be absolute: {}", path.display());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("Path escapes the filesystem root: {}", path.display());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

fn safe_ui_message(message: &str, max_chars: usize) -> String {
    let safe = crate::cli::terminal_safe(message);
    let mut chars = safe.chars();
    let mut shortened = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        shortened.push('…');
    }
    shortened
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn history_snapshots_share_records_until_a_real_mutation() {
        let history = Arc::new(FileHistory::from(vec![sample_record(
            "shared.txt",
            "text/plain",
        )]));
        let mut next = Arc::clone(&history);
        assert!(Arc::ptr_eq(&history, &next));
        Arc::make_mut(&mut next)[0].original_name = "changed.txt".into();
        assert_eq!(history[0].original_name, "shared.txt");
        assert_eq!(next[0].original_name, "changed.txt");
        assert!(!Arc::ptr_eq(&history, &next));
    }

    #[test]
    fn independent_instance_requires_explicit_registry() {
        assert!(DesktopArgs::try_parse_from(["mirelay-desktop", "--new-instance"]).is_err());
        let args = DesktopArgs::try_parse_from([
            "mirelay-desktop",
            "--new-instance",
            "--registry",
            "/tmp/mirelay-preview/folders.toml",
        ])
        .unwrap();
        assert!(args.new_instance);
    }

    #[test]
    fn sidebar_smoke_requires_isolation_and_rejects_conflicting_modes() {
        assert!(DesktopArgs::try_parse_from(["mirelay-desktop", "--sidebar-smoke-test"]).is_err());
        assert!(
            DesktopArgs::try_parse_from([
                "mirelay-desktop",
                "--registry",
                "/tmp/sidebar/folders.toml",
                "--sidebar-smoke-test",
                "--screenshot",
                "/tmp/sidebar.png"
            ])
            .is_err()
        );
    }

    fn sample_record(name: &str, media_type: &str) -> DeliveryRecord {
        DeliveryRecord {
            id: "delivery-1".to_owned(),
            original_name: name.to_owned(),
            media_type: media_type.to_owned(),
            sha256: "ab".repeat(32),
            size: 1536,
            stored_path: PathBuf::from("/tmp/mirelay/library/sample.txt"),
            delivery_status: DeliveryStatus::Acknowledged,
            wallpaper_status: WallpaperStatus::NotApplicable,
            wallpaper_attempts: 0,
            delivery_error: None,
            wallpaper_error: None,
            source_created_at_unix: None,
            received_at_unix: 1,
            acknowledged_at_unix: Some(2),
            imported_at_unix: None,
        }
    }

    fn test_paths(root: &Path) -> DesktopPaths {
        DesktopPaths {
            registry: BridgeRegistryStore::new(root.join("config/bridges.toml")),
            bridge_config_dir: root.join("config/bridges"),
            bridge_data_dir: root.join("data/bridges"),
            bootstrap_config: None,
            bootstrap_only_if_registry_missing: false,
            bootstrap_pending: Arc::new(AtomicBool::new(true)),
        }
    }

    fn register_config(paths: &DesktopPaths, id: &str, name: &str, config_path: PathBuf) {
        paths
            .registry
            .update(|registry| {
                registry.add(BridgeRegistration {
                    kind: Default::default(),
                    id: id.to_owned(),
                    name: name.to_owned(),
                    config_path,
                    auto_receive: false,
                })
            })
            .unwrap();
    }

    #[test]
    fn desktop_configuration_creates_a_secret_free_http_client_config() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("config/mirelay.toml");

        configure_http_with_overrides(
            &config_path,
            "https://relay.example",
            false,
            InitOverrides {
                data_dir: Some(root.path().join("data")),
                ..Default::default()
            },
        )
        .unwrap();

        let config = Config::load(&config_path).unwrap();
        assert!(matches!(
            config.server,
            ServerConfig::Http {
                ref base_url,
                allow_insecure_http: false,
                ..
            } if base_url == "https://relay.example"
        ));
        let raw = std::fs::read_to_string(&config_path).unwrap();
        assert!(!raw.contains("session-secret"));
        let snapshot = load_snapshot(&config_path).unwrap();
        assert_eq!(snapshot.counts, BridgeCounts::default());
        assert!(snapshot.deliveries.is_empty());
    }

    #[test]
    fn desktop_configuration_refuses_plain_http_without_the_explicit_toggle() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let overrides = || InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        };
        assert!(
            configure_http_with_overrides(
                &config_path,
                "http://127.0.0.1:8080",
                false,
                overrides(),
            )
            .is_err()
        );
        assert!(!config_path.exists());
        configure_http_with_overrides(&config_path, "http://127.0.0.1:8080", true, overrides())
            .unwrap();
    }

    #[test]
    fn desktop_sync_uses_the_shared_delivery_pipeline() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.ensure_directories().unwrap();
        config.save(&config_path, false).unwrap();
        let source_file = root.path().join("desktop-note.txt");
        std::fs::write(&source_file, b"delivered through the desktop backend\n").unwrap();
        let ServerConfig::Filesystem { inbox_dir } = &config.server else {
            panic!("defaults must use the filesystem source");
        };
        crate::source::FilesystemSource::new(inbox_dir.clone())
            .enqueue(&source_file, config.limits.max_file_size_bytes)
            .unwrap();

        let result = sync_configured(&config_path, None).unwrap();

        assert_eq!(result.summary.received, 1);
        assert_eq!(result.summary.acknowledged, 1);
        assert_eq!(result.snapshot.counts.total, 1);
        assert_eq!(result.snapshot.counts.acknowledged, 1);
        assert_eq!(
            result.snapshot.deliveries[0].original_name,
            "desktop-note.txt"
        );
    }

    #[test]
    fn automatic_receive_syncs_two_folders_without_crossing_state_or_files() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let mut configs = Vec::new();
        for (id, file_name) in [("art", "sketch.txt"), ("docs", "notes.txt")] {
            let config_path = root.path().join(format!("{id}.toml"));
            let config = Config::defaults(InitOverrides {
                data_dir: Some(root.path().join(format!("{id}-data"))),
                library_dir: Some(root.path().join(format!("{id}-library"))),
                ..Default::default()
            })
            .unwrap();
            config.ensure_directories().unwrap();
            config.save(&config_path, false).unwrap();
            let source_file = root.path().join(file_name);
            std::fs::write(&source_file, format!("payload for {id}\n")).unwrap();
            let ServerConfig::Filesystem { inbox_dir } = &config.server else {
                unreachable!();
            };
            crate::source::FilesystemSource::new(inbox_dir.clone())
                .enqueue(&source_file, config.limits.max_file_size_bytes)
                .unwrap();
            register_config(&paths, id, id, config_path.clone());
            paths
                .registry
                .update(|registry| registry.set_auto_receive(id, true))
                .unwrap();
            configs.push((config_path, file_name));
        }

        let before = load_registry_snapshot(&paths).unwrap();
        let mut requests = automatic_sync_requests(&before.bridges, &HashMap::new());
        assert_eq!(requests.len(), 2);
        requests.insert(
            0,
            SyncRequest {
                bridge_id: "removed".to_owned(),
                bridge_name: "Removed".to_owned(),
                config_path: root.path().join("removed.toml"),
                token: None,
            },
        );
        let outcomes = execute_sync_requests(&paths.registry, requests);
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes[0].result.is_err());
        let art = outcomes
            .iter()
            .find(|outcome| outcome.bridge_id == "art")
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        let docs = outcomes
            .iter()
            .find(|outcome| outcome.bridge_id == "docs")
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        let loaded = load_registry_snapshot(&paths).unwrap();

        assert_eq!(art.snapshot.deliveries.len(), 1);
        assert_eq!(docs.snapshot.deliveries.len(), 1);
        assert_eq!(art.snapshot.deliveries[0].original_name, configs[0].1);
        assert_eq!(docs.snapshot.deliveries[0].original_name, configs[1].1);
        assert_eq!(loaded.bridges.len(), 2);
        assert!(loaded.bridges.iter().all(|bridge| bridge.error.is_none()));
        assert!(loaded.bridges.iter().all(|bridge| {
            bridge
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.counts.total == 1)
        }));
    }

    #[test]
    fn legacy_config_is_imported_once_without_being_modified() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("legacy-data")),
            library_dir: Some(root.path().join("Illustrations")),
            ..Default::default()
        })
        .unwrap();
        config.ensure_directories().unwrap();
        config.save(&config_path, false).unwrap();
        let original = std::fs::read(&config_path).unwrap();
        let mut paths = test_paths(root.path());
        paths.bootstrap_config = Some(config_path.clone());

        let first = load_registry_snapshot(&paths).unwrap();
        let second = load_registry_snapshot(&paths).unwrap();

        assert_eq!(first.bridges.len(), 1);
        assert_eq!(second.bridges.len(), 1);
        assert_eq!(first.bridges[0].registration.name, "Illustrations");
        assert_eq!(first.selected_bridge_id, second.selected_bridge_id);
        assert_eq!(std::fs::read(config_path).unwrap(), original);
    }

    #[test]
    fn removed_legacy_bridge_is_not_reimported_on_refresh_or_restart() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("legacy-data")),
            ..Default::default()
        })
        .unwrap();
        config.ensure_directories().unwrap();
        config.save(&config_path, false).unwrap();
        let mut paths = test_paths(root.path());
        paths.bootstrap_config = Some(config_path);
        paths.bootstrap_only_if_registry_missing = true;

        let imported = load_registry_snapshot(&paths).unwrap();
        let id = imported.bridges[0].registration.id.clone();
        paths
            .registry
            .update(|registry| registry.remove(&id).map(drop))
            .unwrap();
        assert!(load_registry_snapshot(&paths).unwrap().bridges.is_empty());

        paths.bootstrap_pending.store(true, Ordering::Release);
        assert!(load_registry_snapshot(&paths).unwrap().bridges.is_empty());
    }

    #[test]
    fn two_created_bridges_have_independent_configs_state_and_devices() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let first_id = create_bridge(
            &paths,
            "Illustrations".to_owned(),
            root.path().join("library/illustrations"),
            "https://relay.example",
            false,
            false,
        )
        .unwrap();
        let second_id = create_bridge(
            &paths,
            "Documents".to_owned(),
            root.path().join("library/documents"),
            "https://documents.relay.example",
            false,
            false,
        )
        .unwrap();
        update_bridge(
            &paths,
            &first_id,
            "Artwork".to_owned(),
            "https://relay.example",
            false,
            true,
        )
        .unwrap();

        let loaded = load_registry_snapshot(&paths).unwrap();
        assert_eq!(loaded.bridges.len(), 2);
        assert_eq!(
            loaded.selected_bridge_id.as_deref(),
            Some(first_id.as_str())
        );
        assert_ne!(first_id, second_id);
        assert_eq!(loaded.bridges[0].registration.name, "Artwork");
        assert!(loaded.bridges[0].registration.auto_receive);
        assert!(!loaded.bridges[1].registration.auto_receive);
        let first = loaded.bridges[0].snapshot.as_ref().unwrap();
        let second = loaded.bridges[1].snapshot.as_ref().unwrap();
        assert_ne!(first.device_id, second.device_id);
        assert_ne!(
            loaded.bridges[0].registration.config_path,
            loaded.bridges[1].registration.config_path
        );
        let first_config = Config::load(&loaded.bridges[0].registration.config_path).unwrap();
        let second_config = Config::load(&loaded.bridges[1].registration.config_path).unwrap();
        assert_ne!(
            first_config.storage.state_file,
            second_config.storage.state_file
        );
        let persisted = std::fs::read_to_string(paths.registry.path()).unwrap();
        assert!(!persisted.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn separate_server_folders_can_share_a_host_without_competing_for_a_queue() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        for name in ["art", "docs"] {
            let url = format!("https://relay.example/f/{}", Uuid::new_v4());
            create_bridge(
                &paths,
                name.into(),
                root.path().join(name),
                &url,
                false,
                false,
            )
            .unwrap();
        }
        let loaded = load_registry_snapshot(&paths).unwrap();
        assert_eq!(loaded.bridges.len(), 2);
        assert!(loaded.bridges.iter().all(|folder| folder.error.is_none()));
    }

    #[test]
    fn one_broken_bridge_does_not_hide_healthy_bridges() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        create_bridge(
            &paths,
            "Broken".to_owned(),
            root.path().join("broken-library"),
            "https://relay.example",
            false,
            false,
        )
        .unwrap();
        create_bridge(
            &paths,
            "Healthy".to_owned(),
            root.path().join("healthy-library"),
            "https://healthy.relay.example",
            false,
            false,
        )
        .unwrap();
        let registry = paths.registry.load().unwrap();
        std::fs::write(&registry.bridges[0].config_path, b"invalid = [toml\n").unwrap();

        let loaded = load_registry_snapshot(&paths).unwrap();

        assert_eq!(loaded.bridges.len(), 2);
        assert!(loaded.bridges[0].error.is_some());
        assert!(loaded.bridges[1].error.is_none());
        assert!(loaded.bridges[1].snapshot.is_some());
    }

    #[test]
    fn two_bridges_cannot_compete_for_the_same_http_queue() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        create_bridge(
            &paths,
            "First".to_owned(),
            root.path().join("first-library"),
            "https://relay.example/api",
            false,
            false,
        )
        .unwrap();

        let error = create_bridge(
            &paths,
            "Second".to_owned(),
            root.path().join("second-library"),
            "https://relay.example/api/",
            false,
            false,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("HTTP receive queue"));
        assert_eq!(paths.registry.load().unwrap().bridges.len(), 1);
    }

    #[test]
    fn sync_rechecks_cross_bridge_conflicts_before_network_access() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let mut config_paths = Vec::new();
        for id in ["first", "second"] {
            let config_path = root.path().join(format!("{id}.toml"));
            let mut config = Config::defaults(InitOverrides {
                data_dir: Some(root.path().join(format!("{id}-data"))),
                ..Default::default()
            })
            .unwrap();
            set_http_server(&mut config, "https://relay.example", false);
            config.ensure_directories().unwrap();
            config.save(&config_path, false).unwrap();
            register_config(&paths, id, id, config_path.clone());
            config_paths.push(config_path);
        }

        let error = sync_registered_bridge(
            &paths.registry,
            "first",
            &config_paths[0],
            Some("unused-token"),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("HTTP receive queue"));
    }

    #[test]
    fn copied_device_identity_marks_both_bridges_unsafe() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let first_path = root.path().join("first.toml");
        let second_path = root.path().join("second.toml");
        let first = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("first-data")),
            ..Default::default()
        })
        .unwrap();
        let mut second = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("second-data")),
            ..Default::default()
        })
        .unwrap();
        second.device_id.clone_from(&first.device_id);
        first.ensure_directories().unwrap();
        second.ensure_directories().unwrap();
        first.save(&first_path, false).unwrap();
        second.save(&second_path, false).unwrap();
        register_config(&paths, "first", "First", first_path);
        register_config(&paths, "second", "Second", second_path);

        let loaded = load_registry_snapshot(&paths).unwrap();

        assert!(loaded.bridges.iter().all(|bridge| {
            bridge
                .error
                .as_deref()
                .is_some_and(|error| error.contains("device ID"))
        }));
    }

    #[test]
    fn overlapping_library_is_rejected_before_new_files_are_created() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let parent = root.path().join("library");
        create_bridge(
            &paths,
            "Parent".to_owned(),
            parent.clone(),
            "https://relay.example",
            false,
            false,
        )
        .unwrap();
        let nested = parent.join("nested");

        let error = create_bridge(
            &paths,
            "Nested".to_owned(),
            nested.clone(),
            "https://relay.example",
            false,
            false,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("Conflicts with Folder"));
        assert_eq!(paths.registry.load().unwrap().bridges.len(), 1);
        assert!(!nested.exists());
    }

    #[cfg(unix)]
    #[test]
    fn overlapping_library_through_a_symlinked_parent_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let real_root = root.path().join("real");
        std::fs::create_dir(&real_root).unwrap();
        create_bridge(
            &paths,
            "Real".to_owned(),
            real_root.join("library"),
            "https://relay.example/real",
            false,
            false,
        )
        .unwrap();
        let alias = root.path().join("alias");
        symlink(&real_root, &alias).unwrap();
        let nested = alias.join("library/nested");

        assert!(
            create_bridge(
                &paths,
                "Alias".to_owned(),
                nested.clone(),
                "https://relay.example/alias",
                false,
                false,
            )
            .is_err()
        );
        assert!(!nested.exists());
        assert_eq!(paths.registry.load().unwrap().bridges.len(), 1);
    }

    #[test]
    fn invalid_bridge_name_is_rejected_before_touching_the_library() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let library = root.path().join("untouched-library");

        assert!(
            create_bridge(
                &paths,
                " bad name ".to_owned(),
                library.clone(),
                "https://relay.example",
                false,
                false,
            )
            .is_err()
        );
        assert!(!library.exists());
        assert!(paths.registry.load().unwrap().bridges.is_empty());
    }

    #[test]
    fn custom_config_uses_a_sibling_registry_and_scoped_data_directory() {
        let root = tempdir().unwrap();
        let config = root.path().join("nested/../legacy.toml");
        let paths = resolve_desktop_paths(Some(config), None).unwrap();

        assert_eq!(paths.registry.path(), root.path().join("bridges.toml"));
        assert_eq!(paths.bridge_config_dir, root.path().join("bridges"));
        assert_eq!(paths.bridge_data_dir, root.path().join("bridge-data"));
        assert_eq!(
            paths.bootstrap_config,
            Some(root.path().join("legacy.toml"))
        );
    }

    #[test]
    fn stale_worker_cannot_operate_on_an_unregistered_bridge() {
        let root = tempdir().unwrap();
        let paths = test_paths(root.path());
        let config_path = root.path().join("bridge.toml");
        let config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.ensure_directories().unwrap();
        config.save(&config_path, false).unwrap();
        register_config(&paths, "bridge", "Bridge", config_path.clone());
        ensure_bridge_registered(&paths.registry, "bridge", &config_path).unwrap();

        paths
            .registry
            .update(|registry| registry.remove("bridge").map(drop))
            .unwrap();

        assert!(ensure_bridge_registered(&paths.registry, "bridge", &config_path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn editor_refuses_to_replace_a_symlinked_config() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("target.toml");
        let alias = root.path().join("alias.toml");
        let config = Config::defaults(InitOverrides {
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        config.save(&target, false).unwrap();
        symlink(&target, &alias).unwrap();

        assert!(load_editable_config(&alias).is_err());
        assert!(alias.is_symlink());
        assert_eq!(Config::load(&target).unwrap(), config);
    }

    #[test]
    fn byte_counts_are_human_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
    }

    #[test]
    fn bridge_summary_surfaces_the_status_that_needs_attention() {
        let counts = BridgeCounts {
            total: 8,
            ack_pending: 2,
            acknowledged: 6,
            wallpaper_pending: 1,
            wallpaper_attention: 1,
        };
        assert_eq!(
            summary_text(&counts),
            "8 files · 2 awaiting acknowledgement"
        );
    }

    #[test]
    fn bridge_name_comes_from_the_managed_folder() {
        assert_eq!(
            bridge_name_for_path(Path::new("/srv/relay/Illustration")),
            "Illustration"
        );
        assert_eq!(bridge_name_for_path(Path::new("/")), "Root");
    }

    #[test]
    fn ordinary_files_have_a_quiet_success_state() {
        let record = sample_record("notes.txt", "text/plain");
        assert_eq!(
            delivery_state(&record),
            ("emblem-ok-symbolic", "Received", "state-success")
        );
    }
}
