use std::sync::Arc;
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
            "commands" if cfg.plugins.commands => CommandsPlugin.scan(&mut out),
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

/// Run the IPC socket server loop.
/// `tx` sends DaemonEvents into the Iced app subscription channel.
pub async fn run_socket(
    state: Arc<DaemonState>,
    tx: mpsc::Sender<DaemonEvent>,
) -> anyhow::Result<()> {
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
