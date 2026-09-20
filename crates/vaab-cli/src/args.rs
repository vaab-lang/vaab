//! Command-line arguments.
//!
//! Parsed by hand. The grammar is small enough that a dependency would cost more
//! than it saves, and the error messages can then be written in the same voice as
//! the compiler's.

use std::ffi::OsString;
use std::path::PathBuf;

use vaab_syntax::ColorChoice;

pub struct Args {
    pub command: Command,
    pub color: ColorChoice,
}

pub enum Command {
    Parse { path: PathBuf },
    Help,
    Version,
    /// A command the language will have, but does not have yet.
    NotYet { name: String, phase: u8, summary: &'static str },
}

impl Args {
    pub fn from_environment() -> Result<Args, String> {
        Args::from_iter(std::env::args_os().skip(1))
    }

    fn from_iter(arguments: impl Iterator<Item = OsString>) -> Result<Args, String> {
        let mut color = if std::env::var_os("NO_COLOR").is_some() {
            ColorChoice::Never
        } else {
            ColorChoice::Always
        };

        let mut words: Vec<OsString> = Vec::new();
        for argument in arguments {
            match argument.to_str() {
                Some("--no-color") => color = ColorChoice::Never,
                Some("--color") => color = ColorChoice::Always,
                Some("--help" | "-h") => return Ok(Args { command: Command::Help, color }),
                Some("--version" | "-V") => return Ok(Args { command: Command::Version, color }),
                Some(flag) if flag.starts_with('-') && flag != "-" => {
                    return Err(format!("`{flag}` is not an option vaab knows"));
                }
                _ => words.push(argument),
            }
        }

        let mut words = words.into_iter();
        let Some(command) = words.next() else {
            return Ok(Args { command: Command::Help, color });
        };

        let command = match command.to_str() {
            Some("parse") => Command::Parse { path: expect_file(&mut words, "parse")? },
            Some("help") => Command::Help,
            Some("version") => Command::Version,

            Some(name @ "check") => not_yet(name, 2, "checks the types in a file"),
            Some(name @ "run") => not_yet(name, 3, "runs a file"),
            Some(name @ "repl") => not_yet(name, 3, "starts an interactive session"),
            Some(name @ "new") => not_yet(name, 5, "starts a new project"),

            Some(other) => return Err(format!("`{other}` is not a command vaab knows")),
            None => return Err("that command is not valid text".to_string()),
        };

        // A command that does not exist yet is the news; whatever was passed to it
        // is not worth complaining about.
        if matches!(command, Command::NotYet { .. }) {
            return Ok(Args { command, color });
        }

        if let Some(extra) = words.next() {
            return Err(format!("vaab did not expect `{}`", extra.to_string_lossy()));
        }

        Ok(Args { command, color })
    }
}

fn not_yet(name: &str, phase: u8, summary: &'static str) -> Command {
    Command::NotYet { name: name.to_string(), phase, summary }
}

fn expect_file(
    words: &mut impl Iterator<Item = OsString>,
    command: &str,
) -> Result<PathBuf, String> {
    match words.next() {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err(format!("`{command}` needs a file, as in `vaab {command} main.vaab`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Args, String> {
        Args::from_iter(words.iter().map(OsString::from))
    }

    #[test]
    fn a_file_is_taken_from_the_command_line() {
        let args = parse(&["parse", "main.vaab"]).expect("should parse");
        match args.command {
            Command::Parse { path } => assert_eq!(path, PathBuf::from("main.vaab")),
            _ => panic!("expected a parse command"),
        }
    }

    #[test]
    fn options_may_come_before_or_after_the_file() {
        for words in [&["--no-color", "parse", "a.vaab"], &["parse", "a.vaab", "--no-color"]] {
            let args = parse(words).expect("should parse");
            assert_eq!(args.color, ColorChoice::Never);
        }
    }

    #[test]
    fn no_arguments_shows_the_help() {
        assert!(matches!(parse(&[]).map(|a| a.command), Ok(Command::Help)));
    }

    #[test]
    fn a_missing_file_is_explained() {
        match parse(&["parse"]) {
            Err(error) => assert!(error.contains("vaab parse main.vaab"), "{error}"),
            Ok(_) => panic!("`vaab parse` with no file should be rejected"),
        }
    }

    #[test]
    fn unknown_commands_and_options_are_rejected() {
        assert!(parse(&["fly"]).is_err());
        assert!(parse(&["parse", "--loudly", "a.vaab"]).is_err());
    }

    #[test]
    fn later_commands_say_which_phase_brings_them() {
        match parse(&["run", "a.vaab"]).map(|a| a.command) {
            Ok(Command::NotYet { phase, .. }) => assert_eq!(phase, 3),
            _ => panic!("expected a not-yet command"),
        }
    }
}
