//! Snapshot tests for every message the checker can produce.
//!
//! These snapshots are the exact text a person sees in their terminal. Read them
//! as prose when reviewing a change: if a message would not help a newcomer work
//! out what to type next, it is wrong, even if the test passes.
//!
//! There is one test per message, and the tests are grouped the way `messages.rs`
//! is, so a message without a test is easy to spot.

mod support;

use insta::assert_snapshot;
use support::{codes, errors};

// ---------------------------------------------------------------------------
// The two messages the specification names
// ---------------------------------------------------------------------------

#[test]
fn a_maybe_where_a_plain_value_was_wanted() {
    let rendered = errors("let ages = {\"Ada\": 36}\nlet x: Int = ages.get(\"Ada\")\n");
    assert!(rendered.contains("expected Int, found maybe Int"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn a_constructor_missing_a_field_says_which() {
    let rendered = errors(
        "type Account {\n\
         \x20   owner: Text\n\
         \x20   balance: Int = 0\n\
         }\n\
         let account = Account.new()\n",
    );
    assert!(rendered.contains("Account.new is missing `owner`."), "{rendered}");
    assert_snapshot!(rendered);
}

// ---------------------------------------------------------------------------
// Types that do not agree
// ---------------------------------------------------------------------------

#[test]
fn an_annotation_that_the_value_does_not_match() {
    assert_snapshot!(errors("let count: Int = \"three\"\n"));
}

#[test]
fn an_argument_of_the_wrong_type() {
    assert_snapshot!(errors(
        "to double(n: Int) returns Int = n * 2\n\
         let twice = double(\"two\")\n"
    ));
}

#[test]
fn a_function_returning_the_wrong_type() {
    assert_snapshot!(errors("to name() returns Text = 1\n"));
}

#[test]
fn branches_of_an_if_that_disagree() {
    assert_snapshot!(errors("let size = if 1 < 2 { \"big\" } else { 0 }\n"));
}

#[test]
fn arms_of_a_match_that_disagree() {
    assert_snapshot!(errors(
        "let answer = match 1 {\n\
         \x20   when 0 then \"zero\"\n\
         \x20   otherwise then 1\n\
         }\n"
    ));
}

#[test]
fn an_if_used_as_a_value_without_an_else() {
    assert_snapshot!(errors("let size: Text = if 1 < 2 { \"big\" }\n"));
}

#[test]
fn an_int_is_not_a_float() {
    assert_snapshot!(errors("let ratio: Float = 1\n"));
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

#[test]
fn a_name_that_was_never_given_a_value() {
    assert_snapshot!(errors("print(totl)\n"));
}

#[test]
fn a_misspelled_name_is_offered_the_right_one() {
    assert_snapshot!(errors("let total = 1\nprint(totl)\n"));
}

#[test]
fn a_type_that_does_not_exist() {
    assert_snapshot!(errors("to weigh(thing: Widget) returns Int = 1\n"));
}

#[test]
fn a_type_used_where_a_value_belongs() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account\n"
    ));
}

#[test]
fn a_type_called_like_a_function() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account(\"Ada\")\n"
    ));
}

#[test]
fn two_declarations_with_the_same_name() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         choice Account {\n\
         \x20   Frozen\n\
         }\n"
    ));
}

#[test]
fn a_type_declared_inside_a_block() {
    assert_snapshot!(errors(
        "to make() {\n\
         \x20   type Account {\n\
         \x20       owner: Text\n\
         \x20   }\n\
         }\n"
    ));
}

#[test]
fn self_outside_a_type() {
    assert_snapshot!(errors("to describe() returns Text = self.name\n"));
}

// ---------------------------------------------------------------------------
// Assignment
// ---------------------------------------------------------------------------

#[test]
fn assigning_to_a_binding_that_is_not_changing() {
    assert_snapshot!(errors("let total = 0\ntotal = 1\n"));
}

#[test]
fn assigning_to_a_field() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   balance: Int = 0\n\
         }\n\
         let changing account = Account.new()\n\
         account.balance = 5\n"
    ));
}

#[test]
fn assigning_to_an_item_of_a_list() {
    assert_snapshot!(errors("let changing numbers = [1, 2]\nnumbers[0] = 3\n"));
}

