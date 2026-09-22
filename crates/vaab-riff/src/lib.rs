//! Riffs: Vaab's packages.
//!
//! A riff is a directory with a `riff` manifest, Vaab source, and — after
//! `vaab gather` — a `needed.lock` that pins path (and eventually registry)
//! dependencies.

mod gather;
mod home;
mod install;
mod link;
mod lock;
mod manifest;
mod merge;
mod project;

pub use gather::{add_dependency, default_project_manifest, gather, ProjectKind};
pub use home::vaab_home;
pub use install::{ensure, install, install_from_path, installed_path, is_installed};
pub use link::{exports_of, link, Linked, LoadedRiff};
pub use lock::{LockEntry, Lockfile};
pub use manifest::{Dependency, Manifest};
pub use project::{default_entry, find_project_root};
