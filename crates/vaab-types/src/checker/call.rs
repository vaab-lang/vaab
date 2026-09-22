//! Calls, and matching what was passed to what was asked for.
//!
//! Named arguments and defaults need to know *which* function is being called, so
//! they work whenever the callee names one: a function, a method, a constructor, a
//! variant. A function held in a value has only its type to go on, so its arguments
//! are positional. That is the whole rule.

use vaab_syntax::ast::{Argument, Expr, ExprKind, Name};
use vaab_syntax::span::Span;

use super::member::{CallShape, Handle, Member, WhenMissing};
use super::{Checker, Global, Wanted};
use crate::checked::{ArgumentSource, Call, Resolution};
use crate::json::can_json;
use crate::messages;
use crate::types::{FunctionType, Parameter, Signature, Type};

impl Checker {
    pub(super) fn call(&mut self, call: &Expr, callee: &Expr, arguments: &[Argument]) -> Type {
        // `account.deposit(25)`, `Account.new(owner: "Ada")`, `numbers.map(...)`.
        if let ExprKind::Member { target, name } = &callee.kind {
            return self.member_call(call, callee, target, name, arguments);
        }

        // `greet("Ada", greeting: "hi")`: a name standing for a declaration rather
        // than for a value.
        if let ExprKind::Name(name) = &callee.kind {
            if self.lookup_local(&name.text).is_none() {
                if let Some(called) = self.call_by_name(callee, name) {
                    let (signature, shape) = called;
                    let (returns, ready) = self.apply(call, &signature, &shape, arguments);
                    self.checked.types.insert(callee.id, ready.as_type());
                    return returns;
                }

                // Brackets after a bare type name is how most languages build a
                // value, so say where Vaab keeps that instead of "not callable".
                if matches!(self.globals.get(&name.text), Some(Global::Type(_))) {
                    self.report(messages::type_used_as_value(&name.text, name.span));
                    self.walk_arguments(arguments);
                    return Type::Unknown;
                }
            }
        }

        // Anything else has to be a value holding a function.
        let found = self.expression(callee, Wanted::Anything);
        match self.variables.resolve(&found) {
            Type::Function(held) => {
                let signature = positional(&held);
                let shape = CallShape::value(describe_callee(callee));
                self.apply(call, &signature, &shape, arguments).0
            }
            Type::Unknown => {
                self.walk_arguments(arguments);
                Type::Unknown
            }
            other => {
                self.report(messages::not_callable(&other, callee.span));
                self.walk_arguments(arguments);
                Type::Unknown
            }
        }
    }

    /// A plain name in front of brackets: a function in this file, or one from the
    /// prelude. Records what the name refers to and hands back how to call it.
    fn call_by_name(&mut self, callee: &Expr, name: &Name) -> Option<(Signature, CallShape)> {
        if let Some(id) = self.lookup_function(&name.text) {
            let function = self.checked.function(id)?;
            let signature = function.signature.clone();
            let shape = CallShape::function(&name.text, Some(function.span));
            self.resolve_to(callee.id, Resolution::Function(id));
            return Some((signature, shape));
        }

        let builtin = self.builtin_functions.iter().find(|builtin| builtin.name == name.text)?;
        let signature = builtin.signature.clone();
        let shape = CallShape::function(builtin.name, None);
        self.resolve_to(callee.id, Resolution::Builtin(builtin.name));
        Some((signature, shape))
    }

    fn member_call(
        &mut self,
        call: &Expr,
        callee: &Expr,
        target: &Expr,
        name: &Name,
        arguments: &[Argument],
    ) -> Type {
        match self.look_up_member(target, name) {
            Member::Callable { signature, resolution, shape } => {
                // `.update` is the only built-in whose argument has rules of its
                // own, and it is checked once the argument's own `.`s are resolved.
                let updating = resolution == Resolution::BuiltinMethod("update");
                self.resolve_to(callee.id, resolution);
                let (returns, ready) = self.apply(call, &signature, &shape, arguments);
                self.checked.types.insert(callee.id, ready.as_type());
                if updating {
                    self.check_update_change(arguments);
                }
                returns
            }

            // A field holding a function may be called like any other.
            Member::Value { declared, resolution } => {
                self.resolve_to(callee.id, resolution);
                let declared = self.record(callee, declared);
                match self.variables.resolve(&declared) {
                    Type::Function(held) => {
                        let signature = positional(&held);
                        let shape = CallShape::value(name.text.clone());
                        self.apply(call, &signature, &shape, arguments).0
                    }
                    Type::Unknown => {
                        self.walk_arguments(arguments);
                        Type::Unknown
                    }
                    other => {
                        self.report(messages::not_callable(&other, callee.span));
                        self.walk_arguments(arguments);
                        Type::Unknown
                    }
                }
            }

            Member::Handle(handle) => self.handle_call(call, callee, handle, arguments),

            Member::Unknown => {
                self.walk_arguments(arguments);
                Type::Unknown
            }
        }
    }

