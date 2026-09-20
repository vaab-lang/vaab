# ruff: noqa
# AST
patch(
    "crates/vaab-syntax/src/ast.rs",
    """    /// `ability Describable { ... }`
    Ability(Box<AbilityDecl>),
    /// An expression evaluated for its effect, or, if it is last in a block, for
    /// the block's value.
    Expr(Expr),
}""",
    """    /// `ability Describable { ... }`
    Ability(Box<AbilityDecl>),
    /// `serve on port 8080 { route ... }`
    Serve(Box<ServeDecl>),
    /// `reply with value` or `reply explain error`, inside a route body.
    Reply(ReplyStmt),
    /// An expression evaluated for its effect, or, if it is last in a block, for
    /// the block's value.
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
}""",
)

# Token soft keywords
patch(
    "crates/vaab-syntax/src/token.rs",
    "    After,\n\n    // ---- Symbols",
    """    After,
    Serve,
    Route,
    Reply,
    Port,
    Expecting,
    Explain,
    Status,
    With,
    Before,
    Every,
    Anything,

    // ---- Symbols""",
)
patch(
    "crates/vaab-syntax/src/token.rs",
    '    ("after", TokenKind::After),\n];',
    """    ("after", TokenKind::After),
    ("serve", TokenKind::Serve),
    ("route", TokenKind::Route),
    ("reply", TokenKind::Reply),
    ("port", TokenKind::Port),
    ("expecting", TokenKind::Expecting),
    ("explain", TokenKind::Explain),
    ("status", TokenKind::Status),
    ("with", TokenKind::With),
    ("before", TokenKind::Before),
    ("every", TokenKind::Every),
    ("anything", TokenKind::Anything),
];""",
)
patch(
    "crates/vaab-syntax/src/token.rs",
    "                | After\n        )",
    """                | After
                | Serve
                | Route
                | Reply
                | Port
                | Expecting
                | Explain
                | Status
                | With
                | Before
                | Every
                | Anything
        )""",
)

# Parser
patch("crates/vaab-syntax/src/parser/mod.rs", "mod expr;\nmod pattern;\nmod stmt;", "mod expr;\nmod pattern;\nmod serve;\nmod stmt;")
patch(
    "crates/vaab-syntax/src/parser/stmt.rs",
    "    fn previous_span(&self) -> Span {",
    "    pub(crate) fn previous_span(&self) -> Span {",
    skip_if="pub(crate) fn previous_span",
)
patch(
    "crates/vaab-syntax/src/parser/stmt.rs",
    "            Ability => StmtKind::Ability(Box::new(self.ability_declaration()?)),\n\n            // `to` at the start of a statement always defines a function.",
    """            Ability => StmtKind::Ability(Box::new(self.ability_declaration()?)),
            Serve => StmtKind::Serve(Box::new(self.serve_declaration()?)),
            Reply => StmtKind::Reply(self.reply_statement()?),

            // `to` at the start of a statement always defines a function.""",
    skip_if="StmtKind::Serve",
)

# Print
patch(
    "crates/vaab-syntax/src/print.rs",
    "            StmtKind::Ability(declaration) => self.ability_declaration(declaration),\n            StmtKind::Expr(expression) => self.expression(expression),",
    """            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
            StmtKind::Serve(serve) => self.node("serve", |printer| {
                printer.node("port", |printer| printer.expression(&serve.port));
                if let Some(before) = &serve.before {
                    printer.node("before every request", |printer| printer.block(before));
                }
                for route in &serve.routes {
                    printer.node(format!("route {}", route.method.text), |printer| {
                        printer.line(format!("path {:?}", route.path));
                        if let Some(expecting) = &route.expecting {
                            printer.node(
                                format!("expecting as {}", expecting.binding.text),
                                |printer| printer.type_expression(&expecting.declared),
                            );
                        }
                        printer.block(&route.body);
                    });
                }
                if let Some(handler) = &serve.error_handler {
                    printer.node(
                        format!("when anything fails as {}", handler.binding.text),
                        |printer| {
                            printer.type_expression(&handler.error_type);
                            printer.block(&handler.body);
                        },
                    );
                }
            }),
            StmtKind::Reply(reply) => match &reply.kind {
                ReplyKind::With { value, status } => self.node("reply with", |printer| {
                    printer.expression(value);
                    if let Some(status) = status {
                        printer.node("status", |printer| printer.expression(status));
                    }
                }),
                ReplyKind::Explain(value) => {
                    self.node("reply explain", |printer| printer.expression(value))
                }
            },
            StmtKind::Expr(expression) => self.expression(expression),""",
)

