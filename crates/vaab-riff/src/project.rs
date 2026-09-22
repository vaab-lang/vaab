//! Find a riff project on disk.

use std::path::{Path, PathBuf};

/// Walk upward from `path` until a directory containing `riff` is found.
pub fn find_project_root(start: &Path) -> Result<PathBuf, String> {
    let start = if start.is_file() {
        start.parent().ok_or("that file has no parent directory")?
    } else {
        start
    };

    let mut current = start;
    loop {
        if current.join("riff").is_file() {
            return Ok(current.to_path_buf());
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    Err(format!(
        "no riff file was found above `{}` — run `vaab new` or add a `riff` file",
        start.display()
    ))
}

/// Default entry point for a riff project.
pub fn default_entry(project_root: &Path) -> PathBuf {
    let main = project_root.join("main.vaab");
    if main.is_file() {
        return main;
    }
    project_root.join("lib.vaab")
}
