//! Resolve dependencies and write `needed.lock`.

use std::fs;
use std::path::Path;

use crate::install::installed_path;
use crate::lock::{LockEntry, Lockfile};
use crate::manifest::{Dependency, Manifest};

const REGISTRY: &str = "https://riffs.vaab.dev";

/// Resolve every dependency in `riff` and write `needed.lock` beside it.
pub fn gather(project_root: &Path) -> Result<Lockfile, String> {
    let manifest_path = project_root.join("riff");
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("could not read `{}`: {error}", manifest_path.display()))?;
    let manifest = Manifest::parse(&source)?;

    let mut entries = Vec::new();
    for dependency in &manifest.dependencies {
        entries.push(resolve_dependency(project_root, dependency)?);
    }

    let lock = Lockfile { entries };
    let lock_path = project_root.join("needed.lock");
    fs::write(&lock_path, lock.to_string())
        .map_err(|error| format!("could not write `{}`: {error}", lock_path.display()))?;
    Ok(lock)
}

fn resolve_dependency(project_root: &Path, dependency: &Dependency) -> Result<LockEntry, String> {
    match dependency {
        Dependency::Installed { name } => {
            let path = installed_path(name)?;
            Ok(LockEntry::Path { name: name.clone(), path })
        }
        Dependency::Path { name, path } => {
            let absolute = project_root.join(path);
            if !absolute.join("riff").is_file() {
                return Err(format!(
                    "`{}` is not a riff — there is no riff file at `{}`",
                    name,
                    absolute.join("riff").display()
                ));
            }
            Ok(LockEntry::Path { name: name.clone(), path: absolute })
        }
        Dependency::Registry { name, owner, range } => {
            let version = range.clone().unwrap_or_else(|| "0".to_string());
            let url = format!("{REGISTRY}/{owner}/{name}/{version}");
            Err(format!(
                "registry riffs are not fetched yet — `{name}` from `{owner}` would come from {url}. \
                 For now, use a path dependency: `need {name} from ../vaab-riffs/{name}`"
            ))
        }
    }
}

/// Add a dependency line to `riff` if it is not already there.
pub fn add_dependency(project_root: &Path, dependency: Dependency) -> Result<(), String> {
    let manifest_path = project_root.join("riff");
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("could not read `{}`: {error}", manifest_path.display()))?;
    let mut manifest = Manifest::parse(&source)?;
    if manifest.dependencies.iter().any(|existing| existing.name() == dependency.name()) {
        return Err(format!("`{}` is already in the riff file", dependency.name()));
    }
    manifest.dependencies.push(dependency);
    fs::write(&manifest_path, manifest.to_string())
        .map_err(|error| format!("could not write `{}`: {error}", manifest_path.display()))?;
    Ok(())
}

pub fn default_project_manifest(name: &str, kind: ProjectKind) -> Manifest {
    Manifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        description: Some(format!("a small Vaab {kind}")),
        owner: None,
        licence: None,
        needs_vaab: Some("0.1".to_string()),
        dependencies: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ProjectKind {
    App,
    Library,
}

impl std::fmt::Display for ProjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectKind::App => write!(f, "app"),
            ProjectKind::Library => write!(f, "library"),
        }
    }
}
