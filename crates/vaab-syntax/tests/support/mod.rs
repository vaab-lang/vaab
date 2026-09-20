//! Shared helpers for the parser test suites.
//!
//! Each test binary compiles its own copy of this module and uses only the
//! helpers it needs, so unused ones here are expected rather than a sign of rot.
#![allow(dead_code)]

use vaab_syntax::{diagnostic, ColorChoice};

/// Parses `source`, asserting that it is accepted, and returns the printed tree.
pub fn tree(source: &str) -> String {
    let parsed = vaab_syntax::parse(source);
    assert!(
        !parsed.has_errors(),
        "expected this to parse cleanly, but it did not:\n{}",
        diagnostic::render(&parsed.diagnostics, "test.vaab", source, ColorChoice::Never)
    );
    vaab_syntax::print_module(&parsed.module)
}

/// Parses `source`, asserting that it is rejected, and returns the rendered
/// diagnostics: exactly what a person would see in their terminal.
pub fn errors(source: &str) -> String {
    let parsed = vaab_syntax::parse(source);
    assert!(
        parsed.has_errors(),
        "expected this to be rejected, but it parsed cleanly:\n{}",
        vaab_syntax::print_module(&parsed.module)
    );
    diagnostic::render(&parsed.diagnostics, "example.vaab", source, ColorChoice::Never)
}

/// The stable slugs of every diagnostic `source` produces, in order.
pub fn codes(source: &str) -> Vec<&'static str> {
    vaab_syntax::parse(source).diagnostics.iter().map(|d| d.code).collect()
}
