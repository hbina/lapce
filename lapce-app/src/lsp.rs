use std::path::PathBuf;

use tracing::{Level, event};
use url::Url;

// Decode file URLs into Linux workspace paths.

pub fn path_from_url(url: &Url) -> PathBuf {
    event!(Level::DEBUG, "Converting `{:?}` to path", url);
    url.to_file_path().unwrap_or_else(|_| {
        let path = url.path();
        if let Ok(path) = percent_encoding::percent_decode_str(path).decode_utf8() {
            return PathBuf::from(path.into_owned());
        }
        PathBuf::from(path)
    })
}
