//! The instruction set, and the program a compiled file turns into.
//!
//! Vaab compiles to a stack machine: almost every instruction takes what it needs
//! from the top of a value stack and leaves its answer there. The exceptions are
//! the ones that name a slot, a body or a jump, and each of those carries its
//! operand inline rather than on the stack.
//!
//! Two things are worth knowing before reading [`Op`]:
//!
//! * A Vaab call is never a Rust call. [`Op::Call`] and its neighbours push a
//!   frame onto the machine's frame stack and carry on, so the depth of a Vaab
//!   program has nothing to do with the depth of the Rust stack it runs on.
//! * Every instruction has a [`Span`] beside it in [`Body::spans`]. That is what
//!   lets a division by zero point at the division that did it.

use vaab_syntax::span::Span;

use crate::builtin::Builtin;
use crate::error::Feature;
use crate::value::{Ref, RecordLayout, Value, VariantLayout};

/// One instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    // -- Values ------------------------------------------------------------
    /// Pushes a value from the pool: text, a float, or a function that captured
    /// nothing and so could be built once at compile time.
    Constant(u32),
    Int(i64),
    Bool(bool),
    /// The single value of type `Nothing`.
    Nothing,
    /// The empty half of a `maybe`, written `nothing`.
    Absent,
    /// Wraps the top of the stack: `found x`, `success x`, `failure e`.
    Found,
    Success,
    Failure,

    // -- Stack -------------------------------------------------------------
    Pop,
    /// Copies the top, for a test that must not eat the value it is testing.
    Duplicate,

    // -- Where values live -------------------------------------------------
    /// A top-level value of the file, which outlives any one frame.
    LoadGlobal(u32),
    StoreGlobal(u32),
    /// A slot in the current frame.
    LoadLocal(u32),
    StoreLocal(u32),
    /// Replaces a slot's value with a fresh cell holding it, so that a closure
    /// made later shares the slot rather than a copy of it.
    Box(u32),
    LoadCell(u32),
    StoreCell(u32),
    /// A cell this body captured from the frame that made it.
    LoadUpvalue(u32),
    StoreUpvalue(u32),
    /// `self`, inside a type's own function.
    LoadSelf,

    // -- Operators ---------------------------------------------------------
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Negate,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Not,

    // -- Building ----------------------------------------------------------
    /// Takes the top `n` values into a list.
    List(u32),
    /// Takes the top `2n` values, laid out key, value, key, value, into a map.
    Map(u32),
    Tuple(u32),
    /// `1..10`, which becomes the list of whole numbers it stands for.
    Range,
    /// Joins the top `n` values into one piece of text.
    Text(u32),
    Record { layout: u32, fields: u32 },
    Variant { layout: u32, fields: u32 },

    // -- Reaching inside ---------------------------------------------------
    Field(u32),
    /// Stores into a `changing` field of a cast. Pops value, then record.
    SetField(u32),
    /// `items[n]`, which reports when there is no such position.
    Index,
    /// How many items a list or a map holds, or how many characters text has.
    Length,
    IsEmpty,
    /// A map as the list of pairs `for each` walks through.
    Entries,
    /// Appends the top of the stack to the list held in a slot. Building a list
    /// in place is what keeps `.map` over a long list from copying it each time.
    Append(u32),

    // -- Taking apart ------------------------------------------------------
    IsFound,
    IsSuccess,
    IsFailure,
    IsVariant(u32),
    /// Takes what is inside a `found`, a `success` or a `failure`.
    Unwrap,
    VariantField(u32),
    TupleItem(u32),
    /// Whether the list on top is the right length for a list pattern.
    /// `[first, ...]` asks for at least one; `[a, b]` asks for exactly two.
    ListLengthIs { length: u32, at_least: bool },

    // -- Control -----------------------------------------------------------
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    Return,

    // -- Calls -------------------------------------------------------------
    /// Calls the function value below the `n` arguments.
    Call(u32),
    /// A method on a declared type, which the checker resolved statically. The
    /// receiver sits below the arguments.
    CallMethod { body: u32, arity: u32 },
    /// A method reached through an ability, so which body runs is only known once
    /// the receiver is in hand.
    CallAbility { ability: u32, slot: u32, arity: u32 },
    Builtin(Builtin),
    /// Builds a function value, taking a cell for each of the body's captures.
    MakeFunction(u32),

    // -- Concurrency -------------------------------------------------------
    ChannelNew,
    SharedNew,
    Send,
    Receive,
    Close,
    Start(u32),
    TaskWait,
    SharedRead,
    SharedUpdate,
    BeginTogether,
    EndTogether,
    Select(u32),

    ReadFile(u32),
    Now,

    EnvGet,
    EnvRequired(u32),
    DbConnect(u32),
    DbExecute(u32),
    DbQuery(u32),
    StoreOpen(u32),
    StoreGet,
    StoreSet(u32),
    StoreRemove(u32),
    StoreKeys(u32),
    HttpGet(u32),
    HttpPost(u32),
    HttpSend(u32),
    RequestWho { user: u32, unauthorized: u32 },

    ReplyWith(bool),
    ReplyFile(bool),
    ReplyText(bool),
    ReplyExplain,

    // -- Stopping ----------------------------------------------------------
    /// Something the language has, and this phase does not run.
    NotYet(Feature),
    /// Reached when no arm of a `match` applied, which a checked program cannot do.
    NoArm,
}

