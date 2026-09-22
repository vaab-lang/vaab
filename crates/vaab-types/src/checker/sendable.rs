//! Sendability: what may cross from one task to another.
//!
//! This is where Vaab's central promise is kept. A task does not share the stack it
//! was started from, so every value a `start { ... }` body uses from outside itself
//! has to be *carried in*. A value is **sendable** when carrying it in cannot
//! produce a data race — when two tasks holding it can never disagree about what it
//! is.
//!
//! # What is sendable
//!
//! * `Int`, `Float`, `Bool`, `Text` and `Nothing` are sendable. They cannot be
//!   changed at all, so two tasks holding one cannot disagree.
//! * A list, a map, a tuple, a `maybe T` and a `T or fails E` are sendable when
//!   everything inside them is. Vaab has no way to change a list or a `type` in
//!   place (D30), so a collection is a value like any other.
//! * A declared `type` is sendable when every one of its fields is, and a `choice`
//!   when every part of every variant is. A `type` is immutable, so the only way it
//!   could be unsafe is by holding something that is not.
//! * An ability-typed value is sendable when every type declared `can` that ability
//!   is sendable, because any one of them could be the value in hand.
//! * `channel of T`, `shared T` and `task of T` are sendable handles when `T` is.
//!   These are the sanctioned ways to share: a channel moves values between tasks
//!   one at a time, and a `shared` only ever changes through `.update`.
//! * A **function value is not sendable**. A closure holds on to whatever was
//!   around it when it was written, and a function value on its own carries no
//!   record of what that was — so Vaab cannot promise it is safe to move.
//!
//! The function rule is softened in exactly one place, and only where it can be
//! done soundly: a local bound straight to a closure — `let double = n -> n * 2` —
//! remembers which closure it holds, so a task using it is judged by that closure's
//! own captures. This is what makes "a closure that captures something unsendable
//! cannot itself be sent" a real rule rather than a blanket refusal. A function
//! value that arrived any other way — a parameter, the result of a call, a field —
//! cannot be traced, and is refused.
//!
//! A type parameter such as `T` is treated as sendable. Nothing in Vaab bounds a
//! type parameter yet, so there is nothing to check it against; see the notes for
//! phase 4.

use std::collections::HashMap;

use vaab_syntax::ast::{Block, Expr, ExprKind, NodeId, Stmt, StmtKind};
use vaab_syntax::span::Span;

use super::walk::{self, Step, Visit};
use super::Checker;
use crate::checked::{Checked, LocalId, Resolution, Task};
use crate::messages;
use crate::types::Type;

/// Why a value of some type may not cross between tasks.
///
/// The thing in the way is always a function value, because every other type is
/// judged by what it holds. It is still kept as a [`Type`] so a message can print
/// it exactly as it was written.
pub(crate) struct NotSendable {
    pub culprit: Type,
    /// How the culprit is reached from the type asked about, as English a message
    /// can drop in: "`Job`'s field `work`". `None` when the type asked about is
    /// itself the thing in the way.
    pub reached_by: Option<String>,
}

impl NotSendable {
    fn culprit(declared: &Type) -> Self {
        NotSendable { culprit: declared.clone(), reached_by: None }
    }

    /// The same reason, reached through one more step. The innermost step wins,
    /// because that is the one naming the field a person has to change.
    fn through(mut self, step: impl FnOnce() -> String) -> Self {
        self.reached_by = Some(self.reached_by.unwrap_or_else(step));
        self
    }

    /// The same reason, with a step written in front of whatever was already
    /// found. An ability needs this rather than [`NotSendable::through`]: the field
    /// in the way belongs to a type the reader never wrote down, so the path has to
    /// say where that type came from.
    fn behind(mut self, step: impl FnOnce() -> String) -> Self {
        let step = step();
        self.reached_by =
            Some(match self.reached_by {
                Some(path) => format!("{step}, and {path}"),
                None => step,
            });
        self
    }
}

