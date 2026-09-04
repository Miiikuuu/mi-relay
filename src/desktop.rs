use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
use crate::sync::{SyncSummary, status_counts, sync_once};

const APPLICATION_ID: &str = "io.mirelay.Desktop";
const AUTO_RECEIVE_INTERVAL_SECONDS: u64 = 60;

const DESKTOP_CSS: &str = r#"
.workspace {
  background: @window_bg_color;
}

.bridge-sidebar {
  background: @view_bg_color;
  border-right: 1px solid alpha(@window_fg_color, 0.10);
}

.sidebar-heading {
  padding: 24px 20px 16px 20px;
}

.sidebar-title {
  font-size: 22px;
  font-weight: 700;
  letter-spacing: -0.2px;
}

.secondary-text,
.bridge-kicker,
.property-title {
  color: alpha(@window_fg_color, 0.60);
}

.bridge-list {
  background: transparent;
  padding: 0 8px 12px 8px;
}

.bridge-list row {
  border-radius: 12px;
  margin: 2px 0;
}

.bridge-list row:hover {
  background: alpha(@window_fg_color, 0.055);
}

.bridge-list row:selected {
  background: alpha(@accent_bg_color, 0.14);
  color: @window_fg_color;
}

.bridge-row {
  padding: 12px;
}

.bridge-icon {
  background: alpha(@accent_bg_color, 0.12);
  color: @accent_color;
  border-radius: 11px;
  padding: 10px;
}

.bridge-name,
.delivery-name,
.activity-title {
  font-weight: 600;
}

.bridge-dot {
  background: @success_color;
  border-radius: 999px;
  min-width: 8px;
  min-height: 8px;
}

.bridge-dot.state-error {
  background: @error_color;
}

.bridge-page {
  padding: 38px 42px 42px 42px;
}

.bridge-kicker {
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.8px;
}

.bridge-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.4px;
}

.bridge-hero-icon {
  background: alpha(@accent_bg_color, 0.11);
  color: @accent_color;
  border-radius: 17px;
  padding: 16px;
}

.bridge-status {
  font-weight: 600;
  color: @success_color;
}

.state-success {
  color: @success_color;
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
  border-radius: 14px;
}

.property-row {
  padding: 12px 15px;
  border-bottom: 1px solid alpha(@window_fg_color, 0.075);
}

.property-row:last-child {
  border-bottom: none;
}

.property-title {
  font-size: 12px;
  font-weight: 600;
}

.property-value {
  font-size: 13px;
}

.activity-header {
  margin-top: 6px;
}

.activity-title {
  font-size: 17px;
}

.activity-list {
  background: @card_bg_color;
  border: 1px solid alpha(@window_fg_color, 0.09);
  border-radius: 14px;
}

.activity-list row {
  border-bottom: 1px solid alpha(@window_fg_color, 0.075);
}

.activity-list row:last-child {
  border-bottom: none;
}

.delivery-row {
  padding: 11px 14px;
}

.mime-icon {
  background: alpha(@accent_bg_color, 0.10);
  color: @accent_color;
  border-radius: 9px;
  padding: 8px;
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
  padding: 24px;
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

    /// Open the window briefly and exit. Used by automated smoke tests.
    #[arg(long, hide = true)]
    smoke_test: bool,

    /// Save a rendered window for visual regression checks and exit.
    #[arg(long, value_name = "PATH", hide = true)]
    screenshot: Option<PathBuf>,

    /// Run automatic receive on a short interval and exit. Used by integration smoke tests.
    #[arg(long, hide = true)]
    automation_smoke_test: bool,
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
    deliveries: Vec<DeliveryRecord>,
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
}

