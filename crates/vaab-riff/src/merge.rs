//! Merge needed riffs into one module by prefixing exported names.

use std::collections::HashSet;

use vaab_syntax::ast::{
    AbilityDecl, ArmBody, Argument, AssignStmt, Block, ChoiceDecl, ElseBranch, ExpectingDecl, Expr,
    ExprKind, Field, ForEachStmt, FunctionBody, FunctionDecl, IfExpr, LetStmt, MapEntry, MatchArm,
    MatchExpr, Module, Name, Parameter, Pattern, PatternKind, RepeatStmt, ReplyKind, ReplyStmt,
    RouteDecl, SelectArm, SelectExpr, SendStmt, ServeDecl, ServeErrorHandler, Stmt, StmtKind,
    TextPart, TypeDecl, TypeExpr, TypeKind, Variant, VariantField, WhileStmt,
};

use crate::link::{exports_of, LoadedRiff};

enum Mode {
    Prefix { prefix: String, exports: HashSet<String> },
    Qualify { aliases: HashSet<String> },
}

/// One module containing every riff plus the entry file.
pub fn merge(entry: Module, riffs: &[LoadedRiff]) -> Module {
    let aliases: HashSet<String> = riffs.iter().map(|riff| riff.alias.clone()).collect();
    let mut statements = Vec::new();
    for riff in riffs {
        statements.extend(select_riff_statements(riff));
    }
    let entry = map_module(entry, Mode::Qualify { aliases });
    statements.extend(entry.statements);
    Module { statements, span: entry.span }
}

fn select_riff_statements(riff: &LoadedRiff) -> Vec<Stmt> {
    let exports = exports_of(&riff.module);
    match &riff.imports {
        crate::link::RiffImports::Bare(names) => riff
            .module
            .statements
            .iter()
            .filter(|statement| export_name(statement).is_some_and(|name| names.contains(&name)))
            .cloned()
            .collect(),
        crate::link::RiffImports::Qualified => {
            let prefix = format!("{}__", riff.alias);
            map_module(
                riff.module.clone(),
                Mode::Prefix { prefix, exports },
            )
            .statements
        }
    }
}

fn export_name(statement: &Stmt) -> Option<String> {
    match &statement.kind {
        StmtKind::Function(function) => Some(function.name.text.clone()),
        StmtKind::Type(declaration) => Some(declaration.name.text.clone()),
        StmtKind::Choice(declaration) => Some(declaration.name.text.clone()),
        StmtKind::Ability(declaration) => Some(declaration.name.text.clone()),
        _ => None,
    }
}

fn map_module(module: Module, mode: Mode) -> Module {
    Module {
        statements: module
            .statements
            .into_iter()
            .map(|statement| map_stmt(statement, &mode))
            .collect(),
        span: module.span,
    }
}

fn map_stmt(statement: Stmt, mode: &Mode) -> Stmt {
    Stmt {
        kind: map_stmt_kind(statement.kind, mode),
        ..statement
    }
}

fn map_stmt_kind(kind: StmtKind, mode: &Mode) -> StmtKind {
    match kind {
        StmtKind::Let(let_stmt) => StmtKind::Let(LetStmt {
            changing: let_stmt.changing,
            name: let_stmt.name,
            declared_type: let_stmt
                .declared_type
                .map(|declared| map_type_expr(declared, mode)),
            value: map_expr(let_stmt.value, mode),
        }),
        StmtKind::Assign(assign) => StmtKind::Assign(AssignStmt {
            target: map_expr(assign.target, mode),
            value: map_expr(assign.value, mode),
        }),
        StmtKind::Return(value) => StmtKind::Return(value.map(|expression| map_expr(expression, mode))),
        StmtKind::ForEach(for_each) => StmtKind::ForEach(ForEachStmt {
            pattern: map_pattern(for_each.pattern, mode),
            sequence: map_expr(for_each.sequence, mode),
            body: map_block(for_each.body, mode),
        }),
        StmtKind::While(while_stmt) => StmtKind::While(WhileStmt {
            condition: map_expr(while_stmt.condition, mode),
            body: map_block(while_stmt.body, mode),
        }),
        StmtKind::Repeat(repeat) => StmtKind::Repeat(RepeatStmt {
            count: map_expr(repeat.count, mode),
            body: map_block(repeat.body, mode),
        }),
        StmtKind::Send(send) => StmtKind::Send(SendStmt {
            value: map_expr(send.value, mode),
            channel: map_expr(send.channel, mode),
        }),
        StmtKind::Close(expression) => StmtKind::Close(map_expr(expression, mode)),
        StmtKind::Together(block) => StmtKind::Together(map_block(block, mode)),
        StmtKind::Function(function) => StmtKind::Function(Box::new(map_function(*function, mode))),
        StmtKind::Type(declaration) => StmtKind::Type(Box::new(map_type(*declaration, mode))),
        StmtKind::Choice(declaration) => {
            StmtKind::Choice(Box::new(map_choice(*declaration, mode)))
        }
        StmtKind::Ability(declaration) => {
            StmtKind::Ability(Box::new(map_ability(*declaration, mode)))
        }
        StmtKind::Serve(serve) => StmtKind::Serve(Box::new(map_serve(*serve, mode))),
        StmtKind::Reply(reply) => StmtKind::Reply(map_reply(reply, mode)),
        StmtKind::Expr(expression) => StmtKind::Expr(map_expr(expression, mode)),
        StmtKind::Need(_) => kind,
    }
}

