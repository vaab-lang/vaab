//! The machine that runs bytecode.
//!
//! A [`Machine`] is an explicit, complete description of a running Vaab program:
//! a value stack and a stack of frames, and nothing else. That matters more than
//! it might look.
//!
//! * A Vaab call is a frame pushed onto [`Machine`], never a Rust call. The depth
//!   a Vaab program reaches has nothing to do with the Rust stack under it, so
//!   deep recursion is a Vaab problem with a Vaab answer — see
//!   [`Fault::TooManyCalls`] — rather than a crash.
//! * [`Machine::resume`] runs for a budget of instructions and then hands back
//!   [`Step::Yielded`], with the machine sitting wherever it got to, halfway down
//!   a call if that is where the budget ran out. Phase 4 owns one machine per task
//!   and steps them in turn; a machine blocked on a channel will park exactly the
//!   same way, with one more [`Step`] to say why.
//!
//! What a machine does *not* hold is the program and the file's top-level values.
//! Those are the [`World`], shared by every machine, which is what lets phase 4's
//! tasks see the same file.

use std::sync::Arc;

use vaab_syntax::span::Span;

use crate::app_io::{Databases, Stores};
use crate::builtin::{self, Builtin};
use crate::bytecode::{Capture, Op, Program, SelectArm};
use crate::concurrency::{OpResult, SelectStep, WaitSite};
use crate::error::{Fault, Level, Operation, RuntimeError};
use crate::value::{Captured, Closure, Record, Ref, Value, Variant};

/// How deep calls may go before Vaab stops and says so.
///
/// Runaway recursion has to end somewhere. Ending it here, with a trace, is the
/// difference between a language that explains itself and one that dies.
const MOST_CALLS: usize = 10_000;

/// How many whole numbers a range may stand for.
///
/// `1..10` is a list, so `1..1_000_000_000_000` would be a list too, and building
/// it would take the machine down with it.
const LONGEST_RANGE: i64 = 10_000_000;

/// Where `print` writes.
#[derive(Debug)]
pub enum Output {
    /// The terminal, which is what `vaab run` and the REPL want.
    Terminal,
    /// Kept in memory, which is what a test wants.
    Collected(Vec<String>),
}

impl Output {
    pub fn collected() -> Output {
        Output::Collected(Vec::new())
    }

    pub fn line(&mut self, text: &str) {
        match self {
            Output::Terminal => println!("{text}"),
            Output::Collected(lines) => lines.push(text.to_string()),
        }
    }

    /// Everything printed so far, for output that was collected.
    pub fn lines(&self) -> &[String] {
        match self {
            Output::Terminal => &[],
            Output::Collected(lines) => lines,
        }
    }
}

/// Everything a machine runs against but does not own: the program, the file's
/// top-level values, and somewhere for `print` to go.
///
/// Phase 4 gives one of these to a whole scheduler full of machines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub content_type: String,
}

impl HttpResponse {
    pub fn json(status: u16, body: String) -> HttpResponse {
        HttpResponse {
            status,
            body,
            content_type: "application/json".to_string(),
        }
    }
}

pub struct World {
    pub program: Ref<Program>,
    /// The file's top-level values, which outlive any one frame.
    pub globals: Vec<Value>,
    pub output: Output,
    pub response: Option<HttpResponse>,
    /// SQLite connections opened by `Db.connect`, shared across request machines.
    pub databases: Arc<Databases>,
    /// Embedded KV stores opened by `Store.open`, shared across request machines.
    pub stores: Arc<Stores>,
    /// Tier-1 Cranelift JIT for hot pure-integer bodies.
    pub jit: crate::jit::JitEngine,
}

impl World {
    pub fn new(program: Ref<Program>, output: Output) -> World {
        Self::with_app_io(program, output, Databases::shared(), Stores::shared())
    }

    pub fn with_databases(program: Ref<Program>, output: Output, databases: Arc<Databases>) -> World {
        Self::with_app_io(program, output, databases, Stores::shared())
    }

    pub fn with_app_io(
        program: Ref<Program>,
        output: Output,
        databases: Arc<Databases>,
        stores: Arc<Stores>,
    ) -> World {
        let globals = vec![Value::Nothing; program.globals];
        World {
            program,
            globals,
            output,
            response: None,
            databases,
            stores,
            jit: crate::jit::JitEngine::default(),
        }
    }

    /// Takes a freshly compiled program, keeping the values the session already
    /// has. A REPL line adds statements to the end of a file, so the top-level
    /// slots it already filled keep their numbers.
    pub fn reload(&mut self, program: Ref<Program>) {
        self.globals.resize(program.globals.max(self.globals.len()), Value::Nothing);
        self.program = program;
    }
}

/// One suspended call.
struct Frame {
    body: usize,
    /// The instruction to run next.
    pc: usize,
    /// Where this frame's slot 0 sits in the value stack.
    base: usize,
    /// `self`, for a method. `Nothing` everywhere else.
    receiver: Value,
    /// The function value this frame is running, for its captures.
    closure: Option<Ref<Closure>>,
}

/// How many instructions a machine may run before it hands control back.
#[derive(Clone, Copy, Debug)]
pub struct Budget(u64);

impl Budget {
    pub fn of(instructions: u64) -> Budget {
        Budget(instructions)
    }

    /// Run until the program is done. `vaab run` has nothing to share with.
    pub fn unlimited() -> Budget {
        Budget(u64::MAX)
    }
}

/// What came of running a machine for a while.
#[derive(Debug)]
pub enum Step {
    /// The budget ran out. The machine is wherever it got to and may be resumed.
    Yielded,
    /// The task is waiting for a channel, a task, or a `together`.
    Parked(WaitSite),
    /// The body the machine started in returned, with this value.
    Finished(Value),
    Failed(Box<RuntimeError>),
}

enum TickAction {
    Continue,
    Done(Value),
    Park(WaitSite),
}

#[derive(Clone, Debug)]
enum Pending {
    Send { channel: u32, value: Value, span: Span },
    Select {
        descriptor: usize,
        channels: Vec<Value>,
        resume_pc: usize,
        span: Span,
    },
}

/// A running Vaab program.
pub struct Machine {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    pending: Option<Pending>,
    together_group: Option<usize>,
}

