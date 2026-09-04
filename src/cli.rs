use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::client::source_for;
use crate::config::{
    Config, DEFAULT_HTTP_PAGE_SIZE, DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS,
    DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS, DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
    DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS, DEFAULT_HTTP_TOKEN_ENV, InitOverrides, ServerConfig,
    default_config_path,
};
use crate::source::FilesystemSource;
use crate::state::StateStore;
use crate::sync::{self, RetrySummary, SyncSummary, status_counts};

#[derive(Debug, Parser)]
#[command(
    name = "mirelay",
    version,
    about = "Reliably receive files, with an optional Linux wallpaper workflow"
)]
struct Cli {
    /// Use a specific configuration file.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Create the configuration and data directories.
    Init(InitArgs),
    /// Receive pending files, acknowledge them, and run the wallpaper hook for images.
    Sync,
    /// Show delivery and wallpaper queue status.
    Status,
    /// List locally stored deliveries.
    List,
    /// Retry failed or deferred wallpaper imports.
    Retry(RetryArgs),
    /// Utilities for the local filesystem server used during development.
    Mock(MockArgs),
}

#[derive(Debug, Args)]
struct InitArgs {
    /// Replace an existing configuration file.
    #[arg(long)]
    force: bool,

    /// Override the root data directory.
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Override the content-addressed media library directory.
    #[arg(long, value_name = "PATH")]
    library_dir: Option<PathBuf>,

    /// Override the local mock server inbox directory.
    #[arg(long, value_name = "PATH")]
    inbox_dir: Option<PathBuf>,

    /// Configure a real MiRelay HTTP server instead of the local mock inbox.
    #[arg(long, value_name = "URL", conflicts_with = "inbox_dir")]
    server_url: Option<String>,

    /// Read the HTTP bearer token from this environment variable.
    #[arg(long, default_value = DEFAULT_HTTP_TOKEN_ENV, value_name = "NAME")]
    token_env: String,

    /// HTTP request timeout in seconds.
    #[arg(long, default_value_t = DEFAULT_HTTP_REQUEST_TIMEOUT_SECONDS)]
    request_timeout_seconds: u64,

    /// Number of pending deliveries requested per HTTP page.
    #[arg(long, default_value_t = DEFAULT_HTTP_PAGE_SIZE)]
    page_size: u32,

    /// Permit plain HTTP. Intended only for trusted local development.
    #[arg(long)]
    allow_insecure_http: bool,
}

#[derive(Debug, Args)]
struct RetryArgs {
    /// Retry imports whose previous outcome is unknown; this may repeat side effects.
    #[arg(long)]
    include_uncertain: bool,
}

#[derive(Debug, Args)]
struct MockArgs {
    #[command(subcommand)]
    command: MockCommands,
}

