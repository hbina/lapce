//! Host filesystem execution of semantic resource operations.
use lapce_rpc::resource::ResourceOperation;
use std::{fs, io, path::Path};

fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Return whether an operation changed the filesystem. Overwrite takes
/// precedence over ignore-if-exists. Symlink deletion never follows its target.
pub fn apply(operation: &ResourceOperation) -> io::Result<bool> {
    match operation {
        ResourceOperation::Create {
            path,
            overwrite,
            ignore_if_exists,
        } => {
            if exists(path)? && !overwrite {
                if *ignore_if_exists {
                    return Ok(false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "resource already exists",
                ));
            }
            fs::OpenOptions::new()
                .write(true)
                .create(*overwrite)
                .create_new(!overwrite)
                .truncate(*overwrite)
                .open(path)?;
        }
        ResourceOperation::Rename {
            from,
            to,
            overwrite,
            ignore_if_exists,
        } => {
            if exists(to)? && !overwrite {
                if *ignore_if_exists {
                    return Ok(false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "rename destination already exists",
                ));
            }
            fs::rename(from, to)?;
        }
        ResourceOperation::Delete {
            path,
            recursive,
            ignore_if_missing,
        } => {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error)
                    if *ignore_if_missing
                        && error.kind() == io::ErrorKind::NotFound =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            };
            if metadata.is_dir() {
                if *recursive {
                    fs::remove_dir_all(path)?;
                } else {
                    fs::remove_dir(path)?;
                }
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(true)
}