impl Machine {
    /// A machine about to run a body from a given instruction.
    ///
    /// The REPL uses this to run the statements a line added without running the
    /// ones before it again.
    /// A machine about to run a spawned task or a synchronous `.update` closure.
    pub fn for_body(
        program: &Program,
        body: usize,
        arity: usize,
        closure: Option<Ref<Closure>>,
    ) -> Result<Machine, Fault> {
        let mut machine = Machine {
            stack: Vec::new(),
            frames: Vec::new(),
            pending: None,
            together_group: None,
        };
        machine.enter_call(program, body, arity, Value::Nothing, closure)?;
        Ok(machine)
    }

    pub fn entering(program: &Program, body: usize, pc: usize) -> Machine {
        let slots = program.body(body).map(|body| body.slots).unwrap_or_default();
        Machine {
            stack: vec![Value::Nothing; slots],
            frames: vec![Frame {
                body,
                pc,
                base: 0,
                receiver: Value::Nothing,
                closure: None,
            }],
            pending: None,
            together_group: None,
        }
    }

    pub fn enter_call(
        &mut self,
        program: &Program,
        body: usize,
        arity: usize,
        receiver: Value,
        closure: Option<Ref<Closure>>,
    ) -> Result<(), Fault> {
        self.enter(program, body, arity, receiver, closure)
    }

    pub fn set_argument(&mut self, slot: u32, value: Value) -> Result<(), Fault> {
        let Some(frame) = self.frames.last() else {
            return Err(Fault::Confused("an argument was written with no frame"));
        };
        self.set_slot(frame.base, slot, value)
    }

    /// A machine about to run a whole file.
    pub fn start(program: &Program) -> Machine {
        Machine::entering(program, Program::TOP_LEVEL, 0)
    }

    /// How many calls deep the machine currently is.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Whether the machine has nothing left to do.
    pub fn is_finished(&self) -> bool {
        self.frames.is_empty()
    }

    /// Runs until the program finishes, something goes wrong, or the budget runs
    /// out. A machine that yielded may be resumed as often as it takes.
    pub fn resume(
        &mut self,
        world: &mut World,
        host: &mut crate::concurrency::Host,
        budget: Budget,
    ) -> Step {
        let program = Ref::clone(&world.program);
        let mut left = budget.0;

        if let Some(outcome) = self.retry_pending(world, host) {
            return outcome;
        }

        while left > 0 {
            left -= 1;
            match self.tick(&program, world, host) {
                Ok(TickAction::Continue) => {}
                Ok(TickAction::Done(value)) => return Step::Finished(value),
                Ok(TickAction::Park(site)) => return Step::Parked(site),
                Err(fault) => {
                    return Step::Failed(Box::new(RuntimeError {
                        fault,
                        trace: self.trace(&program),
                    }))
                }
            }
        }
        Step::Yielded
    }

    fn retry_pending(
        &mut self,
        world: &mut World,
        host: &mut crate::concurrency::Host,
    ) -> Option<Step> {
        match self.pending.clone() {
            Some(Pending::Send { channel, value, span }) => {
                if host.take_completed_send(host.current()) {
                    self.pending = None;
                    return None;
                }
                match host.send(channel, value.clone(), span) {
                OpResult::Done => {
                    self.pending = None;
                    None
                }
                OpResult::Park(site) => Some(Step::Parked(site)),
                OpResult::Stop(fault) => Some(Step::Failed(Box::new(RuntimeError {
                    fault,
                    trace: self.trace(&world.program),
                }))),
                OpResult::Ready(_) | OpResult::Failed(_) => Some(Step::Failed(Box::new(RuntimeError {
                    fault: Fault::Confused("a send named an impossible outcome"),
                    trace: self.trace(&world.program),
                }))),
                }
            }
            Some(Pending::Select { descriptor, channels, resume_pc, span }) => {
                let Some(meta) = world.program.selects.get(descriptor) else {
                    return Some(Step::Failed(Box::new(RuntimeError {
                        fault: Fault::Confused("a select named a descriptor that is not there"),
                        trace: self.trace(&world.program),
                    })));
                };
                match self.run_select(meta, descriptor, channels, resume_pc, span, host) {
                    Ok(TickAction::Continue) => None,
                    Ok(TickAction::Done(value)) => Some(Step::Finished(value)),
                    Ok(TickAction::Park(site)) => Some(Step::Parked(site)),
                    Err(fault) => Some(Step::Failed(Box::new(RuntimeError {
                        fault,
                        trace: self.trace(&world.program),
                    }))),
                }
            }
            None => None,
        }
    }

    /// Runs to the end on an existing scheduler, which is what `.update` needs.
    pub fn run_alone(
        &mut self,
        world: &mut World,
        host: &mut crate::concurrency::Host,
    ) -> Result<Value, RuntimeError> {
        match self.resume(world, host, Budget::unlimited()) {
            Step::Finished(value) => Ok(value),
            Step::Failed(error) => Err(*error),
            Step::Yielded | Step::Parked(_) => Err(RuntimeError {
                fault: Fault::Confused("the machine stopped without finishing"),
                trace: Vec::new(),
            }),
        }
    }

    /// Where the machine is, innermost call first.
    pub fn trace(&self, program: &Program) -> Vec<Level> {
        self.frames
            .iter()
            .rev()
            .map(|frame| {
                let body = program.body(frame.body);
                let name = body
                    .map(|body| Ref::clone(&body.name))
                    .unwrap_or_else(|| Ref::from("an unknown body"));
                // `pc` has already moved on, so the instruction that was running is
                // the one before it.
                let at = body
                    .and_then(|body| body.spans.get(frame.pc.saturating_sub(1)))
                    .copied()
                    .unwrap_or_default();
                Level { name, at }
            })
            .collect()
    }

    /// Parks with the instruction pointer back on the operation that must run again.
    fn park_at(&mut self, pc: usize, site: WaitSite) -> TickAction {
        if let Some(frame) = self.frames.last_mut() {
            frame.pc = pc;
        }
        TickAction::Park(site)
    }

    // -----------------------------------------------------------------------
    // One instruction
    // -----------------------------------------------------------------------