// ---------------------------------------------------------------------------
// Members
// ---------------------------------------------------------------------------

#[test]
fn a_field_that_does_not_exist() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account.new(owner: \"Ada\")\n\
         print(account.holder)\n"
    ));
}

#[test]
fn a_misspelled_field_in_a_constructor() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account.new(ownr: \"Ada\")\n"
    ));
}

#[test]
fn a_method_that_is_read_without_being_called() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         \n\
         \x20   to describe() returns Text = self.owner\n\
         }\n\
         let account = Account.new(owner: \"Ada\")\n\
         print(account.describe)\n"
    ));
}

#[test]
fn a_built_in_method_on_the_wrong_kind_of_value() {
    assert_snapshot!(errors("print([1, 2].join(\", \"))\n"));
}

#[test]
fn a_member_on_something_that_has_none() {
    assert_snapshot!(errors("let n = 1\nprint(n.size)\n"));
}

#[test]
fn a_static_member_that_is_neither_new_nor_raw() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account.build(owner: \"Ada\")\n"
    ));
}

#[test]
fn raw_used_outside_its_own_type() {
    assert_snapshot!(errors(
        "type Email {\n\
         \x20   text: Text\n\
         \n\
         \x20   to new(text: Text) returns Email or fails Text = success Email.raw(text: text)\n\
         }\n\
         let mail = Email.raw(text: \"not checked\")\n"
    ));
}

#[test]
fn a_constructor_given_an_unnamed_argument() {
    assert_snapshot!(errors(
        "type Account {\n\
         \x20   owner: Text\n\
         }\n\
         let account = Account.new(\"Ada\")\n"
    ));
}

#[test]
fn a_channel_built_without_saying_what_it_carries() {
    assert_snapshot!(errors("let inbox = Channel.new()\n"));
}

#[test]
fn sending_to_something_that_is_not_a_channel() {
    assert_snapshot!(errors("let inbox = [1, 2]\nsend 3 to inbox\n"));
}

#[test]
fn receiving_from_something_that_is_not_a_channel() {
    assert_snapshot!(errors("let inbox = \"ping\"\nlet message = receive from inbox\n"));
}

#[test]
fn closing_something_that_is_not_a_channel() {
    assert_snapshot!(errors("let inbox = 1\nclose inbox\n"));
}

