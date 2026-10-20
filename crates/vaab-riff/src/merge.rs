//! Merge needed riffs into one module by prefixing exported names and
//! assigning fresh `NodeId`s across the combined syntax tree.

use std::collections::{HashMap, HashSet};

use vaab_syntax::ast::{
    AbilityDecl, ArmBody, ArmPattern, Argument, AssignStmt, Block, CastDecl, ChoiceDecl,
    ElseBranch, ExpectingDecl, Expr, ExprKind, Field, ForEachStmt, FunctionBody, FunctionDecl,
    IfExpr, LetStmt, MapEntry, MatchArm, MatchExpr, Module, MountDecl, MountUse, Name, NodeId,
    Parameter, Pattern, PatternKind, RepeatStmt,     ReplyKind, ReplyStmt, RouteDecl, RouteSegment,
    SelectArm, SelectExpr, SendStmt, ServeDecl, ServeErrorHandler, SocketRouteDecl, Stmt, StmtKind,
    TextPart,
    TypeDecl, TypeExpr, TypeKind, Variant, VariantField, WhileStmt,
};

use crate::link::{exports_of, LoadedRiff, RiffImports};

enum Mode {
    Prefix {
        prefix: String,
        exports: HashSet<String>,
    },
    Qualify {
        /// For each riff alias, maps an exported name to its merged top-level name.
        riffs: HashMap<String, HashMap<String, String>>,
    },
}

struct Mapper {
    mode: Mode,
    scopes: Vec<HashSet<String>>,
}

/// One module containing every riff plus the entry file, with unique `NodeId`s.
pub fn merge(entry: Module, riffs: &[LoadedRiff]) -> Module {
    let entry_has_user = entry.statements.iter().any(|s| {
        matches!(&s.kind, StmtKind::Type(t) if t.name.text == "User")
    });
    let mut riff_map: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut prepared = Vec::new();
    for riff in riffs {
        let (riff_stmts, mapping) = select_riff_statements(riff, !entry_has_user);
        riff_map.insert(riff.alias.clone(), mapping);
        prepared.push(Module { statements: riff_stmts, span: riff.module.span });
    }
    let mut statements = Vec::new();
    for module in prepared {
        statements.extend(qualify(module, &riff_map).statements);
    }
    let entry = qualify(entry, &riff_map);
    let span = entry.span;
    statements.extend(entry.statements);
    let mut merged = Module { statements, span };
    renumber_module(&mut merged);
    merged
}

fn qualify(module: Module, riffs: &HashMap<String, HashMap<String, String>>) -> Module {
    Mapper::new(Mode::Qualify { riffs: riffs.clone() }).map_module(module)
}

fn select_riff_statements(
    riff: &LoadedRiff,
    keep_user_bare: bool,
) -> (Vec<Stmt>, HashMap<String, String>) {
    let all_exports = exports_of(&riff.module);
    let prefix = format!("{}__", riff.alias);
    let mut mapping = HashMap::new();

    let mut prefixed_exports: HashSet<String> = match &riff.imports {
        RiffImports::Bare(bare_names) => {
            for name in &all_exports {
                if bare_names.contains(name) {
                    mapping.insert(name.clone(), name.clone());
                } else {
                    mapping.insert(name.clone(), format!("{prefix}{name}"));
                }
            }
            all_exports.difference(bare_names).cloned().collect()
        }
        RiffImports::Qualified => {
            for name in &all_exports {
                mapping.insert(name.clone(), format!("{prefix}{name}"));
            }
            all_exports.clone()
        }
        RiffImports::Flat => {
            for name in &all_exports {
                mapping.insert(name.clone(), name.clone());
            }
            for statement in &riff.module.statements {
                if let StmtKind::Let(binding) = &statement.kind {
                    mapping.insert(binding.name.text.clone(), binding.name.text.clone());
                }
            }
            HashSet::new()
        }
    };

    if keep_user_bare && all_exports.contains("User") {
        mapping.insert("User".to_string(), "User".to_string());
        prefixed_exports.remove("User");
    }

    let mut mapper = Mapper::new(Mode::Prefix {
        prefix,
        exports: prefixed_exports,
    });
    let stmts = mapper.map_module(riff.module.clone()).statements;
    (stmts, mapping)
}

impl Mapper {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            scopes: vec![HashSet::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind_local(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains(name))
    }

    fn map_module(&mut self, module: Module) -> Module {
        Module {
            statements: module
                .statements
                .into_iter()
                .map(|statement| self.map_stmt(statement))
                .collect(),
            span: module.span,
        }
    }

    fn map_stmt(&mut self, statement: Stmt) -> Stmt {
        Stmt {
            kind: self.map_stmt_kind(statement.kind),
            ..statement
        }
    }