    fn tick(
        &mut self,
        program: &Program,
        world: &mut World,
        host: &mut crate::concurrency::Host,
    ) -> Result<TickAction, Fault> {
        let Some(frame) = self.frames.last_mut() else {
            return Err(Fault::Confused("the machine was asked to run with no frame"));
        };
        let body = frame.body;
        let base = frame.base;
        let pc = frame.pc;
        let Some(shape) = program.body(body) else {
            return Err(Fault::Confused("a frame names a body that is not there"));
        };
        let Some(op) = shape.code.get(pc).copied() else {
            return Err(Fault::Confused("the machine ran off the end of a body"));
        };
        let span = shape.spans.get(pc).copied().unwrap_or_default();
        frame.pc += 1;

        match op {
            // -- Values ----------------------------------------------------
            Op::Constant(index) => {
                let Some(value) = program.constants.get(index as usize) else {
                    return Err(Fault::Confused("an instruction named a constant that is not there"));
                };
                self.stack.push(value.clone());
            }
            Op::Int(number) => self.stack.push(Value::Int(number)),
            Op::Bool(held) => self.stack.push(Value::Bool(held)),
            Op::Nothing => self.stack.push(Value::Nothing),
            Op::Absent => self.stack.push(Value::absent()),
            Op::Found => {
                let held = self.pop()?;
                self.stack.push(Value::found(held));
            }
            Op::Success => {
                let held = self.pop()?;
                self.stack.push(Value::success(held));
            }
            Op::Failure => {
                let held = self.pop()?;
                self.stack.push(Value::failure(held));
            }

            // -- Stack -----------------------------------------------------
            Op::Pop => {
                self.pop()?;
            }
            Op::Duplicate => {
                let top = self.peek()?;
                self.stack.push(top);
            }

            // -- Where values live -----------------------------------------
            Op::LoadGlobal(slot) => {
                let Some(value) = world.globals.get(slot as usize) else {
                    return Err(Fault::Confused("a top-level value was read before it existed"));
                };
                self.stack.push(value.clone());
            }
            Op::StoreGlobal(slot) => {
                let value = self.pop()?;
                let Some(held) = world.globals.get_mut(slot as usize) else {
                    return Err(Fault::Confused("a top-level value was written out of range"));
                };
                *held = value;
            }
            Op::LoadLocal(slot) => {
                let value = self.slot(base, slot)?;
                self.stack.push(value);
            }
            Op::StoreLocal(slot) => {
                let value = self.pop()?;
                self.set_slot(base, slot, value)?;
            }
            Op::Box(slot) => {
                let value = self.slot(base, slot)?;
                self.set_slot(base, slot, Value::Captured(Ref::new(Captured::new(value))))?;
            }
            Op::LoadCell(slot) => match self.slot(base, slot)? {
                Value::Captured(cell) => self.stack.push(cell.get()),
                _ => return Err(Fault::Confused("a shared local was not in a cell")),
            },
            Op::StoreCell(slot) => {
                let value = self.pop()?;
                match self.slot(base, slot)? {
                    Value::Captured(cell) => cell.set(value),
                    _ => return Err(Fault::Confused("a shared local was not in a cell")),
                }
            }
            Op::LoadUpvalue(index) => match self.upvalue(index)? {
                Value::Captured(cell) => self.stack.push(cell.get()),
                _ => return Err(Fault::Confused("a captured value was not in a cell")),
            },
            Op::StoreUpvalue(index) => {
                let value = self.pop()?;
                match self.upvalue(index)? {
                    Value::Captured(cell) => cell.set(value),
                    _ => return Err(Fault::Confused("a captured value was not in a cell")),
                }
            }
            Op::LoadSelf => {
                let Some(frame) = self.frames.last() else {
                    return Err(Fault::Confused("`self` was read with no frame"));
                };
                let receiver = frame.receiver.clone();
                self.stack.push(receiver);
            }

            // -- Operators -------------------------------------------------
            Op::Add | Op::Subtract | Op::Multiply | Op::Divide | Op::Remainder => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.stack.push(arithmetic(op, left, right)?);
            }
            Op::Negate => {
                let value = self.pop()?;
                self.stack.push(match value {
                    Value::Int(number) => match number.checked_neg() {
                        Some(negated) => Value::Int(negated),
                        None => return Err(Fault::Overflowed(Operation::Negation)),
                    },
                    Value::Float(number) => Value::Float(-number),
                    _ => return Err(Fault::Confused("`-` reached something that is not a number")),
                });
            }
            Op::Equal | Op::NotEqual => {
                let right = self.pop()?;
                let left = self.pop()?;
                let same = left.equals(&right);
                self.stack.push(Value::Bool(if op == Op::Equal { same } else { !same }));
            }
            Op::Less | Op::LessOrEqual | Op::Greater | Op::GreaterOrEqual => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.stack.push(Value::Bool(ordered(op, &left, &right)?));
            }
            Op::Not => {
                let value = self.pop()?;
                match value {
                    Value::Bool(held) => self.stack.push(Value::Bool(!held)),
                    _ => return Err(Fault::Confused("`not` reached something that is not a Bool")),
                }
            }

            // -- Building --------------------------------------------------
            Op::List(count) => {
                let items = self.take(count as usize)?;
                self.stack.push(Value::list(items));
            }
            Op::Map(count) => {
                let pairs = self.take(count as usize * 2)?;
                self.stack.push(builtin::table(pairs));
            }
            Op::Tuple(count) => {
                let parts = self.take(count as usize)?;
                self.stack.push(Value::Tuple(Ref::new(parts)));
            }
            Op::Range => {
                let end = self.pop()?;
                let start = self.pop()?;
                self.stack.push(range(start, end)?);
            }
            Op::Text(count) => {
                let pieces = self.take(count as usize)?;
                let joined: String = pieces.iter().map(Value::show).collect();
                self.stack.push(Value::text(joined));
            }
            Op::Record { layout, fields } => {
                let values = self.take(fields as usize)?;
                let Some(layout) = program.layouts.get(layout as usize) else {
                    return Err(Fault::Confused("a value was built from a type that is not there"));
                };
                let record = if layout.mutable {
                    Record {
                        layout: Ref::clone(layout),
                        fields: Vec::new(),
                        cells: Some(Ref::new(values.into_iter().map(Captured::new).collect())),
                    }
                } else {
                    Record { layout: Ref::clone(layout), fields: values, cells: None }
                };
                self.stack.push(Value::Record(Ref::new(record)));
            }
            Op::Variant { layout, fields } => {
                let fields = self.take(fields as usize)?;
                let Some(layout) = program.variants.get(layout as usize) else {
                    return Err(Fault::Confused("a variant was built from a choice that is not there"));
                };
                self.stack
                    .push(Value::Variant(Ref::new(Variant { layout: Ref::clone(layout), fields })));
            }

