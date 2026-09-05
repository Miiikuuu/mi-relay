//! Experimental, explicit CLI entry point. Existing GUI/Auto configurations
//! never silently opt in to directory synchronization.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn main() -> anyhow::Result<()> {
    use clap::{Parser, Subcommand};
    use mirelay::directory::{client::DirectoryClient, receiver::Receiver, sender::Sender};
    use std::path::PathBuf;
    #[derive(Parser)]
    #[command(about = "Experimental paired one-way directory synchronization")]
    struct Cli {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        server_url: String,
        #[arg(long, default_value = "MIRELAY_TOKEN")]
        token_env: String,
        #[arg(long)]
        allow_insecure_http: bool,
        #[command(subcommand)]
        command: Command,
    }
    #[derive(Subcommand)]
    enum Command {
        /// Publish the existing Linux directory and receive pending versions.
        Receive,
        /// Read-only comparison; uploads nothing. Stores a reviewable preview.
        Preview,
        /// Accept the unchanged preview. Does not upload until send is run.
        Initialize {
            #[arg(long, required = true)]
            confirm: bool,
        },
        /// Send new/modified files; resume pending uploads. Never delete files.
        Send,
        /// Inspect remote receipt and conflict status without downloading.
        Status,
    }
    let cli = Cli::parse();
    let token = std::env::var(&cli.token_env)
        .map_err(|_| anyhow::anyhow!("Credential environment variable is not set."))?;
    let role = if matches!(cli.command, Command::Receive) {
        "receiver"
    } else {
        "sender"
    };
    let client = DirectoryClient::new(&cli.server_url, &token, cli.allow_insecure_http, role)?;
    match cli.command {
        Command::Receive => {
            let source = mirelay::http_source::HttpSource::new(
                &cli.server_url,
                &token,
                30,
                100,
                cli.allow_insecure_http,
            )?;
            let mut receiver = Receiver::open(&cli.directory, &cli.state_dir, &cli.server_url)?;
            let count = receiver.sync(&client, &source)?;
            println!("Received {count} version(s). Conflicts and retained copies:");
            println!("{}", serde_json::to_string_pretty(receiver.files())?);
        }
        Command::Status => println!("{}", serde_json::to_string_pretty(&client.state()?)?),
        command => {
            let mut sender = Sender::open(&cli.directory, &cli.state_dir, &cli.server_url)?;
            match command {
                Command::Preview => println!(
                    "{}",
                    serde_json::to_string_pretty(&sender.preview(&client)?.comparison)?
                ),
                Command::Initialize { confirm } => {
                    anyhow::ensure!(confirm, "Explicit confirmation is required.");
                    sender.initialize(&client)?;
                    println!("Initialized. Run send to transfer missing or modified files.");
                }
                Command::Send => println!(
                    "Uploaded {} version(s); server upload is not yet a Linux receipt.",
                    sender.send(&client, &token, cli.allow_insecure_http)?
                ),
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn main() {
    eprintln!("The experimental directory CLI currently requires Linux/glibc.");
    std::process::exit(1);
}
