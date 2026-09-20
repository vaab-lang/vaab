//! Shared helpers for the machine's test suites.
//!
//! Each test binary compiles its own copy of this module and uses only the
//! helpers it needs, so unused ones here are expected rather than a sign of rot.
#![allow(dead_code)]

use vaab_syntax::{diagnostic, ColorChoice};
use vaab_types::Checked;
use vaab_vm::{error, Output, Value};

/// Parses and checks `source`, asserting that both are happy.
pub fn ready(source: &str) -> (vaab_syntax::Module, Checked) {
    let parsed = vaab_syntax::parse(source);
    assert!(
        !parsed.has_errors(),
        "this test's source does not parse:\n{}",
        diagnostic::render(&parsed.diagnostics, "test.vaab", source, ColorChoice::Never)
    );
    match vaab_types::check(&parsed.module) {
        Ok(checked) => (parsed.module, checked),
        Err(problems) => panic!(
            "this test's source does not type-check:\n{}",
            diagnostic::render(&problems, "test.vaab", source, ColorChoice::Never)
        ),
    }
}

/// Runs `source`, asserting that it finishes, and gives back what it printed.
pub fn output(source: &str) -> Vec<String> {
    let (module, checked) = ready(source);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(_) => world.output.lines().to_vec(),
        Err(problem) => panic!(
            "expected this to run, but it stopped:\n{}",
            error::render(&problem, "test.vaab", source, ColorChoice::Never)
        ),
    }
}

/// The one line `source` printed. Most tests print exactly one thing.
pub fn printed(source: &str) -> String {
    let lines = output(source);
    assert_eq!(lines.len(), 1, "expected one line, got {lines:?}");
    lines.into_iter().next().unwrap_or_default()
}

/// The value `source` ends with, which is its last expression.
pub fn value(source: &str) -> Value {
    let (module, checked) = ready(source);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(value) => value,
        Err(problem) => panic!(
            "expected this to run, but it stopped:\n{}",
            error::render(&problem, "test.vaab", source, ColorChoice::Never)
        ),
    }
}

/// Runs `source`, asserting that it stops, and returns the rendered report:
/// exactly what a person would see in their terminal.
pub fn stops(source: &str) -> String {
    let (module, checked) = ready(source);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(value) => panic!("expected this to stop, but it finished with {value}"),
        Err(problem) => error::render(&problem, "example.vaab", source, ColorChoice::Never),
    }
}

/// A fault written out as a report, for the ones no program can reach.
pub fn rendered(fault: vaab_vm::Fault, source: &str, at: vaab_syntax::span::Span) -> String {
    let problem = vaab_vm::RuntimeError {
        fault,
        trace: vec![vaab_vm::Level { name: vaab_vm::Ref::from("the top level"), at }],
    };
    error::render(&problem, "example.vaab", source, ColorChoice::Never)
}

/// The compiled bytecode of `source`, written out.
pub fn listing(source: &str) -> String {
    let (module, checked) = ready(source);
    vaab_vm::compile(&module, &checked).disassemble()
}
