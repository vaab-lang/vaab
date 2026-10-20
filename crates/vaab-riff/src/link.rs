//! Link a project entry file with its needed riffs.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use vaab_syntax::ast::{Module, Name, NeedImports, NeedSource, NeedStmt, Stmt, StmtKind};
use vaab_syntax::{diagnostic, parse, ColorChoice, Span};

use crate::install::installed_path;
use crate::lock::Lockfile;
use crate::manifest::Manifest;
use crate::merge;
use crate::project::find_project_root;

/// A project ready for type-checking: main module plus loaded riffs.
#[derive(Clone, Debug)]
pub struct Linked {
    pub project_root: PathBuf,
    pub entry: PathBuf,
    pub module: Module,
    pub sources: HashMap<String, String>,
    pub riffs: Vec<LoadedRiff>,
}

#[derive(Clone, Debug)]
pub struct LoadedRiff {
    pub alias: String,
    pub path: PathBuf,
    pub module: Module,
    pub imports: RiffImports,
}

#[derive(Clone, Debug)]
pub enum RiffImports {
    /// Prefix every export (`alias__name`) and reach it as `alias.name`.
    Qualified,
    /// Leave the listed names unprefixed and prefix everything else.
    Bare(HashSet<String>),
    /// A `.vaab` file included on its own: every name stays as written, and
    /// `alias.name` reaches the same declaration.
    Flat,
}

enum ResolvedNeed {
    File(PathBuf),
    RiffDir(PathBuf),
}

/// Read `main.vaab` (or another entry), resolve `need` lines, and return a linked module.
pub fn link(entry: &Path) -> Result<Linked, String> {
    let entry = fs::canonicalize(entry)
        .map_err(|error| format!("could not read `{}`: {error}", entry.display()))?;
    let project_root = find_project_root(&entry).unwrap_or_else(|_| {
        entry
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let manifest_path = project_root.join("riff");
    let lock_path = project_root.join("needed.lock");

    let _manifest = if manifest_path.is_file() {
        let source = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("could not read `{}`: {error}", manifest_path.display()))?;
        Some(Manifest::parse(&source)?)
    } else {
        None
    };

    let lock = if lock_path.is_file() {
        let source = fs::read_to_string(&lock_path)
            .map_err(|error| format!("could not read `{}`: {error}", lock_path.display()))?;
        Some(Lockfile::parse(&source)?)
    } else {
        None
    };

    let mut loader = Loader {
        project_root,
        lock,
        sources: HashMap::new(),
        visiting: Vec::new(),
        visited: HashSet::new(),
        riffs: Vec::new(),
    };

    loader.visiting.push(entry.clone());
    let (needs, module) = loader.read_module(&entry)?;
    for need in needs {
        loader.load_need(&entry, &need)?;
    }
    loader.visiting.pop();
    loader.visited.insert(entry.clone());

    let module = merge::merge(module, &loader.riffs);

    Ok(Linked {
        project_root: loader.project_root,
        entry,
        module,
        sources: loader.sources,
        riffs: loader.riffs,
    })
}

struct Loader {
    project_root: PathBuf,
    lock: Option<Lockfile>,
    sources: HashMap<String, String>,
    visiting: Vec<PathBuf>,
    visited: HashSet<PathBuf>,
    riffs: Vec<LoadedRiff>,
}

impl Loader {
    /// Load one `need`, including anything that file needs, before its own declarations.
    fn load_need(&mut self, from_file: &Path, need: &NeedStmt) -> Result<(), String> {
        let resolved = self.resolve(from_file, need)?;
        let source_file = resolved.source_file();
        let canonical = fs::canonicalize(&source_file).map_err(|error| {
            format!("could not read `{}`: {error}", source_file.display())
        })?;

        if let Some(start) = self.visiting.iter().position(|path| path == &canonical) {
            let mut chain: Vec<String> = self.visiting[start..]
                .iter()
                .map(|path| file_label(path))
                .collect();
            chain.push(file_label(&canonical));
            let shown = chain
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(format!("cycle detected: {shown}"));
        }
        if self.visited.contains(&canonical) {
            return Ok(());
        }

        self.visiting.push(canonical.clone());
        let (needs, module) = self.read_module(&source_file)?;
        for child in needs {
            self.load_need(&source_file, &child)?;
        }
        self.visiting.pop();
        self.visited.insert(canonical.clone());

        let alias = riff_alias(need);
        if self.riffs.iter().any(|loaded| loaded.alias == alias) {
            return Err(format!("`{alias}` is needed twice"));
        }

        let stored = match &resolved {
            ResolvedNeed::File(_) => canonical,
            ResolvedNeed::RiffDir(dir) => fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()),
        };
        self.riffs.push(LoadedRiff {
            alias,
            path: stored,
            module,
            imports: imports_for(need, &resolved),
        });
        Ok(())
    }

    fn read_module(&mut self, path: &Path) -> Result<(Vec<NeedStmt>, Module), String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("could not read `{}`: {error}", path.display()))?;
        let parsed = parse(&source);
        if parsed.has_errors() {
            let rendered = diagnostic::render(
                &parsed.diagnostics,
                &path.display().to_string(),
                &source,
                ColorChoice::Never,
            );
            return Err(format!("`{}` does not parse\n{rendered}", path.display()));
        }
        self.sources.insert(path.display().to_string(), source);
        Ok(split_needs(parsed.module))
    }

    fn resolve(&self, from_file: &Path, need: &NeedStmt) -> Result<ResolvedNeed, String> {
        let name = need.name.text.as_str();
        if let Some(lock) = &self.lock {
            if let Some(path) = lock.path_for(name) {
                let resolved = if path.is_relative() {
                    self.project_root.join(path)
                } else {
                    path.clone()
                };
                return classify(&resolved).ok_or_else(|| {
                    format!(
                        "`{name}` in needed.lock points at `{}`, which is not a .vaab file or a riff",
                        resolved.display()
                    )
                });
            }
        }

        match &need.source {
            NeedSource::Path { path, .. } => {
                let raw = PathBuf::from(path);
                let from_dir = from_file.parent().unwrap_or_else(|| Path::new("."));
                let primary = if raw.is_absolute() { raw.clone() } else { from_dir.join(&raw) };
                if let Some(found) = classify(&primary) {
                    return Ok(found);
                }
                if !raw.is_absolute() {
                    let fallback = self.project_root.join(&raw);
                    if fallback != primary {
                        if let Some(found) = classify(&fallback) {
                            return Ok(found);
                        }
                    }
                }
                Err(format!(
                    "could not find `{name}` — `{}` is not a .vaab file or a riff",
                    primary.display()
                ))
            }
            NeedSource::Installed => {
                if let Ok(installed) = installed_path(name) {
                    if let Some(found) = classify(&installed) {
                        return Ok(found);
                    }
                }
                let from_dir = from_file.parent().unwrap_or_else(|| Path::new("."));
                let sibling = from_dir.join(format!("{name}.vaab"));
                if let Some(found) = classify(&sibling) {
                    return Ok(found);
                }
                Err(format!(
                    "could not find `{name}` — there is no installed riff and no `{name}.vaab` beside `{}`",
                    from_file.display()
                ))
            }
            NeedSource::Registry { owner } => Err(format!(
                "run `vaab gather` first — `{name}` from `{}` is not in needed.lock yet",
                owner.text
            )),
        }
    }
}

