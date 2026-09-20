//! Snapshot tests for error messages.
//!
//! These snapshots are the exact text a person sees in their terminal. Read them
//! as prose when reviewing a change: if a message would not help a newcomer work
//! out what to type next, it is wrong, even if the test passes.

mod support;

use insta::assert_snapshot;
use support::{codes, errors};

// ---------------------------------------------------------------------------
// Characters Vaab does not use
// ---------------------------------------------------------------------------

#[test]
fn a_semicolon_is_pointed_out_gently() {
    assert_snapshot!(errors("let count = 1;\n"));
}

#[test]
fn bang_suggests_the_word_not() {
    assert_snapshot!(errors("if !ready { stop() }\n"));
}

#[test]
fn double_ampersand_suggests_the_word_and() {
    assert_snapshot!(errors("let both = a && b\n"));
}

#[test]
fn double_slash_is_not_a_comment() {
    assert_snapshot!(errors("// a note\n"));
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn text_that_never_closes() {
    assert_snapshot!(errors("let greeting = \"hello\n"));
}

#[test]
fn a_hole_that_never_closes() {
    assert_snapshot!(errors("print(\"Ada is {age\")\n"));
}

#[test]
fn a_hole_with_nothing_in_it() {
    assert_snapshot!(errors("print(\"nothing here: {}\")\n"));
}

#[test]
fn a_hole_holding_more_than_one_value() {
    assert_snapshot!(errors("print(\"{a b}\")\n"));
}

#[test]
fn an_escape_vaab_does_not_know() {
    assert_snapshot!(errors(r#"print("what is \q")"#));
}

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

#[test]
fn a_let_with_no_value() {
    assert_snapshot!(errors("let count\n"));
}

#[test]
fn a_reserved_word_used_as_a_name() {
    assert_snapshot!(errors("let to = 1\n"));
}

#[test]
fn soft_keywords_are_not_reserved() {
    // The counterpart to the test above: these must parse, because `list` and
    // `each` are only keywords inside a type.
    let parsed = vaab_syntax::parse("let list = [1, 2]\nlet each = 3\nlet map = 4\n");
    assert!(!parsed.has_errors(), "soft keywords should still work as names");
}

#[test]
fn assigning_to_something_that_is_not_a_place() {
    assert_snapshot!(errors("a + b = 3\n"));
}

// ---------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------

#[test]
fn a_parameter_with_no_type() {
    assert_snapshot!(errors("to greet(name) returns Text = name\n"));
}

#[test]
fn a_field_with_no_type() {
    assert_snapshot!(errors("type Account {\n    owner\n}\n"));
}

#[test]
fn a_field_declared_twice() {
    assert_snapshot!(errors("type Account {\n    owner: Text\n    owner: Int\n}\n"));
}

#[test]
fn or_without_fails() {
    assert_snapshot!(errors("to risky() returns Int or Text = 1\n"));
}

#[test]
fn an_ability_whose_function_has_a_body() {
    assert_snapshot!(errors(
        "ability Describable {\n    to describe() returns Text = \"hi\"\n}\n"
    ));
}

#[test]
fn a_choice_with_no_variants() {
    assert_snapshot!(errors("choice AccountError {\n}\n"));
}

#[test]
fn a_variant_listed_twice() {
    assert_snapshot!(errors("choice AccountError {\n    Frozen\n    Frozen\n}\n"));
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[test]
fn comparisons_cannot_be_chained() {
    assert_snapshot!(errors("let ok = low < value < high\n"));
}

#[test]
fn the_left_of_an_arrow_must_be_names() {
    assert_snapshot!(errors("numbers.map(1 -> 2)\n"));
}

#[test]
fn a_number_too_large_for_an_int() {
    assert_snapshot!(errors("let huge = 99999999999999999999\n"));
}

#[test]
fn a_match_with_no_arms() {
    assert_snapshot!(errors("match value {\n}\n"));
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_cannot_interpolate() {
    assert_snapshot!(errors(
        "match name {\n    when \"{greeting}\" then 1\n    otherwise then 2\n}\n"
    ));
}

#[test]
fn the_rest_marker_must_come_last() {
    assert_snapshot!(errors(
        "match items {\n    when [..., last] then last\n    otherwise then 0\n}\n"
    ));
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

#[test]
fn a_block_that_is_never_closed() {
    assert_snapshot!(errors("to work() {\n    print(\"hello\")\n"));
}

#[test]
fn a_closing_brace_with_nothing_to_close() {
    assert_snapshot!(errors("let a = 1\n}\n"));
}

#[test]
fn two_statements_on_one_line() {
    assert_snapshot!(errors("let a = 1 let b = 2\n"));
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

#[test]
fn one_broken_line_does_not_hide_the_next() {
    // Three lines have a mistake and one does not. Each mistake is reported once,
    // in the order a person reads the file, even though the first and last were
    // found while scanning and the middle one while parsing.
    assert_eq!(
        codes("let a = 1;\nlet b\nlet c = 2\nlet d = e && f\n"),
        ["no-semicolons", "let-without-value", "unknown-character"]
    );
}

#[test]
fn a_broken_function_does_not_swallow_the_one_after_it() {
    let source = "to first(a) returns Int = a\nto second(b: Int) returns Int = b\n";
    let parsed = vaab_syntax::parse(source);
    assert_eq!(parsed.diagnostics.len(), 1, "only the first function is broken");
    assert_eq!(parsed.module.statements.len(), 1, "the second function should survive");
}
