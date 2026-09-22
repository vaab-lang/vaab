//! `vaab gather`, `vaab need`, and project riff helpers.

use std::path::{Path, PathBuf};

use vaab_riff::{
    add_dependency, ensure, gather, Dependency, Manifest, ProjectKind, default_project_manifest,
    find_project_root,
};

pub fn gather_project(path: Option<&Path>) -> Result<(), String> {
    let root = project_root(path)?;
    gather(&root)?;
    println!("Gathered riffs into {}/needed.lock", root.display());
    Ok(())
}

/// `vaab need supabase` or `vaab need json from ada`.
pub fn add_need(words: &[String]) -> Result<(), String> {
    if words.is_empty() {
        return Err(
            "a need line looks like `vaab need supabase` or `vaab need json from ada`".to_string(),
        );
    }

    let root = project_root(None)?;
    let dependency = if words.len() == 1 {
        let name = words[0].clone();
        ensure(&name)?;
        Dependency::Installed { name }
    } else {
        parse_explicit_need(words)?
    };

    add_dependency(&root, dependency)?;
    gather(&root)?;
    println!("Updated {}/riff and {}/needed.lock", root.display(), root.display());
    Ok(())
}

fn parse_explicit_need(words: &[String]) -> Result<Dependency, String> {
    let name = words[0].clone();
    if words.get(1).map(String::as_str) != Some("from") {
        return Err(format!("`vaab need {name}` needs the word `from`, or omit it to use an installed riff"));
    }
    let source = words
        .get(2)
        .ok_or_else(|| format!("`vaab need {name} from ...` needs something after `from`"))?
        .clone();
    let range = if words.get(3) == Some(&"at".to_string()) {
        words.get(4).cloned()
    } else {
        None
    };

    if source.starts_with("./") || source.starts_with("../") || source.starts_with('/') {
        Ok(Dependency::Path {
            name,
            path: PathBuf::from(source),
        })
    } else {
        Ok(Dependency::Registry {
            name,
            owner: source,
            range,
        })
    }
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
