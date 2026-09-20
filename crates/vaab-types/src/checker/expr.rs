//! Expressions.
//!
//! [`Checker::expression`] is the one way in. It works out a type, compares it with
//! what the surrounding code wanted, and writes the answer down for phase 3.

use vaab_syntax::ast::{
    ArmBody, BinaryOp, Block, ElseBranch, Expr, ExprKind, FunctionBody, IfExpr, MapEntry,
    MatchExpr, Name, SelectArm, SelectExpr, TextPart, UnaryOp,
};
use vaab_syntax::span::Span;

use super::{Checker, Undecidable, Wanted};
use crate::checked::{Closure, FrameKind, LocalId, Resolution};
use crate::messages;
use crate::types::{Parameter, Signature, Type};

impl Checker {
    /// Checks one expression and records its type.
    pub(super) fn expression(&mut self, expression: &Expr, wanted: Wanted) -> Type {
        let found = self.expression_type(expression, &wanted);
        let found = self.expect_type(found, &wanted, expression.span);
        self.record(expression, found)
    }

    fn expression_type(&mut self, expression: &Expr, wanted: &Wanted) -> Type {
        match &expression.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,

            ExprKind::Text(parts) => {
                // Any value at all may fill a hole, because showing a value is
                // something every type can do.
                for part in parts {
                    if let TextPart::Interpolation(inner) = part {
                        self.expression(inner, Wanted::Anything);
                    }
                }
                Type::Text
            }

            // Which `maybe` a bare `nothing` is can only come from the outside.
            ExprKind::Nothing => match wanted.as_type().map(|w| self.variables.resolve(w)) {
                Some(Type::Maybe(held)) => Type::Maybe(held),
                // Which `maybe` this is may still arrive from the other arm of an
                // `if` or the other half of a `match`, so leave a hole for it.
                _ => Type::maybe(self.undecided(Undecidable::Nothing, expression.span)),
            },

            ExprKind::Name(name) => self.name_expression(expression, name),
            ExprKind::SelfValue => self.self_expression(expression.span),

            ExprKind::List(items) => self.list_literal(expression.span, items, wanted),
            ExprKind::Map(entries) => self.map_literal(expression.span, entries, wanted),
            ExprKind::Tuple(items) => self.tuple_literal(items, wanted),

            ExprKind::Range { start, end } => {
                // A range is a list of whole numbers written the short way, so it can
                // be looped over, printed and passed about like any other list.
                self.expression(start, Wanted::Exactly(Type::Int));
                self.expression(end, Wanted::Exactly(Type::Int));
                Type::list(Type::Int)
            }

            ExprKind::Unary { operator, operand } => self.unary(*operator, operand),
            ExprKind::Binary { operator, left, right } => self.binary(*operator, left, right),

            ExprKind::Call { callee, arguments } => self.call(expression, callee, arguments),
            ExprKind::Member { target, name } => self.member(expression, target, name),
            ExprKind::Index { target, index } => self.index(target, index),

            ExprKind::Closure { parameters, body } => {
                self.closure(expression, parameters, body, wanted)
            }

            ExprKind::If(branch) => self.if_expression(branch, wanted).0,
            ExprKind::Match(subject) => self.match_expression(subject, wanted),

            ExprKind::Found(inner) => {
                let held = match wanted.as_type().map(|w| self.variables.resolve(w)) {
                    Some(Type::Maybe(held)) => self.expression(inner, Wanted::Exactly(*held)),
                    _ => self.expression(inner, Wanted::Anything),
                };
                Type::maybe(held)
            }

            ExprKind::Success(inner) => match wanted.as_type().map(|w| self.variables.resolve(w)) {
                Some(Type::Fallible { ok, error }) => {
                    self.expression(inner, Wanted::Exactly(*ok.clone()));
                    Type::Fallible { ok, error }
                }
                _ => {
                    let ok = self.expression(inner, Wanted::Anything);
                    let error = self.variables.fresh();
                    Type::fallible(ok, error)
                }
            },

            ExprKind::Failure(inner) => match wanted.as_type().map(|w| self.variables.resolve(w)) {
                Some(Type::Fallible { ok, error }) => {
                    self.expression(inner, Wanted::Exactly(*error.clone()));
                    Type::Fallible { ok, error }
                }
                _ => {
                    let error = self.expression(inner, Wanted::Anything);
                    let ok = self.variables.fresh();
                    Type::fallible(ok, error)
                }
            },

            ExprKind::Try(inner) => self.try_expression(inner),

            ExprKind::Otherwise { value, fallback } => {
                let found = self.expression(value, Wanted::Anything);
                match self.variables.resolve(&found) {
                    Type::Maybe(held) => {
                        self.expression(fallback, Wanted::Exactly(*held.clone()));
                        *held
                    }
                    Type::Fallible { ok, .. } => {
                        self.expression(fallback, Wanted::Exactly(*ok.clone()));
                        *ok
                    }
                    Type::Unknown => {
                        self.expression(fallback, Wanted::Anything);
                        Type::Unknown
                    }
                    other => {
                        self.report(messages::otherwise_on_plain(&other, value.span));
                        self.expression(fallback, Wanted::Anything);
                        other
                    }
                }
            }

            ExprKind::Receive { channel } => {
                let found = self.expression(channel, Wanted::Anything);
                match self.variables.resolve(&found) {
                    // A drained channel gives nothing, which is why this is a `maybe`.
                    Type::Channel(carries) => Type::Maybe(carries),
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.report(messages::not_a_channel("receive", &other, channel.span));
                        Type::Unknown
                    }
                }
            }

