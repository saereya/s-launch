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

fn runtime_dir() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // SAFETY: getuid is always successful and has no preconditions.
        let uid = unsafe { libc::getuid() };
        format!("/run/user/{uid}")
    });
    PathBuf::from(dir)
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("slaunch.sock")
}

/// Path of the daemon's single-instance lock. Separate from the socket because
/// the socket file's *existence* says nothing about whether a daemon is live,
/// whereas an flock is held by the kernel and released on process exit.
pub fn lock_path() -> PathBuf {
    runtime_dir().join("slaunch.lock")
}

pub async fn send_command(cmd: Command) -> anyhow::Result<bool> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).await?;
    stream.write_all(&[cmd.to_byte()]).await?;
    let mut resp = [0u8; 1];
    stream.read_exact(&mut resp).await?;
    Ok(resp[0] == RESP_OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_byte_roundtrip() {
        for cmd in [Command::Show, Command::Hide, Command::Reload, Command::Kill] {
            assert_eq!(Command::from_byte(cmd.to_byte()), Some(cmd));
        }
    }

    #[test]
    fn command_bytes_are_distinct() {
        let bytes: std::collections::HashSet<u8> = [
            Command::Show.to_byte(),
            Command::Hide.to_byte(),
            Command::Reload.to_byte(),
            Command::Kill.to_byte(),
        ]
        .into_iter()
        .collect();
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn unknown_byte_does_not_decode() {
        assert_eq!(Command::from_byte(0xff), None);
        assert_eq!(Command::from_byte(0x99), None);
    }

    #[test]
    fn socket_path_honors_xdg_runtime_dir_when_set() {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env = unsafe {
            crate::test_env::EnvVarGuard::set("XDG_RUNTIME_DIR", "/tmp/slaunch-test-runtime")
        };
        assert_eq!(
            socket_path(),
            PathBuf::from("/tmp/slaunch-test-runtime/slaunch.sock")
        );
    }

    #[test]
    fn socket_path_falls_back_to_run_user_uid_when_unset() {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env = unsafe { crate::test_env::EnvVarGuard::remove("XDG_RUNTIME_DIR") };
        let uid = unsafe { libc::getuid() };
        assert_eq!(
            socket_path(),
            PathBuf::from(format!("/run/user/{uid}/slaunch.sock"))
        );
    }

    #[test]
    fn lock_path_sits_beside_the_socket() {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env = unsafe {
            crate::test_env::EnvVarGuard::set("XDG_RUNTIME_DIR", "/tmp/slaunch-test-runtime")
        };
        assert_eq!(
            lock_path(),
            PathBuf::from("/tmp/slaunch-test-runtime/slaunch.lock")
        );
        assert_eq!(lock_path().parent(), socket_path().parent());
    }
}