    fn map_stmt_kind(&mut self, kind: StmtKind) -> StmtKind {
        match kind {
            StmtKind::Let(let_stmt) => {
                let declared_type = let_stmt
                    .declared_type
                    .map(|declared| self.map_type_expr(declared));
                let value = self.map_expr(let_stmt.value);
                self.bind_local(&let_stmt.name.text);
                StmtKind::Let(LetStmt {
                    changing: let_stmt.changing,
                    name: let_stmt.name,
                    declared_type,
                    value,
                })
            }
            StmtKind::Assign(assign) => StmtKind::Assign(AssignStmt {
                target: self.map_expr(assign.target),
                value: self.map_expr(assign.value),
            }),
            StmtKind::Return(value) => {
                StmtKind::Return(value.map(|expression| self.map_expr(expression)))
            }
            StmtKind::Fail(value) => StmtKind::Fail(self.map_expr(value)),
            StmtKind::ForEach(for_each) => {
                let sequence = self.map_expr(for_each.sequence);
                self.push_scope();
                let pattern = self.map_pattern(for_each.pattern);
                let body = self.map_block_in_current_scope(for_each.body);
                self.pop_scope();
                StmtKind::ForEach(ForEachStmt {
                    pattern,
                    sequence,
                    body,
                })
            }
            StmtKind::While(while_stmt) => StmtKind::While(WhileStmt {
                condition: self.map_expr(while_stmt.condition),
                body: self.map_block(while_stmt.body),
            }),
            StmtKind::Repeat(repeat) => StmtKind::Repeat(RepeatStmt {
                count: self.map_expr(repeat.count),
                body: self.map_block(repeat.body),
            }),
            StmtKind::Send(send) => StmtKind::Send(SendStmt {
                value: self.map_expr(send.value),
                channel: self.map_expr(send.channel),
            }),
            StmtKind::Close(expression) => StmtKind::Close(self.map_expr(expression)),
            StmtKind::Together(block) => StmtKind::Together(self.map_block(block)),
            StmtKind::Function(function) => {
                StmtKind::Function(Box::new(self.map_function(*function, true)))
            }
            StmtKind::Type(declaration) => StmtKind::Type(Box::new(self.map_type(*declaration))),
            StmtKind::Cast(declaration) => StmtKind::Cast(Box::new(self.map_cast(*declaration))),
            StmtKind::Choice(declaration) => {
                StmtKind::Choice(Box::new(self.map_choice(*declaration)))
            }
            StmtKind::Ability(declaration) => {
                StmtKind::Ability(Box::new(self.map_ability(*declaration)))
            }
            StmtKind::Serve(serve) => StmtKind::Serve(Box::new(self.map_serve(*serve))),
            StmtKind::Mount(mount) => StmtKind::Mount(Box::new(self.map_mount(*mount))),
            StmtKind::Reply(reply) => StmtKind::Reply(self.map_reply(reply)),
            StmtKind::Expr(expression) => StmtKind::Expr(self.map_expr(expression)),
            StmtKind::Need(_) => kind,
        }
    }

    fn map_serve(&mut self, serve: ServeDecl) -> ServeDecl {
        ServeDecl {
            port: self.map_expr(serve.port),
            on_start: serve.on_start.map(|block| self.map_block(block)),
            before: serve.before.map(|block| {
                self.push_scope();
                self.bind_local("request");
                let mapped = self.map_block_in_current_scope(block);
                self.pop_scope();
                mapped
            }),
            mounts: serve
                .mounts
                .into_iter()
                .map(|target| self.map_mount_use(target))
                .collect(),
            routes: serve
                .routes
                .into_iter()
                .map(|route| self.map_route(route))
                .collect(),
            sockets: serve
                .sockets
                .into_iter()
                .map(|socket| self.map_socket_route(socket))
                .collect(),
            error_handler: serve
                .error_handler
                .map(|handler| self.map_serve_error_handler(handler)),
            span: serve.span,
        }
    }

    fn map_socket_route(&mut self, socket: SocketRouteDecl) -> SocketRouteDecl {
        self.push_scope();
        self.bind_local(&socket.binding.text);
        let path = socket
            .path
            .into_iter()
            .map(|segment| match segment {
                RouteSegment::Literal(text) => RouteSegment::Literal(text),
                RouteSegment::Param { name, declared } => {
                    self.bind_local(&name.text);
                    RouteSegment::Param {
                        name,
                        declared: declared.map(|ty| self.map_type_expr(ty)),
                    }
                }
                RouteSegment::CatchAll { name } => {
                    self.bind_local(&name.text);
                    RouteSegment::CatchAll { name }
                }
            })
            .collect();
        let body = self.map_block_in_current_scope(socket.body);
        self.pop_scope();
        SocketRouteDecl {
            path,
            binding: socket.binding,
            body,
            span: socket.span,
        }
    }

