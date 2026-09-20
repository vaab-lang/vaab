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
    Check { path: PathBuf },
    Run { path: PathBuf },
    Serve { path: PathBuf },
    Repl,
    New { name: String },
    Help,
    Version,
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
            Some("check") => Command::Check { path: expect_file(&mut words, "check")? },
            Some("run") => Command::Run { path: expect_file(&mut words, "run")? },
            Some("serve") => Command::Serve { path: expect_file(&mut words, "serve")? },
            Some("repl") => Command::Repl,
            Some("new") => Command::New { name: expect_name(&mut words, "new")? },
            Some("help") => Command::Help,
            Some("version") => Command::Version,

            Some(other) => return Err(format!("`{other}` is not a command vaab knows")),
            None => return Err("that command is not valid text".to_string()),
        };

        if let Some(extra) = words.next() {
            return Err(format!("vaab did not expect `{}`", extra.to_string_lossy()));
        }

        Ok(Args { command, color })
    }
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

fn expect_name(
    words: &mut impl Iterator<Item = OsString>,
    command: &str,
) -> Result<String, String> {
    match words.next() {
        Some(name) => Ok(name.to_string_lossy().into_owned()),
        None => Err(format!("`{command}` needs a name, as in `vaab {command} orchard`")),
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
    fn check_takes_a_file_like_parse_does() {
        let args = parse(&["check", "main.vaab"]).expect("should parse");
        match args.command {
            Command::Check { path } => assert_eq!(path, PathBuf::from("main.vaab")),
            _ => panic!("expected a check command"),
        }
    }

    #[test]
    fn check_without_a_file_is_explained() {
        match parse(&["check"]) {
            Err(error) => assert!(error.contains("vaab check main.vaab"), "{error}"),
            Ok(_) => panic!("`vaab check` with no file should be rejected"),
        }
    }

    #[test]
    fn check_rejects_a_second_file() {
        assert!(parse(&["check", "a.vaab", "b.vaab"]).is_err());
    }

    #[test]
    fn new_takes_a_name() {
        let args = parse(&["new", "orchard"]).expect("should parse");
        match args.command {
            Command::New { name } => assert_eq!(name, "orchard"),
            _ => panic!("expected a new command"),
        }
    }

    #[test]
    fn new_without_a_name_is_explained() {
        match parse(&["new"]) {
            Err(error) => assert!(error.contains("vaab new orchard"), "{error}"),
            Ok(_) => panic!("`vaab new` with no name should be rejected"),
        }
    }

    #[test]
    fn new_rejects_a_second_name() {
        assert!(parse(&["new", "orchard", "grove"]).is_err());
    }

    #[test]
    fn run_takes_a_file_like_check_does() {
        let args = parse(&["run", "main.vaab"]).expect("should parse");
        match args.command {
            Command::Run { path } => assert_eq!(path, PathBuf::from("main.vaab")),
            _ => panic!("expected a run command"),
        }
    }

    #[test]
    fn run_without_a_file_is_explained() {
        match parse(&["run"]) {
            Err(error) => assert!(error.contains("vaab run main.vaab"), "{error}"),
            Ok(_) => panic!("`vaab run` with no file should be rejected"),
        }
    }

    #[test]
    fn run_rejects_a_second_file() {
        assert!(parse(&["run", "a.vaab", "b.vaab"]).is_err());
    }

    #[test]
    fn the_repl_needs_nothing_after_it() {
        assert!(matches!(parse(&["repl"]).map(|a| a.command), Ok(Command::Repl)));
    }

    #[test]
    fn the_repl_rejects_a_file_because_it_reads_from_the_keyboard() {
        assert!(parse(&["repl", "a.vaab"]).is_err());
    }

    #[test]
    fn the_repl_still_takes_the_colour_options() {
        let args = parse(&["repl", "--no-color"]).expect("should parse");
        assert_eq!(args.color, ColorChoice::Never);
    }
}
