//! Statements, blocks and the bodies of declarations.

use vaab_syntax::ast::{
    Block, ElseBranch, Expr, ExprKind, FunctionBody, FunctionDecl, IfExpr, Stmt, StmtKind, TypeDecl,
};
use vaab_syntax::span::Span;

use super::{Checker, Enclosing, Global, Wanted};
use crate::checked::{FunctionId, Resolution};
use crate::messages;
use crate::types::Type;

/// What a block evaluates to, and where that value was written.
pub(super) struct BlockValue {
    pub declared: Type,
    /// `None` when the block does not end in an expression, so it gives `Nothing`.
    pub span: Option<Span>,
}

impl Checker {
    pub(super) fn statement(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Let(declaration) => {
                let declared = declaration.declared_type.as_ref().map(|declared| {
                    let resolved = self.resolve_type(declared);
                    (resolved, declared.span)
                });

                let wanted = Wanted::of(declared.clone().map(|(resolved, _)| resolved));
                let found = self.expression(&declaration.value, wanted);

                // An annotation is what the value is, even when the value disagreed:
                // the disagreement has been reported, and carrying on with what the
                // programmer asked for keeps the rest of the file sensible.
                let declared = declared.map(|(resolved, _)| resolved).unwrap_or(found);
                let local =
                    self.declare_local(&declaration.name, declared, declaration.changing);
                self.checked.bindings.insert(statement.id, local);

                // A fixed name bound straight to a closure holds that closure and no
                // other, which is what lets a task given it be judged by what the
                // closure captured rather than refused outright.
                if !declaration.changing {
                    if let ExprKind::Closure { .. } = &declaration.value.kind {
                        self.record_closure_of_local(local, declaration.value.id);
                    }
                }
            }

            StmtKind::Assign(assignment) => self.assignment(&assignment.target, &assignment.value),

            StmtKind::Return(value) => self.return_statement(statement.span, value.as_ref()),

            StmtKind::ForEach(loop_) => {
                let sequence = self.expression(&loop_.sequence, Wanted::Anything);
                let item = self.element_of(&sequence, loop_.sequence.span);

                self.push_scope();
                self.pattern(&loop_.pattern, &item);
                self.block_statements(&loop_.body, Wanted::Discarded);
                self.pop_scope();
            }

            StmtKind::While(loop_) => {
                self.condition(&loop_.condition);
                self.block(&loop_.body, Wanted::Discarded);
            }

            StmtKind::Repeat(loop_) => {
                let count = self.expression(&loop_.count, Wanted::Anything);
                if !self.variables.fits(&count, &Type::Int) {
                    let count = self.variables.resolve(&count);
                    self.report(messages::mismatch(&Type::Int, &count, loop_.count.span));
                }
                self.block(&loop_.body, Wanted::Discarded);
            }

            StmtKind::Send(send) => {
                let channel = self.expression(&send.channel, Wanted::Anything);
                match self.variables.resolve(&channel) {
                    Type::Channel(carries) => {
                        // A channel that could not carry what it was asked for has
                        // already been told so, and carries `Unknown` from then on.
                        let explained = carries.is_unknown();
                        let found = self.expression(&send.value, Wanted::Exactly(*carries));
                        if !explained {
                            self.check_sent_value(&send.value, &found);
                        }
                    }
                    Type::Unknown => {
                        self.expression(&send.value, Wanted::Anything);
                    }
                    other => {
                        self.report(messages::not_a_channel("send", &other, send.channel.span));
                        self.expression(&send.value, Wanted::Anything);
                    }
                }
            }

            StmtKind::Close(channel) => {
                let found = self.expression(channel, Wanted::Anything);
                match self.variables.resolve(&found) {
                    Type::Channel(_) | Type::Unknown => {}
                    other => {
                        self.report(messages::not_a_channel("close", &other, channel.span));
                    }
                }
            }

            StmtKind::Together(body) => {
                self.block(body, Wanted::Discarded);
                self.check_together(body, statement.span);
            }

            StmtKind::Function(declaration) => {
                // Hoisting registered it already, so the body is all that is left.
                if let Some(id) = self.lookup_function(&declaration.name.text) {
                    self.function_body(declaration, id);
                }
            }

            StmtKind::Type(declaration) => self.type_body(declaration),

            StmtKind::Choice(declaration) => {
                if self.scopes.len() > 1 {
                    self.report(messages::nested_declaration(
                        "choice",
                        &declaration.name.text,
                        declaration.name.span,
                    ));
                }
            }

            StmtKind::Ability(declaration) => {
                if self.scopes.len() > 1 {
                    self.report(messages::nested_declaration(
                        "ability",
                        &declaration.name.text,
                        declaration.name.span,
                    ));
                }
            }

            StmtKind::Expr(expression) => {
                self.expression(expression, Wanted::Discarded);
            }
        }
    }

    /// `count = count + 1`, which is only legal for a `changing` binding.
    fn assignment(&mut self, target: &Expr, value: &Expr) {
        match &target.kind {
            ExprKind::Name(name) => {
                let Some(found) = self.lookup_local(&name.text) else {
                    let known = self.visible_names();
                    self.report(messages::undefined_name(&name.text, name.span, &known));
                    self.expression(value, Wanted::Anything);
                    return;
                };

                let Some(local) = self.checked.local(found.local) else { return };
                let declared = local.declared.clone();
                let changing = local.changing;
                let declared_at = local.span;

                if !changing {
                    self.report(messages::not_changing(&name.text, target.span, declared_at));
                }

                self.record(target, declared.clone());
                self.resolve_to(target.id, Resolution::Local(found));
                self.expression(value, Wanted::Exactly(declared));
            }

            // A `type` never changes, and neither does a list, so the only honest
            // thing to do is say where the changed copy comes from instead.
            ExprKind::Member { target: owner, name } => {
                let found = self.expression(owner, Wanted::Anything);
                let owner_name = match self.variables.resolve(&found) {
                    Type::Named(owner) => owner,
                    other => other.to_string(),
                };
                self.report(messages::cannot_assign_field(&owner_name, name.span));
                self.expression(value, Wanted::Anything);
            }

            ExprKind::Index { .. } => {
                self.expression(target, Wanted::Anything);
                self.report(messages::cannot_assign_item(target.span));
                self.expression(value, Wanted::Anything);
            }

            // The parser only builds the three shapes above as assignment targets.
            _ => {
                self.expression(target, Wanted::Anything);
                self.expression(value, Wanted::Anything);
            }
        }
    }

    fn return_statement(&mut self, span: Span, value: Option<&Expr>) {
        let Some(enclosing) = self.enclosing.last() else {
            self.report(messages::return_outside_function(span));
            if let Some(value) = value {
                self.expression(value, Wanted::Anything);
            }
            return;
        };

        let returns = enclosing.returns.clone();
        match value {
            Some(value) => {
                self.expression(value, Wanted::Exactly(returns));
            }
            None => {
                if !matches!(returns, Type::Nothing | Type::Unknown) {
                    self.report(messages::mismatch(&returns, &Type::Nothing, span));
                }
            }
        }
    }

    /// The type of one item of whatever `for each` is walking through.
    fn element_of(&mut self, sequence: &Type, span: Span) -> Type {
        match self.variables.resolve(sequence) {
            Type::List(item) | Type::Channel(item) => *item,
            // A map hands over its keys and values together, so a pattern can take
            // them apart: `for each (name, age) in ages`.
            Type::Map { key, value } => Type::Tuple(vec![*key, *value]),
            Type::Unknown => Type::Unknown,
            other => {
                self.report(messages::not_iterable(&other, span));
                Type::Unknown
            }
        }
    }

    /// Checks something that decides a branch, which in Vaab is always a `Bool`.
    pub(super) fn condition(&mut self, condition: &Expr) {
        let found = self.expression(condition, Wanted::Anything);
        if !self.variables.fits(&found, &Type::Bool) {
            let found = self.variables.resolve(&found);
            self.report(messages::condition_not_bool(&found, condition.span));
        }
    }

    // -----------------------------------------------------------------------
    // Blocks
    // -----------------------------------------------------------------------

    /// Checks a block in a scope of its own.
    pub(super) fn block(&mut self, block: &Block, wanted: Wanted) -> BlockValue {
        self.push_scope();
        let value = self.block_statements(block, wanted);
        self.pop_scope();
        value
    }

    /// The same, without opening a scope, for a body whose scope already holds
    /// something — a function's parameters, or a loop's pattern.
    pub(super) fn block_statements(&mut self, block: &Block, wanted: Wanted) -> BlockValue {
        self.hoist_functions(&block.statements);

        let last = block.statements.len().saturating_sub(1);
        let mut value = BlockValue { declared: Type::Nothing, span: None };

        for (position, statement) in block.statements.iter().enumerate() {
            // "The last expression in a block is its value", so that one — and only
            // that one — is checked against what the block is being asked for.
            if position == last {
                if let StmtKind::Expr(expression) = &statement.kind {
                    let declared = self.expression(expression, wanted.clone());
                    value = BlockValue { declared, span: Some(expression.span) };
                    continue;
                }
            }
            self.statement(statement);
        }

        value
    }

    // -----------------------------------------------------------------------
    // Function bodies
    // -----------------------------------------------------------------------

    pub(super) fn function_body(&mut self, declaration: &FunctionDecl, id: FunctionId) {
        let Some(function) = self.checked.function(id) else { return };
        let frame = function.frame;
        let name = function.name.clone();
        let signature = function.signature.clone();

        // A default belongs to the caller, not to the body, so it is checked out
        // here where it cannot see the other parameters.
        for (parameter, declared) in declaration.parameters.iter().zip(&signature.parameters) {
            if let Some(default) = &parameter.default {
                self.expression(default, Wanted::Exactly(declared.declared.clone()));
            }
        }

        let Some(body) = &declaration.body else { return };

        self.enter_frame(frame);

        let mut parameters = Vec::new();
        for (parameter, declared) in declaration.parameters.iter().zip(&signature.parameters) {
            parameters.push(self.declare_local(
                &parameter.name,
                declared.declared.clone(),
                false,
            ));
        }
        if let Some(holder) = self.checked.functions.get_mut(id.index()) {
            holder.parameters = parameters;
        }

        let signature_span = declaration
            .returns
            .as_ref()
            .map(|returns| returns.span)
            .unwrap_or(declaration.name.span);
        self.enclosing.push(Enclosing {
            name: name.clone(),
            returns: signature.returns.clone(),
            signature: signature_span,
        });

        match body {
            FunctionBody::Expr(expression) => {
                self.expression(expression, Wanted::Exactly(signature.returns.clone()));
            }
            FunctionBody::Block(block) => {
                let value = self.block_statements(block, Wanted::Anything);
                self.check_body_result(&name, &signature.returns, block, value);
            }
        }

        self.enclosing.pop();
        self.leave_frame();
    }

    /// Makes sure a block-bodied function really produces what it promised.
    fn check_body_result(
        &mut self,
        name: &str,
        returns: &Type,
        block: &Block,
        value: BlockValue,
    ) {
        if matches!(returns, Type::Nothing | Type::Unknown) {
            return;
        }
        // Every way out of the body is a `return`, so there is nothing to fall off.
        if always_returns(&block.statements) {
            return;
        }

        match value.span {
            // The body ends in an expression, which is the value it gives back.
            Some(span) if !matches!(value.declared, Type::Nothing) => {
                let found = value.declared;
                self.expect_type(found, &Wanted::Exactly(returns.clone()), span);
            }
            _ => {
                self.report(messages::missing_return(name, returns, block.span));
            }
        }
    }

    /// The fields' defaults and the methods of a `type`.
    fn type_body(&mut self, declaration: &TypeDecl) {
        if self.scopes.len() > 1 {
            self.report(messages::nested_declaration(
                "type",
                &declaration.name.text,
                declaration.name.span,
            ));
            return;
        }

        let Some(Global::Type(id)) = self.globals.get(&declaration.name.text).copied() else {
            return;
        };
        let Some(declared) = self.checked.declared_type(id) else { return };
        let fields = declared.fields.clone();
        let methods = declared.methods.clone();

        for (field, declared) in declaration.fields.iter().zip(&fields) {
            if let Some(default) = &field.default {
                self.expression(default, Wanted::Exactly(declared.declared.clone()));
            }
        }

        // Inside the body, `self` is a value of this type and `raw` is available.
        let outside = self.inside.replace(id);
        for (function, method) in declaration.functions.iter().zip(methods) {
            self.function_body(function, method);
        }
        self.inside = outside;
    }
}