    fn map_mount(&mut self, mount: MountDecl) -> MountDecl {
        MountDecl {
            name: self.map_top_level_name(mount.name),
            path: mount.path,
            mounts: mount
                .mounts
                .into_iter()
                .map(|target| self.map_mount_use(target))
                .collect(),
            routes: mount
                .routes
                .into_iter()
                .map(|route| self.map_route(route))
                .collect(),
            span: mount.span,
        }
    }

    fn map_mount_use(&self, mount_use: MountUse) -> MountUse {
        MountUse {
            name: self.map_mount_target(mount_use.name),
            path: mount_use.path,
            span: mount_use.span,
        }
    }

    fn map_mount_target(&self, target: Name) -> Name {
        match &self.mode {
            Mode::Prefix { prefix, exports } if exports.contains(&target.text) => {
                Name::new(format!("{prefix}{}", target.text), target.span)
            }
            Mode::Qualify { riffs } => {
                if let Some((alias, member)) = target.text.split_once('.') {
                    if let Some(exports) = riffs.get(alias) {
                        let resolved = exports
                            .get(member)
                            .or_else(|| exports.get(&format!("{alias}_{member}")))
                            .cloned()
                            .unwrap_or_else(|| format!("{alias}__{member}"));
                        return Name::new(resolved, target.span);
                    }
                }
                if let Some(exports) = riffs.get(&target.text) {
                    if let Some(resolved) = exports
                        .get(&target.text)
                        .or_else(|| exports.get(&format!("{}_all", target.text)))
                    {
                        return Name::new(resolved.clone(), target.span);
                    }
                }
                for exports in riffs.values() {
                    if let Some(resolved) = exports.get(&target.text) {
                        return Name::new(resolved.clone(), target.span);
                    }
                }
                target
            }
            _ => target,
        }
    }

    fn map_route(&mut self, route: RouteDecl) -> RouteDecl {
        self.push_scope();
        self.bind_local("request");
        let path: Vec<RouteSegment> = route
            .path
            .into_iter()
            .map(|segment| match segment {
                RouteSegment::Literal(text) => RouteSegment::Literal(text),
                RouteSegment::Param { name, declared } => {
                    self.bind_local(&name.text);
                    RouteSegment::Param {
                        name,
                        declared: declared.map(|ty| self.map_type_expr(ty)),
                    }
                }
                RouteSegment::CatchAll { name } => {
                    self.bind_local(&name.text);
                    RouteSegment::CatchAll { name }
                }
            })
            .collect();
        let expecting = route.expecting.map(|expecting| {
            let declared = self.map_type_expr(expecting.declared);
            self.bind_local(&expecting.binding.text);
            ExpectingDecl {
                declared,
                binding: expecting.binding,
            }
        });
        let body = self.map_block_in_current_scope(route.body);
        self.pop_scope();
        RouteDecl {
            method: route.method,
            path,
            expecting,
            body,
            span: route.span,
        }
    }

    fn map_serve_error_handler(&mut self, handler: ServeErrorHandler) -> ServeErrorHandler {
        let error_type = self.map_type_expr(handler.error_type);
        self.push_scope();
        self.bind_local(&handler.binding.text);
        let body = self.map_block_in_current_scope(handler.body);
        self.pop_scope();
        ServeErrorHandler {
            error_type,
            binding: handler.binding,
            body,
        }
    }

    fn map_reply(&mut self, reply: ReplyStmt) -> ReplyStmt {
        ReplyStmt {
            kind: match reply.kind {
                ReplyKind::With { value, status } => ReplyKind::With {
                    value: self.map_expr(value),
                    status: status.map(|expression| self.map_expr(expression)),
                },
                ReplyKind::File { path, status } => ReplyKind::File {
                    path: self.map_expr(path),
                    status: status.map(|expression| self.map_expr(expression)),
                },
                ReplyKind::Text {
                    body,
                    content_type,
                    status,
                } => ReplyKind::Text {
                    body: self.map_expr(body),
                    content_type: self.map_expr(content_type),
                    status: status.map(|expression| self.map_expr(expression)),
                },
                ReplyKind::Explain(value) => ReplyKind::Explain(self.map_expr(value)),
            },
            span: reply.span,
        }
    }

    fn map_function(&mut self, function: FunctionDecl, top_level: bool) -> FunctionDecl {
        let name = if top_level {
            self.map_top_level_name(function.name)
        } else {
            function.name
        };
        let parameters: Vec<Parameter> = function
            .parameters
            .into_iter()
            .map(|parameter| self.map_parameter(parameter))
            .collect();
        let returns = function
            .returns
            .map(|declared| self.map_type_expr(declared));

        self.push_scope();
        for parameter in &parameters {
            self.bind_local(&parameter.name.text);
        }
        let body = function.body.map(|body| self.map_function_body(body));
        self.pop_scope();

        FunctionDecl {
            pure: function.pure,
            name,
            parameters,
            returns,
            body,
            class_method: function.class_method,
            span: function.span,
        }
    }

