use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{RwLock, mpsc};

use crate::config::{self, Config};
use crate::ipc::{self, Command, RESP_ERR, RESP_OK};
use crate::plugins::{Entry, Plugin};
use crate::plugins::apps::AppsPlugin;
use crate::plugins::commands::CommandsPlugin;

/// Messages the IPC listener sends into the Iced application.
#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Show,
    Hide,
    ReloadConfig(Box<Config>),
    EntriesUpdated,
    Quit,
}

/// State shared between the socket listener and the UI.
pub struct DaemonState {
    pub config: RwLock<Config>,
    pub entries: RwLock<Vec<Entry>>,
}

impl DaemonState {
    pub fn new(cfg: Config) -> Self {
        Self {
            config: RwLock::new(cfg),
            entries: RwLock::new(Vec::new()),
        }
    }
}

/// Scan all enabled plugins and return the combined entry list.
/// Plugins are scanned in the order defined by `plugins.priority`; entries from
/// earlier plugins sort before later ones in search results.
pub fn scan_entries(cfg: &Config) -> Vec<Entry> {
    let mut out = Vec::new();

    for (priority, name) in cfg.plugins.priority.iter().enumerate() {
        let start = out.len();
        match name.as_str() {
            "apps" if cfg.plugins.apps => {
                AppsPlugin::new(cfg.plugins.terminal.clone()).scan(&mut out)
            }
            "commands" if cfg.plugins.commands => {
                CommandsPlugin::new(cfg.plugins.terminal.clone()).scan(&mut out)
            }
            _ => continue,
        }
        let p = priority as u8;
        for entry in &mut out[start..] {
            entry.priority = p;
        }
    }

    tracing::info!("Scanned {} entries", out.len());
    out
}

/// Spawn a background task that watches XDG application dirs and PATH dirs
/// for filesystem changes and rescans entries when any change is detected.
///
/// Uses a 500ms debounce window so rapid changes (e.g. a package install
/// writing multiple files) collapse into a single rescan.
pub fn spawn_watcher(state: Arc<DaemonState>, cfg: &Config, ui_tx: mpsc::Sender<DaemonEvent>) {
    use notify::{RecursiveMode, Watcher, recommended_watcher};

    let mut dirs: Vec<PathBuf> = Vec::new();

    if cfg.plugins.apps {
        dirs.extend(crate::plugins::apps::xdg_application_dirs());
    }
    if cfg.plugins.commands {
        if let Ok(path_var) = std::env::var("PATH") {
            for d in path_var.split(':') {
                dirs.push(PathBuf::from(d));
            }
        }
    }

    if dirs.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

        let mut watcher = match recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                use notify::EventKind::*;
                if matches!(event.kind, Create(_) | Modify(_) | Remove(_)) {
                    let _ = tx.try_send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Failed to create filesystem watcher: {e}");
                return;
            }
        };

        let mut watched = 0usize;
        for dir in &dirs {
            if dir.exists() {
                match watcher.watch(dir, RecursiveMode::NonRecursive) {
                    Ok(()) => watched += 1,
                    Err(e) => tracing::warn!("Cannot watch {}: {e}", dir.display()),
                }
            }
        }
        tracing::info!("Watching {watched} directories for filesystem changes");

        loop {
            if rx.recv().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            while rx.try_recv().is_ok() {}

            let cfg = state.config.read().await.clone();
            match tokio::task::spawn_blocking(move || scan_entries(&cfg)).await {
                Ok(entries) => {
                    *state.entries.write().await = entries;
                    tracing::info!("Entries rescanned due to filesystem change");
                    let _ = ui_tx.send(DaemonEvent::EntriesUpdated).await;
                }
                Err(e) => tracing::error!("Rescan task panicked: {e}"),
            }
        }
    });
}

/// Run the IPC socket server loop.
/// `tx` sends DaemonEvents into the Iced app subscription channel.
pub async fn run_socket(
    state: Arc<DaemonState>,
    tx: mpsc::Sender<DaemonEvent>,
) -> anyhow::Result<()> {
    {
        let cfg = state.config.read().await;
        spawn_watcher(state.clone(), &cfg, tx.clone());
    }

    let socket_path = ipc::socket_path();

    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("IPC socket listening at {}", socket_path.display());

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let result = loop {
        tokio::select! {
            res = listener.accept() => {
                match res {
                    Err(e) => break Err(e.into()),
                    Ok((mut stream, _)) => {
                        let state = state.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let mut cmd_byte = [0u8; 1];
                            if stream.read_exact(&mut cmd_byte).await.is_err() {
                                return;
                            }

                            let response = match Command::from_byte(cmd_byte[0]) {
                                Some(Command::Show) => {
                                    let _ = tx.send(DaemonEvent::Show).await;
                                    RESP_OK
                                }
                                Some(Command::Hide) => {
                                    let _ = tx.send(DaemonEvent::Hide).await;
                                    RESP_OK
                                }
                                Some(Command::Reload) => {
                                    let new_cfg = config::load();
                                    let new_entries = scan_entries(&new_cfg);
                                    *state.config.write().await = new_cfg.clone();
                                    *state.entries.write().await = new_entries;
                                    let _ = tx.send(DaemonEvent::ReloadConfig(Box::new(new_cfg))).await;
                                    RESP_OK
                                }
                                Some(Command::Kill) => {
                                    let _ = tx.send(DaemonEvent::Quit).await;
                                    RESP_OK
                                }
                                None => {
                                    tracing::warn!("Unknown IPC command byte: 0x{:02x}", cmd_byte[0]);
                                    RESP_ERR
                                }
                            };

                            let _ = stream.write_all(&[response]).await;
                        });
                    }
                }
            }
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, shutting down");
                let _ = tx.send(DaemonEvent::Quit).await;
                break Ok(());
            }
            _ = sigint.recv() => {
                tracing::info!("Received SIGINT, shutting down");
                let _ = tx.send(DaemonEvent::Quit).await;
                break Ok(());
            }
        }
    };

    let _ = std::fs::remove_file(&socket_path);
    result
}
