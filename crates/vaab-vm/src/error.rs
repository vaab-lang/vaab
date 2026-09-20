//! What stops a Vaab program, and how it is reported.
//!
//! "The only way a program stops abnormally is an unrecoverable runtime error."
//! So there are no exceptions to catch and nothing to recover from: a [`Fault`] is
//! the end of the run, and the job here is to explain it as well as a type error
//! is explained. Every one of these goes through the same `ariadne` machinery the
//! parser and the checker use, with a call trace underneath.

use std::fmt;

use vaab_syntax::diagnostic::{self, ColorChoice, Diagnostic};
use vaab_syntax::span::{LineColumn, Span};

use crate::value::Ref;

/// Which calculation overflowed, named the way the message wants to say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Sum,
    Difference,
    Product,
    Division,
    Remainder,
    Negation,
    Size,
    Rounding,
}

impl Operation {
    fn noun(self) -> &'static str {
        match self {
            Operation::Sum => "sum",
            Operation::Difference => "difference",
            Operation::Product => "product",
            Operation::Division => "division",
            Operation::Remainder => "remainder",
            Operation::Negation => "negation",
            Operation::Size => "size",
            Operation::Rounding => "rounding",
        }
    }
}

/// Something the language has a shape for and the runtime does not run yet.
///
/// A program mentioning one of these compiles; reaching it at run time is a
/// report naming the phase that brings it, which is the same promise `vaab run`
/// makes on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    ReadFile,
    Channels,
    Tasks,
    SharedState,
    Select,
    Together,
}

impl Feature {
    /// The subject of the sentence, and the verb that agrees with it.
    fn subject(self) -> (&'static str, &'static str) {
        match self {
            Feature::ReadFile => ("`read_file`", "arrives"),
            Feature::Channels => ("channels", "arrive"),
            Feature::Tasks => ("tasks", "arrive"),
            Feature::SharedState => ("`shared` state", "arrives"),
            Feature::Select => ("`select`", "arrives"),
            Feature::Together => ("`together`", "arrives"),
        }
    }

    fn phase(self) -> u8 {
        match self {
            Feature::ReadFile => 5,
            Feature::Channels
            | Feature::Tasks
            | Feature::SharedState
            | Feature::Select
            | Feature::Together => 4,
        }
    }

    fn help(self) -> &'static str {
        match self {
            Feature::ReadFile => {
                "phase 5 builds the standard library, and file reading comes with it"
            }
            _ => "phase 4 builds the scheduler that tasks and channels run on",
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (subject, verb) = self.subject();
        write!(f, "{subject} {verb} in phase {}", self.phase())
    }
}

/// What went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Vaab checks every calculation instead of wrapping round.
    Overflowed(Operation),
    DividedByZero(Operation),
    IndexOutOfRange { index: i64, length: usize },
    /// A range is a list, so one nobody could hold cannot be built.
    RangeTooLong { length: i64 },
    TooManyCalls { depth: usize },
    /// Every `match` a checked program can run has an arm that applies, so this
    /// says the machine and the checker have fallen out of step.
    NoArmApplied,
    NotYet(Feature),
    /// The machine found its own state impossible. Reported rather than panicked,
    /// because a compiler that crashes teaches nobody anything.
    Confused(&'static str),
}

/// One level of a call trace: a function, and where inside it the machine was.
#[derive(Clone, Debug)]
pub struct Level {
    pub name: Ref<str>,
    pub at: Span,
}

/// A fault, with the calls that led to it. The first level is where it happened.
#[derive(Clone, Debug)]
pub struct RuntimeError {
    pub fault: Fault,
    pub trace: Vec<Level>,
}

impl RuntimeError {
    /// Where the fault happened.
    pub fn span(&self) -> Span {
        self.trace.first().map(|level| level.at).unwrap_or_default()
    }