impl ResolvedNeed {
    fn source_file(&self) -> PathBuf {
        match self {
            ResolvedNeed::File(path) => path.clone(),
            ResolvedNeed::RiffDir(path) => path.join("lib.vaab"),
        }
    }
}

/// A path is a `.vaab` file when that file exists, and a riff when it contains `lib.vaab`.
fn classify(path: &Path) -> Option<ResolvedNeed> {
    if path.is_file() {
        return Some(ResolvedNeed::File(path.to_path_buf()));
    }
    if path.join("lib.vaab").is_file() {
        return Some(ResolvedNeed::RiffDir(path.to_path_buf()));
    }
    if path.extension().is_none() {
        let with_vaab = path.with_extension("vaab");
        if with_vaab.is_file() {
            return Some(ResolvedNeed::File(with_vaab));
        }
    }
    None
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn split_needs(module: Module) -> (Vec<NeedStmt>, Module) {
    let mut needs = Vec::new();
    let mut statements = Vec::new();
    for statement in module.statements {
        match statement.kind {
            StmtKind::Need(need) => needs.push(need),
            other => statements.push(Stmt { kind: other, ..statement }),
        }
    }
    (
        needs,
        Module { statements, span: module.span },
    )
}

fn riff_alias(need: &NeedStmt) -> String {
    match &need.imports {
        NeedImports::As(alias) => alias.text.clone(),
        _ => need.name.text.clone(),
    }
}

fn imports_for(need: &NeedStmt, resolved: &ResolvedNeed) -> RiffImports {
    match (&need.imports, resolved) {
        (NeedImports::Of(names), _) => {
            RiffImports::Bare(names.iter().map(|name| name.text.clone()).collect())
        }
        (NeedImports::As(_), _) | (NeedImports::Qualified, ResolvedNeed::RiffDir(_)) => {
            RiffImports::Qualified
        }
        (NeedImports::Qualified, ResolvedNeed::File(_)) => RiffImports::Flat,
    }
}

/// Names a riff exports from its `lib.vaab`.
pub fn exports_of(module: &Module) -> HashSet<String> {
    let mut names = HashSet::new();
    for statement in &module.statements {
        match &statement.kind {
            StmtKind::Function(function) => names.insert(function.name.text.clone()),
            StmtKind::Type(declaration) => names.insert(declaration.name.text.clone()),
            StmtKind::Cast(declaration) => names.insert(declaration.name.text.clone()),
            StmtKind::Choice(declaration) => names.insert(declaration.name.text.clone()),
            StmtKind::Ability(declaration) => names.insert(declaration.name.text.clone()),
            StmtKind::Mount(declaration) => names.insert(declaration.name.text.clone()),
            _ => false,
        };
    }
    names
}

/// Merge bare imports into the main module as top-level aliases (stub for qualified-only MVP).
pub fn validate_needs_against_manifest(
    needs: &[NeedStmt],
    manifest: &Manifest,
) -> Result<(), String> {
    for need in needs {
        let name = &need.name.text;
        let found = manifest.dependencies.iter().any(|dependency| dependency.name() == name);
        if !found {
            return Err(format!(
                "`need {name}` is not in the riff file — run `vaab need ...` to add it"
            ));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn placeholder_name(text: &str, span: Span) -> Name {
    Name::new(text, span)
}
