//! The `vaab` command-line tool.
//!
//! Phase 1 shipped `vaab parse`, phase 2 `vaab check`, and phase 3 `vaab run` and
//! `vaab repl`. The commands still to come are listed in the help text with the
//! phase that brings them, so the tool never silently pretends to do less than
//! the language can.

mod args;
mod repl;

use std::path::Path;
use std::process::ExitCode;

use vaab_syntax::{diagnostic, ColorChoice};
use vaab_vm::{error, Output};

use args::{Args, Command};

/// What the shell sees when `vaab` finishes.
mod exit {
    use std::process::ExitCode;

    /// Everything worked.
    pub const OK: ExitCode = ExitCode::SUCCESS;
    /// The program was read but has problems in it.
    pub const PROBLEMS: ExitCode = ExitCode::FAILURE;
    /// The command itself was wrong: a bad flag, a missing file.
    pub fn misuse() -> ExitCode {
        ExitCode::from(2)
    }
}

fn main() -> ExitCode {
    match Args::from_environment() {
        Ok(args) => run(args),
        Err(message) => {
            eprintln!("vaab: {message}");
            eprintln!("\nRun `vaab help` to see what vaab can do.");
            exit::misuse()
        }
    }
}

fn run(args: Args) -> ExitCode {
    match args.command {
        Command::Help => {
            println!("{}", help_text());
            exit::OK
        }
        Command::Version => {
            println!("vaab {}", env!("CARGO_PKG_VERSION"));
            exit::OK
        }
        Command::Parse { path } => parse_file(&path, args.color),
        Command::Check { path } => check_file(&path, args.color),
        Command::Run { path } => run_file(&path, args.color),
        Command::Repl => {
            repl::start(args.color);
            exit::OK
        }
        Command::NotYet { name, phase, summary } => {
            eprintln!("vaab: `{name}` is not built yet.");
            eprintln!("      {summary}");
            eprintln!("      It arrives in phase {phase}.");
            exit::misuse()
        }
    }
}

/// `vaab parse file.vaab`: read a file, parse it, and print the tree.
fn parse_file(path: &Path, color: ColorChoice) -> ExitCode {
    let Some(source) = read(path) else { return exit::misuse() };

    // Diagnostics quote the file by name, so use the path exactly as it was typed.
    let name = path.display().to_string();
    let parsed = vaab_syntax::parse(&source);

    if parsed.has_errors() {
        return report(&parsed.diagnostics, &name, &source, color);
    }

    print!("{}", vaab_syntax::print_module(&parsed.module));
    exit::OK
}

/// `vaab check file.vaab`: read a file, parse it, and check its types.
///
/// A file that will not parse is not type-checked: every type error after a syntax
/// error would be guesswork about a program nobody has written yet.
fn check_file(path: &Path, color: ColorChoice) -> ExitCode {
    let Some(source) = read(path) else { return exit::misuse() };

    let name = path.display().to_string();
    let parsed = vaab_syntax::parse(&source);

    if parsed.has_errors() {
        return report(&parsed.diagnostics, &name, &source, color);
    }

    match vaab_types::check(&parsed.module) {
        Ok(_) => {
            println!("{name} checks out.");
            exit::OK
        }
        Err(problems) => report(&problems, &name, &source, color),
    }
}

/// `vaab run file.vaab`: check a file, then run it.
///
/// A program with type errors is not run at all. Half-running something that was
/// never going to work would teach a person less than the errors do, and the
/// whole point of checking first is that the errors come before the damage.
fn run_file(path: &Path, color: ColorChoice) -> ExitCode {
    let Some(source) = read(path) else { return exit::misuse() };

    let name = path.display().to_string();
    let parsed = vaab_syntax::parse(&source);

    if parsed.has_errors() {
        return report(&parsed.diagnostics, &name, &source, color);
    }

    let checked = match vaab_types::check(&parsed.module) {
        Ok(checked) => checked,
        Err(problems) => return report(&problems, &name, &source, color),
    };

    let mut world = vaab_vm::prepare(&parsed.module, &checked, Output::Terminal);
    match vaab_vm::run(&mut world) {
        Ok(_) => exit::OK,
        Err(problem) => {
            eprint!("{}", error::render(&problem, &name, &source, color));
            eprintln!("{}", summarise(1));
            exit::PROBLEMS
        }
    }
}

fn read(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(error) => {
            eprintln!("vaab: could not read `{}`: {error}", path.display());
            None
        }
    }
}

fn report(
    problems: &[diagnostic::Diagnostic],
    name: &str,
    source: &str,
    color: ColorChoice,
) -> ExitCode {
    eprint!("{}", diagnostic::render(problems, name, source, color));
    eprintln!("{}", summarise(problems.len()));
    exit::PROBLEMS
}

fn summarise(count: usize) -> String {
    match count {
        1 => "Found 1 problem.".to_string(),
        other => format!("Found {other} problems."),
    }
}

fn help_text() -> String {
    format!(
        "vaab {version} — a plain-English, strictly typed language

Usage:
  vaab <command> [options] [file]

Commands:
  parse <file>   Read a file and print the syntax tree
  check <file>   Read a file and check its types
  run <file>     Check a file and run it
  repl           Start an interactive session
  help           Show this message
  version        Show the version

Coming later:
  new <name>     Start a new project                   (phase 5)

Options:
  --no-color     Never colour the output
  --color        Always colour the output

Colour is on by default, and off when the NO_COLOR environment variable is set.
",
        version = env!("CARGO_PKG_VERSION")
    )
}
