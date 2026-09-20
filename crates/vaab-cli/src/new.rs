//! `vaab new`: the smallest honest start for a Vaab project.
//!
//! Vaab has no package manager, no modules, and no imports yet, so a project is
//! one file that `vaab run` can execute. That is all this command writes.

use std::fs;
use std::path::{Path, PathBuf};

/// The file every new project gets.
pub const MAIN: &str = "\
# A small Vaab program.
# Run it with: vaab run main.vaab

print(\"hello\")
";

/// Creates `name/` with `main.vaab` inside it.
pub fn create(name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let directory = PathBuf::from(name);
    if directory.exists() {
        return Err(format!("`{}` already exists", directory.display()));
    }
    fs::create_dir(&directory).map_err(|error| {
        format!("could not create `{}`: {error}", directory.display())
    })?;
    let main = directory.join("main.vaab");
    fs::write(&main, MAIN).map_err(|error| {
        format!("could not write `{}`: {error}", main.display())
    })?;
    Ok(directory)
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a project needs a name, as in `vaab new orchard`".to_string());
    }
    if name == "." || name == ".." {
        return Err("a project name cannot be `.` or `..`".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("a project name cannot contain a path separator".to_string());
    }
    if Path::new(name).is_absolute() {
        return Err("a project name has to be a simple name, not a full path".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_name(prefix: &str) -> String {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        format!("vaab-new-test-{prefix}-{stamp}")
    }

    #[test]
    fn a_new_project_is_a_directory_with_main_vaab() {
        let name = temp_name("happy");
        let directory = create(&name).expect("should create the project");
        assert_eq!(directory, PathBuf::from(&name));
        assert!(directory.is_dir());
        let main = directory.join("main.vaab");
        assert!(main.is_file());
        assert_eq!(fs::read_to_string(&main).expect("should read main"), MAIN);
        fs::remove_dir_all(&directory).expect("should clean up");
    }

    #[test]
    fn an_existing_directory_is_refused() {
        let name = temp_name("exists");
        let directory = PathBuf::from(&name);
        fs::create_dir(&directory).expect("should create the directory");
        let Err(error) = create(&name) else {
            panic!("should refuse");
        };
        assert!(error.contains("already exists"), "{error}");
        fs::remove_dir(&directory).expect("should clean up");
    }

    #[test]
    fn a_name_with_a_slash_is_refused() {
        assert!(create("foo/bar").is_err());
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert!(create("").is_err());
    }
}
