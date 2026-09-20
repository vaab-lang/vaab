//! The standard library this phase builds.
//!
//! There is one test per name in the checker's prelude that has an implementation
//! behind it, so a signature added to `vaab-types` with nothing to run would show
//! up here as a missing test rather than as a program that quietly does nothing.

mod support;

use support::{output, printed};

// ---------------------------------------------------------------------------
// print
// ---------------------------------------------------------------------------

#[test]
fn print_shows_a_value_of_any_type_at_all() {
    let source = "print(1)\nprint(\"two\")\nprint(yes)\nprint([1])\nprint((1, 2))\n";
    assert_eq!(output(source), ["1", "two", "yes", "[1]", "(1, 2)"]);
}

#[test]
fn print_shows_text_as_it_is_and_quotes_it_inside_a_list() {
    assert_eq!(output("print(\"a\")\nprint([\"a\"])\n"), ["a", "[\"a\"]"]);
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn upper_and_lower_change_the_case() {
    assert_eq!(output("print(\"ada\".upper())\nprint(\"ADA\".lower())\n"), ["ADA", "ada"]);
}

#[test]
fn contains_says_whether_one_piece_of_text_is_inside_another() {
    let source = "print(\"ada@example.com\".contains(\"@\"))\nprint(\"nope\".contains(\"@\"))\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn text_is_empty_when_it_has_no_characters() {
    assert_eq!(output("print(\"\".is_empty)\nprint(\"a\".is_empty)\n"), ["yes", "no"]);
}

#[test]
fn the_length_of_text_counts_characters_rather_than_bytes() {
    assert_eq!(printed("print(\"héllo\".length)\n"), "5");
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn map_gives_back_a_list_the_same_length_as_the_one_it_walked() {
    assert_eq!(printed("print([1, 2, 3].map(n -> n * 2))\n"), "[2, 4, 6]");
}

#[test]
fn map_over_an_empty_list_gives_an_empty_list() {
    assert_eq!(printed("let items: list of Int = []\nprint(items.map(n -> n * 2))\n"), "[]");
}

#[test]
fn each_runs_its_closure_once_for_every_item() {
    assert_eq!(output("[1, 2].each(n -> print(n))\n"), ["1", "2"]);
}

#[test]
fn a_list_is_empty_when_it_holds_nothing() {
    let source = "let items: list of Int = []\nprint(items.is_empty)\nprint([1].is_empty)\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn count_says_how_many_items_a_list_holds() {
    assert_eq!(printed("print([1, 2, 3].count)\n"), "3");
}

#[test]
fn the_first_of_a_list_is_a_maybe_because_the_list_may_be_empty() {
    let source = "let items: list of Int = []\nprint([1, 2].first)\nprint(items.first)\n";
    assert_eq!(output(source), ["found 1", "nothing"]);
}

#[test]
fn join_puts_a_separator_between_every_piece_of_text() {
    assert_eq!(printed("print([\"a\", \"b\", \"c\"].join(\", \"))\n"), "a, b, c");
}

#[test]
fn joining_one_piece_of_text_adds_no_separator() {
    assert_eq!(printed("print([\"only\"].join(\", \"))\n"), "only");
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

#[test]
fn get_hands_back_a_maybe_because_the_key_may_not_be_there() {
    let source = "let ages = {\"Ada\": 36}\nprint(ages.get(\"Ada\"))\nprint(ages.get(\"Grace\"))\n";
    assert_eq!(output(source), ["found 36", "nothing"]);
}

#[test]
fn a_map_is_empty_when_it_holds_nothing() {
    let source = "let ages: map of Text to Int = {}\nprint(ages.is_empty)\n\
                  print({\"a\": 1}.is_empty)\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn count_says_how_many_entries_a_map_holds() {
    assert_eq!(printed("print({\"a\": 1, \"b\": 2}.count)\n"), "2");
}

#[test]
fn keys_come_back_in_the_order_they_were_first_put_in() {
    assert_eq!(printed("print({\"b\": 1, \"a\": 2}.keys)\n"), "[\"b\", \"a\"]");
}

#[test]
fn a_map_can_be_keyed_by_something_other_than_text() {
    assert_eq!(printed("let names = {1: \"one\"}\nprint(names.get(1) otherwise \"?\")\n"), "one");
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn abs_drops_the_sign_of_a_whole_number_and_of_a_decimal() {
    // A method binds tighter than a minus sign, so the brackets are the ones
    // anyone writing this would have to put in.
    assert_eq!(output("print((-7).abs())\nprint((-1.5).abs())\n"), ["7", "1.5"]);
}

#[test]
fn min_and_max_pick_between_two_whole_numbers() {
    assert_eq!(output("print(3.min(7))\nprint(3.max(7))\n"), ["3", "7"]);
}

#[test]
fn a_whole_number_becomes_a_decimal_only_when_asked() {
    assert_eq!(printed("print(3.to_float() / 2.0)\n"), "1.5");
}

#[test]
fn rounding_goes_to_the_nearest_whole_number() {
    assert_eq!(output("print(2.4.round())\nprint(2.6.round())\n"), ["2", "3"]);
}

#[test]
fn a_half_rounds_away_from_zero_the_way_it_is_taught_at_school() {
    assert_eq!(output("print(2.5.round())\nprint((-2.5).round())\n"), ["3", "-3"]);
}