fn map_serve(serve: ServeDecl, mode: &Mode) -> ServeDecl {
    ServeDecl {
        port: map_expr(serve.port, mode),
        before: serve.before.map(|block| map_block(block, mode)),
        routes: serve
            .routes
            .into_iter()
            .map(|route| map_route(route, mode))
            .collect(),
        error_handler: serve
            .error_handler
            .map(|handler| map_serve_error_handler(handler, mode)),
        span: serve.span,
    }
}

fn map_route(route: RouteDecl, mode: &Mode) -> RouteDecl {
    RouteDecl {
        method: route.method,
        path: route.path,
        expecting: route.expecting.map(|expecting| ExpectingDecl {
            declared: map_type_expr(expecting.declared, mode),
            binding: expecting.binding,
        }),
        body: map_block(route.body, mode),
        span: route.span,
    }
}

fn map_serve_error_handler(handler: ServeErrorHandler, mode: &Mode) -> ServeErrorHandler {
    ServeErrorHandler {
        error_type: map_type_expr(handler.error_type, mode),
        binding: handler.binding,
        body: map_block(handler.body, mode),
    }
}

fn map_reply(reply: ReplyStmt, mode: &Mode) -> ReplyStmt {
    ReplyStmt {
        kind: match reply.kind {
            ReplyKind::With { value, status } => ReplyKind::With {
                value: map_expr(value, mode),
                status: status.map(|expression| map_expr(expression, mode)),
            },
            ReplyKind::File { path, status } => ReplyKind::File {
                path: map_expr(path, mode),
                status: status.map(|expression| map_expr(expression, mode)),
            },
            ReplyKind::Text {
                body,
                content_type,
                status,
            } => ReplyKind::Text {
                body: map_expr(body, mode),
                content_type: map_expr(content_type, mode),
                status: status.map(|expression| map_expr(expression, mode)),
            },
            ReplyKind::Explain(value) => ReplyKind::Explain(map_expr(value, mode)),
        },
        span: reply.span,
    }
}

fn map_function(function: FunctionDecl, mode: &Mode) -> FunctionDecl {
    FunctionDecl {
        pure: function.pure,
        name: map_top_level_name(function.name, mode),
        parameters: function
            .parameters
            .into_iter()
            .map(|parameter| map_parameter(parameter, mode))
            .collect(),
        returns: function
            .returns
            .map(|declared| map_type_expr(declared, mode)),
        body: function.body.map(|body| map_function_body(body, mode)),
        span: function.span,
    }
}

fn map_parameter(parameter: Parameter, mode: &Mode) -> Parameter {
    Parameter {
        name: parameter.name,
        declared_type: map_type_expr(parameter.declared_type, mode),
        default: parameter.default.map(|expression| map_expr(expression, mode)),
        span: parameter.span,
    }
}

fn map_type(declaration: TypeDecl, mode: &Mode) -> TypeDecl {
    TypeDecl {
        name: map_top_level_name(declaration.name, mode),
        fields: declaration
            .fields
            .into_iter()
            .map(|field| map_field(field, mode))
            .collect(),
        functions: declaration
            .functions
            .into_iter()
            .map(|function| map_function(function, mode))
            .collect(),
        abilities: declaration.abilities,
        span: declaration.span,
    }
}

