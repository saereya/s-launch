use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const CMD_SHOW: u8 = 0x01;
pub const CMD_HIDE: u8 = 0x02;
pub const CMD_RELOAD: u8 = 0x03;
pub const CMD_KILL: u8 = 0x04;

pub const RESP_OK: u8 = 0x00;
pub const RESP_ERR: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Show,
    Hide,
    Reload,
    Kill,
}

impl Command {
    pub fn to_byte(self) -> u8 {
        match self {
            Command::Show => CMD_SHOW,
            Command::Hide => CMD_HIDE,
            Command::Reload => CMD_RELOAD,
            Command::Kill => CMD_KILL,
        }
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            CMD_SHOW => Some(Command::Show),
            CMD_HIDE => Some(Command::Hide),
            CMD_RELOAD => Some(Command::Reload),
            CMD_KILL => Some(Command::Kill),
            _ => None,
        }
    }
}

pub fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/1000"));
    PathBuf::from(runtime_dir).join("slaunch.sock")
}

pub async fn send_command(cmd: Command) -> anyhow::Result<bool> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).await?;
    stream.write_all(&[cmd.to_byte()]).await?;
    let mut resp = [0u8; 1];
    stream.read_exact(&mut resp).await?;
    Ok(resp[0] == RESP_OK)
}
