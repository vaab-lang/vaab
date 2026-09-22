//! Every sequential program in `examples/` must run, and print what it always did.
//!
//! The examples are the language's shop window, so an example that stops working
//! is documentation that has started lying. This walks the folder rather than
//! listing the files, so a new example is covered the moment it is added.
//!

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use insta::assert_snapshot;
use vaab_syntax::{diagnostic, ColorChoice};
use vaab_vm::{error, Output};

/// Example runs share process-wide Store/Db registries; serialize them.
static EXAMPLES: Mutex<()> = Mutex::new(());

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

fn sequential_examples() -> Vec<PathBuf> {
    example_files()
}

/// Runs one example, giving back everything it printed.
fn run(path: &Path) -> Result<Vec<String>, String> {
    let _guard = EXAMPLES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let name = name_of(path);

    let parsed = vaab_syntax::parse(&source);
    assert!(!parsed.has_errors(), "{name} no longer parses");
    let checked = match vaab_types::check(&parsed.module) {
        Ok(checked) => checked,
        Err(problems) => {
            return Err(diagnostic::render(&problems, &name, &source, ColorChoice::Never))
        }
    };

    let mut world = vaab_vm::prepare(&parsed.module, &checked, Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(_) => Ok(world.output.lines().to_vec()),
        Err(problem) => Err(error::render(&problem, &name, &source, ColorChoice::Never)),
    }
}

#[test]
fn there_are_examples_to_run() {
    assert!(!sequential_examples().is_empty(), "examples/ should not be empty");
}

#[test]
fn every_sequential_example_runs() {
    let mut failures = String::new();

    for path in sequential_examples() {
        if let Err(problems) = run(&path) {
            failures.push_str(&format!("{}:\n{problems}\n", name_of(&path)));
        }
    }

    assert!(failures.is_empty(), "some examples do not run:\n{failures}");
}

/// What the examples print, all in one snapshot, so that a change to any of them
/// shows up as a change to the documentation it illustrates.
#[test]
fn the_examples_print_what_they_always_printed() {
    let mut transcript = String::new();

    for path in sequential_examples() {
        let name = name_of(&path);
        let lines = match run(&path) {
            Ok(lines) => lines,
            Err(problems) => panic!("{name} does not run:\n{problems}"),
        };
        transcript.push_str(&format!("$ vaab run examples/{name}\n"));
        for line in lines {
            transcript.push_str(&format!("{line}\n"));
        }
        transcript.push('\n');
    }

    assert_snapshot!(transcript);
}