    fn map_parameter(&mut self, parameter: Parameter) -> Parameter {
        Parameter {
            name: parameter.name,
            declared_type: self.map_type_expr(parameter.declared_type),
            default: parameter.default.map(|expression| self.map_expr(expression)),
            span: parameter.span,
        }
    }

    fn map_type(&mut self, declaration: TypeDecl) -> TypeDecl {
        TypeDecl {
            name: self.map_top_level_name(declaration.name),
            fields: declaration
                .fields
                .into_iter()
                .map(|field| self.map_field(field))
                .collect(),
            functions: declaration
                .functions
                .into_iter()
                .map(|function| self.map_function(function, false))
                .collect(),
            abilities: declaration
                .abilities
                .into_iter()
                .map(|ability| self.map_type_name(ability))
                .collect(),
            span: declaration.span,
        }
    }

    fn map_cast(&mut self, declaration: CastDecl) -> CastDecl {
        CastDecl {
            name: self.map_top_level_name(declaration.name),
            entertains: declaration
                .entertains
                .into_iter()
                .map(|parent| self.map_type_name(parent))
                .collect(),
            fields: declaration
                .fields
                .into_iter()
                .map(|field| self.map_field(field))
                .collect(),
            functions: declaration
                .functions
                .into_iter()
                .map(|function| self.map_function(function, false))
                .collect(),
            abilities: declaration
                .abilities
                .into_iter()
                .map(|ability| self.map_type_name(ability))
                .collect(),
            span: declaration.span,
        }
    }

    fn map_choice(&mut self, declaration: ChoiceDecl) -> ChoiceDecl {
        ChoiceDecl {
            name: self.map_top_level_name(declaration.name),
            variants: declaration
                .variants
                .into_iter()
                .map(|variant| self.map_variant(variant))
                .collect(),
            span: declaration.span,
        }
    }

    fn map_field(&mut self, field: Field) -> Field {
        Field {
            name: field.name,
            declared_type: self.map_type_expr(field.declared_type),
            default: field.default.map(|expression| self.map_expr(expression)),
            changing: field.changing,
            span: field.span,
        }
    }

    fn map_variant_field(&mut self, field: VariantField) -> VariantField {
        VariantField {
            name: field.name,
            declared_type: self.map_type_expr(field.declared_type),
            span: field.span,
        }
    }

    fn map_variant(&mut self, variant: Variant) -> Variant {
        Variant {
            name: variant.name,
            fields: variant
                .fields
                .into_iter()
                .map(|field| self.map_variant_field(field))
                .collect(),
            span: variant.span,
        }
    }

    fn map_ability(&mut self, declaration: AbilityDecl) -> AbilityDecl {
        AbilityDecl {
            name: self.map_top_level_name(declaration.name),
            functions: declaration
                .functions
                .into_iter()
                .map(|function| self.map_function(function, false))
                .collect(),
            span: declaration.span,
        }
    }

    fn map_top_level_name(&self, name: Name) -> Name {
        match &self.mode {
            Mode::Prefix { prefix, exports } if exports.contains(&name.text) => {
                Name::new(format!("{prefix}{}", name.text), name.span)
            }
            _ => name,
        }
    }

    fn map_type_expr(&mut self, declared: TypeExpr) -> TypeExpr {
        TypeExpr {
            kind: self.map_type_kind(declared.kind),
            span: declared.span,
        }
    }

    fn map_type_kind(&mut self, kind: TypeKind) -> TypeKind {
        match kind {
            TypeKind::Named(name) => TypeKind::Named(self.map_type_name(name)),
            TypeKind::List(inner) => TypeKind::List(Box::new(self.map_type_expr(*inner))),
            TypeKind::Map { key, value } => TypeKind::Map {
                key: Box::new(self.map_type_expr(*key)),
                value: Box::new(self.map_type_expr(*value)),
            },
            TypeKind::Maybe(inner) => TypeKind::Maybe(Box::new(self.map_type_expr(*inner))),
            TypeKind::Channel(inner) => TypeKind::Channel(Box::new(self.map_type_expr(*inner))),
            TypeKind::Task(inner) => TypeKind::Task(Box::new(self.map_type_expr(*inner))),
            TypeKind::Shared(inner) => TypeKind::Shared(Box::new(self.map_type_expr(*inner))),
            TypeKind::Hub(inner) => TypeKind::Hub(Box::new(self.map_type_expr(*inner))),
            TypeKind::Tuple(items) => {
                TypeKind::Tuple(items.into_iter().map(|item| self.map_type_expr(item)).collect())
            }
            TypeKind::Function {
                parameters,
                returns,
            } => TypeKind::Function {
                parameters: parameters
                    .into_iter()
                    .map(|parameter| self.map_type_expr(parameter))
                    .collect(),
                returns: returns.map(|item| Box::new(self.map_type_expr(*item))),
            },
            TypeKind::Fallible { ok, error } => TypeKind::Fallible {
                ok: Box::new(self.map_type_expr(*ok)),
                error: Box::new(self.map_type_expr(*error)),
            },
        }
    }

