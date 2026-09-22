//! Pure functions: no effects the compiler cannot see.
//!
//! A function declared `pure` may not print, read or write files, use the clock,
//! touch channels or `shared` values, start or wait for tasks, call anything that
//! is not itself `pure`, or change a value declared outside its own body. Changing
//! locals declared inside it is fine.
//!
//! A closure written inside a `pure` function inherits the same rules: whatever the
//! closure does is part of what the function promised. A function held in a value
//! carries no record of whether it is `pure`, so a `pure` function may call a
//! declared function by name but not one reached through a value.

use vaab_syntax::ast::{Expr, ExprKind, FunctionDecl, Stmt, StmtKind};
use vaab_syntax::span::Span;

use super::walk::{self, Step, Visit};
use super::Checker;
use crate::checked::{FunctionId, Resolution};
use crate::messages::{self, PureViolation};
use crate::types::Type;

impl Checker {
    /// Checks a function body against the `pure` promise on its declaration.
    pub(super) fn check_pure_function(
        &mut self,
        declaration: &FunctionDecl,
        id: FunctionId,
        locals_begin: usize,
    ) {
        let Some(function) = self.checked.function(id) else { return };
        if !function.pure {
            return;
        }
        let Some(body) = &declaration.body else { return };

        let signature_span = declaration
            .returns
            .as_ref()
            .map(|returns| returns.span)
            .unwrap_or(declaration.name.span);
        let name = function.name.clone();

        let mut finder = ImpureFinder {
            checked: &self.checked,
            locals_begin,
            found: Vec::new(),
        };
        walk::function_body(body, &mut finder);

        for (span, why) in finder.found {
            self.report(messages::not_pure(&name, why, span, signature_span));
        }
    }
}

/// Everything in a subtree that breaks a `pure` promise.
struct ImpureFinder<'a> {
    checked: &'a crate::checked::Checked,
    /// Locals declared before this function's parameters belong to the outside.
    locals_begin: usize,
    found: Vec<(Span, PureViolation)>,
}

impl Visit for ImpureFinder<'_> {
    fn statement(&mut self, statement: &Stmt) -> Step {
        let why = match &statement.kind {
            StmtKind::Send(_) => Some(PureViolation::Effect("send on a channel")),
            StmtKind::Close(_) => Some(PureViolation::Effect("close a channel")),
            StmtKind::Together(_) => Some(PureViolation::Effect("wait for tasks")),
            StmtKind::Serve(_) => Some(PureViolation::Effect("start a web server")),
            StmtKind::Reply(_) => Some(PureViolation::Effect("send an HTTP response")),
            StmtKind::Assign(assignment) if self.changes_outside(&assignment.target) => {
                Some(PureViolation::ChangesOutside)
            }
            _ => None,
        };
        self.note(statement.span, why)
    }

    fn expression(&mut self, expression: &Expr) -> Step {
        let why = match &expression.kind {
            ExprKind::Receive { .. } => {
                Some(PureViolation::Effect("wait for a value on a channel"))
            }
            ExprKind::Start(_) => Some(PureViolation::Effect("start a task")),
            ExprKind::Select(_) => {
                Some(PureViolation::Effect("wait for something to become ready"))
            }
            ExprKind::Call { callee, .. } => self.impure_call(callee),
            ExprKind::Member { .. } => self.impure_member(expression),
            _ => None,
        };
        self.note(expression.span, why)
    }
}

