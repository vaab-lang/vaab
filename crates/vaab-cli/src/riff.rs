//! `vaab gather` and `vaab need`.

use std::path::{Path, PathBuf};

use vaab_riff::{add_dependency, gather, Dependency, Manifest, ProjectKind, default_project_manifest};
use vaab_riff::find_project_root;

pub fn gather_project(path: Option<&Path>) -> Result<(), String> {
    let root = project_root(path)?;
    gather(&root)?;
    println!("Gathered riffs into {}/needed.lock", root.display());
    Ok(())
}

pub fn add_need(words: &[String]) -> Result<(), String> {
    if words.len() < 4 {
        return Err(
            "a need line looks like `vaab need json from ada` or `vaab need colours from ./vendor/colours`"
                .to_string(),
        );
    }
    let name = words[0].clone();
    if words[1] != "from" {
        return Err(format!("`vaab need {name}` needs the word `from` next"));
    }
    let source = words[2].clone();
    let range = if words.get(3) == Some(&"at".to_string()) {
        words.get(4).cloned()
    } else {
        None
    };

    let root = project_root(None)?;
    let dependency = if source.starts_with("./") || source.starts_with("../") || source.starts_with('/') {
        Dependency::Path {
            name,
            path: PathBuf::from(source),
        }
    } else {
        Dependency::Registry {
            name,
            owner: source,
            range,
        }
    };

    add_dependency(&root, dependency)?;
    gather(&root)?;
    println!("Updated {}/riff and {}/needed.lock", root.display(), root.display());
    Ok(())
}

fn project_root(path: Option<&Path>) -> Result<PathBuf, String> {
    match path {
        Some(path) if path.is_dir() => Ok(path.to_path_buf()),
        Some(path) => find_project_root(path),
        None => find_project_root(Path::new(".")),
    }
}

pub fn manifest_for_new(name: &str) -> Manifest {
    default_project_manifest(name, ProjectKind::App)
}