            // -- Reaching inside -------------------------------------------
            Op::Field(index) => {
                let value = self.pop()?;
                let Value::Record(record) = value else {
                    return Err(Fault::Confused("a field was read from something with none"));
                };
                let Some(field) = record.field(index as usize) else {
                    return Err(Fault::Confused("a field was read out of range"));
                };
                self.stack.push(field);
            }
            Op::SetField(index) => {
                let target = self.pop()?;
                let value = self.pop()?;
                let Value::Record(record) = target else {
                    return Err(Fault::Confused("a field was written to something with none"));
                };
                if !record.set_field(index as usize, value) {
                    return Err(Fault::Confused("a field was written out of range"));
                }
            }
            Op::Index => {
                let position = self.pop()?;
                let target = self.pop()?;
                self.stack.push(item_at(&target, &position)?);
            }
            Op::Length => {
                let value = self.pop()?;
                self.stack.push(Value::Int(length_of(&value)? as i64));
            }
            Op::IsEmpty => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(length_of(&value)? == 0));
            }
            Op::Entries => {
                let value = self.pop()?;
                let Value::Map(entries) = value else {
                    return Err(Fault::Confused("a map was expected and something else arrived"));
                };
                let pairs = entries
                    .iter()
                    .map(|(key, value)| {
                        Value::Tuple(Ref::new(vec![key.0.clone(), value.clone()]))
                    })
                    .collect();
                self.stack.push(Value::list(pairs));
            }
            Op::Append(slot) => {
                let value = self.pop()?;
                self.append(base, slot, value)?;
            }

            // -- Taking apart ----------------------------------------------
            Op::IsFound => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(matches!(value, Value::Maybe(Some(_)))));
            }
            Op::IsSuccess => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(matches!(value, Value::Success(_))));
            }
            Op::IsFailure => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(matches!(value, Value::Failure(_))));
            }
            Op::IsVariant(index) => {
                let value = self.pop()?;
                let matched = match value {
                    Value::Variant(variant) => variant.layout.index == index,
                    _ => false,
                };
                self.stack.push(Value::Bool(matched));
            }
            Op::Unwrap => {
                let value = self.pop()?;
                match value {
                    Value::Maybe(Some(held)) | Value::Success(held) | Value::Failure(held) => {
                        self.stack.push(held.as_ref().clone())
                    }
                    _ => return Err(Fault::Confused("there was nothing inside to take out")),
                }
            }
            Op::VariantField(index) => {
                let value = self.pop()?;
                let Value::Variant(variant) = value else {
                    return Err(Fault::Confused("a variant's part was read from something else"));
                };
                let Some(field) = variant.fields.get(index as usize) else {
                    return Err(Fault::Confused("a variant's part was read out of range"));
                };
                self.stack.push(field.clone());
            }
            Op::TupleItem(index) => {
                let value = self.pop()?;
                let Value::Tuple(parts) = value else {
                    return Err(Fault::Confused("part of a group was read from something else"));
                };
                let Some(part) = parts.get(index as usize) else {
                    return Err(Fault::Confused("part of a group was read out of range"));
                };
                self.stack.push(part.clone());
            }
            Op::ListLengthIs { length, at_least } => {
                let value = self.pop()?;
                let held = length_of(&value)?;
                let matched =
                    if at_least { held >= length as usize } else { held == length as usize };
                self.stack.push(Value::Bool(matched));
            }

            // -- Control ---------------------------------------------------
            Op::Jump(target) => self.jump(target)?,
            Op::JumpIfFalse(target) => {
                let condition = self.pop()?;
                if !truth(&condition)? {
                    self.jump(target)?;
                }
            }
            Op::JumpIfTrue(target) => {
                let condition = self.pop()?;
                if truth(&condition)? {
                    self.jump(target)?;
                }
            }
            Op::Return => {
                let value = self.pop()?;
                let Some(frame) = self.frames.pop() else {
                    return Err(Fault::Confused("a return with no frame to return from"));
                };
                self.stack.truncate(frame.base);
                if self.frames.is_empty() {
                    return Ok(TickAction::Done(value));
                }
                self.stack.push(value);
            }

            // -- Calls -----------------------------------------------------
            Op::Call(arity) => {
                let callee = self.lift(arity as usize)?;
                match callee {
                    Value::Function(closure) => {
                        let body = closure.body as usize;
                        let shape = program.body(body);
                        if closure.captures.is_empty() {
                            if let Some(shape) = shape {
                                if let Some(result) = crate::jit::try_native_call(
                                    &mut world.jit,
                                    program,
                                    body,
                                    shape.parameters,
                                    &self.stack,
                                    arity as usize,
                                ) {
                                    self.stack.truncate(self.stack.len() - arity as usize);
                                    self.stack.push(Value::Int(result));
                                    return Ok(TickAction::Continue);
                                }
                            }
                        }
                        self.enter(program, body, arity as usize, Value::Nothing, Some(closure))?;
                    }
                    Value::Builtin(builtin) => self.apply(builtin, arity as usize, world)?,
                    _ => return Err(Fault::Confused("a call reached something that is not a function")),
                }
            }
            Op::CallMethod { body, arity } => {
                let receiver = self.lift(arity as usize)?;
                self.enter(program, body as usize, arity as usize, receiver, None)?;
            }
            Op::CallAbility { ability, slot, arity } => {
                let receiver = self.lift(arity as usize)?;
                let body = ability_body(&receiver, ability, slot)?;
                self.enter(program, body, arity as usize, receiver, None)?;
            }
            Op::Builtin(builtin) => {
                let arity = builtin.arity();
                self.apply(builtin, arity, world)?;
            }
            Op::MakeFunction(body) => {
                let value = self.make_function(program, body as usize, base)?;
                self.stack.push(value);
            }

            Op::ChannelNew => {
                let capacity = self.pop()?;
                let Value::Int(capacity) = capacity else {
                    return Err(Fault::Confused("a channel size was not a whole number"));
                };
                self.stack.push(host.channel_new(capacity));
            }
            Op::SharedNew => {
                let value = self.pop()?;
                self.stack.push(host.shared_new(value));
            }
            Op::Send => {
                let channel_value = self.pop()?;
                let value = self.pop()?;
                let Some(channel) = crate::concurrency::Host::channel_id(&channel_value) else {
                    return Err(Fault::Confused("a send reached something that is not a channel"));
                };
                match host.send(channel, value.clone(), span) {
                    OpResult::Done => {}
                    OpResult::Park(site) => {
                        self.pending = Some(Pending::Send { channel, value, span });
                        return Ok(TickAction::Park(site));
                    }
                    OpResult::Stop(fault) => return Err(fault),
                    OpResult::Ready(_) | OpResult::Failed(_) => {
                        return Err(Fault::Confused("a send named an impossible outcome"));
                    }
                }
            }
            Op::Receive => {
                let channel_value = self.stack.last().ok_or(Fault::Confused(
                    "a receive was reached with nothing on the stack",
                ))?;
                let Some(channel) = crate::concurrency::Host::channel_id(channel_value) else {
                    return Err(Fault::Confused("a receive reached something that is not a channel"));
                };
                match host.receive(channel, span) {
                    OpResult::Ready(value) => {
                        self.stack.pop();
                        self.stack.push(value);
                    }
                    OpResult::Park(site) => return Ok(self.park_at(pc, site)),
                    OpResult::Stop(fault) => return Err(fault),
                    OpResult::Done | OpResult::Failed(_) => {
                        return Err(Fault::Confused("a receive named an impossible outcome"));
                    }
                }
            }
            Op::Close => {
                let channel_value = self.pop()?;
                let Some(channel) = crate::concurrency::Host::channel_id(&channel_value) else {
                    return Err(Fault::Confused("a close reached something that is not a channel"));
                };
                match host.close(channel) {
                    OpResult::Done => {}
                    OpResult::Stop(fault) => return Err(fault),
                    OpResult::Park(_) | OpResult::Ready(_) | OpResult::Failed(_) => {
                        return Err(Fault::Confused("a close named an impossible outcome"));
                    }
                }
            }
            Op::Start(body) => {
                let function = self.make_function(program, body as usize, base)?;
                match host.spawn(function) {
                    Ok(handle) => self.stack.push(handle),
                    Err(error) => return Err(error.fault),
                }
            }
            Op::TaskWait => {
                let task_value = self.stack.last().ok_or(Fault::Confused(
                    "a wait was reached with nothing on the stack",
                ))?;
                let task = match task_value {
                    Value::Task(id) => *id,
                    _ => return Err(Fault::Confused("a wait reached something that is not a task")),
                };
                match host.wait_task(task, span) {
                    OpResult::Ready(value) => {
                        self.stack.pop();
                        self.stack.push(value);
                    }
                    OpResult::Park(site) => return Ok(self.park_at(pc, site)),
                    OpResult::Stop(fault) => return Err(fault),
                    OpResult::Failed(error) => return Err(error.fault),
                    OpResult::Done => return Err(Fault::Confused("a wait named an impossible outcome")),
                }
            }
            Op::SharedRead => {
                let shared = self.pop()?;
                let Value::Shared(held) = shared else {
                    return Err(Fault::Confused("`.value` reached something that is not shared"));
                };
                self.stack.push(held.get().clone());
            }
            Op::SharedUpdate => {
                let change = self.pop()?;
                let shared = self.pop()?;
                match host.run_shared_update(&shared, change, world) {
                    Ok(value) => self.stack.push(value),
                    Err(error) => return Err(error.fault),
                }
            }
            Op::BeginTogether => {
                let group = host.begin_together();
                self.together_group = Some(group);
            }
            Op::EndTogether => {
                let group = self.together_group.take().unwrap_or(0);
                let task = host.current();
                match host.end_together(group, task) {
                    OpResult::Done => {}
                    OpResult::Park(site) => return Ok(self.park_at(pc, site)),
                    OpResult::Stop(fault) => return Err(fault),
                    OpResult::Failed(error) => return Err(error.fault),
                    OpResult::Ready(_) => {
                        return Err(Fault::Confused("a together named an impossible outcome"));
                    }
                }
            }
            Op::Select(index) => {
                let descriptor = index as usize;
                let Some(meta) = program.selects.get(descriptor) else {
                    return Err(Fault::Confused("a select named a descriptor that is not there"));
                };
                let receive_count = meta
                    .arms
                    .iter()
                    .filter(|arm| matches!(arm, SelectArm::Receive { .. }))
                    .count();
                let channels = if receive_count > 0 { self.take(receive_count)? } else { Vec::new() };
                return self.run_select(meta, descriptor, channels, pc, span, host);
            }
            Op::ReadFile(layout) => {
                let path = self.pop()?;
                let text = match &path {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("read_file expected a path")),
                };
                match std::fs::read_to_string(&text) {
                    Ok(contents) => self.stack.push(Value::success(Value::text(contents))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("read_file named a variant that is not there"),
                        )?;
                        let variant =
                            Value::Variant(Ref::new(Variant { layout, fields: vec![path] }));
                        self.stack.push(Value::failure(variant));
                    }
                    Err(_) => return Err(Fault::Confused("could not read the file")),
                }
            }
            Op::Now => {
                let seconds = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .map_err(|_| Fault::Confused("the clock could not be read"))?;
                self.stack.push(Value::Int(seconds));
            }

            Op::EnvGet => {
                let name = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("env.get expected a name")),
                };
                self.stack.push(crate::app_io::env_get(&name));
            }
            Op::EnvRequired(layout) => {
                let name = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("env.required expected a name")),
                };
                let layout = program.variants.get(layout as usize).cloned().ok_or(
                    Fault::Confused("env.required named a variant that is not there"),
                )?;
                self.stack.push(crate::app_io::env_required(&name, layout));
            }
            Op::DbConnect(layout) => {
                let url = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("Db.connect expected a url")),
                };
                match world.databases.connect(&url) {
                    Ok(handle) => self.stack.push(Value::success(Value::Db(handle))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("Db.connect named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::DbExecute(layout) => {
                let args = match self.pop()? {
                    Value::List(items) => items.as_ref().clone(),
                    _ => return Err(Fault::Confused("db.execute expected a list of arguments")),
                };
                let sql = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("db.execute expected sql text")),
                };
                let handle = match self.pop()? {
                    Value::Db(handle) => handle,
                    _ => return Err(Fault::Confused("db.execute expected a database")),
                };
                match world.databases.execute(handle, &sql, &args) {
                    Ok(changed) => self.stack.push(Value::success(Value::Int(changed))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("db.execute named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::DbQuery(layout) => {
                let args = match self.pop()? {
                    Value::List(items) => items.as_ref().clone(),
                    _ => return Err(Fault::Confused("db.query expected a list of arguments")),
                };
                let sql = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("db.query expected sql text")),
                };
                let handle = match self.pop()? {
                    Value::Db(handle) => handle,
                    _ => return Err(Fault::Confused("db.query expected a database")),
                };
                match world.databases.query(handle, &sql, &args) {
                    Ok(rows) => self.stack.push(Value::success(rows)),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("db.query named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::StoreOpen(layout) => {
                let path = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("Store.open expected a path")),
                };
                match world.stores.open(&path) {
                    Ok(handle) => self.stack.push(Value::success(Value::Store(handle))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("Store.open named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::StoreGet => {
                let key = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("store.get expected a key")),
                };
                let handle = match self.pop()? {
                    Value::Store(handle) => handle,
                    _ => return Err(Fault::Confused("store.get expected a store")),
                };
                match world.stores.get(handle, &key) {
                    Ok(value) => self.stack.push(value),
                    Err(_) => return Err(Fault::Confused("store.get could not read")),
                }
            }
            Op::StoreSet(layout) => {
                let value = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("store.set expected a value")),
                };
                let key = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("store.set expected a key")),
                };
                let handle = match self.pop()? {
                    Value::Store(handle) => handle,
                    _ => return Err(Fault::Confused("store.set expected a store")),
                };
                match world.stores.set(handle, &key, &value) {
                    Ok(()) => self.stack.push(Value::success(Value::Int(1))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("store.set named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::StoreRemove(layout) => {
                let key = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("store.remove expected a key")),
                };
                let handle = match self.pop()? {
                    Value::Store(handle) => handle,
                    _ => return Err(Fault::Confused("store.remove expected a store")),
                };
                match world.stores.remove(handle, &key) {
                    Ok(removed) => {
                        let count = if removed { 1 } else { 0 };
                        self.stack.push(Value::success(Value::Int(count)));
                    }
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("store.remove named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::StoreKeys(layout) => {
                let prefix = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("store.keys expected a prefix")),
                };
                let handle = match self.pop()? {
                    Value::Store(handle) => handle,
                    _ => return Err(Fault::Confused("store.keys expected a store")),
                };
                match world.stores.keys(handle, &prefix) {
                    Ok(keys) => {
                        self.stack.push(Value::success(Value::list(
                            keys.into_iter().map(Value::text).collect(),
                        )))
                    }
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("store.keys named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::HttpGet(layout) => {
                let url = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.get expected a url")),
                };
                match crate::app_io::http_get(&url) {
                    Ok(body) => self.stack.push(Value::success(Value::text(body))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("http.get named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::HttpPost(layout) => {
                let body = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.post expected a body")),
                };
                let url = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.post expected a url")),
                };
                match crate::app_io::http_post(&url, &body) {
                    Ok(response) => self.stack.push(Value::success(Value::text(response))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("http.post named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::HttpSend(layout) => {
                let headers_value = self.pop()?;
                let headers = crate::app_io::headers_from_value(&headers_value)?;
                let body = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.send expected a body")),
                };
                let url = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.send expected a url")),
                };
                let method = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("http.send expected a method")),
                };
                match crate::app_io::http_send(&method, &url, &body, &headers) {
                    Ok(response) => self.stack.push(Value::success(Value::text(response))),
                    Err(message) => {
                        let layout = program.variants.get(layout as usize).cloned().ok_or(
                            Fault::Confused("http.send named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_message(layout, message));
                    }
                }
            }
            Op::RequestWho { user, unauthorized } => {
                let request = self.slot(base, 0)?;
                let headers = match &request {
                    Value::Tuple(parts) if parts.len() >= 4 => {
                        crate::app_io::headers_from_value(&parts[3])?
                    }
                    _ => return Err(Fault::Confused("request.who needs request headers")),
                };
                let layout = program.layouts.get(user as usize).cloned().ok_or(
                    Fault::Confused("request.who needs a User type"),
                )?;
                match crate::app_io::verify_bearer(&headers, layout) {
                    Ok(record) => self.stack.push(Value::success(record)),
                    Err(()) => {
                        let layout = program.variants.get(unauthorized as usize).cloned().ok_or(
                            Fault::Confused("request.who named a variant that is not there"),
                        )?;
                        self.stack.push(crate::app_io::failure_unit(layout));
                    }
                }
            }

            Op::ReplyWith(has_status) => {
                let status = if has_status {
                    match self.pop()? {
                        Value::Int(number) if number > 0 && number <= u16::MAX as i64 => {
                            number as u16
                        }
                        _ => return Err(Fault::Confused("reply status must be a whole number")),
                    }
                } else {
                    200
                };
                let value = self.pop()?;
                let body = crate::json::encode(&value)?;
                world.response = Some(HttpResponse::json(status, body));
            }
            Op::ReplyFile(has_status) => {
                let status = if has_status {
                    match self.pop()? {
                        Value::Int(number) if number > 0 && number <= u16::MAX as i64 => {
                            number as u16
                        }
                        _ => return Err(Fault::Confused("reply status must be a whole number")),
                    }
                } else {
                    200
                };
                let path = match self.pop()? {
                    Value::Text(path) => path.to_string(),
                    _ => return Err(Fault::Confused("reply file expected a path")),
                };
                world.response = Some(crate::static_files::read_response(&path, status));
            }
            Op::ReplyText(has_status) => {
                let status = if has_status {
                    match self.pop()? {
                        Value::Int(number) if number > 0 && number <= u16::MAX as i64 => {
                            number as u16
                        }
                        _ => return Err(Fault::Confused("reply status must be a whole number")),
                    }
                } else {
                    200
                };
                let content_type = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("reply text expected a content type")),
                };
                let body = match self.pop()? {
                    Value::Text(text) => text.to_string(),
                    _ => return Err(Fault::Confused("reply text expected a body")),
                };
                world.response = Some(HttpResponse {
                    status,
                    body,
                    content_type,
                });
            }
            Op::ReplyExplain => {
                let value = self.pop()?;
                let body = crate::json::encode(&value)?;
                world.response = Some(HttpResponse::json(400, body));
            }

            // -- Stopping --------------------------------------------------
            Op::NotYet(feature) => return Err(Fault::NotYet(feature)),
            Op::NoArm => return Err(Fault::NoArmApplied),
        }

        Ok(TickAction::Continue)
    }

    fn run_select(
        &mut self,
        meta: &crate::bytecode::SelectDescriptor,
        descriptor: usize,
        channels: Vec<Value>,
        resume_pc: usize,
        span: Span,
        host: &mut crate::concurrency::Host,
    ) -> Result<TickAction, Fault> {
        let task = host.current();
        match host.try_select(task, meta, &channels, resume_pc) {
            SelectStep::Arm { arm, value } => {
                self.pending = None;
                if let Some(SelectArm::Receive { .. }) = meta.arms.get(arm) {
                    if let Some(held) = value {
                        self.stack.push(Value::found(held));
                    } else {
                        self.stack.push(Value::absent());
                    }
                }
                if let Some(SelectArm::Receive { body } | SelectArm::Timeout { body, .. }) = meta.arms.get(arm) {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.pc = *body as usize;
                    }
                }
                Ok(TickAction::Continue)
            }
            SelectStep::Otherwise => {
                self.pending = None;
                if let Some(target) = meta.otherwise {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.pc = target as usize;
                    }
                }
                Ok(TickAction::Continue)
            }
            SelectStep::Park { resume_pc } => {
                self.pending = Some(Pending::Select {
                    descriptor,
                    channels,
                    resume_pc,
                    span,
                });
                Ok(TickAction::Park(WaitSite::Select { descriptor, resume_pc }))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Working the stack
    // -----------------------------------------------------------------------

    fn pop(&mut self) -> Result<Value, Fault> {
        self.stack.pop().ok_or(Fault::Confused("the value stack ran dry"))
    }

    fn peek(&self) -> Result<Value, Fault> {
        self.stack.last().cloned().ok_or(Fault::Confused("the value stack ran dry"))
    }

    /// Takes the top `count` values, in the order they were pushed.
    fn take(&mut self, count: usize) -> Result<Vec<Value>, Fault> {
        let Some(from) = self.stack.len().checked_sub(count) else {
            return Err(Fault::Confused("the value stack ran dry"));
        };
        Ok(self.stack.split_off(from))
    }

    /// Takes the value sitting just below `arity` arguments: a callee, or the
    /// receiver of a method.
    fn lift(&mut self, arity: usize) -> Result<Value, Fault> {
        let Some(position) = self.stack.len().checked_sub(arity + 1) else {
            return Err(Fault::Confused("a call had nothing to call"));
        };
        if position >= self.stack.len() {
            return Err(Fault::Confused("a call had nothing to call"));
        }
        Ok(self.stack.remove(position))
    }

    fn slot(&self, base: usize, slot: u32) -> Result<Value, Fault> {
        self.stack
            .get(base + slot as usize)
            .cloned()
            .ok_or(Fault::Confused("a local was read out of range"))
    }

    fn set_slot(&mut self, base: usize, slot: u32, value: Value) -> Result<(), Fault> {
        let Some(held) = self.stack.get_mut(base + slot as usize) else {
            return Err(Fault::Confused("a local was written out of range"));
        };
        *held = value;
        Ok(())
    }

    /// Adds to the list in a slot without copying it, which is what keeps `.map`
    /// over a long list from being quadratic.
    fn append(&mut self, base: usize, slot: u32, value: Value) -> Result<(), Fault> {
        let Some(held) = self.stack.get_mut(base + slot as usize) else {
            return Err(Fault::Confused("a list was added to out of range"));
        };
        let Value::List(items) = held else {
            return Err(Fault::Confused("something that is not a list was added to"));
        };
        match Ref::get_mut(items) {
            Some(items) => items.push(value),
            // Somebody else is holding this list, so it is copied rather than
            // changed underneath them.
            None => {
                let mut copy = items.as_ref().clone();
                copy.push(value);
                *held = Value::list(copy);
            }
        }
        Ok(())
    }

    fn upvalue(&self, index: u32) -> Result<Value, Fault> {
        let Some(frame) = self.frames.last() else {
            return Err(Fault::Confused("a captured value was read with no frame"));
        };
        frame
            .closure
            .as_ref()
            .and_then(|closure| closure.captures.get(index as usize))
            .cloned()
            .ok_or(Fault::Confused("a captured value was read out of range"))
    }

    fn jump(&mut self, target: u32) -> Result<(), Fault> {
        let Some(frame) = self.frames.last_mut() else {
            return Err(Fault::Confused("a jump with no frame to jump in"));
        };
        frame.pc = target as usize;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Calls
    // -----------------------------------------------------------------------

    /// Pushes a frame. This is the whole of what calling costs.
    fn enter(
        &mut self,
        program: &Program,
        body: usize,
        arity: usize,
        receiver: Value,
        closure: Option<Ref<Closure>>,
    ) -> Result<(), Fault> {
        if self.frames.len() >= MOST_CALLS {
            return Err(Fault::TooManyCalls { depth: self.frames.len() });
        }
        let Some(shape) = program.body(body) else {
            return Err(Fault::Confused("a call named a body that is not there"));
        };
        let Some(base) = self.stack.len().checked_sub(arity) else {
            return Err(Fault::Confused("a call was made with too few arguments on the stack"));
        };
        // Whatever the arguments did not fill is a slot the body has yet to use.
        self.stack.resize(base + shape.slots.max(arity), Value::Nothing);
        self.frames.push(Frame { body, pc: 0, base, receiver, closure });
        Ok(())
    }

    fn apply(&mut self, builtin: Builtin, arity: usize, world: &mut World) -> Result<(), Fault> {
        let arguments = self.take(arity)?;
        if builtin == Builtin::Print {
            let Some(value) = arguments.first() else {
                return Err(Fault::Confused("`print` was given nothing to show"));
            };
            world.output.line(&value.show());
            self.stack.push(Value::Nothing);
            return Ok(());
        }
        let answer = builtin.apply(&arguments)?;
        self.stack.push(answer);
        Ok(())
    }

    fn make_function(
        &mut self,
        program: &Program,
        body: usize,
        base: usize,
    ) -> Result<Value, Fault> {
        let Some(shape) = program.body(body) else {
            return Err(Fault::Confused("a function was built from a body that is not there"));
        };
        let mut captures = Vec::with_capacity(shape.captures.len());
        for capture in &shape.captures {
            captures.push(match capture {
                Capture::Local(slot) => self.slot(base, *slot)?,
                Capture::Upvalue(index) => self.upvalue(*index)?,
            });
        }
        Ok(Value::Function(Ref::new(Closure {
            body: body as u32,
            name: Ref::clone(&shape.name),
            captures,
        })))
    }
}

// ---------------------------------------------------------------------------
// The work each instruction does
// ---------------------------------------------------------------------------

fn truth(value: &Value) -> Result<bool, Fault> {
    match value {
        Value::Bool(held) => Ok(*held),
        // Vaab has no truthiness, so a jump only ever tests a real `Bool`.
        _ => Err(Fault::Confused("a decision was made on something that is not a Bool")),
    }
}

fn arithmetic(op: Op, left: Value, right: Value) -> Result<Value, Fault> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => whole(op, left, right),
        (Value::Float(left), Value::Float(right)) => decimal(op, left, right),
        _ => Err(Fault::Confused("arithmetic reached something that is not a number")),
    }
}

