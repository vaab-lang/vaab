#!/usr/bin/env python3
"""Apply phases 5b-7 implementation patches. Run from repo root."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]

SERVE_PARSER = ROOT / "scripts/phase6_staging/crates/vaab-syntax/src/parser/serve.rs"
SERVE_DEST = ROOT / "crates/vaab-syntax/src/parser/serve.rs"


def install_serve_parser() -> None:
    SERVE_DEST.parent.mkdir(parents=True, exist_ok=True)
    SERVE_DEST.write_text(SERVE_PARSER.read_text())


def patch(path: str, old: str, new: str, required: bool = True) -> None:
    p = ROOT / path
    text = p.read_text()
    if old in text:
        p.write_text(text.replace(old, new, 1))
    elif required and new.split("\n", 1)[0] not in text:
        print(f"FAIL patch {path}: pattern not found")
        sys.exit(1)


def must(path: str, needle: str) -> None:
    if needle not in (ROOT / path).read_text():
        print(f"VERIFY FAIL {path}: missing {needle!r}")
        sys.exit(1)


install_serve_parser()

# --- syntax ast ---
patch(
    "crates/vaab-syntax/src/ast.rs",
    """    Ability(Box<AbilityDecl>),
    /// An expression evaluated for its effect, or, if it is last in a block, for
    /// the block's value.
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LetStmt {""",
    """    Ability(Box<AbilityDecl>),
    Serve(Box<ServeDecl>),
    Reply(ReplyStmt),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServeDecl {
    pub port: Expr,
    pub before: Option<Block>,
    pub routes: Vec<RouteDecl>,
    pub error_handler: Option<ServeErrorHandler>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServeErrorHandler {
    pub error_type: TypeExpr,
    pub binding: Name,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteDecl {
    pub method: Name,
    pub path: Vec<RouteSegment>,
    pub expecting: Option<ExpectingDecl>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RouteSegment {
    Literal(String),
    Param { name: Name, declared: Option<TypeExpr> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpectingDecl {
    pub declared: TypeExpr,
    pub binding: Name,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplyStmt {
    pub kind: ReplyKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplyKind {
    With { value: Expr, status: Option<Expr> },
    Explain(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LetStmt {""",
    required=False,
)

# token
tok = (ROOT / "crates/vaab-syntax/src/token.rs").read_text()
if "Serve," not in tok:
    tok = tok.replace(
        "    After,\n\n    // ---- Symbols",
        "    After,\n    Serve, Route, Reply, Port, Expecting, Explain, Status, With, Before, Every, Anything,\n\n    // ---- Symbols",
    )
    tok = tok.replace(
        '    ("after", TokenKind::After),\n];',
        """    ("after", TokenKind::After),
    ("serve", TokenKind::Serve), ("route", TokenKind::Route), ("reply", TokenKind::Reply),
    ("port", TokenKind::Port), ("expecting", TokenKind::Expecting), ("explain", TokenKind::Explain),
    ("status", TokenKind::Status), ("with", TokenKind::With), ("before", TokenKind::Before),
    ("every", TokenKind::Every), ("anything", TokenKind::Anything),
];""",
    )
    tok = tok.replace(
        "                | After\n        )",
        "                | After\n                | Serve | Route | Reply | Port | Expecting | Explain | Status | With | Before | Every | Anything\n        )",
    )
    (ROOT / "crates/vaab-syntax/src/token.rs").write_text(tok)

stmt = (ROOT / "crates/vaab-syntax/src/parser/stmt.rs").read_text()
if "Serve =>" not in stmt:
    stmt = stmt.replace(
        "Ability => StmtKind::Ability(Box::new(self.ability_declaration()?)),\n\n            // `to`",
        "Ability => StmtKind::Ability(Box::new(self.ability_declaration()?)),\n            Serve => StmtKind::Serve(Box::new(self.serve_declaration()?)),\n            Reply => StmtKind::Reply(self.reply_statement()?),\n\n            // `to`",
    )
(ROOT / "crates/vaab-syntax/src/parser/stmt.rs").write_text(
    stmt.replace("    fn previous_span", "    pub(crate) fn previous_span")
)

mod = (ROOT / "crates/vaab-syntax/src/parser/mod.rs").read_text()
if "mod serve;" not in mod:
    (ROOT / "crates/vaab-syntax/src/parser/mod.rs").write_text(
        mod.replace("mod pattern;\nmod stmt;", "mod pattern;\nmod serve;\nmod stmt;")
    )

# workspace Cargo.toml
(ROOT / "Cargo.toml").write_text(
    """[workspace]
resolver = \"2\"
members = [
    \"crates/vaab-syntax\",
    \"crates/vaab-types\",
    \"crates/vaab-vm\",
    \"crates/vaab-server\",
    \"crates/vaab-cli\",
]

[workspace.package]
version = \"0.1.0\"
edition = \"2021\"
rust-version = \"1.80\"
license = \"MIT OR Apache-2.0\"
repository = \"https://github.com/vaab-lang/vaab\"

[workspace.dependencies]
ariadne = \"0.5\"
indexmap = \"2\"
logos = \"0.15\"
serde_json = \"1\"
thiserror = \"2\"
insta = \"1\"
crossbeam-deque = \"0.8\"
tokio = { version = \"1\", features = [\"macros\", \"rt-multi-thread\", \"net\"] }
hyper = \"1\"
hyper-util = { version = \"0.1\", features = [\"server\", \"tokio\", \"http1\"] }
http-body-util = \"0.1\"

vaab-syntax = { path = \"crates/vaab-syntax\" }
vaab-types = { path = \"crates/vaab-types\" }
vaab-vm = { path = \"crates/vaab-vm\" }
vaab-server = { path = \"crates/vaab-server\" }

[profile.release]
lto = \"thin\"
"""
)

cargo = (ROOT / "Cargo.toml").read_text()
if "vaab-server" not in cargo:
    cargo = cargo.replace(
        'members = ["crates/vaab-syntax", "crates/vaab-types", "crates/vaab-vm", "crates/vaab-cli"]',
        'members = ["crates/vaab-syntax", "crates/vaab-types", "crates/vaab-vm", "crates/vaab-server", "crates/vaab-cli"]',
    )
if "serde_json" not in cargo:
    cargo = cargo.replace(
        "[workspace.dependencies]\nariadne",
        "[workspace.dependencies]\nserde_json = \"1\"\ncrossbeam-deque = \"0.8\"\ntokio = { version = \"1\", features = [\"macros\", \"rt-multi-thread\", \"net\"] }\nhyper = \"1\"\nhyper-util = { version = \"0.1\", features = [\"server\", \"tokio\", \"http1\"] }\nhttp-body-util = \"0.1\"\nvaab-server = { path = \"crates/vaab-server\" }\nariadne",
    )
(ROOT / "Cargo.toml").write_text(cargo)

vm_cargo = (ROOT / "crates/vaab-vm/Cargo.toml").read_text()
if "serde_json" not in vm_cargo:
    (ROOT / "crates/vaab-vm/Cargo.toml").write_text(
        vm_cargo.replace(
            "[dependencies]\nindexmap.workspace = true",
            "[dependencies]\nserde_json.workspace = true\ncrossbeam-deque.workspace = true\nindexmap.workspace = true",
        )
    )

# bytecode
bc = (ROOT / "crates/vaab-vm/src/bytecode.rs").read_text()
if "ReadFile" not in bc:
    bc = bc.replace(
        "    Select(u32),\n\n    // -- Stopping",
        "    Select(u32),\n    ReadFile(u32), Now, ReplyWith(bool), ReplyExplain,\n\n    // -- Stopping",
    )
if "pub routes:" not in bc:
    bc = bc.replace(
        "    pub selects: Vec<SelectDescriptor>,\n}",
        """    pub selects: Vec<SelectDescriptor>,
    pub routes: Vec<RouteHandler>,
}

#[derive(Clone, Debug)]
pub struct RouteHandler {
    pub method: String,
    pub path: Vec<vaab_types::RouteSegment>,
    pub body: usize,
    pub path_param_count: usize,
    pub expects_body: bool,
}""",
    )
    bc = bc.replace("            selects: Vec::new(),\n        }", "            selects: Vec::new(),\n            routes: Vec::new(),\n        }")
(ROOT / "crates/vaab-vm/src/bytecode.rs").write_text(bc)

# builtin
bi = (ROOT / "crates/vaab-vm/src/builtin.rs").read_text()
if "ToJson" not in bi:
    bi = bi.replace("    Print,\n\n    Upper,", "    Print,\n    ToJson,\n\n    Upper,")
    bi = bi.replace('Builtin::Print => "print",', 'Builtin::Print => "print",\n            Builtin::ToJson => "to_json",')
    bi = bi.replace("Builtin::Print\n            | Builtin::Upper,", "Builtin::Print\n            | Builtin::ToJson\n            | Builtin::Upper,")
    bi = bi.replace(
        "        match (self, arguments) {\n            (Builtin::Upper,",
        "        match (self, arguments) {\n            (Builtin::ToJson, [value]) => crate::json::encode(value).map(Value::text),\n            (Builtin::Upper,",
    )
    (ROOT / "crates/vaab-vm/src/builtin.rs").write_text(bi)

# compile expr
ex = (ROOT / "crates/vaab-vm/src/compile/expr.rs").read_text()
ex = ex.replace("use crate::error::Feature;\n", "")
if "Op::Now" not in ex:
    ex = ex.replace("self.emit(Op::NotYet(Feature::ReadFile), span)", "self.emit(Op::ReadFile(self.file_error_not_found()), span)", 2)
    ex = ex.replace(
        'Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::ReadFile(self.file_error_not_found()), span)\n            }\n            Some(Resolution::Builtin("print"))',
        'Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::ReadFile(self.file_error_not_found()), span)\n            }\n            Some(Resolution::Builtin("now")) => self.emit(Op::Now, span),\n            Some(Resolution::Builtin("print"))',
    )
    ex = ex.replace(
        'Some(Resolution::Builtin("print")) => {\n                self.push_constant(Value::Builtin(Builtin::Print), span)\n            }',
        'Some(Resolution::Builtin("print")) => {\n                self.push_constant(Value::Builtin(Builtin::Print), span)\n            }\n            Some(Resolution::Builtin("to_json")) => {\n                self.push_constant(Value::Builtin(Builtin::ToJson), span)\n            }',
        1,
    )
    ex = ex.replace(
        'Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::ReadFile(self.file_error_not_found()), span)\n            }\n\n            Some(Resolution::Function(id))',
        'Some(Resolution::Builtin("read_file")) => {\n                let count = self.push_arguments(call, arguments, Defaults::None);\n                let _ = count;\n                self.emit(Op::ReadFile(self.file_error_not_found()), span);\n            }\n            Some(Resolution::Builtin("now")) => self.emit(Op::Now, span),\n            Some(Resolution::Builtin("to_json")) => {\n                let count = self.push_arguments(call, arguments, Defaults::None);\n                let _ = count;\n                self.emit(Op::Builtin(Builtin::ToJson), span);\n            }\n\n            Some(Resolution::Function(id))',
    )
if "RequestField" not in ex:
    ex = ex.replace(
        "            Some(Resolution::Field { field, .. }) => {",
        "            Some(Resolution::RequestField(index)) => {\n                self.expression(target);\n                self.emit(Op::TupleItem(index as u32), span);\n            }\n            Some(Resolution::Field { field, .. }) => {",
    )
(ROOT / "crates/vaab-vm/src/compile/expr.rs").write_text(ex)

cm = (ROOT / "crates/vaab-vm/src/compile/mod.rs").read_text()
if "file_error_not_found" not in cm:
    cm = cm.replace(
        "            _ => None,\n        }\n    }\n}\n\nimpl Builder {",
        """            _ => None,
        }
    }

    pub(crate) fn file_error_not_found(&self) -> u32 {
        for (choice, declared) in self.checked.choices.iter().enumerate() {
            if declared.name != "FileError" {
                continue;
            }
            for (variant, shape) in declared.variants.iter().enumerate() {
                if shape.name == "NotFound" {
                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0);
                }
            }
        }
        0
    }
}

impl Builder {""",
    )
    (ROOT / "crates/vaab-vm/src/compile/mod.rs").write_text(cm)

# compile stmt
cs = (ROOT / "crates/vaab-vm/src/compile/stmt.rs").read_text()
if "StmtKind::Serve" not in cs:
    cs = cs.replace(
        "            StmtKind::Choice(_) | StmtKind::Ability(_) => {}\n\n            StmtKind::Expr",
        "            StmtKind::Choice(_) | StmtKind::Ability(_) => {}\n            StmtKind::Serve(serve) => self.serve(statement, serve),\n            StmtKind::Reply(reply) => self.reply(reply, span),\n\n            StmtKind::Expr",
    )
if "fn serve(" not in cs:
    cs = cs.rstrip() + """

    fn serve(&mut self, statement: &Stmt, serve: &vaab_syntax::ast::ServeDecl) {
        use crate::bytecode::RouteHandler;
        use super::Builder;
        use crate::value::Ref;
        let Some(checked) = self.checked.serves.get(&statement.id) else { return };
        for (route, route_decl) in checked.routes.iter().zip(&serve.routes) {
            let body_index = self.builders.len();
            let name = Ref::from(format!("route {}", route.method).as_str());
            self.builders.push(Builder::new(name, route.frame));
            let parameters = 1 + route.path_params.len() + usize::from(route.expecting.is_some());
            self.open_body(body_index, route.frame, parameters);
            self.block_discard(&route_decl.body);
            self.emit(Op::Return, route_decl.span);
            self.open.pop();
            self.program.routes.push(RouteHandler {
                method: route.method.clone(),
                path: route.path.clone(),
                body: body_index,
                path_param_count: route.path_params.len(),
                expects_body: route.expecting.is_some(),
            });
        }
    }

    fn reply(&mut self, reply: &vaab_syntax::ast::ReplyStmt, span: Span) {
        use vaab_syntax::ast::ReplyKind;
        match &reply.kind {
            ReplyKind::With { value, status } => {
                self.expression(value);
                if let Some(status) = status {
                    self.expression(status);
                    self.emit(Op::ReplyWith(true), span);
                } else {
                    self.emit(Op::ReplyWith(false), span);
                }
            }
            ReplyKind::Explain(value) => {
                self.expression(value);
                self.emit(Op::ReplyExplain, span);
            }
        }
    }
}
"""
    (ROOT / "crates/vaab-vm/src/compile/stmt.rs").write_text(cs)

# machine ReadFile/Now + HttpResponse if missing
mach = (ROOT / "crates/vaab-vm/src/machine.rs").read_text()
if "pub struct HttpResponse" not in mach:
    mach = mach.replace(
        "pub struct World {",
        """#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub struct World {""",
    )
    mach = mach.replace(
        "    pub output: Output,\n}",
        "    pub output: Output,\n    pub response: Option<HttpResponse>,\n}",
    )
    mach = mach.replace(
        "World { program, globals, output }",
        "World { program, globals, output, response: None }",
    )
if "Op::ReadFile" not in mach:
    insert = """            Op::ReadFile(layout) => {
                let path = self.pop()?;
                let text = match &path {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("read_file expected a path")),
                };
                match std::fs::read_to_string(&text) {
                    Ok(contents) => self.stack.push(Value::success(Value::text(contents))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("read_file named a variant that is not there"),
                        )?;
                        let variant = Value::Variant(Ref::new(Variant {
                            layout,
                            fields: vec![path],
                        }));
                        self.stack.push(Value::failure(variant));
                    }
                    Err(_) => return Err(Fault::Confused("could not read the file")),
                }
            }
            Op::Now => {
                let seconds = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0);
                self.stack.push(Value::Int(seconds));
            }

"""
    if "Op::ReplyWith" in mach:
        mach = mach.replace("            Op::ReplyWith(has_status) => {", insert + "            Op::ReplyWith(has_status) => {")
    else:
        mach = mach.replace("            // -- Stopping", insert + "            // -- Stopping")
    if "Op::ReplyWith" not in mach:
        mach = mach.replace(
            "            // -- Stopping",
            """            Op::ReplyWith(has_status) => {
                let status = if has_status {
                    match self.pop()? {
                        Value::Int(number) if number > 0 && number <= u16::MAX as i64 => number as u16,
                        _ => return Err(Fault::Confused("reply status must be a whole number")),
                    }
                } else {
                    200
                };
                let value = self.pop()?;
                let body = crate::json::encode(&value)?;
                world.response = Some(HttpResponse { status, body });
            }
            Op::ReplyExplain => {
                let value = self.pop()?;
                let body = crate::json::encode(&value)?;
                world.response = Some(HttpResponse { status: 400, body });
            }

            // -- Stopping""",
        )
(ROOT / "crates/vaab-vm/src/machine.rs").write_text(mach)

lib = (ROOT / "crates/vaab-vm/src/lib.rs").read_text()
if "mod json;" not in lib:
    lib = lib.replace("mod compile;\n", "mod compile;\nmod json;\n")
    (ROOT / "crates/vaab-vm/src/lib.rs").write_text(lib)

# tests library
lib_test = (ROOT / "crates/vaab-vm/tests/library.rs").read_text()
if "fn now_returns" not in lib_test:
    lib_test = lib_test.rstrip() + """

#[test]
fn now_returns_seconds_since_1970() {
    let seconds = printed("print(now())\n").parse::<i64>().expect("a whole number");
    assert!(seconds > 1_600_000_000);
}

#[test]
fn to_json_turns_a_map_into_text() {
    assert_eq!(printed("print(to_json({\"a\": 1}))\n"), "{\"a\":1}");
}

#[test]
fn read_file_returns_the_contents_of_a_file() {
    let path = std::env::temp_dir().join("vaab-read-file-test.txt");
    std::fs::write(&path, "hello from disk").expect("write temp file");
    let path = path.display();
    let source = format!(
        "choice FileError {{ NotFound(path: Text) }}\nmatch read_file(\"{path}\") {{\nwhen success text then print(text)\nwhen failure _ then print(\"missing\")\n}}\n"
    );
    assert_eq!(printed(&source), "hello from disk");
}
"""
    (ROOT / "crates/vaab-vm/tests/library.rs").write_text(lib_test)

bc_test = (ROOT / "crates/vaab-vm/tests/bytecode.rs").read_text()
if "read_file_compiles" not in bc_test:
    bc_test = bc_test.replace(
        'fn a_feature_a_later_phase_brings_compiles_to_one_instruction() {\n    assert_snapshot!(listing("print(read_file(\\"notes.txt\\"))\\n"));\n}',
        'fn read_file_compiles_to_a_read_instruction() {\n    assert_snapshot!(listing(\n        "choice FileError { NotFound(path: Text) }\\nprint(read_file(\\"notes.txt\\"))\\n"\n    ));\n}',
    )
    (ROOT / "crates/vaab-vm/tests/bytecode.rs").write_text(bc_test)

err_test = (ROOT / "crates/vaab-vm/tests/errors.rs").read_text()
if "reading_a_missing_file" not in err_test:
    err_test = err_test.replace(
        """fn reading_a_file_says_which_phase_brings_it() {
    let rendered = stops("print(read_file(\\"notes.txt\\"))\\n");
    assert!(rendered.contains("phase 5"), "{rendered}");
    assert_snapshot!(rendered);
}""",
        """fn reading_a_missing_file_fails_with_file_error() {
    let source = "choice FileError { NotFound(path: Text) }\\n\\
        match read_file(\\"missing.vaab\\") {\\n\\
        when success _ then print(\\"found\\")\\n\\
        when failure _ then print(\\"missing\\")\\n\\
        }\\n";
    assert_snapshot!(support::printed(source));
}""",
    )
    (ROOT / "crates/vaab-vm/tests/errors.rs").write_text(err_test)

# --- types checked ---
checked_path = ROOT / "crates/vaab-types/src/checked.rs"
checked = checked_path.read_text()
if "pub serves:" not in checked:
    checked = checked.replace(
        "    pub tasks: HashMap<NodeId, Task>,\n}",
        """    pub tasks: HashMap<NodeId, Task>,
    pub serves: HashMap<NodeId, Serve>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteSegment {
    Literal(String),
    Param(String),
}

#[derive(Clone, Debug)]
pub struct Route {
    pub method: String,
    pub path: Vec<RouteSegment>,
    pub frame: FrameId,
    pub path_params: Vec<(String, Type)>,
    pub expecting: Option<(Type, LocalId)>,
}

#[derive(Clone, Debug)]
pub struct Serve {
    pub port: u16,
    pub before: Option<FrameId>,
    pub routes: Vec<Route>,
    pub error_handler: Option<(Type, LocalId, FrameId)>,
}""",
    )
    checked = checked.replace(
        "    Closure(NodeId),\n}",
        "    Closure(NodeId),\n    Route(NodeId),\n}",
    )
    checked = checked.replace(
        "    NewShared,\n}",
        "    NewShared,\n    RequestField(usize),\n}",
    )
    checked_path.write_text(checked)

lib_types = (ROOT / "crates/vaab-types/src/lib.rs").read_text()
if "Route," not in lib_types:
    (ROOT / "crates/vaab-types/src/lib.rs").write_text(
        lib_types.replace(
            "LocalRef, Required, Resolution, Task, TypeId, Variant,",
            "LocalRef, Required, Resolution, Route, RouteSegment, Serve, Task, TypeId, Variant,",
        )
    )

cmod = (ROOT / "crates/vaab-types/src/checker/mod.rs").read_text()
if "route_depth" not in cmod:
    cmod = cmod.replace(
        "    reported_captures: HashSet<(LocalId, Span)>,\n}",
        "    reported_captures: HashSet<(LocalId, Span)>,\n    route_depth: usize,\n}",
    )
    cmod = cmod.replace(
        "            reported_captures: HashSet::new(),\n        }",
        "            reported_captures: HashSet::new(),\n            route_depth: 0,\n        }",
    )
    (ROOT / "crates/vaab-types/src/checker/mod.rs").write_text(cmod)

member = (ROOT / "crates/vaab-types/src/checker/member.rs").read_text()
if "RequestField" not in member:
    member = member.replace(
        "        match &found {\n            Type::Named(owner) => self.named_member(owner, &found, name),",
        """        if matches!(&found, Type::Tuple(parts) if parts.len() == 3 && parts.iter().all(|part| *part == Type::Text)) {
            let index = match name.text.as_str() {
                "method" => Some(0),
                "path" => Some(1),
                "body" => Some(2),
                _ => None,
            };
            if let Some(index) = index {
                return Member::Value {
                    declared: Type::Text,
                    resolution: Resolution::RequestField(index),
                };
            }
        }

        match &found {
            Type::Named(owner) => self.named_member(owner, &found, name),""",
    )
    (ROOT / "crates/vaab-types/src/checker/member.rs").write_text(member)

walk = (ROOT / "crates/vaab-types/src/checker/walk.rs").read_text()
if "StmtKind::Serve" not in walk:
    walk = walk.replace(
        "        StmtKind::Type(_) | StmtKind::Choice(_) | StmtKind::Ability(_) => {}\n        StmtKind::Expr(inner) => expression(inner, visitor),",
        """        StmtKind::Type(_) | StmtKind::Choice(_) | StmtKind::Ability(_) => {}
        StmtKind::Serve(serve) => {
            expression(&serve.port, visitor);
            if let Some(before) = &serve.before { block(before, visitor); }
            for route in &serve.routes { block(&route.body, visitor); }
            if let Some(handler) = &serve.error_handler { block(&handler.body, visitor); }
        }
        StmtKind::Reply(reply) => match &reply.kind {
            vaab_syntax::ast::ReplyKind::With { value, status } => {
                expression(value, visitor);
                if let Some(status) = status { expression(status, visitor); }
            }
            vaab_syntax::ast::ReplyKind::Explain(value) => expression(value, visitor),
        },
        StmtKind::Expr(inner) => expression(inner, visitor),""",
    )
    (ROOT / "crates/vaab-types/src/checker/walk.rs").write_text(walk)

msgs = (ROOT / "crates/vaab-types/src/messages.rs").read_text()
if "serve_port_must_be_a_number" not in msgs:
    msgs = msgs.replace(
        "#[cfg(test)]\nmod tests {",
        '''pub fn serve_port_must_be_a_number(span: Span) -> Diagnostic {
    Diagnostic::error("serve-port-must-be-a-number", "the port must be a whole number written as a literal")
        .at(span, "this is not a literal port number")
        .help("write the port directly, such as `serve on port 8080`")
}

pub fn route_param_type(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("route-param-type", format!("a path parameter may be Text or Int, not {}", show_type(found)))
        .at(span, format!("this is {}", show_type(found)))
}

pub fn reply_outside_route(span: Span) -> Diagnostic {
    Diagnostic::error("reply-outside-route", "`reply` only belongs inside a route body").at(span, "`reply` was written here")
}

pub fn unknown_http_method(method: &str, span: Span) -> Diagnostic {
    Diagnostic::error("unknown-http-method", format!("`{method}` is not a supported HTTP method"))
        .at(span, format!("`{method}` was written here"))
        .help("use `get`, `post`, `put`, `patch` or `delete`")
}

pub fn cannot_json(span: Span) -> Diagnostic {
    Diagnostic::error("cannot-json", "this value cannot be turned into JSON").at(span, "this value cannot be serialised")
}

#[cfg(test)]
mod tests {''',
    )
    (ROOT / "crates/vaab-types/src/messages.rs").write_text(msgs)

print("apply-all-phases.py: OK")
