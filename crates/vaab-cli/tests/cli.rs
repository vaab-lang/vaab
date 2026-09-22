//! The tool as a person meets it: a command, some output, an exit code.
//!
//! These run the real binary, because the thing worth testing is the contract the
//! shell sees. The exit codes are the ones the project settled on: 0 when it
//! worked, 1 when the program has problems in it, 2 when the command was wrong.

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;

/// These tests change the process working directory; only one may run at a time.
static WORKING_DIRECTORY: Mutex<()> = Mutex::new(());

const OK: i32 = 0;
const PROBLEMS: i32 = 1;
const MISUSE: i32 = 2;

fn vaab(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vaab"))
        .args(arguments)
        .arg("--no-color")
        .output()
        .unwrap_or_else(|error| panic!("could not run vaab: {error}"))
}

/// Runs the tool with `typed` on its standard input, which is what a REPL wants.
fn vaab_with_input(arguments: &[&str], typed: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vaab"))
        .args(arguments)
        .arg("--no-color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("could not run vaab: {error}"));

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(typed.as_bytes()).expect("should write to vaab");
    }
    child.wait_with_output().expect("vaab should finish")
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes a file next to the test binary and gives back its path.
fn file(name: &str, source: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("vaab-cli-test-{name}"));
    std::fs::write(&path, source).expect("should write the test file");
    path
}

// ---------------------------------------------------------------------------
// vaab run
// ---------------------------------------------------------------------------

#[test]
fn run_prints_what_the_program_prints_and_succeeds() {
    let path = file("hello.vaab", "print(\"hello\")\n");
    let output = vaab(&["run", &path.display().to_string()]);

    assert_eq!(code(&output), OK, "{}", err(&output));
    assert_eq!(out(&output), "hello\n");
}

#[test]
fn run_refuses_a_program_with_type_errors_and_runs_none_of_it() {
    let path = file("mistyped.vaab", "print(\"first\")\nlet wrong: Int = \"two\"\n");
    let output = vaab(&["run", &path.display().to_string()]);

    assert_eq!(code(&output), PROBLEMS);
    assert!(out(&output).is_empty(), "nothing should have run: {}", out(&output));
    assert!(err(&output).contains("expected Int, found Text"), "{}", err(&output));
    assert!(err(&output).contains("Found 1 problem."), "{}", err(&output));
}

#[test]
fn run_refuses_a_program_that_does_not_parse() {
    let path = file("broken.vaab", "let = 1\n");
    let output = vaab(&["run", &path.display().to_string()]);

    assert_eq!(code(&output), PROBLEMS);
    assert!(err(&output).contains("Found"), "{}", err(&output));
}

#[test]
fn run_reports_a_program_that_stops_partway_and_keeps_what_it_printed() {
    let path = file("stops.vaab", "print(\"before\")\nlet none = 0\nprint(1 / none)\n");
    let output = vaab(&["run", &path.display().to_string()]);

    assert_eq!(code(&output), PROBLEMS);
    assert_eq!(out(&output), "before\n");
    assert!(err(&output).contains("this divides by zero"), "{}", err(&output));
    assert!(err(&output).contains("Found 1 problem."), "{}", err(&output));
}

#[test]
fn run_says_so_when_the_file_is_not_there() {
    let output = vaab(&["run", "no-such-file.vaab"]);

    assert_eq!(code(&output), MISUSE);
    assert!(err(&output).contains("could not read"), "{}", err(&output));
}

#[test]
fn run_without_a_file_is_a_misuse() {
    let output = vaab(&["run"]);

    assert_eq!(code(&output), MISUSE);
    assert!(err(&output).contains("vaab run main.vaab"), "{}", err(&output));
}

// ---------------------------------------------------------------------------
// vaab repl
// ---------------------------------------------------------------------------

#[test]
fn the_repl_shows_the_value_of_what_was_typed() {
    let output = vaab_with_input(&["repl"], "1 + 1\n");

    assert_eq!(code(&output), OK);
    assert!(out(&output).contains("2"), "{}", out(&output));
}

#[test]
fn the_repl_remembers_what_earlier_lines_worked_out() {
    let output = vaab_with_input(&["repl"], "let x = 21\nx * 2\n");

    assert!(out(&output).contains("42"), "{}", out(&output));
}

#[test]
fn the_repl_remembers_a_function_declared_on_an_earlier_line() {
    let typed = "to double(n: Int) returns Int = n * 2\ndouble(4)\n";
    let output = vaab_with_input(&["repl"], typed);

    assert!(out(&output).contains("8"), "{}", out(&output));
}