/// Where one of a body's captured values comes from when the function value is
/// built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// A cell in a slot of the frame making the function.
    Local(u32),
    /// A cell that frame had captured itself.
    Upvalue(u32),
}

/// One runnable body: the file's top level, a function, a method, or a closure.
#[derive(Clone, Debug)]
pub struct Body {
    /// What a trace calls it: "`fib`", "a closure", "the top level".
    pub name: Ref<str>,
    pub code: Vec<Op>,
    /// One span per instruction, so any of them can point at its own source.
    pub spans: Vec<Span>,
    /// How many slots a frame running this body needs. The first `parameters` of
    /// them are filled by the caller.
    pub slots: usize,
    pub parameters: usize,
    pub captures: Vec<Capture>,
}

impl Body {
    fn empty(name: Ref<str>) -> Body {
        Body { name, code: Vec::new(), spans: Vec::new(), slots: 0, parameters: 0, captures: Vec::new() }
    }
}

/// A compiled file.
#[derive(Clone, Debug)]
pub struct Program {
    pub bodies: Vec<Body>,
    /// Values known at compile time: text, floats, and functions that captured
    /// nothing and so can be one shared value for the whole run.
    pub constants: Vec<Value>,
    pub layouts: Vec<Ref<RecordLayout>>,
    /// Every variant of every choice, flattened, which is what a pattern tests
    /// against.
    pub variants: Vec<Ref<VariantLayout>>,
    /// How many top-level values the file has.
    pub globals: usize,
    /// Where each top-level statement begins in body [`Program::TOP_LEVEL`], with
    /// a final entry for the end. The REPL runs one statement at a time by
    /// starting the machine partway through this body.
    pub statements: Vec<u32>,
    pub selects: Vec<SelectDescriptor>,
    pub routes: Vec<RouteHandler>,
}

#[derive(Clone, Debug)]
pub struct RouteHandler {
    pub method: String,
    pub path: Vec<vaab_types::RouteSegment>,
    pub body: usize,
    pub frame: vaab_types::FrameId,
    pub path_param_count: usize,
    pub expects_body: bool,
    /// Layout index for `expecting T as name`, when the body should be a record.
    pub body_layout: Option<u32>,
}

/// One arm of a compiled `select`.
#[derive(Clone, Debug)]
pub enum SelectArm {
    Receive { body: u32 },
    Timeout { milliseconds: i64, body: u32 },
}