/// What the rules have to say about one value a task reaches for.
enum Verdict {
    Fine,
    /// The value may change under the task's feet. This is the headline error.
    Changing {
        name: String,
        declared: Type,
        declared_at: Span,
        /// Where a closure standing between the task and the variable reads it,
        /// when the task does not name the variable itself.
        read_at: Option<Span>,
    },
    /// The value's type cannot cross, whoever holds it.
    Unsendable(NotSendable),
}

// ---------------------------------------------------------------------------
// The type-level rules
// ---------------------------------------------------------------------------

impl Checker {
    /// Whether a value of this type may cross between tasks, and if not, why not.
    pub(crate) fn sendability(&self, declared: &Type) -> Result<(), NotSendable> {
        let resolved = self.variables.resolve(declared);
        self.sendability_of(&resolved, &mut Vec::new())
    }

    /// `visiting` holds the declared types currently being looked into, so that a
    /// type holding itself is answered rather than followed for ever.
    fn sendability_of(
        &self,
        declared: &Type,
        visiting: &mut Vec<String>,
    ) -> Result<(), NotSendable> {
        match declared {
            // Nothing can change these, so two tasks holding one cannot disagree.
            Type::Int | Type::Float | Type::Bool | Type::Text | Type::Nothing | Type::Db | Type::Store => {
                Ok(())
            }

            // A hole, or a mistake already reported. Neither is worth a second
            // message, and nothing is known well enough to refuse it.
            Type::Variable(_) | Type::Unknown => Ok(()),

            // Nothing bounds a type parameter in Vaab, so there is nothing to ask.
            Type::Parameter(_) => Ok(()),

            // A handle is as sendable as what it carries; a collection is as
            // sendable as what it holds. Both are the same question.
            Type::List(item)
            | Type::Maybe(item)
            | Type::Channel(item)
            | Type::Task(item)
            | Type::Shared(item) => self.sendability_of(item, visiting),

            Type::Map { key, value } => self
                .sendability_of(key, visiting)
                .and_then(|()| self.sendability_of(value, visiting)),

            Type::Fallible { ok, error } => self
                .sendability_of(ok, visiting)
                .and_then(|()| self.sendability_of(error, visiting)),

            Type::Tuple(items) => {
                for item in items {
                    self.sendability_of(item, visiting)?;
                }
                Ok(())
            }

            Type::Function(_) => Err(NotSendable::culprit(declared)),

            Type::Named(name) => self.sendability_of_named(name, visiting),
            Type::Ability(name) => self.sendability_of_ability(name, visiting),
        }
    }

    fn sendability_of_named(
        &self,
        name: &str,
        visiting: &mut Vec<String>,
    ) -> Result<(), NotSendable> {
        // A type that holds another of its own kind is no less sendable for it:
        // whatever is really in the way will be found on the way down.
        if visiting.iter().any(|seen| seen == name) {
            return Ok(());
        }
        visiting.push(name.to_string());
        let verdict = self.fields_of_named(name, visiting);
        visiting.pop();
        verdict
    }

    fn fields_of_named(
        &self,
        name: &str,
        visiting: &mut Vec<String>,
    ) -> Result<(), NotSendable> {
        if let Some(declared) = self.checked.declared_types.iter().find(|held| held.name == name) {
            for field in &declared.fields {
                self.sendability_of(&field.declared, visiting).map_err(|why| {
                    why.through(|| format!("`{name}`'s field `{}`", field.name))
                })?;
            }
            return Ok(());
        }

        if let Some(choice) = self.checked.choices.iter().find(|held| held.name == name) {
            for variant in &choice.variants {
                for field in &variant.fields {
                    self.sendability_of(&field.declared, visiting).map_err(|why| {
                        why.through(|| {
                            format!("`{name}`'s `{}` holds `{}`", variant.name, field.name)
                        })
                    })?;
                }
            }
        }

        Ok(())
    }