# Checked
patch(
    "crates/vaab-types/src/checked.rs",
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
patch(
    "crates/vaab-types/src/checked.rs",
    "    NewShared,\n}",
    """    NewShared,
    RequestField(usize),
}""",
)
patch(
    "crates/vaab-types/src/checked.rs",
    "    Closure(NodeId),\n}",
    """    Closure(NodeId),
    Route(NodeId),
}""",
)

# Types lib
patch("crates/vaab-types/src/lib.rs", "mod checker;\nmod messages;", "mod checker;\nmod json;\nmod messages;")
patch(
    "crates/vaab-types/src/lib.rs",
    "pub use checked::{",
    "pub use json::can_json;\npub use checked::{",
)
patch(
    "crates/vaab-types/src/lib.rs",
    "    LocalRef, Required, Resolution, Task, TypeId, Variant,",
    "    LocalRef, Required, Resolution, Route, RouteSegment, Serve, Task, TypeId, Variant,",
)

# Checker mod
patch("crates/vaab-types/src/checker/mod.rs", "mod sendable;\nmod stmt;", "mod sendable;\nmod serve;\nmod stmt;")
patch(
    "crates/vaab-types/src/checker/mod.rs",
    "    reported_captures: HashSet<(LocalId, Span)>,\n}",
    "    reported_captures: HashSet<(LocalId, Span)>,\n    route_depth: u32,\n}",
)
patch(
    "crates/vaab-types/src/checker/mod.rs",
    "            reported_captures: HashSet::new(),\n        }",
    "            reported_captures: HashSet::new(),\n            route_depth: 0,\n        }",
)

# Checker stmt
patch(
    "crates/vaab-types/src/checker/stmt.rs",
    """            StmtKind::Ability(declaration) => {
                if self.scopes.len() > 1 {
                    self.report(messages::nested_declaration(
                        "ability",
                        &declaration.name.text,
                        declaration.name.span,
                    ));
                }
            }

            StmtKind::Expr(expression) => {""",
    """            StmtKind::Ability(declaration) => {
                if self.scopes.len() > 1 {
                    self.report(messages::nested_declaration(
                        "ability",
                        &declaration.name.text,
                        declaration.name.span,
                    ));
                }
            }

            StmtKind::Serve(serve) => self.serve(statement, serve),
            StmtKind::Reply(reply) => self.reply(reply),

            StmtKind::Expr(expression) => {""",
)

# Member request fields
patch(
    "crates/vaab-types/src/checker/member.rs",
    "    pub(super) fn look_up_member(&mut self, target: &Expr, name: &Name) -> Member {\n        // A name to the left",
    """    pub(super) fn look_up_member(&mut self, target: &Expr, name: &Name) -> Member {
        if let ExprKind::Name(written) = &target.kind {
            if written.text == "request" && self.route_depth > 0 {
                if let Some(index) = match name.text.as_str() {
                    "method" => Some(0),
                    "path" => Some(1),
                    "body" => Some(2),
                    _ => None,
                } {
                    return Member::Value {
                        declared: Type::Text,
                        resolution: Resolution::RequestField(index),
                    };
                }
            }
        }

        // A name to the left""",
)

# Walk
patch(
    "crates/vaab-types/src/checker/walk.rs",
    "        StmtKind::Type(_) | StmtKind::Choice(_) | StmtKind::Ability(_) => {}\n        StmtKind::Expr(inner) => expression(inner, visitor),",
    """        StmtKind::Type(_) | StmtKind::Choice(_) | StmtKind::Ability(_) => {}
        StmtKind::Serve(serve) => {
            expression(&serve.port, visitor);
            if let Some(before) = &serve.before {
                block(before, visitor);
            }
            for route in &serve.routes {
                block(&route.body, visitor);
            }
            if let Some(handler) = &serve.error_handler {
                block(&handler.body, visitor);
            }
        }
        StmtKind::Reply(reply) => match &reply.kind {
            vaab_syntax::ast::ReplyKind::With { value, status } => {
                expression(value, visitor);
                if let Some(status) = status {
                    expression(status, visitor);
                }
            }
            vaab_syntax::ast::ReplyKind::Explain(value) => expression(value, visitor),
        },
        StmtKind::Expr(inner) => expression(inner, visitor),""",
)

# Pure
patch(
    "crates/vaab-types/src/checker/pure.rs",
    '            StmtKind::Together(_) => Some(PureViolation::Effect("wait for tasks")),',
    """            StmtKind::Together(_) => Some(PureViolation::Effect("wait for tasks")),
            StmtKind::Serve(_) => Some(PureViolation::Effect("start a web server")),
            StmtKind::Reply(_) => Some(PureViolation::Effect("send an HTTP response")),""",
)
patch(
    "crates/vaab-types/src/checker/pure.rs",
    "                | Resolution::SelfValue,\n            ) => None,",
    "                | Resolution::SelfValue\n                | Resolution::RequestField(_),\n            ) => None,",
)

# Messages
patch(
    "crates/vaab-types/src/messages.rs",
    "// ---------------------------------------------------------------------------\n// Sendability: what may cross between tasks",
    """// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

pub fn serve_port_must_be_a_number(span: Span) -> Diagnostic {
    Diagnostic::error(
        "serve-port-must-be-a-number",
        "the port in `serve on port ...` must be a whole number",
    )
    .at(span, "this is not a plain whole number")
    .with_help("write something like `serve on port 8080 { ... }`")
}

pub fn unknown_http_method(method: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "unknown-http-method",
        format!("`{method}` is not an HTTP method Vaab knows"),
    )
    .at(span, format!("`{method}` was written here"))
    .with_help("use `get`, `post`, `put`, `patch` or `delete`")
}

pub fn route_param_type(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error(
        "route-param-type",
        format!("a route parameter may be `Int` or `Text`, not {found}"),
    )
    .at(span, format!("this parameter is declared as {found}"))
}

pub fn reply_outside_route(span: Span) -> Diagnostic {
    Diagnostic::error(
        "reply-outside-route",
        "`reply` may only appear inside a route body or error handler",
    )
    .at(span, "`reply` was written here")
}

pub fn cannot_json(span: Span) -> Diagnostic {
    Diagnostic::error(
        "cannot-json",
        "this value cannot be turned into JSON for the response body",
    )
    .at(span, "Vaab can only reply with JSON-serialisable values")
    .with_help("use text, numbers, lists, maps, records or choices whose fields can all be JSON")
}

// ---------------------------------------------------------------------------
// Sendability: what may cross between tasks""",
)

# Bytecode
patch(
    "crates/vaab-vm/src/bytecode.rs",
    "    Select(u32),\n\n    // -- Stopping",
    """    Select(u32),

    ReplyWith(bool),
    ReplyExplain,

    // -- Stopping""",
    skip_if="ReplyWith",
)
patch(
    "crates/vaab-vm/src/bytecode.rs",
    "    pub selects: Vec<SelectDescriptor>,\n}",
    """    pub selects: Vec<SelectDescriptor>,
    pub routes: Vec<RouteHandler>,
}

#[derive(Clone, Debug)]
pub struct RouteHandler {
    pub method: String,
    pub path: Vec<vaab_types::RouteSegment>,
    pub body: usize,
    pub frame: vaab_types::FrameId,
    pub path_param_count: usize,
    pub expects_body: bool,
}""",
)
patch(
    "crates/vaab-vm/src/bytecode.rs",
    "            selects: Vec::new(),\n        }",
    "            selects: Vec::new(),\n            routes: Vec::new(),\n        }",
)

# Machine
patch(
    "crates/vaab-vm/src/machine.rs",
    "/// Phase 4 gives one of these to a whole scheduler full of machines.\npub struct World {",
    """/// Phase 4 gives one of these to a whole scheduler full of machines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub struct World {""",
)
patch(
    "crates/vaab-vm/src/machine.rs",
    "    pub output: Output,\n}",
    "    pub output: Output,\n    pub response: Option<HttpResponse>,\n}",
)
patch(
    "crates/vaab-vm/src/machine.rs",
    "        World { program, globals, output }",
    "        World { program, globals, output, response: None }",
)
patch(
    "crates/vaab-vm/src/machine.rs",
    "            // -- Stopping --------------------------------------------------\n            Op::NotYet(feature) => return Err(Fault::NotYet(feature)),",
    """            Op::ReplyWith(has_status) => {
                let status = if has_status {
                    match self.pop()? {
                        Value::Int(number) if number > 0 && number <= u16::MAX as i64 => {
                            number as u16
                        }
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

            // -- Stopping --------------------------------------------------
            Op::NotYet(feature) => return Err(Fault::NotYet(feature)),""",
)

# VM lib
patch("crates/vaab-vm/src/lib.rs", "mod compile;\npub mod concurrency;", "mod compile;\nmod json;\npub mod concurrency;")
patch(
    "crates/vaab-vm/src/lib.rs",
    "pub use machine::{Budget, Machine, Output, Step, World};",
    "pub use machine::{Budget, HttpResponse, Machine, Output, Step, World};",
)

# VM Cargo
write(
    "crates/vaab-vm/Cargo.toml",
    """[package]
name = "vaab-vm"
description = "The Vaab bytecode compiler and virtual machine."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
indexmap.workspace = true
serde_json.workspace = true
vaab-syntax.workspace = true
vaab-types.workspace = true

[dev-dependencies]
insta.workspace = true
""",
)

# Compile stmt
patch(
    "crates/vaab-vm/src/compile/stmt.rs",
    "use vaab_syntax::ast::{Block, Expr, ExprKind, Stmt, StmtKind};",
    "use vaab_syntax::ast::{Block, Expr, ExprKind, ReplyKind, ServeDecl, Stmt, StmtKind};\n\nuse crate::bytecode::RouteHandler;\nuse crate::value::Ref;\n\nuse super::Builder;",
)
patch(
    "crates/vaab-vm/src/compile/stmt.rs",
    "            StmtKind::Choice(_) | StmtKind::Ability(_) => {}\n\n            StmtKind::Expr(expression) => {",
    """            StmtKind::Choice(_) | StmtKind::Ability(_) => {}

            StmtKind::Serve(serve) => self.serve(statement, serve),
            StmtKind::Reply(reply) => self.reply(reply, span),

            StmtKind::Expr(expression) => {""",
)
patch(
    "crates/vaab-vm/src/compile/stmt.rs",
    """    pub(super) fn advance(&mut self, position: u32, span: Span) {
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::Int(1), span);
        self.emit(Op::Add, span);
        self.emit(Op::StoreLocal(position), span);
    }
}""",
    """    pub(super) fn advance(&mut self, position: u32, span: Span) {
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::Int(1), span);
        self.emit(Op::Add, span);
        self.emit(Op::StoreLocal(position), span);
    }

    fn serve(&mut self, statement: &'a Stmt, serve: &'a ServeDecl) {
        let Some(checked) = self.checked.serves.get(&statement.id) else { return };

        for (route, route_decl) in checked.routes.iter().zip(&serve.routes) {
            let body_index = self.builders.len();
            let name = Ref::from(format!("route {}", route.method).as_str());
            self.builders.push(Builder::new(name, route.frame));

            let parameters = 1
                + route.path_params.len()
                + usize::from(route.expecting.is_some());
            self.open_body(body_index, route.frame, parameters);
            self.block_discard(&route_decl.body);
            self.emit(Op::Return, route_decl.span);
            self.open.pop();

            self.program.routes.push(RouteHandler {
                method: route.method.clone(),
                path: route.path.clone(),
                body: body_index,
                frame: route.frame,
                path_param_count: route.path_params.len(),
                expects_body: route.expecting.is_some(),
            });
        }
    }

    fn reply(&mut self, reply: &'a vaab_syntax::ast::ReplyStmt, span: Span) {
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
}""",
)

# Compile expr RequestField
patch(
    "crates/vaab-vm/src/compile/expr.rs",
    """    fn member(&mut self, expression: &'a Expr, target: &'a Expr, span: Span) {
        match self.checked.resolution(expression.id).cloned() {
            Some(Resolution::Field { field, .. }) => {""",
    """    fn member(&mut self, expression: &'a Expr, target: &'a Expr, span: Span) {
        match self.checked.resolution(expression.id).cloned() {
            Some(Resolution::RequestField(index)) => {
                self.emit(Op::LoadLocal(0), span);
                self.emit(Op::TupleItem(index as u32), span);
            }
            Some(Resolution::Field { field, .. }) => {""",
)

# CLI
write(
    "crates/vaab-cli/Cargo.toml",
    """[package]
name = "vaab-cli"
description = "The `vaab` command-line tool."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "vaab"
path = "src/main.rs"

[dependencies]
vaab-syntax.workspace = true
vaab-types.workspace = true
vaab-vm.workspace = true
vaab-server.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
""",
)
patch("crates/vaab-cli/src/args.rs", "    Run { path: PathBuf },\n    Repl,", "    Run { path: PathBuf },\n    Serve { path: PathBuf },\n    Repl,")
patch(
    "crates/vaab-cli/src/args.rs",
    '            Some("run") => Command::Run { path: expect_file(&mut words, "run")? },\n            Some("repl") => Command::Repl,',
    '            Some("run") => Command::Run { path: expect_file(&mut words, "run")? },\n            Some("serve") => Command::Serve { path: expect_file(&mut words, "serve")? },\n            Some("repl") => Command::Repl,',
)
patch("crates/vaab-cli/src/main.rs", "        Command::Run { path } => run_file(&path, args.color),\n        Command::Repl => {", "        Command::Run { path } => run_file(&path, args.color),\n        Command::Serve { path } => serve_file(&path, args.color),\n        Command::Repl => {")
patch(
    "crates/vaab-cli/src/main.rs",
    "/// `vaab run file.vaab`: check a file, then run it.",
    """/// `vaab serve file.vaab`: check a file, then start its HTTP server.
fn serve_file(path: &Path, color: ColorChoice) -> ExitCode {
    let Some(source) = read(path) else { return exit::misuse() };

    let name = path.display().to_string();
    let parsed = vaab_syntax::parse(&source);

    if parsed.has_errors() {
        return report(&parsed.diagnostics, &name, &source, color);
    }

    let checked = match vaab_types::check(&parsed.module) {
        Ok(checked) => checked,
        Err(problems) => return report(&problems, &name, &source, color),
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(vaab_server::serve_file(&source, &parsed.module, &checked)) {
        Ok(()) => exit::OK,
        Err(message) => {
            eprintln!("vaab: {message}");
            exit::PROBLEMS
        }
    }
}

/// `vaab run file.vaab`: check a file, then run it.""",
)
patch(
    "crates/vaab-cli/src/main.rs",
    "  run <file>     Check a file and run it\n  repl           Start an interactive session",
    "  run <file>     Check a file and run it\n  serve <file>   Check a file and start its HTTP server\n  repl           Start an interactive session",
)

# Tests
patch(
    "crates/vaab-syntax/tests/parse.rs",
    "// ---------------------------------------------------------------------------\n// Values and bindings",
    """// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

#[test]
fn serve_block() {
    assert_snapshot!(tree(
        r#"
serve on port 8080 {
    before every request { print("{request.method}") }
    route get "/hello" { reply with "hello" }
    when anything fails with ApiError as error { reply explain(error) }
}
"#
    ));
}

// ---------------------------------------------------------------------------
// Values and bindings""",
)
patch(
    "crates/vaab-types/tests/errors.rs",
    "// ---------------------------------------------------------------------------\n// The two messages the specification names",
    """// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

#[test]
fn reply_outside_route() {
    assert_snapshot!(errors(r#"reply with "oops"
"#));
}

// ---------------------------------------------------------------------------
// The two messages the specification names""",
)

# Workspace Cargo.toml
write(
    "Cargo.toml",
    """[workspace]
resolver = "2"
members = [
    "crates/vaab-syntax",
    "crates/vaab-types",
    "crates/vaab-vm",
    "crates/vaab-server",
    "crates/vaab-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
license = "MIT OR Apache-2.0"
repository = "https://github.com/vaab-lang/vaab"

[workspace.dependencies]
ariadne = "0.5"
indexmap = "2"
logos = "0.15"
serde_json = "1"
thiserror = "2"
insta = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "net"] }
hyper = "1"
hyper-util = { version = "0.1", features = ["server", "tokio", "http1"] }
http-body-util = "0.1"
vaab-syntax = { path = "crates/vaab-syntax" }
vaab-types = { path = "crates/vaab-types" }
vaab-vm = { path = "crates/vaab-vm" }
vaab-server = { path = "crates/vaab-server" }

[profile.release]
lto = "thin"
""",
)
