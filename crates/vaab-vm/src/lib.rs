//! The Vaab bytecode compiler and virtual machine.
//!
//! A checked module goes in; a [`Program`] of bytecode comes out, and a
//! [`Machine`] runs it.
//!
//! ```
//! use vaab_vm::{Output, Value};
//!
//! let source = "print(\"hello, {1 + 1}\")\n";
//! let parsed = vaab_syntax::parse(source);
//! let checked = vaab_types::check(&parsed.module).unwrap_or_default();
//!
//! let mut world = vaab_vm::prepare(&parsed.module, &checked, Output::collected());
//! let answer = vaab_vm::run(&mut world);
//!
//! assert!(answer.is_ok());
//! assert_eq!(world.output.lines(), ["hello, 2"]);
//! ```
//!
//! # How it is put together
//!
//! * [`compile`] walks the tree the checker walked and writes instructions. It
//!   never fails: everything a program can get wrong has been reported already,
//!   and everything that is merely not built yet compiles to an instruction that
//!   names the phase bringing it.
//! * [`Machine`] is a value stack and a stack of frames, and nothing else. A Vaab
//!   call is a frame pushed onto it rather than a Rust call, so how deep a Vaab
//!   program goes has nothing to do with the Rust stack it runs on.
//! * [`World`] is everything a machine runs against and does not own: the program,
//!   the file's top-level values, and somewhere for `print` to write.
//!
//! # What phase 4 builds on
//!
//! Phase 4 gives every task its own machine and steps them in turn against one
//! shared world. The three pieces that makes possible are here already:
//!
//! * [`Machine::resume`] runs for a [`Budget`] of instructions and hands back
//!   [`Step::Yielded`] with the machine sitting wherever it got to — halfway down
//!   a call, if that is where the budget ran out. A machine parked on a channel
//!   will stop in the same way, with one more [`Step`] saying what it waits for.
//! * A machine holds no borrow of the world between calls to `resume`, so a
//!   scheduler may own as many as it likes.
//! * [`Value`] keeps every heap value behind one alias, [`Ref`], so that phase 7's
//!   threads are a change to that alias rather than to the machine.

pub mod app_io;
pub mod builtin;
pub mod bytecode;
mod compile;
pub mod json;
mod static_files;
pub mod concurrency;
pub mod error;
pub mod machine;
pub mod value;

use vaab_syntax::Module;
use vaab_types::Checked;

pub use builtin::Builtin;
pub use bytecode::{Body, Capture, Op, Program};
pub use compile::compile;
pub use error::{Fault, Feature, Level, Operation, RuntimeError};
pub use concurrency::Scheduler;
pub use machine::{Budget, HttpResponse, Machine, Output, Step, World};
pub use value::{Closure, Record, Ref, Value, Variant};

/// Compiles a checked module and builds the world it runs in.
pub fn prepare(module: &Module, checked: &Checked, output: Output) -> World {
    World::new(Ref::new(compile(module, checked)), output)
}

/// Runs a whole file, from its first statement to its last.
pub fn run(world: &mut World) -> Result<Value, RuntimeError> {
    concurrency::run(world)
}

/// Runs the top-level statements from `first` onwards, leaving the ones before
/// them alone.
///
/// This is what a REPL needs: a line adds statements to the end of the session,
/// and the values the earlier lines put in the world are still there.
pub fn run_from(world: &mut World, first: usize) -> Result<Value, RuntimeError> {
    let program = Ref::clone(&world.program);
    let start = program.statement_start(first);
    let mut machine = Machine::entering(&program, Program::TOP_LEVEL, start);
    let mut host = concurrency::Host::scratch(&program);
    match machine.resume(world, &mut host, Budget::unlimited()) {
        Step::Finished(value) => Ok(value),
        Step::Failed(error) => Err(*error),
        Step::Yielded | Step::Parked(_) => Err(RuntimeError {
            fault: Fault::Confused("the machine stopped without finishing"),
            trace: Vec::new(),
        }),
    }
}
