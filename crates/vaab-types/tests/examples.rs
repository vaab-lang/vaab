//! Every program in `examples/` must type-check.
//!
//! The examples are the language's shop window, so a change that breaks one is a
//! change that breaks the documentation. This walks the folder rather than listing
//! the files, so a new example is covered the moment it is added.

use std::path::{Path, PathBuf};

use vaab_syntax::{diagnostic, ColorChoice};

fn examples_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("examples")
}

fn example_files() -> Vec<PathBuf> {
    let folder = examples_folder();
    let entries = std::fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", folder.display()));

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "vaab"))
        .collect();
    files.sort();
    files
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Checks one file, giving back the rendered problems if it has any.
fn problems_in(path: &Path) -> Option<String> {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let name = name_of(path);

    let parsed = vaab_syntax::parse(&source);
    assert!(!parsed.has_errors(), "{name} no longer parses");

    match vaab_types::check(&parsed.module) {
        Ok(_) => None,
        Err(problems) => {
            Some(diagnostic::render(&problems, &name, &source, ColorChoice::Never))
        }
    }
}

#[test]
fn there_are_examples_to_check() {
    assert!(!example_files().is_empty(), "examples/ should not be empty");
}

#[test]
fn every_example_type_checks() {
    let mut failures = String::new();

    for path in example_files() {
        if let Some(problems) = problems_in(&path) {
            failures.push_str(&problems);
        }
    }

    assert!(failures.is_empty(), "some examples do not type-check:\n{failures}");
}
