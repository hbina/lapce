use std::{path::Path, process::Command};

use anyhow::{Result, bail};
use tracing::debug;

use super::remote::Remote;
use crate::{new_command, workspace::SshHost};

pub struct SshRemote {
    pub ssh: SshHost,
}

impl SshRemote {
    const SSH_ARGS: &'static [&'static str] = &[
        "-o",
        "ControlMaster=auto",
        "-o",
        "ControlPath=~/.ssh/cm_%C",
        "-o",
        "ControlPersist=30m",
        "-o",
        "ConnectTimeout=15",
    ];
}

impl Remote for SshRemote {
    fn upload_file(&self, local: impl AsRef<Path>, remote: &str) -> Result<()> {
        let mut cmd = new_command("scp");

        cmd.args(Self::SSH_ARGS);

        if let Some(port) = self.ssh.port {
            cmd.arg("-P").arg(port.to_string());
        }

        let output = cmd
            .arg(local.as_ref())
            .arg(format!("{}:{remote}", self.ssh.user_host()))
            .output()?;

        debug!("{}", String::from_utf8_lossy(&output.stderr));
        debug!("{}", String::from_utf8_lossy(&output.stdout));

        check_upload_result(&output)
    }

    fn command_builder(&self) -> Command {
        let mut cmd = new_command("ssh");
        cmd.args(Self::SSH_ARGS);

        if let Some(port) = self.ssh.port {
            cmd.arg("-p").arg(port.to_string());
        }

        cmd.arg(self.ssh.user_host());

        if !std::env::var("LAPCE_DEBUG").unwrap_or_default().is_empty() {
            cmd.arg("-v");
        }

        cmd
    }
}

fn check_upload_result(output: &std::process::Output) -> Result<()> {
    if !output.status.success() {
        bail!(
            "SSH backend upload failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_upload_reports_status_and_remote_error() {
        let output = Command::new("sh")
            .args(["-c", "echo 'remote permission denied' >&2; exit 7"])
            .output()
            .unwrap();

        let message = check_upload_result(&output).unwrap_err().to_string();
        assert!(message.contains("remote permission denied"));
        assert!(message.contains('7'));
    }
}