/// Whole numbers, checked every time.
///
/// "The only way a program stops abnormally is an unrecoverable runtime error" —
/// so a sum that does not fit is one of those, rather than a number nobody meant.
fn whole(op: Op, left: i64, right: i64) -> Result<Value, Fault> {
    let (answer, operation) = match op {
        Op::Add => (left.checked_add(right), Operation::Sum),
        Op::Subtract => (left.checked_sub(right), Operation::Difference),
        Op::Multiply => (left.checked_mul(right), Operation::Product),
        Op::Divide => {
            if right == 0 {
                return Err(Fault::DividedByZero(Operation::Division));
            }
            (left.checked_div(right), Operation::Division)
        }
        Op::Remainder => {
            if right == 0 {
                return Err(Fault::DividedByZero(Operation::Remainder));
            }
            (left.checked_rem(right), Operation::Remainder)
        }
        _ => return Err(Fault::Confused("an operator arrived where arithmetic was expected")),
    };
    answer.map(Value::Int).ok_or(Fault::Overflowed(operation))
}

fn decimal(op: Op, left: f64, right: f64) -> Result<Value, Fault> {
    // A Float divided by zero is an error too, rather than the infinity IEEE
    // would give: Vaab promises one meaning for dividing by zero, not two.
    if matches!(op, Op::Divide | Op::Remainder) && right == 0.0 {
        let operation =
            if op == Op::Divide { Operation::Division } else { Operation::Remainder };
        return Err(Fault::DividedByZero(operation));
    }
    Ok(Value::Float(match op {
        Op::Add => left + right,
        Op::Subtract => left - right,
        Op::Multiply => left * right,
        Op::Divide => left / right,
        Op::Remainder => left % right,
        _ => return Err(Fault::Confused("an operator arrived where arithmetic was expected")),
    }))
}

