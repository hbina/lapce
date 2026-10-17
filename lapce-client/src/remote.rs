use std::{
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Result, anyhow};
use flate2::read::GzDecoder;
use lapce_core::{
    directory::Directory,
    meta::{self, ReleaseType},
};
use lapce_rpc::{core::CoreRpcHandler, proxy::ProxyRpcHandler};
use thiserror::Error;
use tracing::{debug, error};

const UNIX_PROXY_SCRIPT: &[u8] = include_bytes!("../../extra/proxy.sh");

#[derive(Clone, Copy, Error, Debug, PartialEq, Eq, strum_macros::Display)]
#[strum(ascii_case_insensitive)]
enum HostPlatform {
    UnknownOS,
    #[strum(serialize = "linux")]
    Linux,
}

/// serialise via strum to arch name that is used
/// in CI artefacts
#[derive(Clone, Copy, Error, Debug, PartialEq, Eq, strum_macros::Display)]
#[strum(ascii_case_insensitive)]
enum HostArchitecture {
    UnknownArch,
    #[strum(serialize = "x86_64")]
    AMD64,
    #[strum(serialize = "x86")]
    X86,
    #[strum(serialize = "aarch64")]
    ARM64,
    #[strum(serialize = "armv7")]
    ARM32v7,
    #[strum(serialize = "armhf")]
    ARM32v6,
}

pub trait Remote: Sized {
    #[allow(unused)]
    fn home_dir(&self) -> Result<String> {
        let cmd = self
            .command_builder()
            .arg("echo")
            .arg("-n")
            .arg("$HOME")
            .stdout(Stdio::piped())
            .output()?;

        home_directory(cmd)
    }

    fn upload_file(&self, local: impl AsRef<Path>, remote: &str) -> Result<()>;

    fn command_builder(&self) -> Command;

    fn expand_path(&self, path: &str) -> Result<String> {
        Ok(path.to_owned())
    }

    /// Build a proxy invocation; transports without a remote login shell must
    /// expand the home directory before passing the executable to the OS.
    fn proxy_command(&self, executable: &str) -> Result<Command> {
        let mut command = self.command_builder();
        command.arg(executable);
        Ok(command)
    }

    fn unix_install_command(
        &self,
        encoded_script: &str,
        version: &str,
        directory: &str,
    ) -> Result<Command> {
        let mut command = self.command_builder();
        command.args([
            "echo",
            encoded_script,
            "|",
            "base64",
            "-d",
            "|",
            "sh",
            "/dev/stdin",
            version,
            directory,
        ]);
        Ok(command)
    }
}