            ExprKind::Start(body) => {
                let value = self.start_block(expression, body);
                Type::task(value)
            }

            ExprKind::Select(select) => self.select_expression(select, expression.span),
        }
    }

    // -----------------------------------------------------------------------
    // Names
    // -----------------------------------------------------------------------

    fn name_expression(&mut self, expression: &Expr, name: &Name) -> Type {
        if let Some(found) = self.lookup_local(&name.text) {
            self.resolve_to(expression.id, Resolution::Local(found));
            return self.local_type(found.local);
        }

        if let Some(id) = self.lookup_function(&name.text) {
            self.resolve_to(expression.id, Resolution::Function(id));
            return self
                .checked
                .function(id)
                .map(|function| function.signature.as_type())
                .unwrap_or(Type::Unknown);
        }

        if let Some(builtin) =
            self.builtin_functions.iter().find(|builtin| builtin.name == name.text)
        {
            let signature = builtin.signature.clone();
            let builtin = builtin.name;
            self.resolve_to(expression.id, Resolution::Builtin(builtin));
            return signature.as_type();
        }

        // A type is not a value: there is no `Account` to pass around, only an
        // `Account` that was built.
        if self.globals.contains_key(&name.text) {
            self.report(messages::type_used_as_value(&name.text, name.span));
            return Type::Unknown;
        }

        let known = self.visible_names();
        self.report(messages::undefined_name(&name.text, name.span, &known));
        Type::Unknown
    }

    fn self_expression(&mut self, span: Span) -> Type {
        match self.inside.and_then(|id| self.checked.declared_type(id)) {
            Some(declared) => Type::named(&declared.name),
            None => {
                self.report(messages::self_outside_type(span));
                Type::Unknown
            }
        }
    }

    // -----------------------------------------------------------------------
    // Literals that hold other values
    // -----------------------------------------------------------------------

    fn list_literal(&mut self, span: Span, items: &[Expr], wanted: &Wanted) -> Type {
        // Being told what the list holds is what lets `[Person.new(...), Planet.new(...)]`
        // be a `list of Describable` rather than two unrelated types.
        if let Some(Type::List(held)) = wanted.as_type().map(|w| self.variables.resolve(w)) {
            for item in items {
                self.expression(item, Wanted::Exactly(*held.clone()));
            }
            return Type::List(held);
        }

        let Some((first, rest)) = items.split_first() else {
            let what =
                Undecidable::Empty { what: "list", example: "let items: list of Int = []" };
            return Type::list(self.undecided(what, span));
        };

        let held = self.expression(first, Wanted::Anything);
        for item in rest {
            let found = self.expression(item, Wanted::Anything);
            if !self.variables.fits(&found, &held) {
                let found = self.variables.resolve(&found);
                let held = self.variables.resolve(&held);
                self.report(messages::mixed_list(&held, first.span, &found, item.span));
            }
        }
        Type::list(held)
    }

    fn map_literal(&mut self, span: Span, entries: &[MapEntry], wanted: &Wanted) -> Type {
        if let Some(Type::Map { key, value }) = wanted.as_type().map(|w| self.variables.resolve(w)) {
            for entry in entries {
                self.expression(&entry.key, Wanted::Exactly(*key.clone()));
                self.expression(&entry.value, Wanted::Exactly(*value.clone()));
            }
            return Type::Map { key, value };
        }

        let Some((first, rest)) = entries.split_first() else {
            // One report is enough for an empty map, so only the key waits: a map
            // that knows its keys but not its values cannot happen.
            let key = Undecidable::Empty {
                what: "map",
                example: "let ages: map of Text to Int = {}",
            };
            let key = self.undecided(key, span);
            return Type::map(key, self.variables.fresh());
        };

        let key = self.expression(&first.key, Wanted::Anything);
        let value = self.expression(&first.value, Wanted::Anything);

        for entry in rest {
            let found = self.expression(&entry.key, Wanted::Anything);
            if !self.variables.fits(&found, &key) {
                let found = self.variables.resolve(&found);
                let first_key = self.variables.resolve(&key);
                self.report(messages::mixed_map(
                    "key",
                    &first_key,
                    first.key.span,
                    &found,
                    entry.key.span,
                ));
            }

            let found = self.expression(&entry.value, Wanted::Anything);
            if !self.variables.fits(&found, &value) {
                let found = self.variables.resolve(&found);
                let first_value = self.variables.resolve(&value);
                self.report(messages::mixed_map(
                    "value",
                    &first_value,
                    first.value.span,
                    &found,
                    entry.value.span,
                ));
            }
        }

        Type::map(key, value)
    }

    fn tuple_literal(&mut self, items: &[Expr], wanted: &Wanted) -> Type {
        if let Some(Type::Tuple(held)) = wanted.as_type().map(|w| self.variables.resolve(w)) {
            if held.len() == items.len() {
                let found = items
                    .iter()
                    .zip(&held)
                    .map(|(item, held)| self.expression(item, Wanted::Exactly(held.clone())))
                    .collect();
                return Type::Tuple(found);
            }
        }

        Type::Tuple(items.iter().map(|item| self.expression(item, Wanted::Anything)).collect())
    }

    // -----------------------------------------------------------------------
    // Operators
    // -----------------------------------------------------------------------

    fn unary(&mut self, operator: UnaryOp, operand: &Expr) -> Type {
        let found = self.expression(operand, Wanted::Anything);
        let resolved = self.variables.resolve(&found);

        match operator {
            UnaryOp::Not => {
                if !self.variables.fits(&found, &Type::Bool) {
                    self.report(messages::bad_operand(
                        "not",
                        "a Bool",
                        &resolved,
                        operand.span,
                        None,
                    ));
                }
                Type::Bool
            }
            UnaryOp::Negate => match resolved {
                Type::Int | Type::Float | Type::Unknown => resolved,
                other => {
                    self.report(messages::bad_operand("-", "a number", &other, operand.span, None));
                    Type::Unknown
                }
            },
        }
    }

    fn binary(&mut self, operator: BinaryOp, left: &Expr, right: &Expr) -> Type {
        use BinaryOp::*;

        let found = self.expression(left, Wanted::Anything);
        let resolved = self.variables.resolve(&found);

        // What the operator needs of its *left* side is checked first, so that
        // `"a" + "b"` is told about text rather than about its right-hand side.
        let needs: Option<&str> = match operator {
            And | Or => (!self.variables.fits(&found, &Type::Bool)).then_some("two Bools"),
            Add | Subtract | Multiply | Divide | Remainder => {
                (!matches!(resolved, Type::Int | Type::Float | Type::Unknown))
                    .then_some("two numbers of the same type")
            }
            Less | LessOrEqual | Greater | GreaterOrEqual => {
                (!matches!(resolved, Type::Int | Type::Float | Type::Text | Type::Unknown))
                    .then_some("two numbers, or two pieces of text")
            }
            // Anything at all may be compared with itself.
            Equals | NotEquals => None,
        };

        if let Some(needs) = needs {
            let other = self.expression(right, Wanted::Anything);
            let other = self.variables.resolve(&other);
            self.report(messages::bad_operand(
                operator.spelling(),
                needs,
                &resolved,
                left.span,
                Some((right.span, &other)),
            ));
            return match operator {
                And | Or | Less | LessOrEqual | Greater | GreaterOrEqual | Equals | NotEquals => {
                    Type::Bool
                }
                _ => Type::Unknown,
            };
        }

        // Both sides of every operator in Vaab have the same type, so the left one
        // says what the right one has to be.
        self.expression(right, Wanted::Exactly(resolved.clone()));

        match operator {
            And | Or | Equals | NotEquals | Less | LessOrEqual | Greater | GreaterOrEqual => {
                Type::Bool
            }
            _ => resolved,
        }
    }

    fn index(&mut self, target: &Expr, index: &Expr) -> Type {
        let found = self.expression(target, Wanted::Anything);
        match self.variables.resolve(&found) {
            Type::List(item) => {
                let position = self.expression(index, Wanted::Anything);
                if !self.variables.fits(&position, &Type::Int) {
                    let position = self.variables.resolve(&position);
                    self.report(messages::bad_index(&position, index.span));
                }
                *item
            }
            Type::Unknown => {
                self.expression(index, Wanted::Anything);
                Type::Unknown
            }
            other => {
                self.expression(index, Wanted::Anything);
                self.report(messages::not_indexable(&other, target.span));
                Type::Unknown
            }
        }
    }

    // -----------------------------------------------------------------------
    // Closures
    // -----------------------------------------------------------------------

    fn closure(
        &mut self,
        expression: &Expr,
        parameters: &[Name],
        body: &FunctionBody,
        wanted: &Wanted,
    ) -> Type {
        let Some(Type::Function(shape)) = wanted.as_type().map(|w| self.variables.resolve(w)) else {
            // Without somewhere to be, a closure has no way to know what it takes.
            let first = parameters.first().map(|name| name.text.as_str()).unwrap_or("n");
            self.report(messages::closure_needs_context(first, expression.span));
            self.check_closure_body(expression, parameters, body, &[], &Wanted::Anything, false);
            return Type::Unknown;
        };

        let (takes, unpacks) =
            self.closure_parameter_types(expression.span, parameters, &shape.parameters);
        let signature = self.check_closure_body(
            expression,
            parameters,
            body,
            &takes,
            &Wanted::Exactly(shape.returns.clone()),
            unpacks,
        );

        // A closure that takes a pair apart is still, as a value, a function of one
        // pair — so its type comes from where it is going, not from how many names
        // it wrote.
        Type::function(shape.parameters.clone(), signature.returns)
    }

    /// Works out what each of a closure's parameters holds.
    ///
    /// `pairs.map((a, b) -> a + b)` takes one value — a pair — and names both halves,
    /// so a closure whose parameter count does not match a single tuple is allowed to
    /// take that tuple apart instead.
    fn closure_parameter_types(
        &mut self,
        span: Span,
        parameters: &[Name],
        given: &[Type],
    ) -> (Vec<Type>, bool) {
        if parameters.len() == given.len() {
            return (given.to_vec(), false);
        }

        if let [only] = given {
            if let Type::Tuple(parts) = self.variables.resolve(only) {
                if parts.len() == parameters.len() {
                    return (parts, true);
                }
            }
        }

        self.report(messages::closure_parameter_count(given.len(), parameters.len(), span));
        (vec![Type::Unknown; parameters.len()], false)
    }

    fn check_closure_body(
        &mut self,
        expression: &Expr,
        parameters: &[Name],
        body: &FunctionBody,
        takes: &[Type],
        returns: &Wanted,
        unpacks: bool,
    ) -> Signature {
        let frame = self.new_frame(FrameKind::Closure(expression.id));
        self.enter_frame(frame);

        // Everything declared from here on belongs to the closure, so it is not
        // something the closure reached for. See [`super::sendable`].
        let closure_locals_begin = self.checked.locals.len();

        let mut locals: Vec<LocalId> = Vec::new();
        let mut declared: Vec<Parameter> = Vec::new();
        for (position, name) in parameters.iter().enumerate() {
            let held = takes.get(position).cloned().unwrap_or(Type::Unknown);
            locals.push(self.declare_local(name, held.clone(), false));
            declared.push(Parameter::new(&name.text, held));
        }

        let result = match body {
            FunctionBody::Expr(inner) => self.expression(inner, returns.clone()),
            FunctionBody::Block(block) => {
                self.block_statements(block, returns.clone()).declared
            }
        };

        self.leave_frame();
        self.record_closure_captures(expression.id, body, closure_locals_begin);

        let signature = Signature::new(declared, result);
        self.checked.closures.insert(
            expression.id,
            Closure { frame, parameters: locals, signature: signature.clone(), unpacks },
        );
        signature
    }

    // -----------------------------------------------------------------------
    // Branching
    // -----------------------------------------------------------------------

    /// The type of an `if`, and where its first branch's value was written.
    pub(super) fn if_expression(&mut self, branch: &IfExpr, wanted: &Wanted) -> (Type, Span) {
        self.condition(&branch.condition);

        let then_value = self.block(&branch.then_block, wanted.clone());
        let then_span = then_value.span.unwrap_or(branch.then_block.span);

        match &branch.else_branch {
            None => {
                // With no `else` there is a way through that produces nothing, so the
                // `if` can only be used for its effect.
                let Some(wanted) = wanted.as_type() else { return (Type::Nothing, then_span) };
                if matches!(wanted, Type::Nothing | Type::Unknown) {
                    return (Type::Nothing, then_span);
                }

                let wanted = self.variables.resolve(wanted);
                self.report(messages::if_without_else(branch.then_block.span, &wanted));
                // The missing `else` is the whole of the problem, so carry on with
                // what was asked for rather than adding "found Nothing" to it.
                (wanted, then_span)
            }
            Some(ElseBranch::Block(block)) => {
                let other = self.block(block, wanted.clone());
                let other_span = other.span.unwrap_or(block.span);
                let agreed = self.agree(
                    "branches",
                    then_value.declared,
                    then_span,
                    other.declared,
                    other_span,
                    wanted,
                );
                (agreed, then_span)
            }
            Some(ElseBranch::If(nested)) => {
                let (other, other_span) = self.if_expression(nested, wanted);
                let agreed = self.agree(
                    "branches",
                    then_value.declared,
                    then_span,
                    other,
                    other_span,
                    wanted,
                );
                (agreed, then_span)
            }
        }
    }

    fn match_expression(&mut self, subject: &MatchExpr, wanted: &Wanted) -> Type {
        use vaab_syntax::ast::ArmPattern;

        let matched = self.expression(&subject.subject, Wanted::Anything);
        let matched = self.variables.resolve(&matched);

        let mut result: Option<(Type, Span)> = None;
        for arm in &subject.arms {
            self.push_scope();
            if let ArmPattern::Pattern(pattern) = &arm.pattern {
                self.pattern(pattern, &matched);
            }
            if let Some(guard) = &arm.guard {
                self.condition(guard);
            }

            let (found, span) = match &arm.body {
                ArmBody::Expr(expression) => {
                    (self.expression(expression, wanted.clone()), expression.span)
                }
                ArmBody::Block(block) => {
                    let value = self.block_statements(block, wanted.clone());
                    (value.declared, value.span.unwrap_or(block.span))
                }
            };
            self.pop_scope();

            result = Some(match result {
                None => (found, span),
                Some((first, first_span)) => {
                    let agreed =
                        self.agree("arms", first, first_span, found, span, wanted);
                    (agreed, first_span)
                }
            });
        }

        self.check_exhaustive(&matched, &subject.arms, subject.subject.span);

        result.map(|(found, _)| found).unwrap_or(Type::Nothing)
    }

    /// Settles on one type for several branches that all have to agree.
    fn agree(
        &mut self,
        what: &str,
        first: Type,
        first_span: Span,
        second: Type,
        second_span: Span,
        wanted: &Wanted,
    ) -> Type {
        // Being told what is wanted means both branches were already checked
        // against it, and each disagreement has already been reported on its own.
        if let Some(wanted) = wanted.as_type() {
            return wanted.clone();
        }
        if self.variables.fits(&second, &first) {
            return first;
        }
        if self.variables.fits(&first, &second) {
            return second;
        }
        // Nobody is going to use the value, so there is nothing to disagree about.
        if wanted.is_discarded() {
            return Type::Nothing;
        }

        let first = self.variables.resolve(&first);
        let second = self.variables.resolve(&second);
        self.report(messages::branches_differ(what, &first, first_span, &second, second_span));
        Type::Unknown
    }

    // -----------------------------------------------------------------------
    // Failures
    // -----------------------------------------------------------------------

    fn try_expression(&mut self, inner: &Expr) -> Type {
        let enclosing = self.enclosing.last().map(|enclosing| {
            (enclosing.name.clone(), enclosing.returns.clone(), enclosing.signature)
        });

        let found = self.expression(inner, Wanted::Anything);
        let found = self.variables.resolve(&found);

        let (ok, error) = match &found {
            Type::Fallible { ok, error } => ((**ok).clone(), (**error).clone()),
            Type::Unknown => return Type::Unknown,
            other => {
                self.report(messages::try_needs_fallible(other, inner.span));
                return Type::Unknown;
            }
        };

        match enclosing {
            None => {
                self.report(messages::try_outside_fallible(inner.span, None));
            }
            Some((name, Type::Fallible { error: wanted, .. }, signature)) => {
                if !self.variables.same(&error, &wanted) {
                    let error = self.variables.resolve(&error);
                    let wanted = self.variables.resolve(&wanted);
                    self.report(messages::try_error_mismatch(
                        &error,
                        &wanted,
                        inner.span,
                        signature,
                    ));
                }
                let _ = name;
            }
            Some((name, returns, _)) => {
                if !returns.is_unknown() {
                    self.report(messages::try_outside_fallible(
                        inner.span,
                        Some((&name, &returns)),
                    ));
                }
            }
        }

        ok
    }

    // -----------------------------------------------------------------------
    // Tasks and channels
    // -----------------------------------------------------------------------

    /// The value a `start { ... }` block produces, which becomes the task's result.
    ///
    /// A task runs on its own, so once the body has a type the sendability rules
    /// decide whether everything it reaches for outside itself may go with it.
    fn start_block(&mut self, expression: &Expr, body: &Block) -> Type {
        // Everything declared from here on belongs to the task, so it is not
        // something the task reached for. See [`super::sendable`].
        let task_locals_begin = self.checked.locals.len();

        let value = self.block(body, Wanted::Anything).declared;
        self.check_task(expression.id, body, expression.span, &value, task_locals_begin);
        value
    }

    /// `select` is checked for what its arms do, not for a value: it is a way of
    /// waiting, and each arm goes its own way.
    fn select_expression(&mut self, select: &SelectExpr, span: Span) -> Type {
        self.check_select_has_arms(select.arms.len(), span);

        for arm in &select.arms {
            match arm {
                SelectArm::Receive { channel, binding, body, .. } => {
                    let found = self.expression(channel, Wanted::Anything);
                    let carries = match self.variables.resolve(&found) {
                        // An arm only runs when something arrived, so the value is
                        // there: no `maybe` about it.
                        Type::Channel(carries) => *carries,
                        Type::Unknown => Type::Unknown,
                        other => {
                            self.report(messages::not_a_channel(
                                "receive",
                                &other,
                                channel.span,
                            ));
                            Type::Unknown
                        }
                    };

                    self.push_scope();
                    if let Some(name) = binding {
                        self.declare_local(name, carries, false);
                    }
                    self.block_statements(body, Wanted::Discarded);
                    self.pop_scope();
                }
                SelectArm::Timeout { amount, unit, body, .. } => {
                    self.expression(amount, Wanted::Exactly(Type::Int));
                    self.check_time_unit(unit);
                    self.block(body, Wanted::Discarded);
                }
            }
        }

        if let Some(otherwise) = &select.otherwise {
            self.block(otherwise, Wanted::Discarded);
        }

        Type::Nothing
    }
}