#[test]
fn a_select_arm_waiting_on_something_that_is_not_a_channel() {
    assert_snapshot!(errors(
        "let inbox = 1\n\
         select {\n\
         \x20   when receive from inbox as message { print(message) }\n\
         }\n"
    ));
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[test]
fn a_call_that_leaves_out_an_argument() {
    assert_snapshot!(errors(
        "to greet(name: Text, greeting: Text) returns Text = \"{greeting}, {name}\"\n\
         let hello = greet(\"Ada\")\n"
    ));
}

#[test]
fn a_call_that_names_a_parameter_that_does_not_exist() {
    assert_snapshot!(errors(
        "to greet(name: Text) returns Text = \"hello, {name}\"\n\
         let hello = greet(nme: \"Ada\")\n"
    ));
}

#[test]
fn a_call_that_gives_the_same_argument_twice() {
    assert_snapshot!(errors(
        "to greet(name: Text) returns Text = \"hello, {name}\"\n\
         let hello = greet(name: \"Ada\", name: \"Grace\")\n"
    ));
}

#[test]
fn a_call_with_an_unnamed_argument_after_a_named_one() {
    assert_snapshot!(errors(
        "to greet(name: Text, greeting: Text) returns Text = \"{greeting}, {name}\"\n\
         let hello = greet(greeting: \"hi\", \"Ada\")\n"
    ));
}

#[test]
fn a_call_with_too_many_arguments() {
    assert_snapshot!(errors(
        "to double(n: Int) returns Int = n * 2\n\
         let twice = double(2, 3)\n"
    ));
}

#[test]
fn a_call_on_something_that_is_not_a_function() {
    assert_snapshot!(errors("let total = 1\nlet twice = total(2)\n"));
}

#[test]
fn a_function_value_called_with_too_few_arguments() {
    assert_snapshot!(errors(
        "to add(a: Int, b: Int) returns Int = a + b\n\
         let sum = add\n\
         let answer = sum(1)\n"
    ));
}

#[test]
fn a_function_value_whose_arguments_are_named() {
    assert_snapshot!(errors(
        "to double(n: Int) returns Int = n * 2\n\
         let twice = double\n\
         let answer = twice(n: 2)\n"
    ));
}

#[test]
fn a_closure_with_the_wrong_number_of_parameters() {
    assert_snapshot!(errors("let squares = [1, 2].map((a, b, c) -> a)\n"));
}

#[test]
fn a_closure_with_nowhere_to_take_its_types_from() {
    assert_snapshot!(errors("let double = n -> n * 2\n"));
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn a_condition_that_is_not_a_bool() {
    assert_snapshot!(errors("let count = 1\nif count { print(\"yes\") }\n"));
}

#[test]
fn a_while_condition_that_is_not_a_bool() {
    assert_snapshot!(errors("while 1 { print(\"forever\") }\n"));
}

#[test]
fn a_for_each_over_something_that_is_not_a_sequence() {
    assert_snapshot!(errors("for each n in 1 { print(\"{n}\") }\n"));
}

#[test]
fn a_return_outside_a_function() {
    assert_snapshot!(errors("return 1\n"));
}

#[test]
fn a_function_that_can_finish_without_returning() {
    assert_snapshot!(errors(
        "to describe(n: Int) returns Text {\n\
         \x20   if n > 0 { return \"positive\" }\n\
         }\n"
    ));
}

#[test]
fn a_return_of_the_wrong_type() {
    assert_snapshot!(errors(
        "to describe(n: Int) returns Text {\n\
         \x20   return n\n\
         }\n"
    ));
}

#[test]
fn a_repeat_count_that_is_not_a_number() {
    assert_snapshot!(errors("repeat \"three\" times { print(\"again\") }\n"));
}

// ---------------------------------------------------------------------------
// maybe, try and otherwise
// ---------------------------------------------------------------------------

#[test]
fn try_inside_a_function_that_cannot_fail() {
    assert_snapshot!(errors(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         to twice(text: Text) returns Int = try parse(text) * 2\n"
    ));
}

#[test]
fn try_whose_failure_is_not_the_one_the_function_promises() {
    assert_snapshot!(errors(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         to twice(text: Text) returns Int or fails Int {\n\
         \x20   return success try parse(text) * 2\n\
         }\n"
    ));
}

#[test]
fn try_on_something_that_cannot_fail() {
    assert_snapshot!(errors(
        "to twice(n: Int) returns Int or fails Text {\n\
         \x20   return success try n\n\
         }\n"
    ));
}

#[test]
fn otherwise_on_a_value_that_is_always_there() {
    assert_snapshot!(errors("let count = 1 otherwise 0\n"));
}

#[test]
fn a_fallback_of_the_wrong_type() {
    assert_snapshot!(errors("let ages = {\"Ada\": 36}\nlet age = ages.get(\"Ada\") otherwise \"unknown\"\n"));
}

// ---------------------------------------------------------------------------
// Operators, collections and indexing
// ---------------------------------------------------------------------------

#[test]
fn adding_two_things_that_are_not_numbers() {
    assert_snapshot!(errors("let both = yes + no\n"));
}

#[test]
fn adding_text_to_text_points_at_interpolation() {
    assert_snapshot!(errors("let greeting = \"hello\" + \"world\"\n"));
}

#[test]
fn adding_an_int_to_a_float() {
    assert_snapshot!(errors("let total = 1 + 1.5\n"));
}

#[test]
fn comparing_two_things_of_different_types() {
    assert_snapshot!(errors("let same = 1 == \"one\"\n"));
}

#[test]
fn and_on_something_that_is_not_a_bool() {
    assert_snapshot!(errors("let both = 1 and yes\n"));
}

#[test]
fn not_on_something_that_is_not_a_bool() {
    assert_snapshot!(errors("let opposite = not 1\n"));
}

#[test]
fn negating_something_that_is_not_a_number() {
    assert_snapshot!(errors("let backwards = -\"one\"\n"));
}

#[test]
fn a_list_holding_more_than_one_type() {
    assert_snapshot!(errors("let mixed = [1, \"two\"]\n"));
}

#[test]
fn a_map_holding_more_than_one_type_of_value() {
    assert_snapshot!(errors("let mixed = {\"a\": 1, \"b\": \"two\"}\n"));
}

#[test]
fn an_empty_list_with_nothing_to_say_what_it_holds() {
    assert_snapshot!(errors("let nothing_yet = []\n"));
}

#[test]
fn an_empty_map_with_nothing_to_say_what_it_holds() {
    assert_snapshot!(errors("let nothing_yet = {}\n"));
}

#[test]
fn a_bare_nothing_with_nothing_to_say_what_it_might_have_been() {
    assert_snapshot!(errors("let missing = nothing\n"));
}

#[test]
fn indexing_something_that_is_not_a_list() {
    assert_snapshot!(errors("let count = 1\nprint(count[0])\n"));
}

#[test]
fn indexing_a_list_with_something_that_is_not_a_number() {
    assert_snapshot!(errors("let numbers = [1, 2]\nprint(numbers[\"first\"])\n"));
}

#[test]
fn a_range_over_something_that_is_not_a_number() {
    assert_snapshot!(errors("let span = \"a\"..\"z\"\n"));
}

// ---------------------------------------------------------------------------
// Patterns and match
// ---------------------------------------------------------------------------

#[test]
fn a_match_over_a_choice_that_misses_a_variant() {
    let rendered = errors(
        "choice AccountError {\n\
         \x20   InvalidAmount(amount: Int)\n\
         \x20   Frozen\n\
         }\n\
         let error = AccountError.Frozen\n\
         let message = match error {\n\
         \x20   when AccountError.Frozen then \"frozen\"\n\
         }\n",
    );
    assert!(rendered.contains("AccountError.InvalidAmount"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn a_match_over_a_maybe_that_misses_nothing() {
    assert_snapshot!(errors(
        "let ages = {\"Ada\": 36}\n\
         let message = match ages.get(\"Ada\") {\n\
         \x20   when found age then \"{age}\"\n\
         }\n"
    ));
}

#[test]
fn a_match_over_a_maybe_that_misses_the_value() {
    assert_snapshot!(errors(
        "let ages = {\"Ada\": 36}\n\
         let message = match ages.get(\"Ada\") {\n\
         \x20   when nothing then \"not listed\"\n\
         }\n"
    ));
}

#[test]
fn a_literal_inside_found_does_not_cover_every_value() {
    assert_snapshot!(errors(
        "let message = match found 1 {\n\
         \x20   when found 0 then \"zero\"\n\
         \x20   when nothing then \"none\"\n\
         }\n"
    ));
}

#[test]
fn a_match_over_a_result_that_misses_failure() {
    assert_snapshot!(errors(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         let message = match parse(\"1\") {\n\
         \x20   when success n then \"{n}\"\n\
         }\n"
    ));
}

#[test]
fn a_match_over_a_bool_that_misses_one_half() {
    assert_snapshot!(errors(
        "let ready = yes\n\
         let message = match ready {\n\
         \x20   when yes then \"ready\"\n\
         }\n"
    ));
}

#[test]
fn a_match_over_a_number_without_a_catch_all() {
    assert_snapshot!(errors(
        "let message = match 1 {\n\
         \x20   when 0 then \"zero\"\n\
         \x20   when 1 then \"one\"\n\
         }\n"
    ));
}

#[test]
fn a_guarded_arm_does_not_count_as_covering_its_case() {
    assert_snapshot!(errors(
        "let ready = yes\n\
         let message = match ready {\n\
         \x20   when yes then \"ready\"\n\
         \x20   when no if 1 < 2 then \"not yet\"\n\
         }\n"
    ));
}

#[test]
fn a_pattern_that_cannot_match_what_it_is_given() {
    assert_snapshot!(errors(
        "let message = match 1 {\n\
         \x20   when found n then \"{n}\"\n\
         \x20   otherwise then \"something else\"\n\
         }\n"
    ));
}

#[test]
fn a_literal_pattern_of_the_wrong_type() {
    assert_snapshot!(errors(
        "let message = match 1 {\n\
         \x20   when \"one\" then \"one\"\n\
         \x20   otherwise then \"something else\"\n\
         }\n"
    ));
}

#[test]
fn a_variant_that_the_choice_does_not_have() {
    assert_snapshot!(errors(
        "choice AccountError {\n\
         \x20   Frozen\n\
         }\n\
         let error = AccountError.Frozen\n\
         let message = match error {\n\
         \x20   when AccountError.Melted then \"melted\"\n\
         \x20   otherwise then \"something else\"\n\
         }\n"
    ));
}

#[test]
fn a_variant_constructed_with_a_name_it_does_not_have() {
    assert_snapshot!(errors(
        "choice AccountError {\n\
         \x20   Frozen\n\
         }\n\
         let error = AccountError.Melted\n"
    ));
}

#[test]
fn a_variant_pattern_that_names_the_wrong_number_of_parts() {
    assert_snapshot!(errors(
        "choice AccountError {\n\
         \x20   InvalidAmount(amount: Int)\n\
         }\n\
         let error = AccountError.InvalidAmount(amount: 5)\n\
         let message = match error {\n\
         \x20   when AccountError.InvalidAmount(amount, limit) then \"{amount}\"\n\
         }\n"
    ));
}

#[test]
fn a_variant_path_whose_first_name_is_not_a_choice() {
    assert_snapshot!(errors(
        "choice AccountError {\n\
         \x20   Frozen\n\
         }\n\
         let error = AccountError.Frozen\n\
         let message = match error {\n\
         \x20   when AccountErrors.Frozen then \"frozen\"\n\
         \x20   otherwise then \"something else\"\n\
         }\n"
    ));
}

// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

#[test]
fn a_can_clause_naming_something_that_is_not_an_ability() {
    assert_snapshot!(errors(
        "type Person can Describable {\n\
         \x20   name: Text\n\
         }\n"
    ));
}

#[test]
fn a_type_that_promises_an_ability_and_leaves_out_a_function() {
    assert_snapshot!(errors(
        "ability Describable {\n\
         \x20   to describe() returns Text\n\
         }\n\
         type Person can Describable {\n\
         \x20   name: Text\n\
         }\n"
    ));
}

#[test]
fn a_type_whose_function_does_not_match_the_ability() {
    assert_snapshot!(errors(
        "ability Describable {\n\
         \x20   to describe() returns Text\n\
         }\n\
         type Person can Describable {\n\
         \x20   name: Text\n\
         \n\
         \x20   to describe() returns Int = 1\n\
         }\n"
    ));
}

#[test]
fn an_ability_offers_only_what_it_requires() {
    assert_snapshot!(errors(
        "ability Describable {\n\
         \x20   to describe() returns Text\n\
         }\n\
         to announce(thing: Describable) returns Text = thing.name\n"
    ));
}

// ---------------------------------------------------------------------------
// One mistake makes one message
// ---------------------------------------------------------------------------

#[test]
fn a_bad_argument_is_reported_once() {
    assert_eq!(
        codes(
            "to double(n: Int) returns Int = n * 2\n\
             let twice = double(\"two\") + 1\n"
        ),
        ["type-mismatch"]
    );
}

#[test]
fn an_undefined_name_does_not_cascade() {
    assert_eq!(codes("let total: Int = missing + 1\n"), ["undefined-name"]);
}

#[test]
fn a_missing_field_is_reported_once_however_many_are_missing() {
    assert_eq!(
        codes(
            "type Point {\n\
             \x20   x: Int\n\
             \x20   y: Int\n\
             }\n\
             let origin = Point.new()\n"
        ),
        ["missing-field"]
    );
}

#[test]
fn problems_are_reported_from_the_top_of_the_file_down() {
    let rendered = errors("let first: Int = \"a\"\nlet second: Text = 1\n");
    let first = rendered.find("expected Int").unwrap_or(usize::MAX);
    let second = rendered.find("expected Text").unwrap_or(0);
    assert!(first < second, "messages are out of order:\n{rendered}");
}