impl ImpureFinder<'_> {
    fn note(&mut self, span: Span, why: Option<PureViolation>) -> Step {
        match why {
            Some(why) => {
                self.found.push((span, why));
                Step::Over
            }
            None => Step::Into,
        }
    }

    fn changes_outside(&self, target: &Expr) -> bool {
        let ExprKind::Name(_) = &target.kind else { return false };
        let Some(Resolution::Local(found)) = self.checked.resolution(target.id) else {
            return false;
        };
        found.local.index() < self.locals_begin
    }

    fn impure_member(&self, expression: &Expr) -> Option<PureViolation> {
        match self.checked.resolution(expression.id) {
            Some(Resolution::BuiltinMethod("value")) => {
                Some(PureViolation::Effect("read a `shared` value"))
            }
            _ => None,
        }
    }

    fn impure_call(&self, callee: &Expr) -> Option<PureViolation> {
        if let ExprKind::Name(_) = &callee.kind {
            if let Some(Resolution::Local(found)) = self.checked.resolution(callee.id) {
                if let Some(local) = self.checked.local(found.local) {
                    let declared = &local.declared;
                    if matches!(declared, Type::Function(_)) {
                        return Some(PureViolation::Effect("call a function held in a value"));
                    }
                }
            }
        }

        match self.checked.resolution(callee.id) {
            Some(Resolution::Builtin("print")) => Some(PureViolation::Effect("call `print`")),
            Some(Resolution::Builtin("read_file")) => {
                Some(PureViolation::Effect("call `read_file`"))
            }
            Some(Resolution::Builtin(_)) => None,
            Some(Resolution::BuiltinMethod("wait")) => {
                Some(PureViolation::Effect("wait for a task"))
            }
            Some(Resolution::BuiltinMethod("update")) => {
                Some(PureViolation::Effect("change a `shared` value"))
            }
            Some(Resolution::BuiltinMethod("value")) => {
                Some(PureViolation::Effect("read a `shared` value"))
            }
            Some(Resolution::NewChannel) => Some(PureViolation::Effect("build a channel")),
            Some(Resolution::NewShared) => Some(PureViolation::Effect("build a `shared` value")),
            Some(Resolution::NewDb) => Some(PureViolation::Effect("connect to a database")),
            Some(Resolution::NewStore) => Some(PureViolation::Effect("open a store")),
            Some(Resolution::BuiltinMethod("env_get" | "env_required")) => {
                Some(PureViolation::Effect("read the environment"))
            }
            Some(Resolution::BuiltinMethod("http_get" | "http_post" | "http_send")) => {
                Some(PureViolation::Effect("make an HTTP request"))
            }
            Some(Resolution::BuiltinMethod("execute" | "query")) => {
                Some(PureViolation::Effect("talk to a database"))
            }
            Some(Resolution::BuiltinMethod("store_get" | "store_set" | "store_remove" | "store_keys")) => {
                Some(PureViolation::Effect("talk to a store"))
            }
            Some(Resolution::RequestWho) => Some(PureViolation::Effect("read who the caller is")),
            Some(Resolution::Function(id)) => self.impure_declared(*id),
            Some(Resolution::Method { function, .. }) => self.impure_declared(*function),
            Some(Resolution::UserNew { function, .. }) => self.impure_declared(*function),
            // Everything else that resolves to a call is either pure by nature or
            // checked elsewhere. Only calls through a value with no declaration
            // remain suspicious.
            Some(
                Resolution::BuiltinMethod(_)
                | Resolution::AutomaticNew(_)
                | Resolution::Raw(_)
                | Resolution::With(_)
                | Resolution::Variant { .. }
                | Resolution::AbilityMethod { .. }
                | Resolution::Field { .. }
                | Resolution::Local(_)
                | Resolution::SelfValue
                | Resolution::RequestField(_),
            ) => None,
            None => self.impure_value_call(callee),
        }
    }

    fn impure_declared(&self, id: FunctionId) -> Option<PureViolation> {
        let function = self.checked.function(id)?;
        if function.pure {
            return None;
        }
        Some(PureViolation::Calls(function.name.clone()))
    }

    /// A call through a value has no declaration to ask whether it is `pure`.
    fn impure_value_call(&self, callee: &Expr) -> Option<PureViolation> {
        if matches!(callee.kind, ExprKind::Name(_)) {
            return None;
        }
        let found = self.checked.type_of(callee.id)?;
        if matches!(found, Type::Function(_)) {
            return Some(PureViolation::Effect("call a function held in a value"));
        }
        None
    }
}