fn map_choice(declaration: ChoiceDecl, mode: &Mode) -> ChoiceDecl {
    ChoiceDecl {
        name: map_top_level_name(declaration.name, mode),
        variants: declaration
            .variants
            .into_iter()
            .map(|variant| map_variant(variant, mode))
            .collect(),
        span: declaration.span,
    }
}

fn map_field(field: Field, mode: &Mode) -> Field {
    Field {
        name: field.name,
        declared_type: map_type_expr(field.declared_type, mode),
        default: field.default.map(|expression| map_expr(expression, mode)),
        span: field.span,
    }
}

fn map_variant_field(field: VariantField, mode: &Mode) -> VariantField {
    VariantField {
        name: field.name,
        declared_type: map_type_expr(field.declared_type, mode),
        span: field.span,
    }
}

fn map_variant(variant: Variant, mode: &Mode) -> Variant {
    Variant {
        name: variant.name,
        fields: variant
            .fields
            .into_iter()
            .map(|field| map_variant_field(field, mode))
            .collect(),
        span: variant.span,
    }
}

fn map_ability(declaration: AbilityDecl, mode: &Mode) -> AbilityDecl {
    AbilityDecl {
        name: map_top_level_name(declaration.name, mode),
        functions: declaration
            .functions
            .into_iter()
            .map(|function| map_function(function, mode))
            .collect(),
        span: declaration.span,
    }
}

fn map_top_level_name(name: Name, mode: &Mode) -> Name {
    match mode {
        Mode::Prefix { prefix, exports } if exports.contains(&name.text) => {
            Name::new(format!("{prefix}{}", name.text), name.span)
        }
        _ => name,
    }
}

fn map_type_expr(declared: TypeExpr, mode: &Mode) -> TypeExpr {
    TypeExpr {
        kind: map_type_kind(declared.kind, mode),
        span: declared.span,
    }
}

fn map_type_kind(kind: TypeKind, mode: &Mode) -> TypeKind {
    match kind {
        TypeKind::Named(name) => TypeKind::Named(map_type_name(name, mode)),
        TypeKind::List(inner) => TypeKind::List(Box::new(map_type_expr(*inner, mode))),
        TypeKind::Map { key, value } => TypeKind::Map {
            key: Box::new(map_type_expr(*key, mode)),
            value: Box::new(map_type_expr(*value, mode)),
        },
        TypeKind::Maybe(inner) => TypeKind::Maybe(Box::new(map_type_expr(*inner, mode))),
        TypeKind::Channel(inner) => TypeKind::Channel(Box::new(map_type_expr(*inner, mode))),
        TypeKind::Task(inner) => TypeKind::Task(Box::new(map_type_expr(*inner, mode))),
        TypeKind::Shared(inner) => TypeKind::Shared(Box::new(map_type_expr(*inner, mode))),
        TypeKind::Tuple(items) => {
            TypeKind::Tuple(items.into_iter().map(|item| map_type_expr(item, mode)).collect())
        }
        TypeKind::Function { parameters, returns } => TypeKind::Function {
            parameters: parameters
                .into_iter()
                .map(|parameter| map_type_expr(parameter, mode))
                .collect(),
            returns: returns.map(|item| Box::new(map_type_expr(*item, mode))),
        },
        TypeKind::Fallible { ok, error } => TypeKind::Fallible {
            ok: Box::new(map_type_expr(*ok, mode)),
            error: Box::new(map_type_expr(*error, mode)),
        },
    }
}

fn map_type_name(name: Name, mode: &Mode) -> Name {
    map_top_level_name(name, mode)
}

fn map_function_body(body: FunctionBody, mode: &Mode) -> FunctionBody {
    match body {
        FunctionBody::Block(block) => FunctionBody::Block(map_block(block, mode)),
        FunctionBody::Expr(expression) => FunctionBody::Expr(map_expr(expression, mode)),
    }
}

fn map_block(block: Block, mode: &Mode) -> Block {
    Block {
        statements: block
            .statements
            .into_iter()
            .map(|statement| map_stmt(statement, mode))
            .collect(),
        span: block.span,
    }
}