fn ordered(op: Op, left: &Value, right: &Value) -> Result<bool, Fault> {
    let order = match (left, right) {
        (Value::Int(left), Value::Int(right)) => left.partial_cmp(right),
        (Value::Float(left), Value::Float(right)) => left.partial_cmp(right),
        (Value::Text(left), Value::Text(right)) => left.as_ref().partial_cmp(right.as_ref()),
        _ => return Err(Fault::Confused("a comparison reached values it cannot order")),
    };
    let Some(order) = order else {
        // Only a `NaN` gets here, and a `NaN` is neither side of anything.
        return Ok(false);
    };
    Ok(match op {
        Op::Less => order.is_lt(),
        Op::LessOrEqual => order.is_le(),
        Op::Greater => order.is_gt(),
        Op::GreaterOrEqual => order.is_ge(),
        _ => return Err(Fault::Confused("an operator arrived where a comparison was expected")),
    })
}

fn length_of(value: &Value) -> Result<usize, Fault> {
    match value {
        Value::List(items) | Value::Tuple(items) => Ok(items.len()),
        Value::Map(entries) => Ok(entries.len()),
        // Text is measured in characters, because that is what someone reading it
        // counts.
        Value::Text(text) => Ok(text.chars().count()),
        _ => Err(Fault::Confused("something without a length was measured")),
    }
}

