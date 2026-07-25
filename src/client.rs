use crate::ipc::{self, Command};

pub async fn run(cmd: Command) -> anyhow::Result<()> {
    match ipc::send_command(cmd).await {
        Ok(true) => Ok(()),
        Ok(false) => anyhow::bail!("Daemon returned an error response"),
        Err(e) => {
            anyhow::bail!("Could not connect to slaunch daemon: {e}\nIs 'slaunch daemon' running?")
        }
    }
}
