//! Everything the checker learned, in the shape the compiler after it needs.
//!
//! The rule for what belongs here: if a bytecode compiler would otherwise have to
//! work something out a second time, it goes in. So this records not just the type
//! of every expression but which declaration each name refers to, which slot each
//! local lives in, whether a `.new` builds fields directly or runs a validated
//! constructor, and where each argument at a call site comes from once names and
//! defaults have been sorted out.

use std::collections::HashMap;

use vaab_syntax::ast::NodeId;
use vaab_syntax::span::Span;

use crate::types::{Signature, Type};

macro_rules! handle {
    ($(#[$note:meta])* $name:ident) => {
        $(#[$note])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            /// This handle as an index into the table it came from.
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

handle! {
    /// A function: free, a method, or a validated `to new`.
    FunctionId
}
handle! {
    /// A declared `type`.
    TypeId
}
handle! {
    /// A declared `choice`.
    ChoiceId
}
handle! {
    /// A declared `ability`.
    AbilityId
}
handle! {
    /// One local value: a `let`, a parameter, or a name bound by a pattern.
    LocalId
}
handle! {
    /// One set of local slots: the file itself, a function body, or a closure body.
    FrameId
}

/// The result of checking a module.
#[derive(Clone, Debug, Default)]
pub struct Checked {
    /// The type of every expression, by node id.
    pub types: HashMap<NodeId, Type>,
    /// What a name, member access or pattern refers to.
    pub resolutions: HashMap<NodeId, Resolution>,
    /// How to lay out the arguments of each call.
    pub calls: HashMap<NodeId, Call>,
    /// The local each `let`, `for each` and pattern binding introduces, keyed by the
    /// statement or pattern that introduces it. This is where a value is *stored*;
    /// [`Checked::resolutions`] is where it is read.
    pub bindings: HashMap<NodeId, LocalId>,
    /// Every local in the module, in the order they were declared.
    pub locals: Vec<Local>,
    /// Every frame. `frames[0]` is the file itself.
    pub frames: Vec<Frame>,
    /// Every declared function, including the methods of a type and the `to new` that
    /// replaces an automatic constructor.
    pub functions: Vec<Function>,
    /// Declared `type`s, in source order.
    pub declared_types: Vec<DeclaredType>,
    /// Declared `choice`s, in source order.
    pub choices: Vec<Choice>,
    /// Declared `ability`s, in source order.
    pub abilities: Vec<Ability>,
    /// Every closure, by the id of the expression that wrote it.
    pub closures: HashMap<NodeId, Closure>,
    /// Every `start { ... }`, by the id of the expression that wrote it.
    pub tasks: HashMap<NodeId, Task>,
    pub serves: HashMap<NodeId, Serve>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteSegment {
    Literal(String),
    Param(String),
}

#[derive(Clone, Debug)]
pub struct Route {
    pub method: String,
    pub path: Vec<RouteSegment>,
    pub frame: FrameId,
    pub path_params: Vec<(String, Type)>,
    pub expecting: Option<(Type, LocalId)>,
}

#[derive(Clone, Debug)]
pub struct Serve {
    pub port: u16,
    pub before: Option<FrameId>,
    pub routes: Vec<Route>,
    pub error_handler: Option<(Type, LocalId, FrameId)>,
}

impl Checked {
    /// The frame holding the file's own top-level values.
    pub const TOP_LEVEL: FrameId = FrameId(0);

    pub fn type_of(&self, node: NodeId) -> Option<&Type> {
        self.types.get(&node)
    }

    pub fn resolution(&self, node: NodeId) -> Option<&Resolution> {
        self.resolutions.get(&node)
    }

    pub fn local(&self, local: LocalId) -> Option<&Local> {
        self.locals.get(local.index())
    }

    pub fn frame(&self, frame: FrameId) -> Option<&Frame> {
        self.frames.get(frame.index())
    }

    pub fn function(&self, function: FunctionId) -> Option<&Function> {
        self.functions.get(function.index())
    }

    pub fn declared_type(&self, declared: TypeId) -> Option<&DeclaredType> {
        self.declared_types.get(declared.index())
    }

    pub fn choice(&self, choice: ChoiceId) -> Option<&Choice> {
        self.choices.get(choice.index())
    }

    pub fn ability(&self, ability: AbilityId) -> Option<&Ability> {
        self.abilities.get(ability.index())
    }
}

/// What a name, a `.member` or a pattern turned out to mean.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Reading a local value or a parameter.
    Local(LocalRef),
    /// A function declared in this file, named directly.
    Function(FunctionId),
    /// A function from the built-in prelude, by its name.
    Builtin(&'static str),
    /// `self`, inside a type's own function.
    SelfValue,
    /// `account.balance`: field number `field` of `declared`.
    Field { declared: TypeId, field: usize },
    /// `account.deposit(...)`: a function in a type's body, dispatched statically
    /// because the receiver's type is known.
    Method { declared: TypeId, function: FunctionId },
    /// `thing.describe()` where `thing` is typed as an ability. The function to run
    /// is only known once the value is in hand, so this is a dynamic dispatch on
    /// slot `function` of the ability's table.
    AbilityMethod { ability: AbilityId, function: usize },
    /// A method from the built-in prelude, such as `.map`, by its name.
    BuiltinMethod(&'static str),
    /// `Account.new(...)` for a type that declares no `to new`: build it field by
    /// field, filling in defaults.
    AutomaticNew(TypeId),
    /// `Email.new(...)` where `Email` declares its own validated `to new`.
    UserNew { declared: TypeId, function: FunctionId },
    /// `Email.raw(...)`: the field-by-field constructor, legal only inside `Email`.
    Raw(TypeId),
    /// `account.with(field: value)`: copy, replacing the fields given.
    With(TypeId),
    /// `AccountError.Frozen`, whether constructed or matched against.
    Variant { choice: ChoiceId, variant: usize },
    /// `Channel.new(of: Text, size: 4)`.
    NewChannel,
    /// `Shared.new(0)`.
    NewShared,
    /// `Db.connect("sqlite:tasks.db")`.
    NewDb,
    /// `Store.open("data/cache.kv")`.
    NewStore,
    RequestField(usize),
    /// `request.who`: the signed bearer user, or AuthError.
    RequestWho,
}

/// Where a local lives, relative to the frame doing the reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalRef {
    pub local: LocalId,
    /// How many function or closure boundaries lie between the use and the
    /// declaration. `0` is a slot in the current frame; anything more is captured
    /// from an enclosing one.
    pub hops: u32,
}

/// One local value.
#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub frame: FrameId,
    /// Its slot within that frame.
    pub slot: usize,
    /// `true` for `let changing`, which is the only kind that may be assigned to.
    pub changing: bool,
    pub declared: Type,
    pub span: Span,
}

/// One set of local slots.
#[derive(Clone, Debug)]
pub struct Frame {
    pub kind: FrameKind,
    /// How many slots this frame needs, which is what the VM allocates.
    pub slots: usize,
    /// The frame this one is nested inside, for working out captures.
    pub parent: Option<FrameId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// The file's own top level.
    Module,
    Function(FunctionId),
    /// A closure, identified by the expression that wrote it.
    Closure(NodeId),
    Route(NodeId),
}

/// How to fill each parameter of a call, once names, order and defaults have been
/// worked out. One entry per parameter — or per field, for a constructor.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub arguments: Vec<ArgumentSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentSource {
    /// Argument number `n` at the call site, which may have been written out of
    /// order or by name.
    Given(usize),
    /// The default written on the declaration, which has to be evaluated.
    Default,
    /// For `.with(...)`: this field keeps the value the original already had.
    Kept,
}

/// A function, wherever it was declared.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    /// The type whose body declared it, for a method or a validated `to new`.
    pub owner: Option<TypeId>,
    pub signature: Signature,
    /// The frame its body runs in.
    pub frame: FrameId,
    /// Its parameters, as locals of that frame, in declared order.
    pub parameters: Vec<LocalId>,
    /// `true` when written `pure to f(...)`.
    pub pure: bool,
    /// The whole declaration, so the compiler can find the body and any defaults.
    /// For a free function this is the statement's own id; a method has no
    /// statement of its own, so it is the id of the `type` that holds it.
    pub declaration: NodeId,
    pub span: Span,
}

/// A closure, which is a function with no name and with values captured from
/// around it.
#[derive(Clone, Debug)]
pub struct Closure {
    pub frame: FrameId,
    /// Its parameters, as locals of that frame, in written order.
    pub parameters: Vec<LocalId>,
    pub signature: Signature,
    /// `true` for `pairs.map((a, b) -> a + b)`, which is handed one pair and names
    /// both halves of it. The caller still passes one value; whoever runs the
    /// closure has to take it apart into the slots above.
    pub unpacks: bool,
}

/// A `start { ... }` block, which becomes a task of its own.
///
/// A task does not share the stack it was started from: whatever it reaches for
/// outside itself has to be carried in when it starts. [`Task::captures`] is that
/// list, worked out by the sendability rules, so nothing has to find it again.
#[derive(Clone, Debug, Default)]
pub struct Task {
    /// Every local the body uses that was declared outside it, in first-use order.
    /// Locals reached only by a task nested inside this one are here too, because
    /// this task has to carry them as far as that one.
    pub captures: Vec<LocalId>,
}

/// A declared `type`.
#[derive(Clone, Debug)]
pub struct DeclaredType {
    pub name: String,
    /// Fields in declaration order, which is the order `raw` takes them in.
    pub fields: Vec<Field>,
    pub methods: Vec<FunctionId>,
    pub abilities: Vec<AbilityId>,
    pub constructor: Constructor,
    pub declaration: NodeId,
    pub span: Span,
}

impl DeclaredType {
    pub fn field(&self, name: &str) -> Option<(usize, &Field)> {
        self.fields.iter().enumerate().find(|(_, field)| field.name == name)
    }
}

impl Checked {
    /// The function named `name` in `declared`'s body, if it has one.
    pub fn method(&self, declared: TypeId, name: &str) -> Option<FunctionId> {
        let holder = self.declared_type(declared)?;
        holder.methods.iter().copied().find(|function| {
            self.function(*function).is_some_and(|function| function.name == name)
        })
    }
}

/// How a type is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constructor {
    /// `.new` is generated from the fields.
    Automatic,
    /// The type declares `to new`, which replaces the generated one.
    UserDefined(FunctionId),
}

/// One field of a type, or one part of a choice variant's payload.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub declared: Type,
    /// Whether a default was written, so the field may be left out of `.new`.
    pub has_default: bool,
    pub span: Span,
}

/// A declared `choice`.
#[derive(Clone, Debug)]
pub struct Choice {
    pub name: String,
    /// Variants in declaration order. The position in this list is the variant's
    /// number, which is what the VM stores in a value.
    pub variants: Vec<Variant>,
    pub declaration: NodeId,
    pub span: Span,
}

impl Choice {
    pub fn variant(&self, name: &str) -> Option<(usize, &Variant)> {
        self.variants.iter().enumerate().find(|(_, variant)| variant.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

/// A declared `ability`.
#[derive(Clone, Debug)]
pub struct Ability {
    pub name: String,
    /// Required functions in declaration order. The position is the slot a
    /// providing type's dispatch table must fill.
    pub functions: Vec<Required>,
    pub declaration: NodeId,
    pub span: Span,
}

impl Ability {
    pub fn required(&self, name: &str) -> Option<(usize, &Required)> {
        self.functions.iter().enumerate().find(|(_, function)| function.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct Required {
    pub name: String,
    pub signature: Signature,
    pub span: Span,
}