    /// A value known only by the ability it provides could be any of the types that
    /// provide it, so it is sendable only when every one of them is.
    fn sendability_of_ability(
        &self,
        ability: &str,
        visiting: &mut Vec<String>,
    ) -> Result<(), NotSendable> {
        let wanted = self
            .checked
            .abilities
            .iter()
            .position(|held| held.name == ability)
            .map(|position| position as u32);
        let Some(wanted) = wanted else { return Ok(()) };

        for declared in &self.checked.declared_types {
            if !declared.abilities.iter().any(|held| held.0 == wanted) {
                continue;
            }
            let provider = declared.name.clone();
            self.sendability_of_named(&provider, visiting)
                .map_err(|why| why.behind(|| format!("any `{ability}` could be a `{provider}`")))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// What a task reaches for
// ---------------------------------------------------------------------------

impl Checker {
    /// Checks a `start { ... }` block, and records what it has to carry in.
    ///
    /// `result` is what the body produces, which travels the other way: back to
    /// whoever waits for the task. A `task of T` may itself be handed to several
    /// tasks, so `T` has to be sendable too.
    ///
    /// `task_locals_begin` is how many locals the module had before the body was
    /// checked: anything from there on is the task's own, and so is not something
    /// the task reached for.
    pub(super) fn check_task(
        &mut self,
        task: NodeId,
        body: &Block,
        span: Span,
        result: &Type,
        task_locals_begin: usize,
    ) {
        let mut reaches = Reaches::default();
        walk::block(body, &mut reaches);

        let mut captures: Vec<LocalId> = Vec::new();
        for (node, at) in reaches.names {
            let Some(Resolution::Local(found)) = self.checked.resolution(node).cloned() else {
                continue;
            };
            if found.local.index() >= task_locals_begin {
                continue;
            }
            // One report per value, however many times the task uses it.
            if captures.contains(&found.local) {
                continue;
            }
            captures.push(found.local);
            self.check_capture(found.local, at);
        }

        for (node, at) in reaches.selves {
            let Some(found) = self.checked.type_of(node).cloned() else { continue };
            if let Err(why) = self.sendability(&found) {
                self.report(messages::unsendable_self(
                    &found,
                    &why.culprit,
                    why.reached_by.as_deref(),
                    at,
                ));
            }
        }

        self.checked.tasks.insert(task, Task { captures });

        if let Err(why) = self.sendability(result) {
            let result = self.variables.resolve(result);
            self.report(messages::unsendable_task_result(
                &result,
                &why.culprit,
                why.reached_by.as_deref(),
                span,
            ));
        }
    }

    /// One value a task uses from outside itself, reported if the rules refuse it.
    fn check_capture(&mut self, local: LocalId, at: Span) {
        // A task inside a task reaches for the same value, and the innermost one is
        // checked first. Saying it twice would bury the explanation.
        if !self.reported_captures.insert((local, at)) {
            return;
        }

        let Some(held) = self.checked.local(local) else { return };
        let name = held.name.clone();
        let found = self.variables.resolve(&held.declared);
        let found_at = held.span;

        match self.verdict(local, &mut Vec::new()) {
            Verdict::Fine => {}
            Verdict::Changing { name: changing, declared, declared_at, read_at } => {
                self.report(match read_at {
                    None => messages::changing_captured_by_task(
                        &changing,
                        &declared,
                        at,
                        declared_at,
                    ),
                    Some(read_at) => messages::changing_captured_through(
                        &changing,
                        &name,
                        &declared,
                        at,
                        read_at,
                        declared_at,
                    ),
                });
            }
            Verdict::Unsendable(why) => self.report(messages::unsendable_capture(
                &name,
                &found,
                &why.culprit,
                why.reached_by.as_deref(),
                at,
                found_at,
            )),
        }
    }

    /// What the rules say about carrying one local into a task.
    ///
    /// `seen` holds the closures already being looked into. A closure can only
    /// mention locals declared before it, so it can never reach itself; the list is
    /// there so that the walk is bounded by construction rather than by hope.
    fn verdict(&self, local: LocalId, seen: &mut Vec<NodeId>) -> Verdict {
        let Some(held) = self.checked.local(local) else { return Verdict::Fine };
        let name = held.name.clone();
        let declared = self.variables.resolve(&held.declared);
        let declared_at = held.span;

        if held.changing {
            return Verdict::Changing { name, declared, declared_at, read_at: None };
        }

        // A function value is refused unless it can be traced to the closure it
        // holds, in which case that closure's own captures are the real question.
        if matches!(declared, Type::Function(_)) {
            return self.verdict_of_closure(local, &declared, seen);
        }

        match self.sendability(&declared) {
            Ok(()) => Verdict::Fine,
            Err(why) => Verdict::Unsendable(why),
        }
    }

    fn verdict_of_closure(
        &self,
        local: LocalId,
        declared: &Type,
        seen: &mut Vec<NodeId>,
    ) -> Verdict {
        let Some(closure) = self.closure_of_local.get(&local).copied() else {
            return Verdict::Unsendable(NotSendable::culprit(declared));
        };
        if seen.contains(&closure) {
            return Verdict::Unsendable(NotSendable::culprit(declared));
        }

        seen.push(closure);
        let verdict = self.verdict_of_captures(closure, seen);
        seen.pop();
        verdict
    }

    fn verdict_of_captures(&self, closure: NodeId, seen: &mut Vec<NodeId>) -> Verdict {
        let Some(reaches) = self.closure_captures.get(&closure) else { return Verdict::Fine };

        for (held, at) in reaches {
            match self.verdict(*held, seen) {
                Verdict::Fine => {}
                Verdict::Changing { name, declared, declared_at, read_at } => {
                    return Verdict::Changing {
                        name,
                        declared,
                        declared_at,
                        // The place the closure reads it is the one worth showing,
                        // so a deeper one already found keeps its label.
                        read_at: read_at.or(Some(*at)),
                    };
                }
                Verdict::Unsendable(why) => return Verdict::Unsendable(why),
            }
        }

        Verdict::Fine
    }

    /// Remembers which locals a closure body uses from outside itself, so that a
    /// task given the closure later can be told what it would really be sending.
    pub(super) fn record_closure_captures(
        &mut self,
        closure: NodeId,
        body: &vaab_syntax::ast::FunctionBody,
        closure_locals_begin: usize,
    ) {
        let mut reaches = Reaches::default();
        walk::function_body(body, &mut reaches);

        let mut captures: Vec<(LocalId, Span)> = Vec::new();
        for (node, at) in reaches.names {
            let Some(Resolution::Local(found)) = self.checked.resolution(node) else { continue };
            if found.local.index() >= closure_locals_begin {
                continue;
            }
            if captures.iter().any(|(held, _)| *held == found.local) {
                continue;
            }
            captures.push((found.local, at));
        }

        self.closure_captures.insert(closure, captures);
    }

    /// Notes that a fixed local holds exactly the closure written next to it, which
    /// is what lets a task using it be judged by that closure's captures.
    pub(super) fn record_closure_of_local(&mut self, local: LocalId, closure: NodeId) {
        self.closure_of_local.insert(local, closure);
    }
}

/// The names and `self`s a subtree reaches for, with where each was written.
#[derive(Default)]
struct Reaches {
    names: Vec<(NodeId, Span)>,
    selves: Vec<(NodeId, Span)>,
}

impl Visit for Reaches {
    fn expression(&mut self, expression: &Expr) -> Step {
        match &expression.kind {
            ExprKind::Name(name) => self.names.push((expression.id, name.span)),
            ExprKind::SelfValue => self.selves.push((expression.id, expression.span)),
            _ => {}
        }
        Step::Into
    }
}

// ---------------------------------------------------------------------------
// Values crossing the other ways
// ---------------------------------------------------------------------------

impl Checker {
    /// `send value to channel`. What a channel carries is settled when the channel
    /// is built, so this catches the one case that cannot be: a channel that
    /// arrived already typed, as a parameter does.
    pub(super) fn check_sent_value(&mut self, value: &Expr, found: &Type) {
        if let Err(why) = self.sendability(found) {
            let found = self.variables.resolve(found);
            self.report(messages::unsendable_sent_value(
                &found,
                &why.culprit,
                why.reached_by.as_deref(),
                value.span,
            ));
        }
    }

    /// `Channel.new(of: T)`. Reports and hands back what the channel should be
    /// treated as carrying, so that one refusal does not become one per `send`.
    pub(super) fn check_channel_type(&mut self, carries: Type, span: Span) -> Type {
        match self.sendability(&carries) {
            Ok(()) => carries,
            Err(why) => {
                let carries = self.variables.resolve(&carries);
                self.report(messages::unsendable_channel(
                    &carries,
                    &why.culprit,
                    why.reached_by.as_deref(),
                    span,
                ));
                Type::Unknown
            }
        }
    }

    /// `Shared.new(value)`. The same shape as a channel: refuse once, then treat
    /// what it holds as already explained.
    pub(super) fn check_shared_value(&mut self, held: Type, span: Span) -> Type {
        match self.sendability(&held) {
            Ok(()) => held,
            Err(why) => {
                let held = self.variables.resolve(&held);
                self.report(messages::unsendable_shared_value(
                    &held,
                    &why.culprit,
                    why.reached_by.as_deref(),
                    span,
                ));
                Type::Unknown
            }
        }
    }
}

// ---------------------------------------------------------------------------
// What `.update` may do
// ---------------------------------------------------------------------------

impl Checker {
    /// The change handed to `shared.update` runs while the value is held, so it may
    /// not wait for anything: a task that waited there would hold the value while it
    /// waited, and whatever it was waiting for could be waiting for the value.
    pub(super) fn check_update_change(&mut self, arguments: &[vaab_syntax::ast::Argument]) {
        let waiting = {
            let mut waits = Waits { checked: &self.checked, found: Vec::new() };
            for argument in arguments {
                walk::expression(&argument.value, &mut waits);
            }
            waits.found
        };

        for (span, what) in waiting {
            self.report(messages::update_cannot_wait(what, span));
        }
    }
}

/// Everything inside a subtree that would have to wait for another task.
struct Waits<'a> {
    checked: &'a Checked,
    found: Vec<(Span, &'static str)>,
}

impl Visit for Waits<'_> {
    fn statement(&mut self, statement: &Stmt) -> Step {
        let what = match &statement.kind {
            StmtKind::Send(_) => Some("sends a value, which waits until the channel has room"),
            StmtKind::Together(_) => Some("waits for every task started inside it"),
            _ => None,
        };
        self.note(statement.span, what)
    }

    fn expression(&mut self, expression: &Expr) -> Step {
        let what = match &expression.kind {
            ExprKind::Receive { .. } => Some("waits for a value to arrive"),
            ExprKind::Select(_) => Some("waits for whichever arm is ready first"),
            ExprKind::Start(_) => Some("starts a task, which needs its turn to run"),
            ExprKind::Call { callee, .. } => self.waiting_call(callee),
            _ => None,
        };
        self.note(expression.span, what)
    }
}

impl Waits<'_> {
    /// Records a wait, and stops there. A wait written inside another — a `receive`
    /// in the body of a `start`, say — goes once the outer one does, so pointing at
    /// both would ask for the same edit twice.
    fn note(&mut self, span: Span, what: Option<&'static str>) -> Step {
        match what {
            Some(what) => {
                self.found.push((span, what));
                Step::Over
            }
            None => Step::Into,
        }
    }

    /// `.wait()` and a second `.update(...)` both wait, and the checker has already
    /// worked out which built-in each `.` reached, so the answer is looked up rather
    /// than guessed from the name.
    fn waiting_call(&self, callee: &Expr) -> Option<&'static str> {
        match self.checked.resolution(callee.id) {
            Some(Resolution::BuiltinMethod("wait")) => Some("waits for a task to finish"),
            Some(Resolution::BuiltinMethod("update")) => {
                Some("changes a shared value, which waits for its turn")
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// `together` and `select`
// ---------------------------------------------------------------------------

impl Checker {
    /// `together` waits for the tasks started inside it. Tasks can only come from a
    /// `start`, written here or reached through a call, so a body holding neither
    /// provably waits for nothing.
    pub(super) fn check_together(&mut self, body: &Block, span: Span) {
        let mut looking = CouldStartATask { found: false };
        walk::block(body, &mut looking);

        if !looking.found {
            self.report(messages::together_waits_for_nothing(span));
        }
    }

    /// A `select` with no `when` arms has nothing that could ever become ready.
    pub(super) fn check_select_has_arms(&mut self, arms: usize, span: Span) {
        if arms == 0 {
            self.report(messages::select_with_no_arms(span));
        }
    }

    /// `when timeout after 2 seconds`. The parser takes any word here, because only
    /// the checker knows which words are units.
    pub(super) fn check_time_unit(&mut self, unit: &vaab_syntax::ast::Name) {
        if TIME_UNITS.contains(&unit.text.as_str()) {
            return;
        }
        let known: Vec<String> = TIME_UNITS.iter().map(|unit| unit.to_string()).collect();
        self.report(messages::unknown_time_unit(&unit.text, unit.span, &known));
    }
}

/// The units a timeout may be written in. Both spellings of each are accepted, so
/// `after 1 second` and `after 2 seconds` both read as English.
const TIME_UNITS: &[&str] = &[
    "millisecond",
    "milliseconds",
    "second",
    "seconds",
    "minute",
    "minutes",
    "hour",
    "hours",
];

/// Whether anything in a subtree could start a task.
struct CouldStartATask {
    found: bool,
}

impl Visit for CouldStartATask {
    fn expression(&mut self, expression: &Expr) -> Step {
        if matches!(expression.kind, ExprKind::Start(_) | ExprKind::Call { .. }) {
            self.found = true;
            // The answer cannot change once it is yes.
            return Step::Over;
        }
        Step::Into
    }
}

/// The closures a module wrote, and which local each is held in.
pub(super) type ClosuresOfLocals = HashMap<LocalId, NodeId>;

/// What each closure reaches for outside itself, and where.
pub(super) type ClosureCaptures = HashMap<NodeId, Vec<(LocalId, Span)>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_of_time_has_both_of_its_spellings() {
        for singular in ["millisecond", "second", "minute", "hour"] {
            assert!(TIME_UNITS.contains(&singular), "{singular} is missing");
            let plural = format!("{singular}s");
            assert!(TIME_UNITS.contains(&plural.as_str()), "{plural} is missing");
        }
    }

    #[test]
    fn the_innermost_step_names_the_field_to_change() {
        let function = Type::function(vec![Type::Int], Type::Int);
        let why = NotSendable::culprit(&function)
            .through(|| "`Inner`'s field `work`".to_string())
            .through(|| "`Outer`'s field `inner`".to_string());
        assert_eq!(why.reached_by.as_deref(), Some("`Inner`'s field `work`"));
    }

    #[test]
    fn a_step_written_in_front_keeps_what_was_already_found() {
        let function = Type::function(Vec::new(), Type::Int);
        let why = NotSendable::culprit(&function)
            .through(|| "`Job`'s field `work`".to_string())
            .behind(|| "any `Runnable` could be a `Job`".to_string());
        assert_eq!(
            why.reached_by.as_deref(),
            Some("any `Runnable` could be a `Job`, and `Job`'s field `work`")
        );
    }
}
