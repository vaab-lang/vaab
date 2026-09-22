//! Snapshot tests for every way a running program can stop.
//!
//! These snapshots are the exact text a person sees in their terminal. Read them
//! as prose when reviewing a change: a message that leaves a newcomer no wiser
//! about what to change is wrong, even though the test passes.
//!
//! There is one test per message in `error.rs`, so a message without a test is
//! easy to spot.

mod support;

use insta::assert_snapshot;
use support::stops;
use vaab_syntax::span::Span;
use vaab_vm::Fault;

// ---------------------------------------------------------------------------
// Numbers that do not fit
// ---------------------------------------------------------------------------

#[test]
fn a_sum_too_large_for_a_whole_number() {
    let rendered = stops("let big = 9_223_372_036_854_775_807\nprint(big + 1)\n");
    assert!(rendered.contains("does not fit"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn a_difference_too_small_for_a_whole_number() {
    assert_snapshot!(stops("let small = -9_223_372_036_854_775_807\nprint(small - 2)\n"));
}

#[test]
fn a_product_too_large_for_a_whole_number() {
    assert_snapshot!(stops("let big = 4_000_000_000\nprint(big * big)\n"));
}

#[test]
fn a_negation_that_has_no_answer() {
    // The smallest Int cannot be written down, because the minus sign is an
    // operator and the number after it is one past the largest Int.
    assert_snapshot!(stops("let smallest = -9_223_372_036_854_775_807 - 1\nprint(-smallest)\n"));
}

#[test]
fn the_one_division_whose_answer_does_not_fit() {
    let source = "let smallest = -9_223_372_036_854_775_807 - 1\nprint(smallest / -1)\n";
    assert_snapshot!(stops(source));
}

#[test]
fn the_one_remainder_whose_answer_does_not_fit() {
    let source = "let smallest = -9_223_372_036_854_775_807 - 1\nprint(smallest % -1)\n";
    assert_snapshot!(stops(source));
}

#[test]
fn the_size_of_the_smallest_whole_number() {
    let source = "let smallest = -9_223_372_036_854_775_807 - 1\nprint(smallest.abs())\n";
    assert_snapshot!(stops(source));
}

#[test]
fn rounding_a_decimal_too_large_to_be_a_whole_number() {
    assert_snapshot!(stops("let huge = 99_999_999_999_999_999_999_999.0\nprint(huge.round())\n"));
}

// ---------------------------------------------------------------------------
// Dividing by zero
// ---------------------------------------------------------------------------

#[test]
fn dividing_a_whole_number_by_zero() {
    let rendered = stops("let none = 0\nprint(10 / none)\n");
    assert!(rendered.contains("this divides by zero"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn dividing_a_decimal_by_zero() {
    assert_snapshot!(stops("let none = 0.0\nprint(10.0 / none)\n"));
}

#[test]
fn asking_for_the_remainder_after_dividing_by_zero() {
    assert_snapshot!(stops("let none = 0\nprint(10 % none)\n"));
}

// ---------------------------------------------------------------------------
// Reaching outside a list
// ---------------------------------------------------------------------------

#[test]
fn reaching_past_the_end_of_a_list() {
    let rendered = stops("let names = [\"Ada\", \"Grace\"]\nprint(names[5])\n");
    assert!(rendered.contains("this list has no item at 5"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn reaching_into_an_empty_list() {
    assert_snapshot!(stops("let names: list of Text = []\nprint(names[0])\n"));
}

#[test]
fn reaching_before_the_start_of_a_list() {
    assert_snapshot!(stops("let names = [\"Ada\"]\nlet back = 0 - 1\nprint(names[back])\n"));
}

#[test]
fn reaching_past_the_end_of_a_list_inside_a_function() {
    let source = "to at(names: list of Text, position: Int) returns Text = names[position]\n\
                  print(at([\"Ada\"], 9))\n";
    assert_snapshot!(stops(source));
}

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

#[test]
fn a_range_holding_more_numbers_than_anyone_could_want() {
    let rendered = stops("let far = 1_000_000_000\nfor each n in 1..far { print(n) }\n");
    assert!(rendered.contains("too many numbers"), "{rendered}");
    assert_snapshot!(rendered);
}

// ---------------------------------------------------------------------------
// Calls that never come back
// ---------------------------------------------------------------------------

#[test]
fn a_function_that_calls_itself_for_ever() {
    let rendered = stops("to forever(n: Int) returns Int = forever(n + 1)\nprint(forever(0))\n");
    assert!(rendered.contains("too many calls inside one another"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn two_functions_that_call_each_other_for_ever() {
    let source = "to ping(n: Int) returns Int = pong(n + 1)\n\
                  to pong(n: Int) returns Int = ping(n + 1)\n\
                  print(ping(0))\n";
    assert_snapshot!(stops(source));
}

// ---------------------------------------------------------------------------
// The two the checker makes unreachable
//
// A `match` in a program the checker accepted always has an arm that applies, and
// the machine only ever calls itself confused about its own state. Both are
// safety nets rather than things a program can do, so they are rendered here
// directly: the point of the test is that the words are worth reading if a bug in
// Vaab ever does reach them.
// ---------------------------------------------------------------------------

#[test]
fn a_match_where_nothing_applied() {
    assert_snapshot!(support::rendered(
        Fault::NoArmApplied,
        "let answer = match 1 {\n\x20   when 1 then \"one\"\n}\n",
        Span::new(13, 21),
    ));
}

#[test]
fn a_machine_that_cannot_explain_itself() {
    assert_snapshot!(support::rendered(
        Fault::Confused("a frame names a body that is not there"),
        "print(1)\n",
        Span::new(0, 8),
    ));
}

// ---------------------------------------------------------------------------
// Things a later phase brings
// ---------------------------------------------------------------------------

#[test]
fn reading_a_missing_file_fails_with_file_error() {
    let source = "\
        match read_file(\"missing.vaab\") {\n\
        when success _ then print(\"found\")\n\
        when failure _ then print(\"missing\")\n\
        }\n";
    assert_snapshot!(support::printed(source));
}

#[test]
fn starting_a_task_returns_what_the_body_returned() {
    let source = "to work() returns Int = 1\nlet job = start { work() }\nprint(job.wait())\n";
    assert_snapshot!(support::printed(source));
}

#[test]
fn sending_to_a_closed_channel_stops() {
    let source = "let jobs = Channel.new(of: Int, size: 1)\nclose jobs\nsend 1 to jobs\n";
    assert_snapshot!(stops(source));
}

#[test]
fn shared_state_can_be_read_after_an_update() {
    let source = "let total = Shared.new(0)\ntotal.update(n -> n + 1)\nprint(total.value)\n";
    assert_snapshot!(support::printed(source));
}

#[test]
fn together_waits_for_tasks_started_inside_it() {
    assert_snapshot!(support::output("together {\n\x20   start { print(\"working\") }\n}\nprint(\"done\")\n").join("\n"));
}

#[test]
fn select_with_an_otherwise_arm_runs_without_blocking() {
    let source = "select {\n\
                  \x20   when timeout after 1 seconds { print(\"all quiet\") }\n\
                  \x20   otherwise { print(\"nothing ready\") }\n\
                  }\n";
    assert_snapshot!(support::printed(source));
}

// ---------------------------------------------------------------------------
// The trace
// ---------------------------------------------------------------------------

#[test]
fn a_fault_deep_in_a_program_names_every_call_that_led_to_it() {
    let source = "to innermost(n: Int) returns Int = n / 0\n\
                  to middle(n: Int) returns Int = innermost(n)\n\
                  to outermost(n: Int) returns Int = middle(n)\n\
                  print(outermost(1))\n";
    let rendered = stops(source);
    assert!(rendered.contains("innermost"), "{rendered}");
    assert!(rendered.contains("outermost"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn a_fault_inside_a_closure_says_it_was_inside_a_closure() {
    assert_snapshot!(stops("let none = 0\nprint([1, 2].map(n -> n / none))\n"));
}

#[test]
fn a_fault_inside_a_method_names_the_type_the_method_belongs_to() {
    let source = "type Rate {\n\
                  \x20   per_hour: Int\n\n\
                  \x20   to per(minutes: Int) returns Int = self.per_hour / minutes\n\
                  }\n\
                  print(Rate.new(per_hour: 60).per(0))\n";
    assert_snapshot!(stops(source));
}