    /// The fault as a diagnostic, in the voice the rest of the compiler uses.
    pub fn diagnostic(&self) -> Diagnostic {
        let at = self.span();
        match &self.fault {
            Fault::Overflowed(operation) => Diagnostic::error(
                "integer-overflow",
                format!("this {} does not fit in an Int", operation.noun()),
            )
            .at(at, "this went past what an Int can hold")
            .with_help(
                "an Int is 64 bits, so it holds whole numbers from \
                 -9223372036854775808 to 9223372036854775807",
            )
            .with_note("Vaab checks every calculation rather than quietly wrapping round"),

            Fault::DividedByZero(Operation::Remainder) => {
                Diagnostic::error("divide-by-zero", "this takes a remainder by zero")
                    .at(at, "the right-hand side is 0")
                    .with_help("check the right-hand side first, as in `if step != 0 { ... }`")
            }
            Fault::DividedByZero(_) => Diagnostic::error("divide-by-zero", "this divides by zero")
                .at(at, "the right-hand side is 0")
                .with_help("check the divisor first, as in `if count != 0 { ... }`"),

            Fault::IndexOutOfRange { index, length } => {
                let held = match length {
                    0 => "the list is empty".to_string(),
                    1 => "the list holds 1 item".to_string(),
                    _ => format!("the list holds {length} items"),
                };
                let diagnostic = Diagnostic::error(
                    "index-out-of-range",
                    format!("this list has no item at {index}"),
                )
                .at(at, held);
                if *index < 0 {
                    diagnostic.with_help(
                        "positions count forwards from 0, so one below 0 never reaches \
                         an item",
                    )
                } else if *length == 0 {
                    diagnostic.with_help(
                        "the list is empty, so there is nothing to reach for: check \
                         `items.is_empty` first",
                    )
                } else {
                    diagnostic.with_help(format!(
                        "positions start at 0, so the last item is at {}; `items.count` \
                         says how many there are",
                        length - 1
                    ))
                }
            }

            Fault::RangeTooLong { length } => {
                Diagnostic::error("range-too-long", "this range holds too many numbers to build")
                    .at(at, format!("this stands for {length} whole numbers"))
                    .with_help(
                        "a range is a list, so it is built all at once: count with a \
                         `while` loop instead when there are this many",
                    )
            }

            Fault::TooManyCalls { depth } => {
                Diagnostic::error("too-many-calls", "too many calls inside one another")
                    .at(at, format!("this call is {depth} deep"))
                    .with_help("a function that calls itself needs a case that stops it")
                    .with_note(
                        "Vaab stops here so that runaway recursion is a report rather than \
                         a crash",
                    )
            }

            Fault::NoArmApplied => {
                Diagnostic::error("no-arm-applied", "no arm of this `match` applied")
                    .at(at, "nothing here matched the value")
                    .with_help("add an `otherwise` arm, which matches whatever is left")
            }

            Fault::NotYet(feature) => Diagnostic::error("not-yet", feature.to_string())
                .at(at, "this is not built yet")
                .with_help(feature.help())
                .with_note("it type-checks today, so the program is ready for the phase that runs it"),

            Fault::Confused(detail) => {
                Diagnostic::error("confused", "the Vaab machine found a state it cannot explain")
                    .at(at, "this is where it stopped")
                    .with_help("this is a bug in Vaab itself, not in the program")
                    .with_note(*detail)
            }
        }
    }
}

/// Draws a runtime error the way a type error is drawn, with the calls underneath.
pub fn render(error: &RuntimeError, file: &str, source: &str, color: ColorChoice) -> String {
    let mut out = diagnostic::render_one(&error.diagnostic(), file, source, color);
    out.push_str(&trace(&error.trace, file, source));
    out
}

/// How many of the innermost and outermost calls a trace shows before it gives up
/// and counts the rest. A recursion that ran away has thousands of identical
/// lines, and nobody reads the middle of those.
const NEAREST: usize = 6;
const FURTHEST: usize = 3;

fn trace(levels: &[Level], file: &str, source: &str) -> String {
    // With one level the snippet above has already said where the program was.
    if levels.len() < 2 {
        return String::new();
    }

    let mut out = String::from("Trace:\n");
    let line = |level: &Level| {
        let LineColumn { line, column } = LineColumn::of(source, level.at.start);
        format!("  in {} at {file}:{line}:{column}\n", level.name)
    };

    if levels.len() <= NEAREST + FURTHEST + 1 {
        for level in levels {
            out.push_str(&line(level));
        }
        return out;
    }

    let hidden = levels.len() - NEAREST - FURTHEST;
    for level in &levels[..NEAREST] {
        out.push_str(&line(level));
    }
    out.push_str(&format!("  ... {hidden} more calls ...\n"));
    for level in &levels[levels.len() - FURTHEST..] {
        out.push_str(&line(level));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(name: &str, at: usize) -> Level {
        Level { name: Ref::from(name), at: Span::new(at, at + 1) }
    }

    #[test]
    fn a_feature_says_which_phase_brings_it() {
        assert_eq!(Feature::ReadFile.to_string(), "`read_file` arrives in phase 5");
        assert_eq!(Feature::Channels.to_string(), "channels arrive in phase 4");
    }

    #[test]
    fn one_level_needs_no_trace_because_the_snippet_said_it_all() {
        assert_eq!(trace(&[level("the top level", 0)], "main.vaab", "let a = 1\n"), "");
    }

    #[test]
    fn a_long_trace_counts_the_calls_it_leaves_out() {
        let source = "\n".repeat(40);
        let levels: Vec<Level> = (0..40).map(|line| level("`fib`", line)).collect();
        let drawn = trace(&levels, "main.vaab", &source);
        assert!(drawn.contains("... 31 more calls ..."), "{drawn}");
        assert_eq!(drawn.lines().count(), 1 + NEAREST + 1 + FURTHEST);
    }
}
