use anyhow::{Result, anyhow};

pub fn register_lapce_path() -> Result<()> {
    let exedir = std::env::current_exe()?
        .parent()
        .ok_or(anyhow!("can't get parent dir of exe"))?
        .canonicalize()?;

    let current_path = std::env::var("PATH")?;
    let paths = std::env::split_paths(&current_path);
    for path in paths {
        if exedir == path.canonicalize()? {
            return Ok(());
        }
    }
    let paths = std::env::split_paths(&current_path);
    let paths = std::env::join_paths(std::iter::once(exedir).chain(paths))?;

    unsafe {
        std::env::set_var("PATH", paths);
    }

    Ok(())
}
