//! Every program in `examples/` must parse.
//!
//! The examples are the language's shop window, so a change that breaks one is a
//! change that breaks the documentation. This walks the folder rather than listing
//! the files, so a new example is covered the moment it is added.

use std::path::{Path, PathBuf};

use vaab_syntax::{diagnostic, ColorChoice};

fn examples_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
}

/// Every `.vaab` file in `examples/`, sorted so failures are reported in a
/// predictable order.
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

#[test]
fn there_are_examples_to_check() {
    assert!(!example_files().is_empty(), "examples/ should not be empty");
}

#[test]
fn every_example_parses() {
    let mut failures = String::new();

    for path in example_files() {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        let parsed = vaab_syntax::parse(&source);
        if parsed.has_errors() {
            failures.push_str(&diagnostic::render(
                &parsed.diagnostics,
                &name,
                &source,
                ColorChoice::Never,
            ));
        }
    }

    assert!(failures.is_empty(), "some examples do not parse:\n{failures}");
}

#[test]
fn no_example_uses_bare_type_call_syntax() {
    // Vaab has no `Type(...)` call form: construction always goes through
    // `Type.new(...)`, and choice variants are written `Error.Variant(...)`.
    //
    // The printed tree makes this easy to see precisely. A call renders its callee
    // on the line after `callee`, so `Account.new(...)` shows `member .new` there
    // while a bare `Account(...)` shows `name Account`. Reading the tree rather
    // than the source also avoids mistaking a variant *declaration* for a call.
    let mut offenders = Vec::new();

    for path in example_files() {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        let parsed = vaab_syntax::parse(&source);
        let tree = vaab_syntax::print_module(&parsed.module);

        let lines: Vec<&str> = tree.lines().map(str::trim).collect();
        for pair in lines.windows(2) {
            let [callee, target] = pair else { continue };
            if *callee != "callee" {
                continue;
            }
            if let Some(name) = target.strip_prefix("name ") {
                if name.starts_with(char::is_uppercase) {
                    offenders.push(format!(
                        "{}: `{name}(...)` should be `{name}.new(...)`",
                        path.display()
                    ));
                }
            }
        }
    }

    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}
