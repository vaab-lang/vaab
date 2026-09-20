//! Expressions.
//!
//! Most of this is a straight reading of the tree: the checker has already said
//! what each name means and how each call lines up, so there is one instruction
//! after another and very little to decide.
//!
//! Two shapes are worth reading in full. `.map` and `.each` are written out as
//! loops rather than called as native functions, because they call a closure and
//! a Vaab call has to be a frame on the machine rather than a Rust one. And `try`
//! is a test and a `Return`, which is what makes the happy path stay flat without
//! anything resembling an exception.

use vaab_syntax::ast::{
    Argument, ArmBody, ArmPattern, BinaryOp, Expr, ExprKind, FunctionBody, IfExpr, MatchExpr,
    TextPart, UnaryOp,
};
use vaab_syntax::span::Span;
use vaab_types::{ArgumentSource, Resolution, Type, TypeId};

use super::{Builder, Compiler};
use crate::builtin::Builtin;
use crate::bytecode::Op;
use crate::error::Feature;
use crate::value::{Ref, Value};

/// Where the value of a parameter a call did not mention comes from.
#[derive(Clone, Copy)]
enum Defaults<'a> {
    /// Nothing has a default: a builtin, or a choice variant.
    None,
    Parameters(&'a [vaab_syntax::ast::Parameter]),
    /// `Account.new(...)`, whose fields may have been given defaults.
    Fields(&'a [vaab_syntax::ast::Field]),
}

impl<'a> Compiler<'a> {
    pub(super) fn expression(&mut self, expression: &'a Expr) {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Int(number) => self.emit(Op::Int(*number), span),
            ExprKind::Float(number) => self.push_constant(Value::Float(*number), span),
            ExprKind::Bool(held) => self.emit(Op::Bool(*held), span),
            ExprKind::Text(parts) => self.text(parts, span),
            ExprKind::Nothing => self.emit(Op::Absent, span),

            ExprKind::Name(_) => self.name(expression, span),
            ExprKind::SelfValue => self.emit(Op::LoadSelf, span),

            ExprKind::List(items) => {
                for item in items {
                    self.expression(item);
                }
                self.emit(Op::List(items.len() as u32), span);
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.expression(&entry.key);
                    self.expression(&entry.value);
                }
                self.emit(Op::Map(entries.len() as u32), span);
            }
            ExprKind::Tuple(parts) => {
                for part in parts {
                    self.expression(part);
                }
                self.emit(Op::Tuple(parts.len() as u32), span);
            }
            ExprKind::Range { start, end } => {
                self.expression(start);
                self.expression(end);
                self.emit(Op::Range, span);
            }

            ExprKind::Unary { operator, operand } => {
                self.expression(operand);
                self.emit(
                    match operator {
                        UnaryOp::Not => Op::Not,
                        UnaryOp::Negate => Op::Negate,
                    },
                    span,
                );
            }
            ExprKind::Binary { operator, left, right } => {
                self.binary(*operator, left, right, span)
            }

            ExprKind::Call { callee, arguments } => self.call(expression, callee, arguments),
            ExprKind::Member { target, .. } => self.member(expression, target, span),
            ExprKind::Index { target, index } => {
                self.expression(target);
                self.expression(index);
                self.emit(Op::Index, span);
            }

            ExprKind::Closure { body, .. } => self.closure(expression, body, span),

            ExprKind::If(branch) => self.branch(branch, span),
            ExprKind::Match(matching) => self.matching(matching, span),

            ExprKind::Found(inner) => {
                self.expression(inner);
                self.emit(Op::Found, span);
            }
            ExprKind::Success(inner) => {
                self.expression(inner);
                self.emit(Op::Success, span);
            }
            ExprKind::Failure(inner) => {
                self.expression(inner);
                self.emit(Op::Failure, span);
            }

            ExprKind::Try(inner) => self.attempt(inner, span),
            ExprKind::Otherwise { value, fallback } => self.otherwise(value, fallback, span),

            ExprKind::Receive { .. } => self.emit(Op::NotYet(Feature::Channels), span),
            ExprKind::Start(_) => self.emit(Op::NotYet(Feature::Tasks), span),
            ExprKind::Select(_) => self.emit(Op::NotYet(Feature::Select), span),
        }
    }

    // -----------------------------------------------------------------------
    // Simple shapes
    // -----------------------------------------------------------------------

    fn text(&mut self, parts: &'a [TextPart], span: Span) {
        match parts {
            [] => self.push_constant(Value::text(""), span),
            // Text with no holes is one piece already; there is nothing to join.
            [TextPart::Literal(only)] => self.push_constant(Value::text(only), span),
            parts => {
                for part in parts {
                    match part {
                        TextPart::Literal(run) => self.push_constant(Value::text(run), span),
                        TextPart::Interpolation(hole) => self.expression(hole),
                    }
                }
                self.emit(Op::Text(parts.len() as u32), span);
            }
        }
    }

    fn name(&mut self, expression: &'a Expr, span: Span) {
        match self.checked.resolution(expression.id).cloned() {
            Some(Resolution::Local(reference)) => {
                let place = self.place_of(reference.local);
                self.load(place, span);
            }
            Some(Resolution::Function(id)) => {
                let body = self.body_of.get(&id).copied();
                let frame = self.checked.function(id).map(|function| function.frame);
                match (body, frame) {
                    (Some(body), Some(frame)) => self.push_function(body, frame, span),
                    _ => self.emit(Op::Nothing, span),
                }
            }
            Some(Resolution::Builtin("read_file")) => {
                self.emit(Op::NotYet(Feature::ReadFile), span)
            }
            Some(Resolution::Builtin("print")) => {
                self.push_constant(Value::Builtin(Builtin::Print), span)
            }
            // Every other resolution is something a name cannot be on its own, and
            // the checker has already said so.
            _ => self.emit(Op::Nothing, span),
        }
    }

    fn binary(&mut self, operator: BinaryOp, left: &'a Expr, right: &'a Expr, span: Span) {
        // `and` and `or` decide whether to look at their right-hand side at all,
        // so they are jumps rather than instructions.
        if matches!(operator, BinaryOp::And | BinaryOp::Or) {
            self.expression(left);
            self.emit(Op::Duplicate, span);
            let settled = match operator {
                BinaryOp::And => self.jump(Op::JumpIfFalse(0), span),
                _ => self.jump(Op::JumpIfTrue(0), span),
            };
            self.emit(Op::Pop, span);
            self.expression(right);
            self.land(settled);
            return;
        }

        self.expression(left);
        self.expression(right);
        let op = match operator {
            BinaryOp::Add => Op::Add,
            BinaryOp::Subtract => Op::Subtract,
            BinaryOp::Multiply => Op::Multiply,
            BinaryOp::Divide => Op::Divide,
            BinaryOp::Remainder => Op::Remainder,
            BinaryOp::Equals => Op::Equal,
            BinaryOp::NotEquals => Op::NotEqual,
            BinaryOp::Less => Op::Less,
            BinaryOp::LessOrEqual => Op::LessOrEqual,
            BinaryOp::Greater => Op::Greater,
            BinaryOp::GreaterOrEqual => Op::GreaterOrEqual,
            BinaryOp::And | BinaryOp::Or => Op::Nothing,
        };
        self.emit(op, span);
    }

    // -----------------------------------------------------------------------
    // Reaching into a value
    // -----------------------------------------------------------------------

    fn member(&mut self, expression: &'a Expr, target: &'a Expr, span: Span) {
        match self.checked.resolution(expression.id).cloned() {
            Some(Resolution::Field { field, .. }) => {
                self.expression(target);
                self.emit(Op::Field(field as u32), span);
            }
            Some(Resolution::Variant { choice, variant }) => {
                let layout = self.variant_of.get(&(choice.index(), variant)).copied();
                match layout {
                    Some(layout) => self.emit(Op::Variant { layout, fields: 0 }, span),
                    None => self.emit(Op::Nothing, span),
                }
            }
            Some(Resolution::BuiltinMethod(name)) => {
                // Only a property is read rather than called, so this is one of the
                // handful that are.
                let op = match name {
                    "is_empty" => Op::IsEmpty,
                    "count" | "length" => Op::Length,
                    "first" => Op::Builtin(Builtin::First),
                    "keys" => Op::Builtin(Builtin::Keys),
                    "value" => Op::NotYet(Feature::SharedState),
                    _ => Op::Nothing,
                };
                if matches!(op, Op::NotYet(_)) {
                    self.emit(op, span);
                    return;
                }
                self.expression(target);
                self.emit(op, span);
            }
            _ => self.emit(Op::Nothing, span),
        }
    }

    // -----------------------------------------------------------------------
    // Calls
    // -----------------------------------------------------------------------

    fn call(&mut self, call: &'a Expr, callee: &'a Expr, arguments: &'a [Argument]) {
        let span = call.span;
        let target = match &callee.kind {
            ExprKind::Member { target, .. } => Some(target.as_ref()),
            _ => None,
        };

        match self.checked.resolution(callee.id).cloned() {
            Some(Resolution::Builtin("print")) => {
                let count = self.push_arguments(call, arguments, Defaults::None);
                let _ = count;
                self.emit(Op::Builtin(Builtin::Print), span);
            }
            Some(Resolution::Builtin("read_file")) => {
                self.emit(Op::NotYet(Feature::ReadFile), span)
            }

            Some(Resolution::Function(id)) | Some(Resolution::UserNew { function: id, .. }) => {
                let body = self.body_of.get(&id).copied();
                let frame = self.checked.function(id).map(|function| function.frame);
                let (Some(body), Some(frame)) = (body, frame) else {
                    return self.emit(Op::Nothing, span);
                };
                self.push_function(body, frame, span);
                let defaults = self.parameters_of(id);
                let count = self.push_arguments(call, arguments, defaults);
                self.emit(Op::Call(count), span);
            }

            Some(Resolution::Method { function, .. }) => {
                let Some(body) = self.body_of.get(&function).copied() else {
                    return self.emit(Op::Nothing, span);
                };
                self.receiver(target, span);
                let defaults = self.parameters_of(function);
                let arity = self.push_arguments(call, arguments, defaults);
                self.emit(Op::CallMethod { body: body as u32, arity }, span);
            }

            Some(Resolution::AbilityMethod { ability, function }) => {
                self.receiver(target, span);
                let arity = self.push_arguments(call, arguments, Defaults::None);
                self.emit(
                    Op::CallAbility { ability: ability.0, slot: function as u32, arity },
                    span,
                );
            }

            Some(Resolution::BuiltinMethod(name)) => {
                self.builtin_method(call, target, name, arguments, span)
            }

            Some(Resolution::AutomaticNew(declared)) | Some(Resolution::Raw(declared)) => {
                let defaults = match self.fields_of(declared) {
                    Some(fields) => Defaults::Fields(fields),
                    None => Defaults::None,
                };
                let fields = self.push_arguments(call, arguments, defaults);
                self.emit(Op::Record { layout: declared.0, fields }, span);
            }

            Some(Resolution::With(declared)) => self.with(call, target, arguments, declared, span),

            Some(Resolution::Variant { choice, variant }) => {
                let fields = self.push_arguments(call, arguments, Defaults::None);
                match self.variant_of.get(&(choice.index(), variant)).copied() {
                    Some(layout) => self.emit(Op::Variant { layout, fields }, span),
                    None => self.emit(Op::Nothing, span),
                }
            }

            Some(Resolution::NewChannel) => self.emit(Op::NotYet(Feature::Channels), span),
            Some(Resolution::NewShared) => self.emit(Op::NotYet(Feature::SharedState), span),

            // Anything else is a value that holds a function: a parameter typed
            // `to(Int) returns Int`, a field, a local given a closure.
            _ => {
                self.expression(callee);
                let count = self.push_arguments(call, arguments, Defaults::None);
                self.emit(Op::Call(count), span);
            }
        }
    }

    /// The value to the left of the dot in a method call.
    fn receiver(&mut self, target: Option<&'a Expr>, span: Span) {
        match target {
            Some(target) => self.expression(target),
            // A method call always has something to the left of its dot; without
            // one the checker would not have resolved it.
            None => self.emit(Op::Nothing, span),
        }
    }

    fn parameters_of(&self, id: vaab_types::FunctionId) -> Defaults<'a> {
        match self.declaration_of(id) {
            Some(declaration) => Defaults::Parameters(&declaration.parameters),
            None => Defaults::None,
        }
    }

    /// Pushes one value per parameter, in the order the callee wants them.
    ///
    /// The checker worked out where each one comes from, so an argument written
    /// out of order or by name is already in its place here.
    fn push_arguments(
        &mut self,
        call: &'a Expr,
        arguments: &'a [Argument],
        defaults: Defaults<'a>,
    ) -> u32 {
        let Some(sources) =
            self.checked.calls.get(&call.id).map(|wanted| wanted.arguments.clone())
        else {
            for argument in arguments {
                self.expression(&argument.value);
            }
            return arguments.len() as u32;
        };

        for (slot, source) in sources.iter().enumerate() {
            match source {
                ArgumentSource::Given(position) => match arguments.get(*position) {
                    Some(argument) => self.expression(&argument.value),
                    None => self.emit(Op::Nothing, call.span),
                },
                ArgumentSource::Default | ArgumentSource::Kept => {
                    self.default(defaults, slot, call.span)
                }
            }
        }
        sources.len() as u32
    }

    /// The expression written as a parameter's or a field's default.
    fn default(&mut self, defaults: Defaults<'a>, slot: usize, span: Span) {
        let written = match defaults {
            Defaults::Parameters(parameters) => {
                parameters.get(slot).and_then(|parameter| parameter.default.as_ref())
            }
            Defaults::Fields(fields) => fields.get(slot).and_then(|field| field.default.as_ref()),
            Defaults::None => None,
        };
        match written {
            Some(written) => self.expression(written),
            None => self.emit(Op::Nothing, span),
        }
    }

    /// `account.with(balance: 5)`: a copy, with the fields not mentioned kept.
    fn with(
        &mut self,
        call: &'a Expr,
        target: Option<&'a Expr>,
        arguments: &'a [Argument],
        declared: TypeId,
        span: Span,
    ) {
        let original = self.temporary();
        self.receiver(target, span);
        self.emit(Op::StoreLocal(original), span);

        let sources = self
            .checked
            .calls
            .get(&call.id)
            .map(|wanted| wanted.arguments.clone())
            .unwrap_or_default();

        for (slot, source) in sources.iter().enumerate() {
            match source {
                ArgumentSource::Given(position) => match arguments.get(*position) {
                    Some(argument) => self.expression(&argument.value),
                    None => self.emit(Op::Nothing, span),
                },
                _ => {
                    self.emit(Op::LoadLocal(original), span);
                    self.emit(Op::Field(slot as u32), span);
                }
            }
        }

        self.emit(Op::Record { layout: declared.0, fields: sources.len() as u32 }, span);
    }

    // -----------------------------------------------------------------------
    // Builtin methods
    // -----------------------------------------------------------------------

    fn builtin_method(
        &mut self,
        call: &'a Expr,
        target: Option<&'a Expr>,
        name: &str,
        arguments: &'a [Argument],
        span: Span,
    ) {
        match name {
            "map" => return self.walk(target, arguments, span, Gathering::Into),
            "each" => return self.walk(target, arguments, span, Gathering::Nothing),
            "wait" => return self.emit(Op::NotYet(Feature::Tasks), span),
            "update" => return self.emit(Op::NotYet(Feature::SharedState), span),
            _ => {}
        }

        let builtin = match name {
            "upper" => Builtin::Upper,
            "lower" => Builtin::Lower,
            "contains" => Builtin::Contains,
            "join" => Builtin::Join,
            "get" => Builtin::Get,
            "abs" => Builtin::Abs,
            "min" => Builtin::Min,
            "max" => Builtin::Max,
            "to_float" => Builtin::ToFloat,
            "round" => Builtin::Round,
            // A name in the checker's prelude and not in this list would be a
            // method with nothing behind it. `tests/library.rs` calls every one.
            _ => return self.emit(Op::Nothing, span),
        };

        self.receiver(target, span);
        self.push_arguments(call, arguments, Defaults::None);
        self.emit(Op::Builtin(builtin), span);
    }

    /// `.map` and `.each`, written out as a loop over the receiver.
    fn walk(
        &mut self,
        target: Option<&'a Expr>,
        arguments: &'a [Argument],
        span: Span,
        gathering: Gathering,
    ) {
        let items = self.temporary();
        let change = self.temporary();
        let position = self.temporary();
        let gathered = self.temporary();

        self.receiver(target, span);
        self.emit(Op::StoreLocal(items), span);

        match arguments.first() {
            Some(argument) => self.expression(&argument.value),
            None => self.emit(Op::Nothing, span),
        }
        self.emit(Op::StoreLocal(change), span);

        self.emit(Op::Int(0), span);
        self.emit(Op::StoreLocal(position), span);
        if gathering == Gathering::Into {
            self.emit(Op::List(0), span);
            self.emit(Op::StoreLocal(gathered), span);
        }

        let start = self.here();
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::LoadLocal(items), span);
        self.emit(Op::Length, span);
        self.emit(Op::Less, span);
        let done = self.jump(Op::JumpIfFalse(0), span);

        self.emit(Op::LoadLocal(change), span);
        self.emit(Op::LoadLocal(items), span);
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::Index, span);
        self.emit(Op::Call(1), span);

        match gathering {
            Gathering::Into => self.emit(Op::Append(gathered), span),
            Gathering::Nothing => self.emit(Op::Pop, span),
        }

        self.advance(position, span);
        self.emit(Op::Jump(start), span);
        self.land(done);

        match gathering {
            Gathering::Into => self.emit(Op::LoadLocal(gathered), span),
            Gathering::Nothing => self.emit(Op::Nothing, span),
        }
    }

    // -----------------------------------------------------------------------
    // Closures
    // -----------------------------------------------------------------------

    fn closure(&mut self, expression: &'a Expr, body: &'a FunctionBody, span: Span) {
        let Some(closure) = self.checked.closures.get(&expression.id).cloned() else {
            return self.emit(Op::Nothing, span);
        };

        let index = self.builders.len();
        self.builders.push(Builder::new(Ref::from(CLOSURE_NAME), closure.frame));

        // `pairs.map((a, b) -> a + b)` is handed one pair and names both halves, so
        // it takes one argument and pulls the rest of its slots out of it.
        let arity = if closure.unpacks { 1 } else { closure.parameters.len() };
        self.open_body(index, closure.frame, arity);

        if closure.unpacks {
            let pair = self.temporary();
            self.emit(Op::LoadLocal(0), span);
            self.emit(Op::StoreLocal(pair), span);
            for (position, local) in closure.parameters.iter().enumerate() {
                let Some(slot) = self.checked.local(*local).map(|local| local.slot as u32) else {
                    continue;
                };
                self.emit(Op::LoadLocal(pair), span);
                self.emit(Op::TupleItem(position as u32), span);
                self.emit(Op::StoreLocal(slot), span);
            }
        }

        for local in &closure.parameters {
            self.box_if_shared(*local, span);
        }

        match body {
            FunctionBody::Expr(inner) => self.expression(inner),
            FunctionBody::Block(block) => self.block_value(block),
        }
        self.emit(Op::Return, span);
        self.open.pop();

        self.push_function(index, closure.frame, span);
    }

    // -----------------------------------------------------------------------
    // Branching
    // -----------------------------------------------------------------------

    fn branch(&mut self, branch: &'a IfExpr, span: Span) {
        use vaab_syntax::ast::ElseBranch;

        self.expression(&branch.condition);
        let otherwise = self.jump(Op::JumpIfFalse(0), span);
        self.block_value(&branch.then_block);
        let end = self.jump(Op::Jump(0), span);

        self.land(otherwise);
        match &branch.else_branch {
            Some(ElseBranch::Block(block)) => self.block_value(block),
            Some(ElseBranch::If(nested)) => self.branch(nested, span),
            // An `if` with no `else` is `Nothing` when the condition is false.
            None => self.emit(Op::Nothing, span),
        }
        self.land(end);
    }

    fn matching(&mut self, matching: &'a MatchExpr, span: Span) {
        let subject = self.temporary();
        self.expression(&matching.subject);
        self.emit(Op::StoreLocal(subject), span);

        let mut ends = Vec::new();
        for arm in &matching.arms {
            let mut failed = Vec::new();
            if let ArmPattern::Pattern(pattern) = &arm.pattern {
                self.take_apart(pattern, subject, &mut failed);
            }
            // The guard is checked after the pattern has bound its names, because
            // `when n if n < 0` is about the name the pattern just introduced.
            if let Some(guard) = &arm.guard {
                self.expression(guard);
                failed.push(self.jump(Op::JumpIfFalse(0), arm.span));
            }

            match &arm.body {
                ArmBody::Expr(inner) => self.expression(inner),
                ArmBody::Block(block) => self.block_value(block),
            }
            ends.push(self.jump(Op::Jump(0), arm.span));
            self.land_all(&failed);
        }

        // A `match` that checked always has an arm that applies, so falling off the
        // end means the machine and the checker disagree.
        self.emit(Op::NoArm, span);
        self.land_all(&ends);
    }

    // -----------------------------------------------------------------------
    // Failures
    // -----------------------------------------------------------------------

    fn attempt(&mut self, inner: &'a Expr, span: Span) {
        self.expression(inner);
        self.emit(Op::Duplicate, span);
        self.emit(Op::IsSuccess, span);
        let worked = self.jump(Op::JumpIfTrue(0), span);
        // The failure is the same failure the caller declared, so it goes back
        // untouched. That is the whole of what `try` does.
        self.emit(Op::Return, span);
        self.land(worked);
        self.emit(Op::Unwrap, span);
    }

    fn otherwise(&mut self, value: &'a Expr, fallback: &'a Expr, span: Span) {
        let present = match self.checked.type_of(value.id) {
            Some(Type::Fallible { .. }) => Op::IsSuccess,
            _ => Op::IsFound,
        };

        self.expression(value);
        self.emit(Op::Duplicate, span);
        self.emit(present, span);
        let missing = self.jump(Op::JumpIfFalse(0), span);
        self.emit(Op::Unwrap, span);
        let end = self.jump(Op::Jump(0), span);

        self.land(missing);
        self.emit(Op::Pop, span);
        self.expression(fallback);
        self.land(end);
    }
}

/// Whether a walk over a list keeps what its closure gave back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gathering {
    /// `.map`, which builds a list.
    Into,
    /// `.each`, which is run for what it does.
    Nothing,
}

/// What a trace calls a closure, which has no name of its own.
const CLOSURE_NAME: &str = "a closure";