    /// `Channel.new(of: Text, size: 4)` and `Shared.new(0)`.
    ///
    /// `Channel.new` is the one call whose argument names a *type* rather than giving
    /// a value, so it cannot go through the ordinary machinery. Phase 4 is where
    /// channels gain their real rules; this is enough to give them a type.
    fn handle_call(
        &mut self,
        call: &Expr,
        callee: &Expr,
        handle: Handle,
        arguments: &[Argument],
    ) -> Type {
        let mut sources = Vec::new();
        let built = match handle {
            Handle::Channel => {
                self.resolve_to(callee.id, Resolution::NewChannel);
                let mut carries = None;
                for (position, argument) in arguments.iter().enumerate() {
                    let label = argument.name.as_ref().map(|name| name.text.as_str());
                    match label {
                        Some("size") => {
                            self.expression(&argument.value, Wanted::Exactly(Type::Int));
                        }
                        _ if carries.is_none() => {
                            carries =
                                Some((self.type_argument(&argument.value), argument.value.span));
                        }
                        _ => {
                            self.expression(&argument.value, Wanted::Anything);
                        }
                    }
                    sources.push(ArgumentSource::Given(position));
                }
                match carries {
                    // A channel is how values cross between tasks, so what it
                    // carries has to be able to make the journey.
                    Some((carries, span)) => {
                        Type::channel(self.check_channel_type(carries, span))
                    }
                    None => {
                        self.report(messages::needs_a_type_name(call.span));
                        Type::channel(Type::Unknown)
                    }
                }
            }

            Handle::Shared => {
                self.resolve_to(callee.id, Resolution::NewShared);
                let mut held = None;
                for (position, argument) in arguments.iter().enumerate() {
                    let found = self.expression(&argument.value, Wanted::Anything);
                    if position == 0 {
                        held = Some((found, argument.value.span));
                    }
                    sources.push(ArgumentSource::Given(position));
                }
                match held {
                    Some((held, span)) => Type::shared(self.check_shared_value(held, span)),
                    None => Type::shared(Type::Unknown),
                }
            }

            Handle::Db => {
                self.resolve_to(callee.id, Resolution::NewDb);
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(&argument.value, Wanted::Exactly(Type::Text));
                    sources.push(ArgumentSource::Given(position));
                }
                if arguments.is_empty() {
                    self.report(messages::missing_argument("Db.connect", "url", call.span, None));
                }
                Type::fallible(Type::Db, Type::named("DbError"))
            }

            Handle::Store => {
                self.resolve_to(callee.id, Resolution::NewStore);
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(&argument.value, Wanted::Exactly(Type::Text));
                    sources.push(ArgumentSource::Given(position));
                }
                if arguments.is_empty() {
                    self.report(messages::missing_argument("Store.open", "path", call.span, None));
                }
                Type::fallible(Type::Store, Type::named("StoreError"))
            }

            Handle::LoggerStdout => {
                self.resolve_to(callee.id, Resolution::NewLoggerStdout);
                self.walk_arguments(arguments);
                Type::Logger
            }

            Handle::LoggerStderr => {
                self.resolve_to(callee.id, Resolution::NewLoggerStderr);
                self.walk_arguments(arguments);
                Type::Logger
            }