/// Everything a [`Op::Select`] needs that does not fit in the instruction.
#[derive(Clone, Debug)]
pub struct SelectDescriptor {
    pub arms: Vec<SelectArm>,
    pub otherwise: Option<u32>,
    pub span: Span,
}

impl Program {
    /// The body holding the file's own statements.
    pub const TOP_LEVEL: usize = 0;

    pub(crate) fn with_top_level(name: Ref<str>) -> Program {
        Program {
            bodies: vec![Body::empty(name)],
            constants: Vec::new(),
            layouts: Vec::new(),
            variants: Vec::new(),
            globals: 0,
            statements: Vec::new(),
            selects: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub fn body(&self, index: usize) -> Option<&Body> {
        self.bodies.get(index)
    }

    /// Where the machine should start to run top-level statements from `first`
    /// onwards, which is how a REPL adds to a session without running it again.
    pub fn statement_start(&self, first: usize) -> usize {
        self.statements.get(first).copied().unwrap_or(0) as usize
    }

    /// The whole program, written out. This is a reading aid and a test fixture:
    /// a snapshot of it says exactly what the compiler decided.
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        for (index, body) in self.bodies.iter().enumerate() {
            out.push_str(&format!(
                "body {index}: {} — {}, {}",
                body.name,
                count(body.slots, "slot"),
                count(body.parameters, "parameter"),
            ));
            if !body.captures.is_empty() {
                out.push_str(&format!(", captures {:?}", body.captures));
            }
            out.push('\n');
            for (position, op) in body.code.iter().enumerate() {
                out.push_str(&format!("{position:5}  {}\n", write(op, self)));
            }
            out.push('\n');
        }
        out
    }
}

/// One instruction, written the way a reader wants it: a constant shows its value
/// and a call shows the name of what it calls.
fn write(op: &Op, program: &Program) -> String {
    match op {
        Op::Constant(index) => match program.constants.get(*index as usize) {
            Some(value) => format!("Constant {index} ({})", value.quoted()),
            None => format!("Constant {index}"),
        },
        Op::Record { layout, fields } => {
            let name = program
                .layouts
                .get(*layout as usize)
                .map(|layout| layout.name.clone())
                .unwrap_or_default();
            format!("Record {name} ({fields} fields)")
        }
        Op::Variant { layout, fields } => {
            let name = program
                .variants
                .get(*layout as usize)
                .map(|variant| format!("{}.{}", variant.choice, variant.name))
                .unwrap_or_default();
            format!("Variant {name} ({fields} fields)")
        }
        Op::CallMethod { body, arity } => {
            let name = program
                .body(*body as usize)
                .map(|body| body.name.to_string())
                .unwrap_or_default();
            format!("CallMethod {name} ({arity})")
        }
        Op::MakeFunction(body) => {
            let name = program
                .body(*body as usize)
                .map(|body| body.name.to_string())
                .unwrap_or_default();
            format!("MakeFunction {name}")
        }
        Op::Builtin(builtin) => format!("Builtin {}", builtin.name()),
        Op::ReadFile(layout) => format!("ReadFile {layout}"),
        Op::Now => "Now".to_string(),
        Op::NotYet(feature) => format!("NotYet ({feature})"),
        other => format!("{other:?}"),
    }
}

/// `1 slot`, `2 slots`: a listing is read by people, so it says it in English.
fn count(how_many: usize, thing: &str) -> String {
    match how_many {
        1 => format!("1 {thing}"),
        _ => format!("{how_many} {thing}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disassembly_names_what_an_instruction_works_on() {
        let mut program = Program::with_top_level(Ref::from("the top level"));
        program.constants.push(Value::text("ada"));
        if let Some(body) = program.bodies.first_mut() {
            body.code.push(Op::Constant(0));
            body.spans.push(Span::new(0, 1));
        }
        let listing = program.disassemble();
        assert!(listing.contains("Constant 0 (\"ada\")"), "{listing}");
    }
}
