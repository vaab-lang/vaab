//! `needed.lock`: exact versions and paths after gather.

use std::path::PathBuf;

/// A parsed `needed.lock` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lockfile {
    pub entries: Vec<LockEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockEntry {
    Path { name: String, path: PathBuf },
    Registry {
        name: String,
        owner: String,
        version: String,
        digest: Option<String>,
        source: Option<String>,
    },
}

impl Lockfile {
    pub fn parse(source: &str) -> Result<Lockfile, String> {
        let mut entries = Vec::new();
        let mut current: Option<RegistryBuilder> = None;

        for line in source.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("path ") {
                flush_registry(&mut entries, &mut current)?;
                let (name, path_text) = parse_path_need(rest)?;
                entries.push(LockEntry::Path { name, path: PathBuf::from(path_text) });
                continue;
            }
            if let Some(rest) = line.strip_prefix("need ") {
                flush_registry(&mut entries, &mut current)?;
                let (name, owner, version) = parse_registry_need(rest)?;
                current = Some(RegistryBuilder { name, owner, version, digest: None, source: None });
                continue;
            }
            if let Some(digest) = line.strip_prefix("digest ") {
                let builder = current.as_mut().ok_or("a digest line has to follow a registry need")?;
                builder.digest = Some(digest.to_string());
                continue;
            }
            if let Some(source_url) = line.strip_prefix("source ") {
                let builder = current.as_mut().ok_or("a source line has to follow a registry need")?;
                builder.source = Some(source_url.to_string());
                continue;
            }
            return Err(format!("`{line}` is not something a lockfile understands"));
        }

        flush_registry(&mut entries, &mut current)?;
        Ok(Lockfile { entries })
    }

    pub fn to_string(&self) -> String {
        let mut out = String::from("# written by vaab gather. do not edit it.\n\n");
        for entry in &self.entries {
            match entry {
                LockEntry::Path { name, path } => {
                    out.push_str(&format!("path {name} {}\n", path.display()));
                }
                LockEntry::Registry { name, owner, version, digest, source } => {
                    out.push_str(&format!("need {name} from {owner} at {version}\n"));
                    if let Some(digest) = digest {
                        out.push_str(&format!("digest {digest}\n"));
                    }
                    if let Some(source) = source {
                        out.push_str(&format!("source {source}\n"));
                    }
                }
            }
        }
        out
    }

    pub fn path_for(&self, name: &str) -> Option<&PathBuf> {
        self.entries.iter().find_map(|entry| match entry {
            LockEntry::Path { name: entry_name, path } if entry_name == name => Some(path),
            _ => None,
        })
    }
}

struct RegistryBuilder {
    name: String,
    owner: String,
    version: String,
    digest: Option<String>,
    source: Option<String>,
}

fn flush_registry(entries: &mut Vec<LockEntry>, current: &mut Option<RegistryBuilder>) -> Result<(), String> {
    if let Some(builder) = current.take() {
        entries.push(LockEntry::Registry {
            name: builder.name,
            owner: builder.owner,
            version: builder.version,
            digest: builder.digest,
            source: builder.source,
        });
    }
    Ok(())
}

fn parse_registry_need(rest: &str) -> Result<(String, String, String), String> {
    let mut words = rest.split_whitespace();
    let name = words.next().ok_or("a lock need line is incomplete")?.to_string();
    let from = words.next().ok_or("a lock need line needs `from`")?;
    if from != "from" {
        return Err(format!("a lock need line needs `from`, not `{from}`"));
    }
    let owner = words.next().ok_or("a lock need line needs an owner")?.to_string();
    let at = words.next().ok_or("a lock need line needs `at`")?;
    if at != "at" {
        return Err(format!("a lock need line needs `at`, not `{at}`"));
    }
    let version = words.next().ok_or("a lock need line needs a version")?.to_string();
    if words.next().is_some() {
        return Err("a lock need line has too many words".to_string());
    }
    Ok((name, owner, version))
}

fn parse_path_need(rest: &str) -> Result<(String, String), String> {
    let mut words = rest.split_whitespace();
    let name = words.next().ok_or("a path lock line is incomplete")?.to_string();
    let path = words.next().ok_or("a path lock line needs a path")?.to_string();
    if words.next().is_some() {
        return Err("a path lock line has too many words".to_string());
    }
    Ok((name, path))
}
