mod client;
mod config;
mod daemon;
mod ipc;
mod plugins;
mod search;
mod ui;

use clap::{Parser, Subcommand};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "slaunch", version, about = "Wayland app launcher daemon")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the background daemon (app scanner + IPC socket + UI)
    Daemon,
    /// Show the launcher window
    Show,
    /// Hide the launcher window
    Hide,
    /// Reload config and rescan apps without restarting
    Reload,
    /// Shut down the daemon
    Kill,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("slaunch=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon => run_daemon(),
        cmd => run_client(cmd),
    }
}

// ── Daemon entry point ────────────────────────────────────────────────────────

fn run_daemon() {
    let cfg = config::load();
    tracing::info!("Starting slaunch daemon");

    // Scan entries synchronously before starting the event loop so the
    // first Show is instant.
    let entries = daemon::scan_entries(&cfg);
    let state = Arc::new(daemon::DaemonState::new(cfg));
    // Populate entry cache synchronously before the event loop starts.
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let state_ref = state.clone();
        rt.block_on(async move {
            *state_ref.entries.write().await = entries;
        });
    }

    // Channel from the IPC socket task into the Iced subscription
    let (tx, rx) = mpsc::channel::<daemon::DaemonEvent>(32);

    // Spawn the tokio IPC socket listener on a background thread
    let state_clone = state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            if let Err(e) = daemon::run_socket(state_clone, tx).await {
                tracing::error!("IPC socket error: {e}");
            }
        });
    });

    // Run the Iced application on the main thread (required by most Wayland
    // compositors and by iced_layershell).
    if let Err(e) = ui::run(state, rx) {
        tracing::error!("UI error: {e}");
        std::process::exit(1);
    }
}

// ── Client entry point ────────────────────────────────────────────────────────

fn run_client(cmd: Commands) {
    let ipc_cmd = match cmd {
        Commands::Show => ipc::Command::Show,
        Commands::Hide => ipc::Command::Hide,
        Commands::Reload => ipc::Command::Reload,
        Commands::Kill => ipc::Command::Kill,
        Commands::Daemon => unreachable!(),
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    if let Err(e) = rt.block_on(client::run(ipc_cmd)) {
        eprintln!("slaunch: {e}");
        std::process::exit(1);
    }
}