#[derive(Debug, Subcommand)]
enum MockCommands {
    /// Publish a file into the local mock server inbox.
    Enqueue {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

pub fn run() -> Result<()> {
    run_cli(Cli::parse())
}

fn run_cli(cli: Cli) -> Result<()> {
    let config_path = match cli.config {
        Some(path) => path,
        None => default_config_path()?,
    };
    let config_path = if config_path.is_absolute() {
        config_path
    } else {
        std::env::current_dir()
            .map_err(anyhow::Error::from)?
            .join(config_path)
    };

    match cli.command {
        Commands::Init(args) => init(&config_path, args),
        Commands::Sync => {
            let config = load_config(&config_path)?;
            let source = source_for(&config, None)?;
            let summary = sync::sync_once(&config, source.as_ref())?;
            print_sync_summary(&summary);
            if !summary.failures.is_empty() {
                bail!("sync completed with {} error(s)", summary.failures.len());
            }
            Ok(())
        }
        Commands::Status => status(&config_path),
        Commands::List => list(&config_path),
        Commands::Retry(args) => {
            let config = load_config(&config_path)?;
            let summary = sync::retry_wallpapers(&config, args.include_uncertain)?;
            print_retry_summary(&summary);
            if !summary.failures.is_empty() {
                bail!("retry completed with {} error(s)", summary.failures.len());
            }
            Ok(())
        }
        Commands::Mock(args) => mock_command(&config_path, args),
    }
}

fn init(config_path: &Path, args: InitArgs) -> Result<()> {
    let mut config = Config::defaults(InitOverrides {
        data_dir: args.data_dir,
        library_dir: args.library_dir,
        inbox_dir: args.inbox_dir,
    })?;
    if let Some(base_url) = args.server_url {
        config.server = ServerConfig::Http {
            base_url,
            token_env: args.token_env,
            request_timeout_seconds: args.request_timeout_seconds,
            page_size: args.page_size,
            retry_max_attempts: DEFAULT_HTTP_RETRY_MAX_ATTEMPTS,
            retry_base_delay_milliseconds: DEFAULT_HTTP_RETRY_BASE_DELAY_MILLISECONDS,
            retry_max_delay_milliseconds: DEFAULT_HTTP_RETRY_MAX_DELAY_MILLISECONDS,
            allow_insecure_http: args.allow_insecure_http,
        };
    }
    config.ensure_directories()?;
    config.save(config_path, args.force)?;

    let store = StateStore::new(config.storage.state_file.clone());
    let _lock = store.lock_exclusive()?;
    if !store.path().exists() {
        store.save(&Default::default())?;
    }

    println!("Initialized MiRelay Linux CLI");
    println!("  config:  {}", safe_path(config_path));
    match &config.server {
        ServerConfig::Filesystem { inbox_dir } => {
            println!("  source:  filesystem ({})", safe_path(inbox_dir));
        }
        ServerConfig::Http {
            base_url,
            token_env,
            ..
        } => {
            println!("  source:  HTTP ({})", terminal_safe(base_url));
            println!(
                "  token:   environment variable {}",
                terminal_safe(token_env)
            );
        }
    }
    println!("  library: {}", safe_path(&config.storage.library_dir));
    println!();
    println!("Wallpaper import is disabled until wallpaper.command is configured.");
    Ok(())
}

fn status(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    let store = StateStore::new(config.storage.state_file.clone());
    let state = {
        let _lock = store.lock_shared()?;
        store.load()?
    };
    let counts = status_counts(&state);
    let source = source_for(&config, None)?;
    let scan = source.scan_pending(config.limits.max_file_size_bytes)?;

    println!("MiRelay status");
    println!(
        "  device:                  {}",
        terminal_safe(&config.device_id)
    );
    println!("  config:                  {}", safe_path(config_path));
    println!(
        "  library:                 {}",
        safe_path(&config.storage.library_dir)
    );
    println!("  source pending:          {}", scan.deliveries.len());
    println!("  source issues:           {}", scan.issues.len());
    println!("  stored total:            {}", counts["total"]);
    println!("  delivery ack pending:    {}", counts["ack_pending"]);
    println!("  delivery acknowledged:   {}", counts["acknowledged"]);
    println!("  wallpaper pending:       {}", counts["wallpaper_pending"]);
    println!(
        "  wallpaper not applicable: {}",
        counts["wallpaper_not_applicable"]
    );
    println!("  wallpaper applied:       {}", counts["wallpaper_applied"]);
    println!("  wallpaper failed:        {}", counts["wallpaper_failed"]);
    println!(
        "  wallpaper uncertain:     {}",
        counts["wallpaper_uncertain"]
    );
    println!(
        "  wallpaper not configured: {}",
        counts["wallpaper_not_configured"]
    );
    if !scan.issues.is_empty() {
        println!();
        println!("Source issues:");
        for issue in scan.issues {
            println!(
                "  {}: {}",
                terminal_safe(&issue.item),
                terminal_safe(&issue.message)
            );
        }
    }
    Ok(())
}

fn list(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    let store = StateStore::new(config.storage.state_file.clone());
    let _lock = store.lock_shared()?;
    let state = store.load()?;

    if state.deliveries.is_empty() {
        println!("No deliveries have been stored.");
        return Ok(());
    }
    println!("ID\tDELIVERY\tWALLPAPER\tORIGINAL\tPATH");
    let mut records = state.deliveries.values().collect::<Vec<_>>();
    records.sort_by_key(|record| {
        (
            record
                .source_created_at_unix
                .unwrap_or(record.received_at_unix),
            &record.id,
        )
    });
    for record in records {
        println!(
            "{}\t{:?}\t{:?}\t{}\t{}",
            record.id,
            record.delivery_status,
            record.wallpaper_status,
            terminal_safe(&record.original_name),
            safe_path(&record.stored_path)
        );
        if let Some(error) = record.delivery_error.as_deref() {
            println!("  delivery error: {}", terminal_safe(error));
        }
        if let Some(error) = record.wallpaper_error.as_deref() {
            println!("  wallpaper error: {}", terminal_safe(error));
        }
    }
    Ok(())
}

fn mock_command(config_path: &Path, args: MockArgs) -> Result<()> {
    let config = load_config(config_path)?;
    let source = match &config.server {
        ServerConfig::Filesystem { inbox_dir } => FilesystemSource::new(inbox_dir.clone()),
        ServerConfig::Http { .. } => {
            bail!("mock commands are unavailable when server.kind is http")
        }
    };
    match args.command {
        MockCommands::Enqueue { file } => {
            let delivery = source.enqueue(&file, config.limits.max_file_size_bytes)?;
            println!("Published mock delivery {}", delivery.id);
            println!("  file:   {}", safe_path(&file));
            println!("  sha256: {}", delivery.sha256);
            Ok(())
        }
    }
}

fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        bail!(
            "MiRelay is not initialized; run `mirelay --config {} init` first",
            path.display()
        );
    }
    Config::load(path)
}