pub(super) fn home_directory(output: std::process::Output) -> Result<String> {
    anyhow::ensure!(
        output.status.success(),
        "Remote home-directory lookup failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let home = String::from_utf8(output.stdout)?;
    let home = home.trim_end_matches(['\r', '\n']);
    anyhow::ensure!(
        !home.trim().is_empty() && home != "$HOME",
        "Remote home-directory lookup returned no expanded directory"
    );
    Ok(home.into())
}

pub fn start_remote(
    remote: impl Remote,
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
) -> Result<()> {
    start_remote_with_executable(remote, core_rpc, proxy_rpc, None)
}

pub fn start_remote_with_executable(
    remote: impl Remote,
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
    executable: Option<&str>,
) -> Result<()> {
    use HostPlatform::*;
    let (platform, architecture) = host_specification(&remote)?;

    if platform == UnknownOS || architecture == HostArchitecture::UnknownArch {
        error!("detected remote host: {platform}/{architecture}");
        return Err(anyhow!("Unknown OS and/or architecture"));
    }

    let remote_proxy_path = remote.expand_path(&format!(
        "~/.local/share/{}/proxy",
        meta::NAME.to_lowercase()
    ))?;
    let remote_proxy_file = format!("{remote_proxy_path}/lapce");
    let remote_proxy_file = match executable {
        Some(path) => remote.expand_path(path)?,
        None => remote_proxy_file,
    };

    if !remote
        .proxy_command(&remote_proxy_file)?
        .arg("--version")
        .output()
        .map(|output| {
            if !output.status.success() {
                return false;
            }
            if meta::RELEASE == ReleaseType::Debug {
                String::from_utf8_lossy(&output.stdout).starts_with("Lapce-proxy")
            } else {
                String::from_utf8_lossy(&output.stdout).trim()
                    == format!("Lapce-proxy {}", meta::VERSION)
            }
        })
        .unwrap_or(false)
    {
        anyhow::ensure!(
            executable.is_none(),
            "Configured remote backend failed its version check: {remote_proxy_file}"
        );
        download_remote(
            &remote,
            &platform,
            &architecture,
            &remote_proxy_path,
            &remote_proxy_file,
        )?;
    };

    debug!("remote proxy path: {remote_proxy_path}");

    let child = remote
        .proxy_command(&remote_proxy_file)?
        .args(["--proxy", "--no-cli-socket"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    crate::process::run_process(child, core_rpc, proxy_rpc)
}

fn download_remote(
    remote: &impl Remote,
    platform: &HostPlatform,
    architecture: &HostArchitecture,
    remote_proxy_path: &str,
    remote_proxy_file: &str,
) -> Result<()> {
    use base64::{Engine as _, engine::general_purpose};
    let proxy_script = general_purpose::STANDARD.encode(UNIX_PROXY_SCRIPT);
    let version = match meta::RELEASE {
        ReleaseType::Stable => format!("v{}", meta::VERSION),
        _ => "nightly".to_owned(),
    };
    let output = remote
        .unix_install_command(&proxy_script, &version, remote_proxy_path)?
        .output()?;
    debug!("{}", String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        return Ok(());
    }
    let exists = remote
        .command_builder()
        .args(["test", "-e", remote_proxy_file])
        .status()?;
    if exists.success() {
        return Ok(());
    }
    let proxy_filename = format!("lapce-proxy-{platform}-{architecture}");
    let local_proxy_file = Directory::proxy_directory()
        .ok_or_else(|| anyhow!("can't find proxy directory"))?
        .join(&proxy_filename);
    let proxy_version = match meta::RELEASE {
        ReleaseType::Stable => meta::VERSION,
        _ => "nightly",
    };
    let url = format!(
        "https://github.com/lapce/lapce/releases/download/{proxy_version}/{proxy_filename}.gz"
    );
    let mut response = lapce_services::http::get_url(url, None)?;
    anyhow::ensure!(
        response.status().is_success(),
        "Remote proxy download failed with: {}",
        response.status()
    );
    let mut out = std::fs::File::create(&local_proxy_file)?;
    std::io::copy(&mut GzDecoder::new(&mut response), &mut out)?;
    let status = remote
        .command_builder()
        .args(["mkdir", "-p", remote_proxy_path])
        .status()?;
    anyhow::ensure!(
        status.success(),
        "Creating remote proxy directory failed: {status}"
    );
    remote.upload_file(&local_proxy_file, remote_proxy_file)?;
    let status = remote
        .command_builder()
        .args(["chmod", "+x", remote_proxy_file])
        .status()?;
    anyhow::ensure!(
        status.success(),
        "Making remote proxy executable failed: {status}"
    );
    Ok(())
}

fn host_specification(
    remote: &impl Remote,
) -> Result<(HostPlatform, HostArchitecture)> {
    let output = remote.command_builder().args(["uname", "-sm"]).output()?;
    anyhow::ensure!(
        output.status.success(),
        "Remote transport failed during platform detection: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let specification = String::from_utf8_lossy(&output.stdout).to_lowercase();
    let (os, arch) = specification
        .trim()
        .split_once(' ')
        .ok_or_else(|| anyhow!("Invalid remote platform specification"))?;
    let platform = parse_os(os);
    anyhow::ensure!(
        platform == HostPlatform::Linux,
        "Unsupported remote operating system: {os}; only Linux is supported"
    );
    Ok((platform, parse_arch(arch)))
}

fn parse_arch(arch: &str) -> HostArchitecture {
    use HostArchitecture::*;
    // processor architectures be like that
    match arch {
        "amd64" | "x64" | "x86_64" => AMD64,
        "x86" | "i386" | "i586" | "i686" => X86,
        "arm" | "armhf" | "armv6" => ARM32v6,
        "armv7" | "armv7l" => ARM32v7,
        "arm64" | "armv8" | "aarch64" => ARM64,
        _ => UnknownArch,
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn remote_platform_detection_accepts_only_linux() {
        struct ReportedHost(&'static str);
        impl Remote for ReportedHost {
            fn upload_file(&self, _: impl AsRef<Path>, _: &str) -> Result<()> {
                unreachable!()
            }
            fn command_builder(&self) -> Command {
                let mut command = Command::new("sh");
                command.args(["-c", "printf '%s\\n' \"$0\"", self.0]);
                command
            }
        }
        assert_eq!(
            host_specification(&ReportedHost("Linux x86_64")).unwrap(),
            (HostPlatform::Linux, HostArchitecture::AMD64)
        );
        for specification in ["Darwin arm64", "Windows_NT AMD64", "FreeBSD amd64"] {
            let error = host_specification(&ReportedHost(specification))
                .unwrap_err()
                .to_string();
            assert!(error.contains("only Linux is supported"), "{error}");
        }
    }
    fn output(text: &str, successful: bool) -> std::process::Output {
        use std::os::unix::process::ExitStatusExt;

        std::process::Output {
            status: std::process::ExitStatus::from_raw(if successful {
                0
            } else {
                256
            }),
            stdout: text.as_bytes().to_vec(),
            stderr: b"access denied".to_vec(),
        }
    }
    #[test]
    fn home_lookup_requires_success_and_expansion_preserving_spaces() {
        assert_eq!(
            home_directory(output("/home/a b\n", true)).unwrap(),
            "/home/a b"
        );
        assert!(home_directory(output("$HOME", true)).is_err());
        assert!(home_directory(output("\n", true)).is_err());
        assert!(
            home_directory(output("/home/user", false))
                .unwrap_err()
                .to_string()
                .contains("access denied")
        );
    }

    #[test]
    fn ssh_transport_failure_preserves_error_instead_of_trying_other_shells() {
        struct FailedRemote;
        impl Remote for FailedRemote {
            fn upload_file(&self, _: impl AsRef<Path>, _: &str) -> Result<()> {
                unreachable!()
            }
            fn command_builder(&self) -> Command {
                let mut command = Command::new("sh");
                command.args(["-c", "echo 'connection refused' >&2; exit 255"]);
                command
            }
        }
        let error = host_specification(&FailedRemote).unwrap_err().to_string();
        assert!(error.contains("connection refused"));
    }
}

fn parse_os(os: &str) -> HostPlatform {
    use HostPlatform::*;
    match os {
        "linux" => Linux,
        _ => UnknownOS,
    }
}
