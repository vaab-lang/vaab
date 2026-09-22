//! Install riffs into `~/.vaab/riffs` from dedicated `vaab-lang/{name}`
//! repos, falling back to the [vaab-riffs](https://github.com/vaab-lang/vaab-riffs)
//! catalog monorepo.

use std::fs;
use std::path::{Path, PathBuf};

use crate::home::riff_dir;
use crate::manifest::Manifest;

/// Catalog monorepo — shared riffs that do not yet have their own repo.
const CATALOG: &str = "https://raw.githubusercontent.com/vaab-lang/vaab-riffs/main";

/// Dedicated per-riff repos live at `github.com/vaab-lang/{name}`.
fn dedicated_base(name: &str) -> String {
    format!("https://raw.githubusercontent.com/vaab-lang/{name}/main")
}

/// Install a riff from its dedicated repo, falling back to the catalog monorepo.
pub fn install(name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let destination = riff_dir(name)?;
    if destination.join("riff").is_file() && destination.join("lib.vaab").is_file() {
        return Ok(destination);
    }
    fs::create_dir_all(&destination).map_err(|error| {
        format!("could not create `{}`: {error}", destination.display())
    })?;
    fetch_official(name, &destination)?;
    Ok(destination)
}

/// Install only if missing; used by `vaab need`.
pub fn ensure(name: &str) -> Result<PathBuf, String> {
    let destination = riff_dir(name)?;
    if destination.join("lib.vaab").is_file() {
        return Ok(destination);
    }
    install(name)
}

/// Copy a riff directory already on disk — for tests and local development.
pub fn install_from_path(name: &str, source: &Path) -> Result<PathBuf, String> {
    validate_name(name)?;
    if !source.join("riff").is_file() || !source.join("lib.vaab").is_file() {
        return Err(format!(
            "`{}` is not a riff — it needs `riff` and `lib.vaab`",
            source.display()
        ));
    }
    let destination = riff_dir(name)?;
    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(|error| {
            format!("could not replace `{}`: {error}", destination.display())
        })?;
    }
    copy_dir(source, &destination)?;
    Ok(destination)
}

pub fn is_installed(name: &str) -> bool {
    riff_dir(name)
        .ok()
        .is_some_and(|path| path.join("lib.vaab").is_file())
}

pub fn installed_path(name: &str) -> Result<PathBuf, String> {
    let path = riff_dir(name)?;
    if path.join("lib.vaab").is_file() {
        Ok(path)
    } else {
        Err(format!(
            "`{name}` is not installed — run `riff install {name}` first"
        ))
    }
}

fn fetch_official(name: &str, destination: &Path) -> Result<(), String> {
    let bases = [
        dedicated_base(name),
        format!("{CATALOG}/{name}"),
    ];
    let mut last_error = String::new();
    for base in &bases {
        match fetch_pair(base) {
            Ok((riff, lib)) => {
                Manifest::parse(&riff).map_err(|error| {
                    format!("`{name}` from the catalog did not parse as a riff: {error}")
                })?;
                fs::write(destination.join("riff"), riff).map_err(|error| {
                    format!(
                        "could not write `{}`: {error}",
                        destination.join("riff").display()
                    )
                })?;
                fs::write(destination.join("lib.vaab"), lib).map_err(|error| {
                    format!(
                        "could not write `{}`: {error}",
                        destination.join("lib.vaab").display()
                    )
                })?;
                return Ok(());
            }
            Err(error) => last_error = error,
        }
    }
    Err(format!(
        "could not install `{name}` from vaab-lang/{name} or the catalog: {last_error}"
    ))
}

fn fetch_pair(base: &str) -> Result<(String, String), String> {
    let riff = fetch_text(&format!("{base}/riff"))?;
    let lib = fetch_text(&format!("{base}/lib.vaab"))?;
    Ok((riff, lib))
}

fn fetch_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .call()
        .map_err(|error| format!("could not fetch `{url}`: {error}"))?
        .into_string()
        .map_err(|error| format!("could not read `{url}`: {error}"))
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!("could not create `{}`: {error}", destination.display())
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        format!("could not read `{}`: {error}", source.display())
    })? {
        let entry = entry.map_err(|error| format!("could not read a directory entry: {error}"))?;
        let file_type = entry.file_type().map_err(|error| {
            format!("could not inspect `{}`: {error}", entry.path().display())
        })?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|error| {
                format!(
                    "could not copy `{}` to `{}`: {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a riff name cannot be empty".to_string());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit())
    {
        return Err(format!(
            "`{name}` has to be snake_case, like `supabase` or `task_api`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_from_path_puts_a_riff_in_the_home_directory() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../vaab-riffs/supabase");
        if !source.join("lib.vaab").is_file() {
            return;
        }
        let name = format!("test_install_{}", std::process::id());
        let installed = install_from_path(&name, &source).expect("should install");
        assert!(installed.join("lib.vaab").is_file());
        let _ = fs::remove_dir_all(installed);
    }
}