fn map_expr(expression: Expr, mode: &Mode) -> Expr {
    let kind = match expression.kind {
        ExprKind::Name(name) => ExprKind::Name(map_name(name, mode)),
        ExprKind::Member { target, name } => match mode {
            Mode::Qualify { aliases } => {
                if let ExprKind::Name(alias) = &target.kind {
                    if aliases.contains(&alias.text) {
                        return Expr {
                            id: expression.id,
                            kind: ExprKind::Name(Name::new(
                                format!("{}__{}", alias.text, name.text),
                                expression.span,
                            )),
                            span: expression.span,
                        };
                    }
                }
                ExprKind::Member {
                    target: Box::new(map_expr(*target, mode)),
                    name,
                }
            }
            Mode::Prefix { exports, .. } => {
                if let ExprKind::Name(owner) = &target.kind {
                    if exports.contains(&owner.text) {
                        return Expr {
                            id: expression.id,
                            kind: ExprKind::Member {
                                target: Box::new(Expr {
                                    id: target.id,
                                    kind: ExprKind::Name(map_name(owner.clone(), mode)),
                                    span: target.span,
                                }),
                                name,
                            },
                            span: expression.span,
                        };
                    }
                }
                ExprKind::Member {
                    target: Box::new(map_expr(*target, mode)),
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
                        TextPart::Interpolation(map_expr(inner, mode))
                    }
                })
                .collect(),
        ),
        ExprKind::List(items) => ExprKind::List(items.into_iter().map(|item| map_expr(item, mode)).collect()),
        ExprKind::Map(entries) => ExprKind::Map(
            entries
                .into_iter()
                .map(|entry| MapEntry {
                    key: map_expr(entry.key, mode),
                    value: map_expr(entry.value, mode),
                    span: entry.span,
                })
                .collect(),
        ),
        ExprKind::Tuple(items) => {
            ExprKind::Tuple(items.into_iter().map(|item| map_expr(item, mode)).collect())
        }
        ExprKind::Range { start, end } => ExprKind::Range {
            start: Box::new(map_expr(*start, mode)),
            end: Box::new(map_expr(*end, mode)),
        },
        ExprKind::Unary { operator, operand } => ExprKind::Unary {
            operator,
            operand: Box::new(map_expr(*operand, mode)),
        },
        ExprKind::Binary { operator, left, right } => ExprKind::Binary {
            operator,
            left: Box::new(map_expr(*left, mode)),
            right: Box::new(map_expr(*right, mode)),
        },
        ExprKind::Call { callee, arguments } => ExprKind::Call {
            callee: Box::new(map_expr(*callee, mode)),
            arguments: arguments
                .into_iter()
                .map(|argument| Argument {
                    name: argument.name,
                    value: map_expr(argument.value, mode),
                    span: argument.span,
                })
                .collect(),
        },
        ExprKind::Index { target, index } => ExprKind::Index {
            target: Box::new(map_expr(*target, mode)),
            index: Box::new(map_expr(*index, mode)),
        },
        ExprKind::Closure { parameters, body } => ExprKind::Closure {
            parameters,
            body: Box::new(map_function_body(*body, mode)),
        },
        ExprKind::If(branch) => ExprKind::If(Box::new(map_if(*branch, mode))),
        ExprKind::Match(subject) => ExprKind::Match(Box::new(map_match(*subject, mode))),
        ExprKind::Found(inner) => ExprKind::Found(Box::new(map_expr(*inner, mode))),
        ExprKind::Success(inner) => ExprKind::Success(Box::new(map_expr(*inner, mode))),
        ExprKind::Failure(inner) => ExprKind::Failure(Box::new(map_expr(*inner, mode))),
        ExprKind::Try(inner) => ExprKind::Try(Box::new(map_expr(*inner, mode))),
        ExprKind::Otherwise { value, fallback } => ExprKind::Otherwise {
            value: Box::new(map_expr(*value, mode)),
            fallback: Box::new(map_expr(*fallback, mode)),
        },
        ExprKind::Receive { channel } => ExprKind::Receive {
            channel: Box::new(map_expr(*channel, mode)),
        },
        ExprKind::Start(body) => ExprKind::Start(Box::new(map_block(*body, mode))),
        ExprKind::Select(select) => ExprKind::Select(Box::new(map_select(*select, mode))),
        other => other,
    };
    Expr { id: expression.id, kind, span: expression.span }
}

