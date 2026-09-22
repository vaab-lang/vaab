//! One walk over a piece of a program, shared by the concurrency rules.
//!
//! Most of the checker asks "what type is this?", which the tree answers node by
//! node on the way down. Phase 4's rules ask something different: what does this
//! whole subtree *reach for*, and what does it *do*? Which names a task body uses
//! from outside itself, whether anything inside a `together` could start a task,
//! and whether the change handed to `.update` would have to wait are all that same
//! question about a subtree, so they share one walk.
//!
//! A visitor says, for each node, whether to look inside it. That is what lets the
//! capture walk stop at a nested `start`, whose own check is the better place to
//! explain anything wrong in it.

use vaab_syntax::ast::{
    ArmBody, Block, ElseBranch, Expr, ExprKind, FunctionBody, FunctionDecl, IfExpr, MatchExpr,
    SelectArm, SelectExpr, Stmt, StmtKind, TextPart,
};

/// Whether to look inside the node that was just visited.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Into,
    Over,
}

/// Something that wants to see the statements and expressions of a subtree.
///
/// Both methods default to looking and carrying on, so an implementor writes only
/// the half it cares about.
pub(super) trait Visit {
    fn statement(&mut self, statement: &Stmt) -> Step {
        let _ = statement;
        Step::Into
    }

    fn expression(&mut self, expression: &Expr) -> Step {
        let _ = expression;
        Step::Into
    }
}

pub(super) fn block(body: &Block, visitor: &mut impl Visit) {
    for step in &body.statements {
        statement(step, visitor);
    }
}

pub(super) fn function_body(body: &FunctionBody, visitor: &mut impl Visit) {
    match body {
        FunctionBody::Block(inner) => block(inner, visitor),
        FunctionBody::Expr(inner) => expression(inner, visitor),
    }
}

pub(super) fn statement(node: &Stmt, visitor: &mut impl Visit) {
    if visitor.statement(node) == Step::Over {
        return;
    }

    match &node.kind {
        StmtKind::Let(declaration) => expression(&declaration.value, visitor),
        StmtKind::Assign(assignment) => {
            expression(&assignment.target, visitor);
            expression(&assignment.value, visitor);
        }
        StmtKind::Return(value) => {
            if let Some(value) = value {
                expression(value, visitor);
            }
        }
        StmtKind::ForEach(loop_) => {
            expression(&loop_.sequence, visitor);
            block(&loop_.body, visitor);
        }
        StmtKind::While(loop_) => {
            expression(&loop_.condition, visitor);
            block(&loop_.body, visitor);
        }
        StmtKind::Repeat(loop_) => {
            expression(&loop_.count, visitor);
            block(&loop_.body, visitor);
        }
        StmtKind::Send(send) => {
            expression(&send.value, visitor);
            expression(&send.channel, visitor);
        }
        StmtKind::Close(channel) => expression(channel, visitor),
        StmtKind::Together(body) => block(body, visitor),
        StmtKind::Function(declaration) => function(declaration, visitor),
        // A `type`, `choice` or `ability` belongs at the top level of a file, so one
        // inside a block has already been reported and its body is not worth
        // walking into.
        StmtKind::Type(_) | StmtKind::Cast(_) | StmtKind::Choice(_) | StmtKind::Ability(_) => {}
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
            vaab_syntax::ast::ReplyKind::File { path, status } => {
                expression(path, visitor);
                if let Some(status) = status {
                    expression(status, visitor);
                }
            }
            vaab_syntax::ast::ReplyKind::Text {
                body,
                content_type,
                status,
            } => {
                expression(body, visitor);
                expression(content_type, visitor);
                if let Some(status) = status {
                    expression(status, visitor);
                }
            }
            vaab_syntax::ast::ReplyKind::Explain(value) => expression(value, visitor),
        },
        StmtKind::Expr(inner) => expression(inner, visitor),
        StmtKind::Need(_) => {}
    }
}

fn function(declaration: &FunctionDecl, visitor: &mut impl Visit) {
    for parameter in &declaration.parameters {
        if let Some(default) = &parameter.default {
            expression(default, visitor);
        }
    }
    if let Some(body) = &declaration.body {
        function_body(body, visitor);
    }
}

pub(super) fn expression(node: &Expr, visitor: &mut impl Visit) {
    if visitor.expression(node) == Step::Over {
        return;
    }

    match &node.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Nothing
        | ExprKind::Name(_)
        | ExprKind::SelfValue => {}

        ExprKind::Text(parts) => {
            for part in parts {
                if let TextPart::Interpolation(inner) = part {
                    expression(inner, visitor);
                }
            }
        }

        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                expression(item, visitor);
            }
        }

        ExprKind::Map(entries) => {
            for entry in entries {
                expression(&entry.key, visitor);
                expression(&entry.value, visitor);
            }
        }

        ExprKind::Range { start, end } => {
            expression(start, visitor);
            expression(end, visitor);
        }

        ExprKind::Unary { operand, .. } => expression(operand, visitor),

        ExprKind::Binary { left, right, .. } => {
            expression(left, visitor);
            expression(right, visitor);
        }

        ExprKind::Call { callee, arguments } => {
            expression(callee, visitor);
            for argument in arguments {
                expression(&argument.value, visitor);
            }
        }

        ExprKind::Member { target, .. } => expression(target, visitor),

        ExprKind::Index { target, index } => {
            expression(target, visitor);
            expression(index, visitor);
        }

        ExprKind::Closure { body, .. } => function_body(body, visitor),

        ExprKind::If(branch) => if_expression(branch, visitor),
        ExprKind::Match(subject) => match_expression(subject, visitor),

        ExprKind::Found(inner)
        | ExprKind::Success(inner)
        | ExprKind::Failure(inner)
        | ExprKind::Try(inner) => expression(inner, visitor),

        ExprKind::Otherwise { value, fallback } => {
            expression(value, visitor);
            expression(fallback, visitor);
        }

        ExprKind::Receive { channel } => expression(channel, visitor),
        ExprKind::Start(body) => block(body, visitor),
        ExprKind::Select(select) => select_expression(select, visitor),
    }
}

fn if_expression(branch: &IfExpr, visitor: &mut impl Visit) {
    expression(&branch.condition, visitor);
    block(&branch.then_block, visitor);
    match &branch.else_branch {
        Some(ElseBranch::Block(body)) => block(body, visitor),
        Some(ElseBranch::If(nested)) => if_expression(nested, visitor),
        None => {}
    }
}

fn match_expression(subject: &MatchExpr, visitor: &mut impl Visit) {
    expression(&subject.subject, visitor);
    for arm in &subject.arms {
        if let Some(guard) = &arm.guard {
            expression(guard, visitor);
        }
        match &arm.body {
            ArmBody::Expr(inner) => expression(inner, visitor),
            ArmBody::Block(body) => block(body, visitor),
        }
    }
}

fn select_expression(select: &SelectExpr, visitor: &mut impl Visit) {
    for arm in &select.arms {
        match arm {
            SelectArm::Receive { channel, body, .. } => {
                expression(channel, visitor);
                block(body, visitor);
            }
            SelectArm::Timeout { amount, body, .. } => {
                expression(amount, visitor);
                block(body, visitor);
            }
        }
    }
    if let Some(otherwise) = &select.otherwise {
        block(otherwise, visitor);
    }
}
