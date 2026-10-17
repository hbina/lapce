use anyhow::{Result, anyhow};
use flate2::read::GzDecoder;
use lapce_core::directory::Directory;
use lapce_rpc::plugin::{VoltInfo, VoltMetadata};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tar::Archive;

/// Find all installed volts.
/// `plugin_dev_path` allows launching Lapce with a plugin on your local system for testing
/// purposes.
/// As well, this function skips any volt in the typical plugin directory that match the name
/// of the dev plugin so as to support developing a plugin you actively use.
pub fn find_all_volts(extra_plugin_paths: &[PathBuf]) -> Vec<VoltMetadata> {
    let Some(plugin_dir) = Directory::plugins_directory() else {
        return Vec::new();
    };

    let mut plugins: Vec<VoltMetadata> = plugin_dir
        .read_dir()
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let entry = result.ok()?;
            let metadata = entry.metadata().ok()?;

            // Ignore any loose files or '.' prefixed hidden directories
            if metadata.is_file() || entry.file_name().to_str()?.starts_with('.') {
                return None;
            }

            Some(entry.path())
        })
        .filter_map(|path| match load_volt(&path) {
            Ok(metadata) => Some(metadata),
            Err(e) => {
                tracing::error!("Failed to load plugin: {:?}", e);
                None
            }
        })
        .collect();

    for plugin_path in extra_plugin_paths {
        let mut metadata = match load_volt(plugin_path) {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::error!("Failed to load extra plugin: {:?}", e);
                continue;
            }
        };

        let pos = plugins.iter().position(|meta| {
            meta.name == metadata.name && meta.author == metadata.author
        });

        if let Some(pos) = pos {
            std::mem::swap(&mut plugins[pos], &mut metadata);
        } else {
            plugins.push(metadata);
        }
    }

    plugins
}

/// Returns an instance of "VoltMetadata" or an error if there is no file in the path,
/// the contents of the file cannot be read into a string, or the content read cannot
/// be converted to an instance of "VoltMetadata".
///
/// # Examples
///
/// ```
/// use std::fs::File;
/// use std::io::Write;
/// use lapce_services::plugin::load_volt;
/// use lapce_rpc::plugin::VoltMetadata;
///
/// let temp = tempfile::tempdir().unwrap();
/// let parent_path = temp.path().to_path_buf();
/// let mut file = File::create(parent_path.join("volt.toml")).unwrap();
/// let _ = writeln!(file, "name = \"plugin\" \n version = \"0.1\"");
/// let _ = writeln!(file, "display-name = \"Plugin\" \n author = \"Author\"");
/// let _ = writeln!(file, "description = \"Useful plugin\"");///
/// let volt_metadata = match load_volt(&parent_path) {
///     Ok(volt_metadata) => volt_metadata,
///     Err(error) => panic!("{}", error),
/// };
/// assert_eq!(
///     volt_metadata,
///     VoltMetadata {
///         name: "plugin".to_string(),
///         version: "0.1".to_string(),
///         display_name: "Plugin".to_string(),
///         author: "Author".to_string(),
///         description: "Useful plugin".to_string(),
///         icon: None,
///         repository: None,
///         wasm: None,
///         color_themes: None,
///         icon_themes: None,
///         dir: parent_path.canonicalize().ok(),
///         activation: None,
///         config: None
///     }
/// );
/// let _ = std::fs::remove_file(parent_path.join("volt.toml"));
/// ```
pub fn load_volt(path: &Path) -> Result<VoltMetadata> {
    let path = path.canonicalize()?;
    let mut file = fs::File::open(path.join("volt.toml"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let mut meta: VoltMetadata = toml::from_str(&contents)?;

    meta.dir = Some(path.clone());
    meta.wasm = meta.wasm.as_ref().and_then(|wasm| {
        Some(path.join(wasm).canonicalize().ok()?.to_str()?.to_string())
    });
    // FIXME: This does `meta.color_themes = Some([])` in case, for example,
    // it cannot find matching files, but in that case it should do `meta.color_themes = None`
    meta.color_themes = meta.color_themes.as_ref().map(|themes| {
        themes
            .iter()
            .filter_map(|theme| {
                Some(path.join(theme).canonicalize().ok()?.to_str()?.to_string())
            })
            .collect()
    });
    // FIXME: This does `meta.icon_themes = Some([])` in case, for example,
    // it cannot find matching files, but in that case it should do `meta.icon_themes = None`
    meta.icon_themes = meta.icon_themes.as_ref().map(|themes| {
        themes
            .iter()
            .filter_map(|theme| {
                Some(path.join(theme).canonicalize().ok()?.to_str()?.to_string())
            })
            .collect()
    });

    Ok(meta)
}

pub fn volt_icon(volt: &VoltMetadata) -> Option<Vec<u8>> {
    let dir = volt.dir.as_ref()?;
    let icon = dir.join(volt.icon.as_ref()?);
    std::fs::read(icon).ok()
}

pub fn download_volt(volt: &VoltInfo) -> Result<VoltMetadata> {
    let url = format!(
        "https://plugins.lapce.dev/api/v1/plugins/{}/{}/{}/download",
        volt.author, volt.name, volt.version
    );

    let resp = crate::http::get_url(url, None)?;
    if !resp.status().is_success() {
        return Err(anyhow!("can't download plugin"));
    }

    // this is the s3 url
    let url = resp.text()?;

    let mut resp = crate::http::get_url(url, None)?;
    if !resp.status().is_success() {
        return Err(anyhow!("can't download plugin"));
    }

    let is_zstd = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        == Some("application/zstd");

    let id = volt.id();
    let plugin_dir = Directory::plugins_directory()
        .ok_or_else(|| anyhow!("can't get plugin directory"))?
        .join(id.to_string());
    if let Err(err) = fs::remove_dir_all(&plugin_dir) {
        tracing::error!("{:?}", err);
    }
    fs::create_dir_all(&plugin_dir)?;

    if is_zstd {
        let tar = zstd::Decoder::new(&mut resp).unwrap();
        let mut archive = Archive::new(tar);
        archive.unpack(&plugin_dir)?;
    } else {
        let tar = GzDecoder::new(&mut resp);
        let mut archive = Archive::new(tar);
        archive.unpack(&plugin_dir)?;
    }

    let meta = load_volt(&plugin_dir)?;
    Ok(meta)
}