#[test]
fn the_repl_keeps_reading_while_a_brace_is_open() {
    let typed = "let changing total = 0\nfor each n in 1..4 {\n    total = total + n\n}\ntotal\n";
    let output = vaab_with_input(&["repl"], typed);

    assert!(out(&output).contains("10"), "{}", out(&output));
}

#[test]
fn a_type_error_does_not_end_the_session() {
    let output = vaab_with_input(&["repl"], "let wrong: Int = \"two\"\n1 + 1\n");

    assert_eq!(code(&output), OK);
    assert!(err(&output).contains("expected Int, found Text"), "{}", err(&output));
    assert!(out(&output).contains("2"), "{}", out(&output));
}

#[test]
fn a_runtime_error_does_not_end_the_session() {
    let output = vaab_with_input(&["repl"], "let none = 0\n1 / none\n\"still here\"\n");

    assert_eq!(code(&output), OK);
    assert!(err(&output).contains("this divides by zero"), "{}", err(&output));
    assert!(out(&output).contains("still here"), "{}", out(&output));
}

#[test]
fn a_line_that_only_does_something_shows_nothing_extra() {
    let output = vaab_with_input(&["repl"], "print(\"said\")\n");

    // `print` has spoken already; there is no value left to answer with.
    assert_eq!(out(&output).matches("said").count(), 1, "{}", out(&output));
}

#[test]
fn a_blank_line_is_passed_over() {
    let output = vaab_with_input(&["repl"], "\n\n1\n");

    assert_eq!(code(&output), OK);
    assert!(out(&output).contains("1"), "{}", out(&output));
}

#[test]
fn the_session_ends_cleanly_when_the_input_runs_out() {
    let output = vaab_with_input(&["repl"], "");
    assert_eq!(code(&output), OK);
}

// ---------------------------------------------------------------------------
// vaab new
// ---------------------------------------------------------------------------

#[test]
fn new_writes_a_project_that_vaab_run_can_execute() {
    let _guard = WORKING_DIRECTORY.lock().expect("should lock the working directory");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let parent = std::env::temp_dir().join(format!("vaab-cli-new-parent-{stamp}"));
    let name = format!("demo_{stamp}");
    let directory = parent.join(&name);
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).expect("should create the parent directory");

    let previous = std::env::current_dir().expect("should know the working directory");
    std::env::set_current_dir(&parent).expect("should move into the temp directory");

    let create = vaab(&["new", &name]);
    assert_eq!(code(&create), OK, "{}", err(&create));

    let main = directory.join("main.vaab");
    let run = vaab(&["run", &main.display().to_string()]);
    assert_eq!(code(&run), OK, "{}", err(&run));
    assert_eq!(out(&run), "hello\n");

    std::env::set_current_dir(previous).expect("should restore the working directory");
    std::fs::remove_dir_all(&parent).expect("should clean up");
}

#[test]
fn new_refuses_to_overwrite_an_existing_directory() {
    let _guard = WORKING_DIRECTORY.lock().expect("should lock the working directory");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let parent = std::env::temp_dir().join(format!("vaab-cli-new-exists-parent-{stamp}"));
    let name = format!("taken_{stamp}");
    let directory = parent.join(&name);
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&directory).expect("should create the directory");

    let previous = std::env::current_dir().expect("should know the working directory");
    std::env::set_current_dir(&parent).expect("should move into the temp directory");

    let output = vaab(&["new", &name]);
    assert_eq!(code(&output), MISUSE);
    assert!(err(&output).contains("already exists"), "{}", err(&output));

    std::env::set_current_dir(previous).expect("should restore the working directory");
    std::fs::remove_dir_all(&parent).expect("should clean up");
}

#[test]
fn new_without_a_name_is_a_misuse() {
    let output = vaab(&["new"]);
    assert_eq!(code(&output), MISUSE);
    assert!(err(&output).contains("vaab new orchard"), "{}", err(&output));
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

#[test]
fn the_help_lists_the_commands_this_phase_added() {
    let output = vaab(&["help"]);

    assert_eq!(code(&output), OK);
    assert!(out(&output).contains("run <file>"), "{}", out(&output));
    assert!(out(&output).contains("repl"), "{}", out(&output));
    assert!(out(&output).contains("new <name>"), "{}", out(&output));
}

#[test]
fn the_help_no_longer_promises_run_for_a_later_phase() {
    let output = vaab(&["help"]);
    let text = out(&output);

    let coming = text.split("Coming later:").nth(1).unwrap_or_default();
    assert!(!coming.contains("run <file>"), "{text}");
    assert!(!coming.contains("repl"), "{text}");
}
