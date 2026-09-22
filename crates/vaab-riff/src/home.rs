//! Where installed riffs live on disk.

use std::path::PathBuf;

const APP: &str = "vaab";

/// `~/.vaab/riffs/supabase`, or the nearest platform equivalent.
pub fn riff_dir(name: &str) -> Result<PathBuf, String> {
    let home = home_dir()?;
    Ok(home.join("riffs").join(name))
}

/// `~/.vaab`
pub fn vaab_home() -> Result<PathBuf, String> {
    home_dir()
}

fn home_dir() -> Result<PathBuf, String> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| format!("could not find a home directory to install riffs into"))
        .map(|home| home.join(APP))
}
