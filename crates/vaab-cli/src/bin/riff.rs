//! The `riff` tool — install and manage Vaab packages.
//!
//! Usage:
//!   riff install supabase
//!   riff list

use std::process::ExitCode;

use vaab_riff::{install, is_installed, vaab_home};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("riff: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut words = args.into_iter();
    let Some(command) = words.next() else {
        print_help();
        return Ok(());
    };

    match command.as_str() {
        "install" => {
            let Some(name) = words.next() else {
                return Err("an install needs a name, as in `riff install supabase`".to_string());
            };
            if words.next().is_some() {
                return Err("install takes one name".to_string());
            }
            let path = install(&name)?;
            println!("Installed {name} to {}", path.display());
            Ok(())
        }
        "list" => {
            let home = vaab_home()?.join("riffs");
            if !home.is_dir() {
                println!("No riffs installed yet.");
                return Ok(());
            }
            let mut names = Vec::new();
            for entry in std::fs::read_dir(&home).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if entry.file_type().map_err(|error| error.to_string())?.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if is_installed(&name) {
                        names.push(name);
                    }
                }
            }
            names.sort();
            if names.is_empty() {
                println!("No riffs installed yet.");
            } else {
                for name in names {
                    println!("{name}");
                }
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("`{other}` is not a command riff knows")),
    }
}

fn print_help() {
    println!(
        "riff — install Vaab packages

Usage:
  riff install <name>   Install a riff into ~/.vaab/riffs
  riff list             List installed riffs
  riff help             Show this message

Then in your app:
  need supabase
"
    );
}