impl BridgeResources {
    fn from_config(config: &Config) -> Self {
        Self {
            library_dir: comparable_path(&config.storage.library_dir),
            state_file: comparable_path(&config.storage.state_file),
            device_id: config.device_id.clone(),
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
    sync_spinner: gtk::Spinner,
    sync_label: gtk::Label,
    add_bridge_button: gtk::Button,
    settings_button: gtk::Button,
    open_library_button: gtk::Button,
    error_revealer: gtk::Revealer,
    error_label: gtk::Label,
    toast_overlay: adw::ToastOverlay,
    bridge_count_label: gtk::Label,
    bridge_list: gtk::ListBox,
    summary_label: gtk::Label,
    content_stack: gtk::Stack,
    empty_page: adw::StatusPage,
    empty_action: gtk::Button,
    bridge_error_page: adw::StatusPage,
    bridge_error_action: gtk::Button,
    bridge_name: gtk::Label,
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
    activity_empty: adw::StatusPage,
    delivery_list: gtk::ListBox,
}

struct DesktopUi {
    paths: DesktopPaths,
    bridges: RefCell<Vec<BridgeView>>,
    bridge_ids: RefCell<Vec<String>>,
    selected_bridge_id: RefCell<Option<String>>,
    tokens: RefCell<HashMap<String, String>>,
    busy: Cell<bool>,
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
        smoke_test,
        screenshot,
        automation_smoke_test,
    } = DesktopArgs::parse();
    let paths = resolve_desktop_paths(config, registry)?;

    let flags = if smoke_test || screenshot.is_some() || automation_smoke_test {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(flags)
        .build();
    application.connect_activate(move |application| {
        install_css();
        build_window(
            application,
            paths.clone(),
            smoke_test,
            screenshot.clone(),
            automation_smoke_test,
        );
    });
    let exit_code = application.run_with_args(&["mirelay-desktop"]);
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
    screenshot_path: Option<PathBuf>,
    automation_smoke_test: bool,
) {
    let (widgets, error_close) = build_widgets(application);
    let (sender, receiver) = mpsc::channel();
    let ui = Rc::new(DesktopUi {
        paths,
        bridges: RefCell::new(Vec::new()),
        bridge_ids: RefCell::new(Vec::new()),
        selected_bridge_id: RefCell::new(None),
        tokens: RefCell::new(HashMap::new()),
        busy: Cell::new(false),
        sender,
        widgets,
    });

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
        button.connect_clicked(move |_| show_bridge_editor(&ui, BridgeEditorMode::New));
    }
    {
        let ui = Rc::clone(&ui);
        let button = ui.widgets.empty_action.clone();
        button.connect_clicked(move |_| show_bridge_editor(&ui, BridgeEditorMode::New));
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
            if let Some(row) = row {
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

    let weak_ui = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_millis(75), move || {
        let Some(ui) = weak_ui.upgrade() else {
            return glib::ControlFlow::Break;
        };
        while let Ok(message) = receiver.try_recv() {
            ui.handle_worker_message(message);
        }
        glib::ControlFlow::Continue
    });

    if !smoke_test && screenshot_path.is_none() {
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

    ui.widgets.window.present();
    ui.start_load_all();

    if let Some(path) = screenshot_path {
        let application = application.clone();
        let window = ui.widgets.window.clone();
        glib::timeout_add_local_once(Duration::from_millis(900), move || {
            if let Err(error) = capture_window(&window, &path) {
                eprintln!("failed to capture desktop window: {error:#}");
            }
            application.quit();
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
        .default_width(1120)
        .default_height(760)
        .width_request(820)
        .height_request(560)
        .build();

    let title = adw::WindowTitle::new("MiRelay", "Folder");
    let header = adw::HeaderBar::builder().title_widget(&title).build();

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

    let sync_spinner = gtk::Spinner::new();
    sync_spinner.set_size_request(16, 16);
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
    sidebar.set_width_request(280);

    let sidebar_heading = gtk::Box::new(gtk::Orientation::Vertical, 5);
    sidebar_heading.add_css_class("sidebar-heading");
    let heading_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let sidebar_title = gtk::Label::builder()
        .label("Folder")
        .xalign(0.0)
        .hexpand(true)
        .build();
    sidebar_title.add_css_class("sidebar-title");
    let bridge_count_label = gtk::Label::builder().label("0").xalign(1.0).build();
    bridge_count_label.add_css_class("secondary-text");
    let add_bridge_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Folder")
        .build();
    add_bridge_button.add_css_class("flat");
    heading_line.append(&sidebar_title);
    heading_line.append(&bridge_count_label);
    heading_line.append(&add_bridge_button);
    let sidebar_subtitle = gtk::Label::builder()
        .label("Managed folders")
        .xalign(0.0)
        .build();
    sidebar_subtitle.add_css_class("secondary-text");
    sidebar_heading.append(&heading_line);
    sidebar_heading.append(&sidebar_subtitle);
    sidebar.append(&sidebar_heading);

    let bridge_list = gtk::ListBox::new();
    bridge_list.add_css_class("bridge-list");
    bridge_list.set_selection_mode(gtk::SelectionMode::Single);
    let bridge_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&bridge_list)
        .build();
    sidebar.append(&bridge_scroll);

    let sidebar_hint = gtk::Label::builder()
        .label("One Folder, one delivery workflow")
        .xalign(0.0)
        .margin_start(20)
        .margin_end(20)
        .margin_top(12)
        .margin_bottom(16)
        .build();
    sidebar_hint.add_css_class("secondary-text");
    sidebar.append(&sidebar_hint);

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
    bridge_icon.set_pixel_size(36);
    bridge_icon.add_css_class("bridge-hero-icon");
    bridge_icon.set_valign(gtk::Align::Start);
    let bridge_name = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .wrap(true)
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
    let bridge_titles = gtk::Box::new(gtk::Orientation::Vertical, 5);
    bridge_titles.set_hexpand(true);
    bridge_titles.append(&bridge_name);
    bridge_titles.append(&bridge_path);
    let open_library_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Open folder")
        .sensitive(false)
        .valign(gtk::Align::Center)
        .build();
    let bridge_heading = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    bridge_heading.append(&bridge_icon);
    bridge_heading.append(&bridge_titles);
    bridge_heading.append(&open_library_button);

    let bridge_state_icon = gtk::Image::from_icon_name("network-transmit-receive-symbolic");
    bridge_state_icon.set_pixel_size(16);
    bridge_state_icon.add_css_class("state-success");
    let bridge_state = gtk::Label::builder().label("Ready").xalign(0.0).build();
    bridge_state.add_css_class("bridge-status");
    let summary_label = gtk::Label::builder().label("Loading…").xalign(0.0).build();
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
        .label("Recent Transfers")
        .xalign(0.0)
        .hexpand(true)
        .build();
    activity_title.add_css_class("activity-title");
    let activity_caption = gtk::Label::builder()
        .label("Folder activity")
        .xalign(1.0)
        .build();
    activity_caption.add_css_class("secondary-text");
    let activity_header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    activity_header.add_css_class("activity-header");
    activity_header.append(&activity_title);
    activity_header.append(&activity_caption);

    let delivery_list = gtk::ListBox::new();
    delivery_list.add_css_class("activity-list");
    delivery_list.set_selection_mode(gtk::SelectionMode::None);
    let activity_empty = adw::StatusPage::builder()
        .icon_name("document-send-symbolic")
        .title("No transfers yet")
        .description("Received files will appear here.")
        .build();
    activity_empty.set_height_request(210);
    let activity_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    activity_stack.add_named(&activity_empty, Some("empty"));
    activity_stack.add_named(&delivery_list, Some("list"));
    activity_stack.set_visible_child_name("empty");

    let bridge_kicker = gtk::Label::builder()
        .label("MIRELAY FOLDER")
        .xalign(0.0)
        .build();
    bridge_kicker.add_css_class("bridge-kicker");
    let bridge_page = gtk::Box::new(gtk::Orientation::Vertical, 22);
    bridge_page.add_css_class("bridge-page");
    bridge_page.append(&bridge_kicker);
    bridge_page.append(&bridge_heading);
    bridge_page.append(&state_line);
    bridge_page.append(&property_group);
    bridge_page.append(&activity_header);
    bridge_page.append(&activity_stack);

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
    workspace.set_position(300);
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
            sync_spinner,
            sync_label,
            add_bridge_button,
            settings_button,
            open_library_button,
            error_revealer,
            error_label,
            toast_overlay,
            bridge_count_label,
            bridge_list,
            summary_label,
            content_stack,
            empty_page,
            empty_action,
            bridge_error_page,
            bridge_error_action,
            bridge_name,
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
        },
        error_close,
    )
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
        if self.busy.get() {
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
        std::thread::spawn(move || {
            let outcomes = execute_sync_requests(&registry, requests);
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
        self.widgets.sync_label.set_label(label);
        self.widgets
            .sync_indicator
            .set_visible_child_name("spinner");
        self.widgets.sync_spinner.start();
        true
    }

    fn finish_task(&self) {
        self.busy.set(false);
        self.widgets.sync_label.set_label("Receive");
        self.widgets.sync_spinner.stop();
        self.widgets.sync_indicator.set_visible_child_name("icon");
        self.update_action_sensitivity();
    }

    fn handle_worker_message(&self, message: WorkerMessage) {
        self.finish_task();
        match message {
            WorkerMessage::Loaded(Ok(snapshot)) => {
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
            match outcome.result {
                Ok(result) => {
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
                Err(error) => failures.push(format!("{}: {error}", outcome.bridge_name)),
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
        }
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
                self.selected_bridge_id.replace(Some(bridge_id));
                self.render_current_bridge();
                self.hide_error();
            }
            Err(error) => {
                self.show_error(&format!("Could not save Folder selection: {error:#}"));
                self.select_current_row();
            }
        }
    }

    fn update_bridge_snapshot(&self, bridge_id: &str, mut snapshot: BridgeSnapshot) {
        if let Some(bridge) = self
            .bridges
            .borrow_mut()
            .iter_mut()
            .find(|bridge| bridge.registration.id == bridge_id)
        {
            snapshot.name.clone_from(&bridge.registration.name);
            bridge.snapshot = Some(snapshot);
            bridge.error = None;
        }
    }

    fn render_bridge_list(&self) {
        clear_list_box(&self.widgets.bridge_list);
        let bridges = self.bridges.borrow();
        self.bridge_ids.replace(
            bridges
                .iter()
                .map(|bridge| bridge.registration.id.clone())
                .collect(),
        );
        self.widgets
            .bridge_count_label
            .set_label(&bridges.len().to_string());
        for bridge in bridges.iter() {
            self.widgets.bridge_list.append(&bridge_row(bridge));
        }
        drop(bridges);

        if self.bridges.borrow().is_empty() {
            self.selected_bridge_id.replace(None);
            self.render_empty_state();
            return;
        }
        if self.selected_bridge().is_none() {
            let first = self.bridges.borrow()[0].registration.id.clone();
            self.selected_bridge_id.replace(Some(first));
        }
        self.select_current_row();
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
        self.widgets.bridge_count_label.set_label("0");
        clear_list_box(&self.widgets.delivery_list);
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
        self.widgets.title.set_subtitle("Folder");
        self.widgets.bridge_name.set_label(&snapshot.name);
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
            .summary_label
            .set_label(&summary_text(&snapshot.counts));
        clear_list_box(&self.widgets.delivery_list);
        for record in snapshot.deliveries.iter().take(12) {
            self.widgets.delivery_list.append(&delivery_row(record));
        }
        if snapshot.deliveries.is_empty() {
            self.widgets
                .activity_empty
                .set_description(Some("Select Receive to see this Folder's activity here."));
            self.widgets.activity_stack.set_visible_child_name("empty");
        } else {
            self.widgets.activity_stack.set_visible_child_name("list");
        }
        self.widgets.content_stack.set_visible_child_name("bridge");
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
        self.widgets.sync_button.set_sensitive(idle && valid);
        self.widgets.open_library_button.set_sensitive(valid);
    }
}

#[derive(Debug, Clone)]
enum BridgeEditorMode {
    New,
    Edit(String),
}

fn show_bridge_editor(ui: &Rc<DesktopUi>, mode: BridgeEditorMode) {
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
        return;
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
    let dialog_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(dialog_title, "One Folder, one delivery workflow");
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

    let form = gtk::Box::new(gtk::Orientation::Vertical, 20);
    form.add_css_class("settings-page");

    let folder_group = adw::PreferencesGroup::builder()
        .title("Folder")
        .description("Choose the local folder MiRelay will manage.")
        .build();
    let name_entry = gtk::Entry::builder()
        .text(proposed_name)
        .placeholder_text("Uses the folder name when empty")
        .width_chars(30)
        .hexpand(true)
        .build();
    let name_row = adw::ActionRow::builder()
        .title("Display Name")
        .subtitle("Shown in the Folder list")
        .build();
    name_row.add_suffix(&name_entry);
    folder_group.add(&name_row);
    let folder_entry = gtk::Entry::builder()
        .text(proposed_library_dir.to_string_lossy())
        .editable(is_new_bridge)
        .width_chars(30)
        .hexpand(true)
        .build();
    let folder_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Choose folder")
        .sensitive(is_new_bridge)
        .valign(gtk::Align::Center)
        .build();
    let folder_row = adw::ActionRow::builder()
        .title("Folder Path")
        .subtitle(if is_new_bridge {
            "Files received by this Folder are stored here"
        } else {
            "The folder path cannot be changed after creation"
        })
        .build();
    folder_row.add_suffix(&folder_entry);
    folder_row.add_suffix(&folder_button);
    folder_group.add(&folder_row);

    let connection_group = adw::PreferencesGroup::builder()
        .title("Connection")
        .description("Connect this Folder to your MiRelay server.")
        .build();
    let url_entry = gtk::Entry::builder()
        .text(existing_url)
        .placeholder_text("https://relay.example.com")
        .width_chars(28)
        .build();
    url_entry.set_input_purpose(gtk::InputPurpose::Url);
    let url_row = adw::ActionRow::builder().title("Server URL").build();
    url_row.add_suffix(&url_entry);
    let token_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Optional")
        .show_peek_icon(true)
        .width_chars(28)
        .build();
    if let Some(token) = registration
        .as_ref()
        .and_then(|bridge| ui.tokens.borrow().get(&bridge.id).cloned())
    {
        token_entry.set_text(&token);
    }
    let token_row = adw::ActionRow::builder()
        .title("Session Token")
        .subtitle(format!("Leave empty to read {token_env}"))
        .build();
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

    let automation_group = adw::PreferencesGroup::builder()
        .title("Automation")
        .description("Choose whether MiRelay checks this Folder in the background.")
        .build();
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
            let result = match &mode {
                BridgeEditorMode::New => create_bridge(
                    &ui.paths,
                    bridge_name,
                    library_dir,
                    &server_url,
                    insecure.is_active(),
                    auto_receive.is_active(),
                ),
                BridgeEditorMode::Edit(id) => update_bridge(
                    &ui.paths,
                    id,
                    bridge_name,
                    &server_url,
                    insecure.is_active(),
                    auto_receive.is_active(),
                ),
            };
            match result {
                Ok(bridge_id) => {
                    let token = token_entry.text().trim().to_owned();
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
        });
    }
    dialog.present();
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

fn sync_registered_bridge(
    registry: &BridgeRegistryStore,
    bridge_id: &str,
    config_path: &Path,
    token: Option<&str>,
) -> Result<SyncResult> {
    let config = load_registered_config(registry, bridge_id, config_path)?;
    let source = source_for(&config, token)?;
    let summary = sync_once(&config, source.as_ref())?;
    let snapshot = snapshot_from_config(config)?;
    Ok(SyncResult { summary, snapshot })
}

fn execute_sync_requests(
    registry: &BridgeRegistryStore,
    requests: Vec<SyncRequest>,
) -> Vec<SyncOutcome> {
    requests
        .into_iter()
        .map(|request| {
            let result = sync_registered_bridge(
                registry,
                &request.bridge_id,
                &request.config_path,
                request.token.as_deref(),
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
        deliveries,
    })
}

#[cfg(test)]
fn sync_configured(config_path: &Path, token: Option<&str>) -> Result<SyncResult> {
    if !config_path.exists() {
        bail!("This Folder is not configured. Open Folder Settings first.");
    }
    let config = load_config_for_read(config_path)?;
    let source = source_for(&config, token)?;
    let summary = sync_once(&config, source.as_ref())?;
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

fn delivery_row(record: &DeliveryRecord) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_tooltip_text(Some(&record.stored_path.to_string_lossy()));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.add_css_class("delivery-row");
    let icon = gtk::Image::from_icon_name(icon_for_media_type(&record.media_type));
    icon.set_pixel_size(20);
    icon.add_css_class("mime-icon");
    content.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&record.original_name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("delivery-name");
    let details = gtk::Label::builder()
        .label(format!(
            "{}  ·  {}",
            format_bytes(record.size),
            format_timestamp(record.received_at_unix)
        ))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    details.add_css_class("secondary-text");
    text.append(&name);
    text.append(&details);
    content.append(&text);

    let (state_icon, state_text, state_class) = delivery_state(record);
    let state_image = gtk::Image::from_icon_name(state_icon);
    state_image.set_pixel_size(13);
    state_image.add_css_class(state_class);
    let state_label = gtk::Label::new(Some(state_text));
    state_label.add_css_class("row-state");
    state_label.add_css_class(state_class);
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    state.set_valign(gtk::Align::Center);
    state.append(&state_image);
    state.append(&state_label);
    content.append(&state);

    row.set_child(Some(&content));
    row
}

fn bridge_row(bridge: &BridgeView) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(true);
    row.set_selectable(true);
    row.set_tooltip_text(Some(&bridge.registration.config_path.to_string_lossy()));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.add_css_class("bridge-row");
    let icon = gtk::Image::from_icon_name("folder-symbolic");
    icon.set_pixel_size(22);
    icon.add_css_class("bridge-icon");
    content.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&bridge.registration.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("bridge-name");
    let details_text = if bridge.error.is_some() {
        "Configuration needs attention".to_owned()
    } else {
        bridge
            .snapshot
            .as_ref()
            .map(|snapshot| summary_text(&snapshot.counts))
            .unwrap_or_else(|| "Loading…".to_owned())
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

    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("bridge-dot");
    if bridge.error.is_some() {
        dot.add_css_class("state-error");
    }
    dot.set_size_request(8, 8);
    dot.set_halign(gtk::Align::Center);
    dot.set_valign(gtk::Align::Center);
    content.append(&dot);
    row.set_child(Some(&content));
    row
}

fn property_row(title: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.add_css_class("property-row");
    let title = gtk::Label::builder().label(title).xalign(0.0).build();
    title.add_css_class("property-title");
    let value = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .hexpand(true)
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

fn capture_window(window: &adw::ApplicationWindow, path: &Path) -> Result<()> {
    let width = window.width();
    let height = window.height();
    if width <= 0 || height <= 0 {
        bail!("window has not been laid out yet");
    }
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot
        .to_node()
        .context("window snapshot produced no render node")?;
    let surface = window.surface().context("window has no native surface")?;
    let renderer = gtk::gsk::Renderer::for_surface(&surface)
        .context("no renderer is available for the window surface")?;
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(path)
        .with_context(|| format!("failed to save {}", path.display()))?;
    renderer.unrealize();
    Ok(())
}

fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(DESKTOP_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
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
