//! Shared helpers for the type checker's test suites.
//!
//! Each test binary compiles its own copy of this module and uses only the
//! helpers it needs, so unused ones here are expected rather than a sign of rot.
#![allow(dead_code)]

use vaab_syntax::{diagnostic, ColorChoice};
use vaab_types::{Checked, Type};

/// Parses `source`, asserting that it parses, and hands back the module.
fn parsed(source: &str) -> vaab_syntax::Module {
    let parsed = vaab_syntax::parse(source);
    assert!(
        !parsed.has_errors(),
        "this test's source does not parse:\n{}",
        diagnostic::render(&parsed.diagnostics, "test.vaab", source, ColorChoice::Never)
    );
    parsed.module
}

/// Checks `source`, asserting that it is accepted.
pub fn check(source: &str) -> Checked {
    match vaab_types::check(&parsed(source)) {
        Ok(checked) => checked,
        Err(problems) => panic!(
            "expected this to check cleanly, but it did not:\n{}",
            diagnostic::render(&problems, "example.vaab", source, ColorChoice::Never)
        ),
    }
}

/// Checks `source`, asserting that it is rejected, and returns the rendered
/// diagnostics: exactly what a person would see in their terminal.
pub fn errors(source: &str) -> String {
    match vaab_types::check(&parsed(source)) {
        Ok(_) => panic!("expected this to be rejected, but it checked cleanly:\n{source}"),
        Err(problems) => {
            diagnostic::render(&problems, "example.vaab", source, ColorChoice::Never)
        }
    }
}

/// The stable slugs of every diagnostic `source` produces, in order.
pub fn codes(source: &str) -> Vec<&'static str> {
    match vaab_types::check(&parsed(source)) {
        Ok(_) => Vec::new(),
        Err(problems) => problems.iter().map(|problem| problem.code).collect(),
    }
}

/// The type of the value `let answer = ...` was given, as Vaab spells it.
///
/// Most tests about inference only care about one type, and naming the binding
/// `answer` keeps them reading as one line of Vaab plus one assertion.
pub fn type_of_answer(source: &str) -> String {
    let checked = check(source);
    let local = checked
        .locals
        .iter()
        .find(|local| local.name == "answer")
        .expect("the source should declare `answer`");
    local.declared.to_string()
}

/// Asserts that `let answer = ...` has the type `expected`.
pub fn assert_answer(source: &str, expected: &str) {
    assert_eq!(type_of_answer(source), expected, "in:\n{source}");
}

/// Every type the checker recorded, as Vaab spells them, deduplicated and sorted.
/// Useful for asserting that nothing unresolved reached the tables.
pub fn recorded_types(checked: &Checked) -> Vec<String> {
    let mut found: Vec<String> =
        checked.types.values().map(Type::to_string).collect::<Vec<String>>();
    found.sort();
    found.dedup();
    found
}
