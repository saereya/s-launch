use crate::ipc::{self, Command};

pub async fn run(cmd: Command) -> anyhow::Result<()> {
    match ipc::send_command(cmd).await {
        Ok(true) => Ok(()),
        Ok(false) => match cmd {
            Command::Reload => anyhow::bail!(
                "Config reload failed — config.toml has an error, previous config is still active (see daemon logs for details)"
            ),
            _ => anyhow::bail!("Daemon returned an error response"),
        },
        Err(e) => {
            anyhow::bail!("Could not connect to slaunch daemon: {e}\nIs 'slaunch daemon' running?")
        }
    }
}
