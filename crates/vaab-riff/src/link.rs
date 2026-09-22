//! Link a project entry file with its needed riffs.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use vaab_syntax::ast::{Module, Name, NeedImports, NeedSource, NeedStmt, Stmt, StmtKind};
use vaab_syntax::{parse, Span};

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
    Qualified,
    Bare(HashSet<String>),
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

    let entry_source = fs::read_to_string(&entry)
        .map_err(|error| format!("could not read `{}`: {error}", entry.display()))?;
    let parsed = parse(&entry_source);
    if parsed.has_errors() {
        return Err("the entry file does not parse".to_string());
    }

    let mut sources = HashMap::from([(entry.display().to_string(), entry_source.clone())]);
    let (needs, module) = split_needs(parsed.module);
    let mut riffs = Vec::new();

    for need in needs {
        let alias = riff_alias(&need);
        if riffs.iter().any(|loaded: &LoadedRiff| loaded.alias == alias) {
            return Err(format!("`{alias}` is needed twice"));
        }
        let path = resolve_need_path(&project_root, &lock, &need)?;
        let lib = path.join("lib.vaab");
        if !lib.is_file() {
            return Err(format!(
                "`{}` has no `lib.vaab` — a riff library needs one",
                path.display()
            ));
        }
        let lib_source = fs::read_to_string(&lib)
            .map_err(|error| format!("could not read `{}`: {error}", lib.display()))?;
        let lib_parsed = parse(&lib_source);
        if lib_parsed.has_errors() {
            return Err(format!("`{}` does not parse", lib.display()));
        }
        sources.insert(lib.display().to_string(), lib_source);
        riffs.push(LoadedRiff {
            alias,
            path,
            module: lib_parsed.module,
            imports: riff_imports(&need),
        });
    }

    let module = merge::merge(module, &riffs);

    Ok(Linked {
        project_root,
        entry,
        module,
        sources,
        riffs,
    })
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

fn resolve_need_path(
    project_root: &Path,
    lock: &Option<Lockfile>,
    need: &NeedStmt,
) -> Result<PathBuf, String> {
    let name = need.name.text.clone();
    if let Some(lock) = lock {
        if let Some(path) = lock.path_for(&name) {
            return Ok(path.clone());
        }
    }
    match &need.source {
        NeedSource::Installed => installed_path(&name),
        NeedSource::Path { path, .. } => Ok(project_root.join(path)),
        NeedSource::Registry { owner } => Err(format!(
            "run `vaab gather` first — `{name}` from `{}` is not in needed.lock yet",
            owner.text
        )),
    }
}

fn riff_alias(need: &NeedStmt) -> String {
    match &need.imports {
        NeedImports::As(alias) => alias.text.clone(),
        _ => need.name.text.clone(),
    }
}

fn riff_imports(need: &NeedStmt) -> RiffImports {
    match &need.imports {
        NeedImports::Of(names) => {
            RiffImports::Bare(names.iter().map(|name| name.text.clone()).collect())
        }
        NeedImports::Qualified | NeedImports::As(_) => RiffImports::Qualified,
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
