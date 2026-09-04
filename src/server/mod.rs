mod api;
mod store;
mod tus_api;
mod tus_store;

use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

pub use api::{ApiState, router};
pub use store::{ReconcileReport, ServerStats, ServerStore, StoredDelivery};
pub use tus_store::{AppendUploadOutcome, NewUpload, UploadInfo};

const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;
const DEFAULT_TOKEN_ENV: &str = "MIRELAY_SERVER_TOKEN";

#[derive(Debug, Parser)]
#[command(
    name = "mirelay-server",
    version,
    about = "Self-hosted MiRelay delivery server"
)]
struct ServerCli {
    /// Server data directory. Defaults to the XDG data directory.
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Device whose pending queue is managed by this server instance.
    #[arg(long, global = true, default_value = "linux", value_name = "ID")]
    device_id: String,

    /// Maximum accepted file size in bytes.
    #[arg(
        long,
        global = true,
        default_value_t = DEFAULT_MAX_FILE_SIZE_BYTES,
        value_name = "BYTES"
    )]
    max_file_size_bytes: u64,

    #[command(subcommand)]
    command: ServerCommand,
}

#[derive(Debug, Subcommand)]
enum ServerCommand {
    /// Run the HTTP delivery service.
    Serve(ServeArgs),
    /// Add a local file to a device's pending queue.
    Enqueue(EnqueueArgs),
    /// Show pending and acknowledged delivery counts.
    Status,
    /// Verify referenced objects and remove unreferenced MiRelay content.
    Reconcile,
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// TCP address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080", value_name = "ADDRESS")]
    listen: SocketAddr,

    /// Read the device bearer token from this environment variable.
    #[arg(long, default_value = DEFAULT_TOKEN_ENV, value_name = "NAME")]
    token_env: String,

    /// Permit an unencrypted listener on a non-loopback address.
    #[arg(long)]
    allow_public_http: bool,
}

#[derive(Debug, Args)]
struct EnqueueArgs {
    /// File to stage for delivery.
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Original file name exposed to the receiving device.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
}

pub async fn run() -> Result<()> {
    run_cli(ServerCli::parse()).await
}

async fn run_cli(cli: ServerCli) -> Result<()> {
    store::validate_device_id(&cli.device_id)?;
    let data_dir = match cli.data_dir {
        Some(path) => absolute_path(path)?,
        None => default_server_data_dir()?,
    };
    let store = ServerStore::new(data_dir.clone(), cli.max_file_size_bytes)?;
    store.initialize()?;

    match cli.command {
        ServerCommand::Serve(args) => serve(store, cli.device_id, args).await,
        ServerCommand::Enqueue(args) => {
            let file = absolute_path(args.file)?;
            let original_name = match args.name {
                Some(name) => name,
                None => file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("file name is not valid UTF-8; pass --name")?
                    .to_owned(),
            };
            let delivery = store.enqueue(&cli.device_id, &file, original_name)?;
            println!("Enqueued MiRelay delivery {}", delivery.id);
            println!("  device: {}", cli.device_id);
            println!("  sha256: {}", delivery.sha256);
            println!("  size:   {}", delivery.size);
            println!("  type:   {}", delivery.media_type);
            Ok(())
        }
        ServerCommand::Status => {
            let stats = store.stats(&cli.device_id)?;
            println!("MiRelay server status");
            println!("  data:         {}", safe_path(&data_dir));
            println!("  device:       {}", cli.device_id);
            println!("  pending:      {}", stats.pending);
            println!("  acknowledged: {}", stats.acknowledged);
            Ok(())
        }
        ServerCommand::Reconcile => {
            let report = store.reconcile_content()?;
            print_reconcile_report(&report);
            if report.missing_objects != 0 || report.corrupt_objects != 0 {
                bail!(
                    "content reconciliation found {} missing and {} corrupt referenced object(s)",
                    report.missing_objects,
                    report.corrupt_objects
                );
            }
            Ok(())
        }
    }
}

async fn serve(store: ServerStore, device_id: String, args: ServeArgs) -> Result<()> {
    let reconciliation = store.reconcile_content()?;
    if reconciliation.removed_unreferenced_objects != 0
        || reconciliation.stale_staging_files_removed != 0
    {
        eprintln!(
            "server storage cleanup removed {} unreferenced object(s) and {} stale staging file(s)",
            reconciliation.removed_unreferenced_objects, reconciliation.stale_staging_files_removed
        );
    }
    if reconciliation.missing_objects != 0 || reconciliation.corrupt_objects != 0 {
        eprintln!(
            "warning: server storage has {} missing and {} corrupt referenced object(s); affected downloads will fail integrity checks",
            reconciliation.missing_objects, reconciliation.corrupt_objects
        );
    }
    validate_environment_variable_name(&args.token_env)?;
    if !args.listen.ip().is_loopback() && !args.allow_public_http {
        bail!(
            "refusing an unencrypted non-loopback listener; bind to loopback behind an HTTPS reverse proxy or pass --allow-public-http for isolated testing"
        );
    }
    let token = env::var(&args.token_env).with_context(|| {
        format!(
            "server token environment variable {} is not set or is not valid UTF-8",
            args.token_env
        )
    })?;
    let state = ApiState::new(store, device_id.clone(), &token)?;
    drop(token);
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind MiRelay server to {}", args.listen))?;
    let local_address = listener
        .local_addr()
        .context("failed to determine MiRelay server listen address")?;

    println!("MiRelay server listening on http://{local_address}");
    println!("  device:    {device_id}");
    println!("  health:    http://{local_address}/healthz");
    if !local_address.ip().is_loopback() {
        eprintln!(
            "warning: the server is publicly reachable over unencrypted HTTP; do not use this mode outside an isolated test network"
        );
    }

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("MiRelay HTTP server stopped unexpectedly")
}

fn print_reconcile_report(report: &ReconcileReport) {
    println!("MiRelay server content reconciliation");
    println!("  referenced:           {}", report.referenced_objects);
    println!("  healthy:              {}", report.healthy_objects);
    println!("  missing:              {}", report.missing_objects);
    println!("  corrupt:              {}", report.corrupt_objects);
    println!(
        "  unreferenced removed: {}",
        report.removed_unreferenced_objects
    );
    println!(
        "  stale staging removed: {}",
        report.stale_staging_files_removed
    );
    println!("  unexpected retained:  {}", report.unexpected_entries);
}

async fn shutdown_signal() {
    if let Err(error) = wait_for_shutdown_signal().await {
        eprintln!(
            "failed to install a graceful shutdown handler: {}",
            crate::cli::terminal_safe(&format!("{error:#}"))
        );
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).context("failed to listen for SIGTERM")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.context("failed to listen for Ctrl-C")?;
        }
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for Ctrl-C")
}

fn validate_environment_variable_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("token environment variable name is invalid");
    }
    Ok(())
}

fn default_server_data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return absolute_path(PathBuf::from(path).join("mirelay-server"));
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context(
            "cannot determine server data directory; set HOME, XDG_DATA_HOME, or --data-dir",
        )?;
    absolute_path(home.join(".local/share/mirelay-server"))
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("failed to determine the current directory")?
            .join(path))
    }
}

fn safe_path(path: &Path) -> String {
    crate::cli::terminal_safe(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn server_cli_definition_is_consistent() {
        ServerCli::command().debug_assert();
    }
}