    fn map_type_name(&self, name: Name) -> Name {
        self.map_top_level_name(name)
    }

    fn map_function_body(&mut self, body: FunctionBody) -> FunctionBody {
        match body {
            FunctionBody::Block(block) => {
                FunctionBody::Block(self.map_block_in_current_scope(block))
            }
            FunctionBody::Expr(expression) => FunctionBody::Expr(self.map_expr(expression)),
        }
    }

    fn map_block(&mut self, block: Block) -> Block {
        self.push_scope();
        let mapped = self.map_block_in_current_scope(block);
        self.pop_scope();
        mapped
    }

    fn map_block_in_current_scope(&mut self, block: Block) -> Block {
        Block {
            statements: block
                .statements
                .into_iter()
                .map(|statement| self.map_stmt(statement))
                .collect(),
            span: block.span,
        }
    }

    fn map_expr(&mut self, expression: Expr) -> Expr {
        let kind = match expression.kind {
            ExprKind::Name(name) => ExprKind::Name(self.map_name(name)),
            ExprKind::Member { target, name } => match &self.mode {
                Mode::Qualify { riffs } => {
                    if let ExprKind::Name(alias) = &target.kind {
                        if !self.is_local(&alias.text) {
                            if let Some(exports) = riffs.get(&alias.text) {
                                let resolved = exports
                                    .get(&name.text)
                                    .cloned()
                                    .unwrap_or_else(|| format!("{}__{}", alias.text, name.text));
                                return Expr {
                                    id: expression.id,
                                    kind: ExprKind::Name(Name::new(resolved, expression.span)),
                                    span: expression.span,
                                };
                            }
                        }
                    }
                    ExprKind::Member {
                        target: Box::new(self.map_expr(*target)),
                        name,
                    }
                }
                Mode::Prefix { exports, .. } => {
                    if let ExprKind::Name(owner) = &target.kind {
                        if !self.is_local(&owner.text) && exports.contains(&owner.text) {
                            return Expr {
                                id: expression.id,
                                kind: ExprKind::Member {
                                    target: Box::new(Expr {
                                        id: target.id,
                                        kind: ExprKind::Name(self.map_name(owner.clone())),
                                        span: target.span,
                                    }),
                                    name,
                                },
                                span: expression.span,
                            };
                        }
                    }
                    ExprKind::Member {
                        target: Box::new(self.map_expr(*target)),
                        name,
                    }
                }
            },
            ExprKind::Text(parts) => ExprKind::Text(
                parts
                    .into_iter()
                    .map(|part| match part {
                        TextPart::Literal(text) => TextPart::Literal(text),
                        TextPart::Interpolation(inner) => {
                            TextPart::Interpolation(self.map_expr(inner))
                        }
                    })
                    .collect(),
            ),
            ExprKind::List(items) => {
                ExprKind::List(items.into_iter().map(|item| self.map_expr(item)).collect())
            }
            ExprKind::Map(entries) => ExprKind::Map(
                entries
                    .into_iter()
                    .map(|entry| MapEntry {
                        key: self.map_expr(entry.key),
                        value: self.map_expr(entry.value),
                        span: entry.span,
                    })
                    .collect(),
            ),
            ExprKind::Tuple(items) => {
                ExprKind::Tuple(items.into_iter().map(|item| self.map_expr(item)).collect())
            }
            ExprKind::Range { start, end } => ExprKind::Range {
                start: Box::new(self.map_expr(*start)),
                end: Box::new(self.map_expr(*end)),
            },
            ExprKind::Unary { operator, operand } => ExprKind::Unary {
                operator,
                operand: Box::new(self.map_expr(*operand)),
            },
            ExprKind::Binary {
                operator,
                left,
                right,
            } => ExprKind::Binary {
                operator,
                left: Box::new(self.map_expr(*left)),
                right: Box::new(self.map_expr(*right)),
            },
            ExprKind::Call { callee, arguments } => ExprKind::Call {
                callee: Box::new(self.map_expr(*callee)),
                arguments: arguments
                    .into_iter()
                    .map(|argument| Argument {
                        name: argument.name,
                        value: self.map_expr(argument.value),
                        span: argument.span,
                    })
                    .collect(),
            },
            ExprKind::Index { target, index } => ExprKind::Index {
                target: Box::new(self.map_expr(*target)),
                index: Box::new(self.map_expr(*index)),
            },
            ExprKind::Closure { parameters, body } => {
                self.push_scope();
                for parameter in &parameters {
                    self.bind_local(&parameter.text);
                }
                let body = Box::new(self.map_function_body(*body));
                self.pop_scope();
                ExprKind::Closure { parameters, body }
            }
            ExprKind::If(branch) => ExprKind::If(Box::new(self.map_if(*branch))),
            ExprKind::Match(subject) => ExprKind::Match(Box::new(self.map_match(*subject))),
            ExprKind::Found(inner) => ExprKind::Found(Box::new(self.map_expr(*inner))),
            ExprKind::Success(inner) => ExprKind::Success(Box::new(self.map_expr(*inner))),
            ExprKind::Failure(inner) => ExprKind::Failure(Box::new(self.map_expr(*inner))),
            ExprKind::Try(inner) => ExprKind::Try(Box::new(self.map_expr(*inner))),
            ExprKind::Otherwise { value, fallback } => ExprKind::Otherwise {
                value: Box::new(self.map_expr(*value)),
                fallback: Box::new(self.map_expr(*fallback)),
            },
            ExprKind::Receive { channel } => ExprKind::Receive {
                channel: Box::new(self.map_expr(*channel)),
            },
            ExprKind::Start(body) => ExprKind::Start(Box::new(self.map_block(*body))),
            ExprKind::Transaction { database, body } => ExprKind::Transaction {
                database: Box::new(self.map_expr(*database)),
                body: Box::new(self.map_block(*body)),
            },
            ExprKind::Select(select) => ExprKind::Select(Box::new(self.map_select(*select))),
            other => other,
        };
        Expr {
            id: expression.id,
            kind,
            span: expression.span,
        }
    }

