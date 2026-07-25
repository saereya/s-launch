mod client;
mod config;
mod daemon;
mod ipc;
mod plugins;
mod search;
#[cfg(test)]
mod test_env;
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("slaunch=info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon => {
            if let Err(e) = run_daemon() {
                eprintln!("slaunch: {e}");
                std::process::exit(1);
            }
        }
        cmd => run_client(cmd),
    }
}

// ── Daemon entry point ────────────────────────────────────────────────────────

fn run_daemon() -> anyhow::Result<()> {
    // Auto-reap launched children. We spawn apps/wl-copy with std::process and
    // never wait() on them, so without this each one becomes a zombie when it
    // exits. Ignoring SIGCHLD makes the kernel reap them automatically.
    // SAFETY: setting a signal disposition has no preconditions; we never call
    // wait() ourselves so the ECHILD interaction with SIG_IGN doesn't apply.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }

    let cfg = config::load();
    tracing::info!("Starting slaunch daemon");

    // Scan entries synchronously before starting the event loop so the
    // first Show is instant.
    let entries = daemon::scan_entries(&cfg);
    let state = Arc::new(daemon::DaemonState::new(cfg));
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build tokio runtime: {e}"))?;
        let state_ref = state.clone();
        rt.block_on(async move {
            *state_ref.entries.write().await = entries;
        });
    }

    // Channel from the IPC socket task into the GTK event loop
    let (tx, rx) = mpsc::channel::<daemon::DaemonEvent>(32);

    // Spawn the tokio IPC socket listener on a background thread
    let state_clone = state.clone();
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(async move {
                if let Err(e) = daemon::run_socket(state_clone, tx).await {
                    tracing::error!("IPC socket error: {e}");
                }
            }),
            Err(e) => tracing::error!("Failed to start IPC runtime: {e}"),
        }
    });

    // GTK must run on the main thread (required by most Wayland compositors).
    ui::run(state, rx)
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

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("slaunch: failed to create runtime: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = rt.block_on(client::run(ipc_cmd)) {
        eprintln!("slaunch: {e}");
        std::process::exit(1);
    }
}