fn item_at(target: &Value, position: &Value) -> Result<Value, Fault> {
    let Value::List(items) = target else {
        return Err(Fault::Confused("something that is not a list was indexed"));
    };
    let Value::Int(index) = position else {
        return Err(Fault::Confused("a list was indexed by something that is not a number"));
    };
    let found = usize::try_from(*index).ok().and_then(|index| items.get(index));
    match found {
        Some(item) => Ok(item.clone()),
        None => Err(Fault::IndexOutOfRange { index: *index, length: items.len() }),
    }
}

/// `1..10`, which is the list of whole numbers it stands for.
fn range(start: Value, end: Value) -> Result<Value, Fault> {
    let (Value::Int(start), Value::Int(end)) = (start, end) else {
        return Err(Fault::Confused("a range was made from something that is not a number"));
    };
    if end < start {
        return Ok(Value::list(Vec::new()));
    }
    let Some(length) = end.checked_sub(start).and_then(|span| span.checked_add(1)) else {
        return Err(Fault::RangeTooLong { length: i64::MAX });
    };
    if length > LONGEST_RANGE {
        return Err(Fault::RangeTooLong { length });
    }
    Ok(Value::list((start..=end).map(Value::Int).collect()))
}

/// Which body a call through an ability runs, which only the value can say.
fn ability_body(receiver: &Value, ability: u32, slot: u32) -> Result<usize, Fault> {
    let Value::Record(record) = receiver else {
        return Err(Fault::Confused("an ability was used on something that does not provide it"));
    };
    let found = record
        .layout
        .tables
        .get(ability as usize)
        .and_then(|table| table.get(slot as usize))
        .copied()
        .unwrap_or_default();
    if found == 0 {
        return Err(Fault::Confused("a type promised an ability and had no function for it"));
    }
    Ok(found as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sum_that_does_not_fit_is_reported_rather_than_wrapped() {
        let answer = whole(Op::Add, i64::MAX, 1);
        assert!(matches!(answer, Err(Fault::Overflowed(Operation::Sum))));
    }

    #[test]
    fn dividing_by_zero_stops_whether_the_numbers_are_whole_or_not() {
        assert!(matches!(whole(Op::Divide, 1, 0), Err(Fault::DividedByZero(_))));
        assert!(matches!(decimal(Op::Divide, 1.0, 0.0), Err(Fault::DividedByZero(_))));
    }

    #[test]
    fn a_backwards_range_holds_nothing() {
        let empty = range(Value::Int(5), Value::Int(1));
        assert!(matches!(empty, Ok(Value::List(items)) if items.is_empty()));
    }

    #[test]
    fn a_range_nobody_could_hold_is_reported() {
        let huge = range(Value::Int(0), Value::Int(i64::MAX));
        assert!(matches!(huge, Err(Fault::RangeTooLong { .. })));
    }

    #[test]
    fn reaching_past_the_end_of_a_list_says_how_long_it_is() {
        let list = Value::list(vec![Value::Int(1), Value::Int(2)]);
        let missing = item_at(&list, &Value::Int(5));
        assert!(matches!(missing, Err(Fault::IndexOutOfRange { index: 5, length: 2 })));
    }

    #[test]
    fn a_negative_position_is_out_of_range_rather_than_counting_backwards() {
        let list = Value::list(vec![Value::Int(1)]);
        assert!(matches!(item_at(&list, &Value::Int(-1)), Err(Fault::IndexOutOfRange { .. })));
    }

    #[test]
    fn text_is_measured_in_characters() {
        assert_eq!(length_of(&Value::text("héllo")), Ok(5));
    }
}