    fn map_name(&self, name: Name) -> Name {
        if self.is_local(&name.text) {
            return name;
        }
        match &self.mode {
            Mode::Prefix { prefix, exports } if exports.contains(&name.text) => {
                Name::new(format!("{prefix}{}", name.text), name.span)
            }
            _ => name,
        }
    }

    fn map_if(&mut self, branch: IfExpr) -> IfExpr {
        IfExpr {
            condition: self.map_expr(branch.condition),
            then_block: self.map_block(branch.then_block),
            else_branch: branch.else_branch.map(|else_branch| match else_branch {
                ElseBranch::If(nested) => ElseBranch::If(Box::new(self.map_if(*nested))),
                ElseBranch::Block(block) => ElseBranch::Block(self.map_block(block)),
            }),
        }
    }

    fn map_match(&mut self, subject: MatchExpr) -> MatchExpr {
        MatchExpr {
            subject: self.map_expr(subject.subject),
            arms: subject
                .arms
                .into_iter()
                .map(|arm| {
                    self.push_scope();
                    let pattern = self.map_arm_pattern(arm.pattern);
                    let guard = arm.guard.map(|expression| self.map_expr(expression));
                    let body = match arm.body {
                        ArmBody::Expr(expression) => ArmBody::Expr(self.map_expr(expression)),
                        ArmBody::Block(block) => {
                            ArmBody::Block(self.map_block_in_current_scope(block))
                        }
                    };
                    self.pop_scope();
                    MatchArm {
                        pattern,
                        guard,
                        body,
                        span: arm.span,
                    }
                })
                .collect(),
        }
    }

    fn map_select(&mut self, select: SelectExpr) -> SelectExpr {
        SelectExpr {
            arms: select
                .arms
                .into_iter()
                .map(|arm| match arm {
                    SelectArm::Receive {
                        channel,
                        binding,
                        body,
                        span,
                    } => {
                        let channel = self.map_expr(channel);
                        self.push_scope();
                        if let Some(binding) = &binding {
                            self.bind_local(&binding.text);
                        }
                        let body = self.map_block_in_current_scope(body);
                        self.pop_scope();
                        SelectArm::Receive {
                            channel,
                            binding,
                            body,
                            span,
                        }
                    }
                    SelectArm::Timeout {
                        amount,
                        unit,
                        body,
                        span,
                    } => SelectArm::Timeout {
                        amount: self.map_expr(amount),
                        unit,
                        body: self.map_block(body),
                        span,
                    },
                })
                .collect(),
            otherwise: select.otherwise.map(|block| self.map_block(block)),
        }
    }

    fn map_arm_pattern(&mut self, pattern: ArmPattern) -> ArmPattern {
        match pattern {
            ArmPattern::Pattern(pattern) => ArmPattern::Pattern(self.map_pattern(pattern)),
            ArmPattern::Otherwise => ArmPattern::Otherwise,
        }
    }

    fn map_pattern(&mut self, pattern: Pattern) -> Pattern {
        Pattern {
            id: pattern.id,
            kind: self.map_pattern_kind(pattern.kind),
            span: pattern.span,
        }
    }

