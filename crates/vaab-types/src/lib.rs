//! The Vaab type checker.
//!
//! [`check`] takes a parsed [`Module`](vaab_syntax::Module) and either hands back a
//! [`Checked`] or the reasons it could not. Nothing is rejected silently and nothing
//! is coerced: Vaab has no implicit conversions and no null, so every disagreement
//! between what was written and what was needed is a diagnostic.
//!
//! # What checking decides
//!
//! * Every expression's type, including the type arguments of a generic call.
//! * What every name means: which local slot, which function, which field.
//! * How each call's arguments line up with its parameters, once names, order and
//!   defaults have been sorted out.
//! * Whether every `match` covers every case.
//!
//! Function signatures are never inferred — Vaab requires them in full — so
//! inference is only ever local: the type of a `let` without an annotation, the
//! parameters of a closure, what an empty list holds.
//!
//! # The contract with the compiler
//!
//! [`Checked`] is written for the phase after this one. The rule for what it holds
//! is that a bytecode compiler should never have to work anything out twice, so:
//!
//! * [`Checked::types`] gives the type of any expression, keyed by
//!   [`NodeId`](vaab_syntax::NodeId). Spans cannot serve as keys, because grouping
//!   brackets are dropped from the tree and so several nodes can share a span.
//! * [`Checked::resolutions`] says what each name and each `.member` refers to. This
//!   is where the compiler learns that one call is a method and the next a free
//!   function, that a `.new` is the automatic one rather than a validated `to new`,
//!   and which numbered variant of a choice a name stands for.
//! * [`Checked::calls`] gives, per parameter, where its value comes from: an
//!   argument at the call site, a default to evaluate, or — for `.with` — the value
//!   the original already had. Arguments written out of order are already in order
//!   here.
//! * [`Checked::locals`], [`Checked::frames`] and [`Checked::bindings`] give every
//!   local a numbered slot in a numbered frame, and say how many frames out a
//!   captured value lives. Nothing needs resolving again at compile time.
//! * [`Checked::functions`], [`Checked::declared_types`], [`Checked::choices`] and
//!   [`Checked::abilities`] hold the declarations, with each ability's required
//!   functions in the slot order a dispatch table should use.
//!
//! Generics are erased. A call site unifies the type variables it needs and the
//! result is recorded as an ordinary type, so the compiler never sees a variable.
//!
//! # Errors
//!
//! Diagnostics are [`Diagnostic`](vaab_syntax::diagnostic::Diagnostic)s, the same
//! ones the parser produces and rendered the same way. They arrive in source order.

mod checked;
mod checker;
mod json;
mod messages;
mod prelude;
mod types;
mod unify;

pub use json::can_json;
pub use checked::{
    Ability, AbilityId, ArgumentSource, Call, Checked, Choice, ChoiceId, Closure, Constructor,
    DeclaredType, Field, Frame, FrameId, FrameKind, Function, FunctionId, Local, LocalId,
    LocalRef, Required, Resolution, Route, RouteSegment, Serve, Task, TypeId, Variant,
};
pub use checker::check;
pub use types::{FunctionType, Parameter, Signature, Type, VariableId};