/// Whether every way out of these statements is a `return`.
///
/// This is deliberately shallow: it looks at the last statement, and through an
/// `if` or a `match` whose every branch returns. A loop never counts, because
/// whether it runs at all is a question about values rather than types.
fn always_returns(statements: &[Stmt]) -> bool {
    let Some(last) = statements.last() else { return false };
    match &last.kind {
        StmtKind::Return(_) => true,
        StmtKind::Expr(expression) => expression_always_returns(expression),
        _ => false,
    }
}

fn expression_always_returns(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::If(branch) => if_always_returns(branch),
        ExprKind::Match(subject) => {
            use vaab_syntax::ast::ArmBody;
            !subject.arms.is_empty()
                && subject.arms.iter().all(|arm| match &arm.body {
                    ArmBody::Block(block) => always_returns(&block.statements),
                    ArmBody::Expr(expression) => expression_always_returns(expression),
                })
        }
        _ => false,
    }
}

fn if_always_returns(branch: &IfExpr) -> bool {
    if !always_returns(&branch.then_block.statements) {
        return false;
    }
    match &branch.else_branch {
        Some(ElseBranch::Block(block)) => always_returns(&block.statements),
        Some(ElseBranch::If(nested)) => if_always_returns(nested),
        // Without an `else` the condition may simply be false.
        None => false,
    }
}