    fn map_pattern_kind(&mut self, kind: PatternKind) -> PatternKind {
        match kind {
            PatternKind::Binding(name) => {
                self.bind_local(&name.text);
                PatternKind::Binding(name)
            }
            PatternKind::Int(value) => PatternKind::Int(value),
            PatternKind::Float(value) => PatternKind::Float(value),
            PatternKind::Bool(value) => PatternKind::Bool(value),
            PatternKind::Text(value) => PatternKind::Text(value),
            PatternKind::Nothing => PatternKind::Nothing,
            PatternKind::Found(inner) => PatternKind::Found(Box::new(self.map_pattern(*inner))),
            PatternKind::Success(inner) => PatternKind::Success(Box::new(self.map_pattern(*inner))),
            PatternKind::Failure(inner) => PatternKind::Failure(Box::new(self.map_pattern(*inner))),
            PatternKind::Variant { path, fields } => {
                let mapped_path = match (&self.mode, path.as_slice()) {
                    (Mode::Qualify { riffs }, [alias, choice, variant]) => {
                        if let Some(exports) = riffs.get(&alias.text) {
                            let resolved = exports
                                .get(&choice.text)
                                .cloned()
                                .unwrap_or_else(|| format!("{}__{}", alias.text, choice.text));
                            vec![
                                Name::new(resolved, alias.span.to(choice.span)),
                                variant.clone(),
                            ]
                        } else {
                            path
                        }
                    }
                    (Mode::Prefix { .. }, [choice, variant]) => {
                        vec![self.map_type_name(choice.clone()), variant.clone()]
                    }
                    _ => path,
                };
                PatternKind::Variant {
                    path: mapped_path,
                    fields: fields
                        .into_iter()
                        .map(|field| self.map_pattern(field))
                        .collect(),
                }
            }
            PatternKind::List { elements, rest } => PatternKind::List {
                elements: elements
                    .into_iter()
                    .map(|element| self.map_pattern(element))
                    .collect(),
                rest,
            },
            PatternKind::Tuple(items) => PatternKind::Tuple(
                items
                    .into_iter()
                    .map(|item| self.map_pattern(item))
                    .collect(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Renumbering NodeIds across a merged module
// ---------------------------------------------------------------------------

struct Renumber {
    next: u32,
}

fn renumber_module(module: &mut Module) {
    let mut renumber = Renumber { next: 0 };
    for statement in &mut module.statements {
        renumber.stmt(statement);
    }
}

impl Renumber {
    fn id(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next = self.next.saturating_add(1);
        id
    }

    fn block(&mut self, block: &mut Block) {
        for statement in &mut block.statements {
            self.stmt(statement);
        }
    }

    fn function_body(&mut self, body: &mut FunctionBody) {
        match body {
            FunctionBody::Block(block) => self.block(block),
            FunctionBody::Expr(expr) => self.expr(expr),
        }
    }

    fn function(&mut self, function: &mut FunctionDecl) {
        for parameter in &mut function.parameters {
            if let Some(default) = &mut parameter.default {
                self.expr(default);
            }
        }
        if let Some(body) = &mut function.body {
            self.function_body(body);
        }
    }

    fn fields(&mut self, fields: &mut [Field]) {
        for field in fields {
            if let Some(default) = &mut field.default {
                self.expr(default);
            }
        }
    }

    fn stmt(&mut self, statement: &mut Stmt) {
        match &mut statement.kind {
            StmtKind::Let(let_stmt) => self.expr(&mut let_stmt.value),
            StmtKind::Assign(assign) => {
                self.expr(&mut assign.target);
                self.expr(&mut assign.value);
            }
            StmtKind::Return(value) => {
                if let Some(expr) = value {
                    self.expr(expr);
                }
            }
            StmtKind::Fail(expr) => self.expr(expr),
            StmtKind::ForEach(for_each) => {
                self.pattern(&mut for_each.pattern);
                self.expr(&mut for_each.sequence);
                self.block(&mut for_each.body);
            }
            StmtKind::While(while_stmt) => {
                self.expr(&mut while_stmt.condition);
                self.block(&mut while_stmt.body);
            }
            StmtKind::Repeat(repeat) => {
                self.expr(&mut repeat.count);
                self.block(&mut repeat.body);
            }
            StmtKind::Send(send) => {
                self.expr(&mut send.value);
                self.expr(&mut send.channel);
            }
            StmtKind::Close(expr) => self.expr(expr),
            StmtKind::Together(block) => self.block(block),
            StmtKind::Function(function) => self.function(function),
            StmtKind::Type(declaration) => {
                self.fields(&mut declaration.fields);
                for function in &mut declaration.functions {
                    self.function(function);
                }
            }
            StmtKind::Cast(declaration) => {
                self.fields(&mut declaration.fields);
                for function in &mut declaration.functions {
                    self.function(function);
                }
            }
            StmtKind::Choice(_) => {}
            StmtKind::Ability(declaration) => {
                for function in &mut declaration.functions {
                    self.function(function);
                }
            }
            StmtKind::Serve(serve) => {
                self.expr(&mut serve.port);
                if let Some(on_start) = &mut serve.on_start {
                    self.block(on_start);
                }
                if let Some(before) = &mut serve.before {
                    self.block(before);
                }
                for route in &mut serve.routes {
                    self.block(&mut route.body);
                }
                for socket in &mut serve.sockets {
                    self.block(&mut socket.body);
                }
                if let Some(handler) = &mut serve.error_handler {
                    self.block(&mut handler.body);
                }
            }
            StmtKind::Mount(mount) => {
                for route in &mut mount.routes {
                    self.block(&mut route.body);
                }
            }
            StmtKind::Reply(reply) => match &mut reply.kind {
                ReplyKind::With { value, status } => {
                    self.expr(value);
                    if let Some(status) = status {
                        self.expr(status);
                    }
                }
                ReplyKind::File { path, status } => {
                    self.expr(path);
                    if let Some(status) = status {
                        self.expr(status);
                    }
                }
                ReplyKind::Text {
                    body,
                    content_type,
                    status,
                } => {
                    self.expr(body);
                    self.expr(content_type);
                    if let Some(status) = status {
                        self.expr(status);
                    }
                }
                ReplyKind::Explain(value) => self.expr(value),
            },
            StmtKind::Expr(expr) => self.expr(expr),
            StmtKind::Need(_) => {}
        }
        statement.id = self.id();
    }

    fn if_expr(&mut self, branch: &mut IfExpr) {
        self.expr(&mut branch.condition);
        self.block(&mut branch.then_block);
        if let Some(else_branch) = &mut branch.else_branch {
            match else_branch {
                ElseBranch::If(nested) => self.if_expr(nested),
                ElseBranch::Block(block) => self.block(block),
            }
        }
    }

    fn expr(&mut self, expression: &mut Expr) {
        match &mut expression.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Nothing
            | ExprKind::Name(_)
            | ExprKind::SelfValue => {}
            ExprKind::Text(parts) => {
                for part in parts {
                    if let TextPart::Interpolation(inner) = part {
                        self.expr(inner);
                    }
                }
            }
            ExprKind::List(items) | ExprKind::Tuple(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.expr(&mut entry.key);
                    self.expr(&mut entry.value);
                }
            }
            ExprKind::Range { start, end } => {
                self.expr(start);
                self.expr(end);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Call { callee, arguments } => {
                self.expr(callee);
                for argument in arguments {
                    self.expr(&mut argument.value);
                }
            }
            ExprKind::Member { target, .. } => self.expr(target),
            ExprKind::Index { target, index } => {
                self.expr(target);
                self.expr(index);
            }
            ExprKind::Closure { body, .. } => self.function_body(body),
            ExprKind::If(branch) => self.if_expr(branch),
            ExprKind::Match(subject) => {
                self.expr(&mut subject.subject);
                for arm in &mut subject.arms {
                    if let ArmPattern::Pattern(pattern) = &mut arm.pattern {
                        self.pattern(pattern);
                    }
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard);
                    }
                    match &mut arm.body {
                        ArmBody::Expr(inner) => self.expr(inner),
                        ArmBody::Block(block) => self.block(block),
                    }
                }
            }
            ExprKind::Found(inner)
            | ExprKind::Success(inner)
            | ExprKind::Failure(inner)
            | ExprKind::Try(inner) => self.expr(inner),
            ExprKind::Otherwise { value, fallback } => {
                self.expr(value);
                self.expr(fallback);
            }
            ExprKind::Receive { channel } => self.expr(channel),
            ExprKind::Start(body) => self.block(body),
            ExprKind::Transaction { database, body } => {
                self.expr(database);
                self.block(body);
            }
            ExprKind::Select(select) => {
                for arm in &mut select.arms {
                    match arm {
                        SelectArm::Receive { channel, body, .. } => {
                            self.expr(channel);
                            self.block(body);
                        }
                        SelectArm::Timeout { amount, body, .. } => {
                            self.expr(amount);
                            self.block(body);
                        }
                    }
                }
                if let Some(otherwise) = &mut select.otherwise {
                    self.block(otherwise);
                }
            }
        }
        expression.id = self.id();
    }

    fn pattern(&mut self, pattern: &mut Pattern) {
        match &mut pattern.kind {
            PatternKind::Int(_)
            | PatternKind::Float(_)
            | PatternKind::Bool(_)
            | PatternKind::Text(_)
            | PatternKind::Binding(_)
            | PatternKind::Nothing => {}
            PatternKind::Found(inner)
            | PatternKind::Success(inner)
            | PatternKind::Failure(inner) => self.pattern(inner),
            PatternKind::Variant { fields, .. } => {
                for field in fields {
                    self.pattern(field);
                }
            }
            PatternKind::List { elements, .. } | PatternKind::Tuple(elements) => {
                for element in elements {
                    self.pattern(element);
                }
            }
        }
        pattern.id = self.id();
    }
}
