//! The `vaab` command-line tool.
//!
//! Phase 1 shipped `vaab parse`, phase 2 `vaab check`, and phase 3 `vaab run` and
//! `vaab repl`. The commands still to come are listed in the help text with the
//! phase that brings them, so the tool never silently pretends to do less than
//! the language can.

mod args;
mod new;
mod repl;
mod riff;

use std::path::Path;
use std::process::ExitCode;

use vaab_syntax::{diagnostic, ast::StmtKind, ColorChoice};
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
        Command::Serve { path } => serve_file(&path, args.color),
        Command::Repl => {
            repl::start(args.color);
            exit::OK
        }
        Command::New { name } => new_project(&name),
        Command::Gather { path } => gather_project(path.as_deref()),
        Command::Need { words } => add_need(&words),
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
    let module = match load_module(path, &source, &name, color) {
        Ok(module) => module,
        Err(code) => return code,
    };

    match vaab_types::check(&module) {
        Ok(_) => {
            println!("{name} checks out.");
            exit::OK
        }
        Err(problems) => report(&problems, &name, &source, color),
    }
}

/// `vaab serve file.vaab`: check a file, then start its HTTP server.
fn serve_file(path: &Path, color: ColorChoice) -> ExitCode {
    let Some(source) = read(path) else { return exit::misuse() };

    let name = path.display().to_string();
    let module = match load_module(path, &source, &name, color) {
        Ok(module) => module,
        Err(code) => return code,
    };

    let checked = match vaab_types::check(&module) {
        Ok(checked) => checked,
        Err(problems) => return report(&problems, &name, &source, color),
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(vaab_server::serve_file(&source, &module, &checked)) {
        Ok(()) => exit::OK,
        Err(message) => {
            eprintln!("vaab: {message}");
            exit::PROBLEMS
        }
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
    let module = match load_module(path, &source, &name, color) {
        Ok(module) => module,
        Err(code) => return code,
    };

    let checked = match vaab_types::check(&module) {
        Ok(checked) => checked,
        Err(problems) => return report(&problems, &name, &source, color),
    };

    let mut world = vaab_vm::prepare(&module, &checked, Output::Terminal);
    match vaab_vm::run(&mut world) {
        Ok(_) => exit::OK,
        Err(problem) => {
            eprint!("{}", error::render(&problem, &name, &source, color));
            eprintln!("{}", summarise(1));
            exit::PROBLEMS
        }
    }
}

/// `vaab new orchard`: write the smallest project Vaab knows how to run today.
fn new_project(name: &str) -> ExitCode {
    match new::create(name) {
        Ok(directory) => {
            println!("Created {}/", directory.display());
            println!("  riff");
            println!("  main.vaab");
            println!();
            println!("Run it with:");
            println!("  vaab run {}/main.vaab", directory.display());
            println!();
            println!("Add a riff with:");
            println!("  cd {} && vaab need json from ada", directory.display());
            exit::OK
        }
        Err(message) => {
            eprintln!("vaab: {message}");
            exit::misuse()
        }
    }
}

fn gather_project(path: Option<&Path>) -> ExitCode {
    match riff::gather_project(path) {
        Ok(()) => exit::OK,
        Err(message) => {
            eprintln!("vaab: {message}");
            exit::misuse()
        }
    }
}

fn add_need(words: &[String]) -> ExitCode {
    match riff::add_need(words) {
        Ok(()) => exit::OK,
        Err(message) => {
            eprintln!("vaab: {message}");
            exit::misuse()
        }
    }
}

fn load_module(
    path: &Path,
    source: &str,
    name: &str,
    color: ColorChoice,
) -> Result<vaab_syntax::ast::Module, ExitCode> {
    let parsed = vaab_syntax::parse(source);
    if parsed.has_errors() {
        return Err(report(&parsed.diagnostics, name, source, color));
    }
    if parsed
        .module
        .statements
        .iter()
        .any(|statement| matches!(statement.kind, StmtKind::Need(_)))
    {
        match vaab_riff::link(path) {
            Ok(linked) => Ok(linked.module),
            Err(message) => {
                eprintln!("vaab: {message}");
                Err(exit::misuse())
            }
        }
    } else {
        Ok(parsed.module)
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
  serve <file>   Check a file and start its HTTP server
  repl           Start an interactive session
  new <name>     Start a new project in a new directory
  gather         Resolve riff dependencies into needed.lock
  need ...       Add a riff dependency, as in `vaab need json from ada`
  help           Show this message
  version        Show the version

Options:
  --no-color     Never colour the output
  --color        Always colour the output

Colour is on by default, and off when the NO_COLOR environment variable is set.
",
        version = env!("CARGO_PKG_VERSION")
    )
}
