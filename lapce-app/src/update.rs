use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use lapce_core::{directory::Directory, meta};
use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub target_commitish: String,
    pub assets: Vec<ReleaseAsset>,
    #[serde(skip)]
    pub version: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

pub fn get_latest_release() -> Result<ReleaseInfo> {
    let url = match meta::RELEASE {
        meta::ReleaseType::Debug => {
            return Err(anyhow!("no release for debug"));
        }
        meta::ReleaseType::Nightly => {
            "https://api.github.com/repos/lapce/lapce/releases/tags/nightly"
        }
        _ => "https://api.github.com/repos/lapce/lapce/releases/latest",
    };

    let resp = lapce_proxy::get_url(url, Some("Lapce"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("get release info failed {}", resp.text()?));
    }
    let mut release: ReleaseInfo = serde_json::from_str(&resp.text()?)?;

    release.version = match release.tag_name.as_str() {
        "nightly" => format!(
            "{}+Nightly.{}",
            env!("CARGO_PKG_VERSION"),
            &release.target_commitish[..7]
        ),
        _ => release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name)
            .to_owned(),
    };

    Ok(release)
}

pub fn download_release(release: &ReleaseInfo) -> Result<PathBuf> {
    let dir =
        Directory::updates_directory().ok_or_else(|| anyhow!("no directory"))?;
    let name = match std::env::consts::OS {
        "macos" => "Lapce-macos.dmg",
        "linux" => match std::env::consts::ARCH {
            "aarch64" => "lapce-linux-arm64.tar.gz",
            "x86_64" => "lapce-linux-amd64.tar.gz",
            _ => return Err(anyhow!("arch not supported")),
        },
        #[cfg(feature = "portable")]
        "windows" => "Lapce-windows-portable.zip",
        #[cfg(not(feature = "portable"))]
        "windows" => "Lapce-windows.msi",
        _ => return Err(anyhow!("os not supported")),
    };
    let file_path = dir.join(name);

    for asset in &release.assets {
        if asset.name == name {
            let mut resp = lapce_proxy::get_url(&asset.browser_download_url, None)?;
            if !resp.status().is_success() {
                return Err(anyhow!("download file error {}", resp.text()?));
            }
            let mut out = std::fs::File::create(&file_path)?;
            resp.copy_to(&mut out)?;
            return Ok(file_path);
        }
    }

    Err(anyhow!("can't download release"))
}

pub fn extract(src: &Path, process_path: &Path) -> Result<PathBuf> {
    let tar_gz = std::fs::File::open(src)?;
    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(tar);
    let parent = src.parent().ok_or_else(|| anyhow::anyhow!("no parent"))?;
    archive.unpack(parent)?;
    std::fs::remove_file(process_path)?;
    std::fs::copy(parent.join("Lapce").join("lapce"), process_path)?;
    Ok(process_path.to_path_buf())
}

pub fn restart(path: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(path).arg("-n").exec();
    Ok(())
}

pub fn cleanup() {
    // Nothing to do yet
}