            Handle::LoggerFile => {
                self.resolve_to(callee.id, Resolution::NewLoggerFile);
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(&argument.value, Wanted::Exactly(Type::Text));
                    sources.push(ArgumentSource::Given(position));
                }
                if arguments.is_empty() {
                    self.report(messages::missing_argument("Logger.file", "path", call.span, None));
                }
                Type::fallible(Type::Logger, Type::named("LogError"))
            }

            Handle::LoggerMemory => {
                self.resolve_to(callee.id, Resolution::NewLoggerMemory);
                self.walk_arguments(arguments);
                Type::Logger
            }

            Handle::LoggerMulti => {
                self.resolve_to(callee.id, Resolution::NewLoggerMulti);
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(
                        &argument.value,
                        Wanted::Exactly(Type::list(Type::Logger)),
                    );
                    sources.push(ArgumentSource::Given(position));
                }
                if arguments.is_empty() {
                    self.report(messages::missing_argument(
                        "Logger.multi",
                        "loggers",
                        call.span,
                        None,
                    ));
                }
                Type::Logger
            }
        };

        self.checked.calls.insert(call.id, Call { arguments: sources });
        built
    }

    /// Reads an argument that names a type, as `Channel.new(of: Text)` does.
    fn type_argument(&mut self, argument: &Expr) -> Type {
        if let ExprKind::Name(name) = &argument.kind {
            let named = self.resolve_named_type(name);
            // The type's own name stands where a value usually would, so record the
            // type it names rather than leaving the node without an entry.
            self.checked.types.insert(argument.id, named.clone());
            return named;
        }
        self.report(messages::needs_a_type_name(argument.span));
        Type::Unknown
    }

    /// Checks arguments that nothing will be done with, so that mistakes inside them
    /// are still reported once the call itself has gone wrong.
    fn walk_arguments(&mut self, arguments: &[Argument]) {
        for argument in arguments {
            self.expression(&argument.value, Wanted::Anything);
        }
    }

    // -----------------------------------------------------------------------
    // Matching arguments to parameters
    // -----------------------------------------------------------------------

    /// Checks the arguments of a call against a signature and writes down where each
    /// parameter's value comes from. Gives back what the call produces, along with
    /// the signature as it was instantiated, whose type is the callee's own.
    fn apply(
        &mut self,
        call: &Expr,
        signature: &Signature,
        shape: &CallShape,
        arguments: &[Argument],
    ) -> (Type, Signature) {
        // Generics are resolved per call site: a fresh variable for each parameter
        // means `T` in one call has nothing to do with `T` in the next.
        let signature = self.variables.instantiate(signature);
        let sources = self.match_arguments(call, &signature, shape, arguments);
        self.checked.calls.insert(call.id, Call { arguments: sources });
        (signature.returns.clone(), signature)
    }

    fn match_arguments(
        &mut self,
        call: &Expr,
        signature: &Signature,
        shape: &CallShape,
        arguments: &[Argument],
    ) -> Vec<ArgumentSource> {
        let parameters = &signature.parameters;
        let mut sources: Vec<Option<ArgumentSource>> = vec![None; parameters.len()];
        let mut already: Vec<Option<Span>> = vec![None; parameters.len()];

        let mut named_seen = false;
        let mut next = 0usize;
        let mut said_too_many = false;
        let mut said_names_required = false;
        // A name that went nowhere has already been explained, and the parameter it
        // was meant for is then "missing" only as a consequence of that.
        let mut name_went_nowhere = false;

        for (position, argument) in arguments.iter().enumerate() {
            let Some(label) = &argument.name else {
                if named_seen {
                    self.report(messages::positional_after_named(argument.span));
                } else if shape.names_required && !said_names_required {
                    said_names_required = true;
                    let first = parameters.first().map(|parameter| parameter.name.as_str());
                    self.report(messages::new_needs_names(&shape.name, first, argument.span));
                }

                // Positional arguments fill the first slot no name has claimed, so
                // `f(1, b: 2)` and `f(b: 2, 1)` agree about where the `1` goes.
                while next < parameters.len() && sources[next].is_some() {
                    next += 1;
                }

                match parameters.get(next) {
                    Some(parameter) => {
                        let declared = parameter.declared.clone();
                        sources[next] = Some(ArgumentSource::Given(position));
                        already[next] = Some(argument.span);
                        next += 1;
                        let found = self.expression(&argument.value, Wanted::Exactly(declared));
                        self.check_to_json_argument(shape, argument, &found);
                    }
                    None => {
                        if !said_too_many {
                            said_too_many = true;
                            self.report(messages::too_many_arguments(
                                &shape.name,
                                parameters.len(),
                                arguments.len(),
                                argument.span,
                                shape.declared,
                            ));
                        }
                        self.expression(&argument.value, Wanted::Anything);
                    }
                }
                continue;
            };

            named_seen = true;

            if !shape.names_allowed {
                name_went_nowhere = true;
                self.report(messages::names_need_a_declaration(&label.text, label.span));
                self.expression(&argument.value, Wanted::Anything);
                continue;
            }

            let Some(slot) =
                parameters.iter().position(|parameter| parameter.name == label.text)
            else {
                name_went_nowhere = true;
                let known: Vec<String> =
                    parameters.iter().map(|parameter| parameter.name.clone()).collect();
                self.report(match (&shape.missing, &shape.owner) {
                    (WhenMissing::NewField | WhenMissing::Keep, Some(owner)) => {
                        messages::unknown_field(owner, &label.text, label.span, &known)
                    }
                    _ => messages::unknown_argument(
                        &shape.name,
                        &label.text,
                        label.span,
                        &known,
                    ),
                });
                self.expression(&argument.value, Wanted::Anything);
                continue;
            };

            match already[slot] {
                Some(first) => self.report(messages::duplicate_argument(
                    &label.text,
                    label.span,
                    first,
                )),
                None => {
                    sources[slot] = Some(ArgumentSource::Given(position));
                    already[slot] = Some(argument.span);
                }
            }

            let declared = parameters
                .get(slot)
                .map_or(Type::Unknown, |parameter| parameter.declared.clone());
            let found = self.expression(&argument.value, Wanted::Exactly(declared));
            self.check_to_json_argument(shape, argument, &found);
        }

        self.fill_the_rest(
            call,
            parameters,
            shape,
            arguments.len(),
            !name_went_nowhere,
            &mut sources,
        );

        // Every parameter has a source by now, even in a program being rejected, so
        // that the compiler can trust the shape of this list.
        sources.into_iter().map(|source| source.unwrap_or(ArgumentSource::Default)).collect()
    }

    /// Whatever no argument claimed either has a default, keeps what it had, or is
    /// missing and has to be said so.
    ///
    /// `worth_saying` is false once an argument has already been reported as going
    /// nowhere: the parameter it was aimed at is then only missing because of that,
    /// and saying so twice helps nobody.
    fn fill_the_rest(
        &mut self,
        call: &Expr,
        parameters: &[Parameter],
        shape: &CallShape,
        given: usize,
        worth_saying: bool,
        sources: &mut [Option<ArgumentSource>],
    ) {
        let mut missing = Vec::new();

        for (slot, parameter) in parameters.iter().enumerate() {
            if sources.get(slot).is_some_and(Option::is_some) {
                continue;
            }
            let source = match shape.missing {
                // `.with` names only what changes, so every other field is kept.
                WhenMissing::Keep => ArgumentSource::Kept,
                _ if parameter.optional => ArgumentSource::Default,
                _ => {
                    missing.push(parameter.name.clone());
                    ArgumentSource::Default
                }
            };
            if let Some(slot) = sources.get_mut(slot) {
                *slot = Some(source);
            }
        }

        if missing.is_empty() || !worth_saying {
            return;
        }

        match shape.missing {
            // "Account.new is missing `owner`." — one report for the whole set,
            // because a half-filled constructor is one mistake, not four.
            WhenMissing::NewField => {
                let owner = shape.owner.as_deref().unwrap_or(&shape.name);
                self.report(messages::missing_fields(
                    &shape.name,
                    owner,
                    &missing,
                    call.span,
                ));
            }
            WhenMissing::Count => {
                let wanted = parameters.len();
                self.report(messages::too_few_arguments(
                    &shape.name,
                    wanted,
                    given,
                    call.span,
                ));
            }
            _ => {
                for name in &missing {
                    self.report(messages::missing_argument(
                        &shape.name,
                        name,
                        call.span,
                        shape.declared,
                    ));
                }
            }
        }
    }
    fn check_to_json_argument(&mut self, shape: &CallShape, argument: &Argument, found: &Type) {
        if shape.name != "to_json" { return; }
        let resolved = self.variables.resolve(found);
        if matches!(resolved, Type::Unknown | Type::Variable(_)) { return; }
        if !can_json(&resolved, &self.checked) {
            self.report(messages::not_json(&resolved, argument.value.span));
        }
    }

}

/// Turns a function's type into a signature with placeholder names, so that a
/// function held in a value goes through the same matching as a declared one. The
/// names are never shown: such a call cannot use them.
fn positional(held: &FunctionType) -> Signature {
    Signature::new(
        held.parameters
            .iter()
            .enumerate()
            .map(|(position, declared)| Parameter::new(position.to_string(), declared.clone()))
            .collect(),
        held.returns.clone(),
    )
}

/// What to call the thing being called, when it has no declaration to name it.
fn describe_callee(callee: &Expr) -> String {
    match &callee.kind {
        ExprKind::Name(name) => name.text.clone(),
        ExprKind::Member { name, .. } => name.text.clone(),
        _ => "this".to_string(),
    }
}