fn map_name(name: Name, mode: &Mode) -> Name {
    match mode {
        Mode::Prefix { prefix, exports } if exports.contains(&name.text) => {
            Name::new(format!("{prefix}{}", name.text), name.span)
        }
        _ => name,
    }
}

fn map_if(branch: IfExpr, mode: &Mode) -> IfExpr {
    IfExpr {
        condition: map_expr(branch.condition, mode),
        then_block: map_block(branch.then_block, mode),
        else_branch: branch.else_branch.map(|else_branch| match else_branch {
            ElseBranch::If(nested) => ElseBranch::If(Box::new(map_if(*nested, mode))),
            ElseBranch::Block(block) => ElseBranch::Block(map_block(block, mode)),
        }),
    }
}

fn map_match(subject: MatchExpr, mode: &Mode) -> MatchExpr {
    MatchExpr {
        subject: map_expr(subject.subject, mode),
            arms: subject
            .arms
            .into_iter()
            .map(|arm| MatchArm {
                pattern: map_arm_pattern(arm.pattern, mode),
                guard: arm.guard.map(|expression| map_expr(expression, mode)),
                body: match arm.body {
                    ArmBody::Expr(expression) => ArmBody::Expr(map_expr(expression, mode)),
                    ArmBody::Block(block) => ArmBody::Block(map_block(block, mode)),
                },
                span: arm.span,
            })
            .collect(),
    }
}

fn map_select(select: SelectExpr, mode: &Mode) -> SelectExpr {
    SelectExpr {
        arms: select
            .arms
            .into_iter()
            .map(|arm| match arm {
                SelectArm::Receive { channel, binding, body, span } => SelectArm::Receive {
                    channel: map_expr(channel, mode),
                    binding,
                    body: map_block(body, mode),
                    span,
                },
                SelectArm::Timeout { amount, unit, body, span } => SelectArm::Timeout {
                    amount: map_expr(amount, mode),
                    unit,
                    body: map_block(body, mode),
                    span,
                },
            })
            .collect(),
        otherwise: select.otherwise.map(|block| map_block(block, mode)),
    }
}

fn map_arm_pattern(pattern: vaab_syntax::ast::ArmPattern, mode: &Mode) -> vaab_syntax::ast::ArmPattern {
    match pattern {
        vaab_syntax::ast::ArmPattern::Pattern(pattern) => {
            vaab_syntax::ast::ArmPattern::Pattern(map_pattern(pattern, mode))
        }
        vaab_syntax::ast::ArmPattern::Otherwise => vaab_syntax::ast::ArmPattern::Otherwise,
    }
}

fn map_pattern(pattern: Pattern, mode: &Mode) -> Pattern {
    Pattern {
        id: pattern.id,
        kind: map_pattern_kind(pattern.kind, mode),
        span: pattern.span,
    }
}

fn map_pattern_kind(kind: PatternKind, mode: &Mode) -> PatternKind {
    match kind {
        PatternKind::Binding(name) => PatternKind::Binding(name),
        PatternKind::Int(value) => PatternKind::Int(value),
        PatternKind::Float(value) => PatternKind::Float(value),
        PatternKind::Bool(value) => PatternKind::Bool(value),
        PatternKind::Text(value) => PatternKind::Text(value),
        PatternKind::Nothing => PatternKind::Nothing,
        PatternKind::Found(inner) => PatternKind::Found(Box::new(map_pattern(*inner, mode))),
        PatternKind::Success(inner) => PatternKind::Success(Box::new(map_pattern(*inner, mode))),
        PatternKind::Failure(inner) => PatternKind::Failure(Box::new(map_pattern(*inner, mode))),
        PatternKind::Variant { path, fields } => PatternKind::Variant {
            path: path
                .into_iter()
                .enumerate()
                .map(|(index, name)| {
                    if index == 0 {
                        map_type_name(name, mode)
                    } else {
                        name
                    }
                })
                .collect(),
            fields: fields
                .into_iter()
                .map(|field| map_pattern(field, mode))
                .collect(),
        },
        PatternKind::List { elements, rest } => PatternKind::List {
            elements: elements
                .into_iter()
                .map(|element| map_pattern(element, mode))
                .collect(),
            rest,
        },
        PatternKind::Tuple(items) => PatternKind::Tuple(
            items
                .into_iter()
                .map(|item| map_pattern(item, mode))
                .collect(),
        ),
    }
}