fn print_sync_summary(summary: &SyncSummary) {
    println!("Sync complete");
    println!(
        "  stale staging removed:    {}",
        summary.stale_staging_files_removed
    );
    println!("  received:                 {}", summary.received);
    println!("  repaired:                 {}", summary.repaired);
    println!("  already stored:           {}", summary.already_stored);
    println!("  acknowledged:             {}", summary.acknowledged);
    println!("  wallpaper applied:        {}", summary.wallpaper_applied);
    println!(
        "  wallpaper not applicable: {}",
        summary.wallpaper_not_applicable
    );
    println!(
        "  wallpaper not configured: {}",
        summary.wallpaper_not_configured
    );
    print_failures(&summary.failures);
}

fn print_retry_summary(summary: &RetrySummary) {
    println!("Wallpaper retry complete");
    println!("  attempted:                {}", summary.attempted);
    println!("  applied:                  {}", summary.applied);
    println!("  not configured:           {}", summary.not_configured);
    println!("  uncertain skipped:        {}", summary.skipped_uncertain);
    print_failures(&summary.failures);
}

fn print_failures(failures: &[sync::SyncFailure]) {
    if failures.is_empty() {
        return;
    }
    println!("  errors:                   {}", failures.len());
    for failure in failures {
        println!(
            "    {}: {}",
            terminal_safe(&failure.item),
            terminal_safe(&failure.message)
        );
    }
}

pub fn terminal_safe(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn safe_path(path: &Path) -> String {
    terminal_safe(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WallpaperStatus;

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn uncertain_status_variant_remains_visible() {
        assert_eq!(format!("{:?}", WallpaperStatus::Uncertain), "Uncertain");
    }

    #[test]
    fn terminal_control_characters_are_escaped() {
        assert_eq!(terminal_safe("safe\n\u{1b}"), "safe\\n\\u{1b}");
    }
}
