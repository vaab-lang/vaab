//! The `riff` manifest: identity and dependencies in plain English.

use std::path::PathBuf;

/// A parsed `riff` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub licence: Option<String>,
    pub needs_vaab: Option<String>,
    pub dependencies: Vec<Dependency>,
}

/// One line in the manifest: `need json from ada at 1` or `need colours from ./vendor/colours`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    Registry { name: String, owner: String, range: Option<String> },
    Path { name: String, path: PathBuf },
}

impl Manifest {
    pub fn parse(source: &str) -> Result<Manifest, String> {
        let mut lines = source.lines().map(str::trim).filter(|line| !line.is_empty());
        let first = lines.next().ok_or("a riff file needs a first line: `riff name 1.0.0`")?;
        let (name, version) = parse_header(first)?;

        let mut manifest = Manifest {
            name,
            version,
            description: None,
            owner: None,
            licence: None,
            needs_vaab: None,
            dependencies: Vec::new(),
        };

        for line in lines {
            if line.starts_with('#') {
                continue;
            }
            if let Some(text) = line.strip_prefix("description ") {
                manifest.description = Some(text.to_string());
                continue;
            }
            if let Some(text) = line.strip_prefix("by ") {
                manifest.owner = Some(text.to_string());
                continue;
            }
            if let Some(text) = line.strip_prefix("licence ") {
                manifest.licence = Some(text.to_string());
                continue;
            }
            if let Some(text) = line.strip_prefix("needs vaab at ") {
                manifest.needs_vaab = Some(text.to_string());
                continue;
            }
            if let Some(rest) = line.strip_prefix("need ") {
                manifest.dependencies.push(parse_need_line(rest)?);
                continue;
            }
            return Err(format!("`{line}` is not something a riff file understands"));
        }

        Ok(manifest)
    }

    pub fn to_string(&self) -> String {
        let mut out = format!("riff {} {}\n", self.name, self.version);
        if let Some(description) = &self.description {
            out.push_str(&format!("description {description}\n"));
        }
        if let Some(owner) = &self.owner {
            out.push_str(&format!("by {owner}\n"));
        }
        if let Some(licence) = &self.licence {
            out.push_str(&format!("licence {licence}\n"));
        }
        if let Some(version) = &self.needs_vaab {
            out.push_str(&format!("needs vaab at {version}\n"));
        }
        for dependency in &self.dependencies {
            out.push_str(&dependency.to_string());
            out.push('\n');
        }
        out
    }
}

impl Dependency {
    pub fn name(&self) -> &str {
        match self {
            Dependency::Registry { name, .. } | Dependency::Path { name, .. } => name,
        }
    }

    fn to_string(&self) -> String {
        match self {
            Dependency::Registry { name, owner, range } => match range {
                Some(range) => format!("need {name} from {owner} at {range}"),
                None => format!("need {name} from {owner}"),
            },
            Dependency::Path { name, path } => {
                format!("need {name} from {}", path.display())
            }
        }
    }
}

fn parse_header(line: &str) -> Result<(String, String), String> {
    let mut words = line.split_whitespace();
    let riff = words.next().ok_or("a riff file needs a first line: `riff name 1.0.0`")?;
    if riff != "riff" {
        return Err(format!("a riff file starts with `riff name 1.0.0`, not `{line}`"));
    }
    let name = words
        .next()
        .ok_or("a riff file needs a name on its first line")?
        .to_string();
    let version = words
        .next()
        .ok_or("a riff file needs a version on its first line")?
        .to_string();
    if words.next().is_some() {
        return Err(format!("the first line has too many words: `{line}`"));
    }
    validate_snake_case(&name, "a riff name")?;
    Ok((name, version))
}

fn parse_need_line(rest: &str) -> Result<Dependency, String> {
    let mut words = rest.split_whitespace();
    let name = words
        .next()
        .ok_or("a need line looks like `need json from ada`")?
        .to_string();
    let from = words.next().ok_or("a need line needs the word `from`")?;
    if from != "from" {
        return Err(format!("a need line needs the word `from`, not `{from}`"));
    }
    let source = words
        .next()
        .ok_or("a need line needs something after `from`")?
        .to_string();

    let path_text = source.trim_matches('"');
    if path_text.starts_with("./") || path_text.starts_with("../") || path_text.starts_with('/') {
        let path = PathBuf::from(path_text);
        validate_trailing_words(words)?;
        return Ok(Dependency::Path { name, path });
    }

    let owner = source.trim_matches('"').to_string();
    let range = match words.next() {
        Some("at") => Some(
            words
                .next()
                .ok_or("a need line needs a version after `at`")?
                .to_string(),
        ),
        Some(other) => return Err(format!("a registry need ends with `at 1`, not `{other}`")),
        None => None,
    };
    validate_trailing_words(words)?;
    Ok(Dependency::Registry { name, owner, range })
}

fn validate_trailing_words<'a, I: Iterator<Item = &'a str>>(mut words: I) -> Result<(), String> {
    if let Some(extra) = words.next() {
        return Err(format!("`{extra}` was not expected on this line"));
    }
    Ok(())
}

fn validate_snake_case(name: &str, what: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("{what} cannot be empty"));
    }
    if !name.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit()) {
        return Err(format!("{what} has to be snake_case, like `task_api`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_riff_file_parses() {
        let manifest = Manifest::parse("riff blog 0.1.0\n").expect("should parse");
        assert_eq!(manifest.name, "blog");
        assert_eq!(manifest.version, "0.1.0");
    }

    #[test]
    fn dependencies_round_trip() {
        let source = "\
riff demo 1.0.0
description a demo
by ada
licence MIT
needs vaab at 0.1
need json from ada at 1
need colours from ./vendor/colours
";
        let manifest = Manifest::parse(source).expect("should parse");
        assert_eq!(manifest.description.as_deref(), Some("a demo"));
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(manifest.to_string(), source);
    }
}
