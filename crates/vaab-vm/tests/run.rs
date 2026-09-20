//! What each part of the language does when it runs.
//!
//! One small test per idea, in the order `docs/LANGUAGE.md` introduces them, so
//! that a feature with no test here is easy to spot.

mod support;

use support::{output, printed};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[test]
fn whole_numbers_and_decimals_keep_themselves_apart() {
    assert_eq!(printed("print(1 + 2)\n"), "3");
    assert_eq!(printed("print(1.5 + 2.25)\n"), "3.75");
}

#[test]
fn digit_groups_are_only_a_way_of_writing() {
    assert_eq!(printed("print(1_000_000)\n"), "1000000");
}

#[test]
fn the_two_booleans_print_as_the_words_vaab_uses() {
    assert_eq!(output("print(yes)\nprint(no)\n"), ["yes", "no"]);
}

#[test]
fn text_fills_in_its_holes_when_it_is_built() {
    assert_eq!(printed("let name = \"world\"\nprint(\"hello, {name}\")\n"), "hello, world");
}

#[test]
fn a_hole_can_hold_a_whole_expression() {
    assert_eq!(printed("print(\"two and two is {2 + 2}\")\n"), "two and two is 4");
}

#[test]
fn escapes_are_put_back_as_the_characters_they_stand_for() {
    assert_eq!(printed("print(\"a\\tb\")\n"), "a\tb");
    assert_eq!(printed("print(\"\\{not a hole\\}\")\n"), "{not a hole}");
}

#[test]
fn a_changing_value_may_be_assigned_to() {
    assert_eq!(printed("let changing count = 0\ncount = count + 1\nprint(count)\n"), "1");
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_follows_the_precedence_the_specification_gives() {
    assert_eq!(printed("print(2 + 3 * 4)\n"), "14");
    assert_eq!(printed("print((2 + 3) * 4)\n"), "20");
}

#[test]
fn division_between_whole_numbers_throws_the_rest_away() {
    assert_eq!(printed("print(7 / 2)\n"), "3");
    assert_eq!(printed("print(7 % 2)\n"), "1");
}

#[test]
fn a_minus_sign_in_front_negates() {
    assert_eq!(printed("let n = 3\nprint(-n)\n"), "-3");
}

#[test]
fn comparisons_answer_with_a_bool() {
    assert_eq!(output("print(1 < 2)\nprint(2 <= 2)\nprint(3 > 4)\n"), ["yes", "yes", "no"]);
}

#[test]
fn text_compares_in_the_order_it_would_be_listed() {
    assert_eq!(printed("print(\"ada\" < \"alan\")\n"), "yes");
}

#[test]
fn anything_may_be_compared_with_something_of_its_own_type() {
    assert_eq!(printed("print([1, 2] == [1, 2])\n"), "yes");
    assert_eq!(printed("print({\"a\": 1} == {\"a\": 2})\n"), "no");
}

#[test]
fn and_stops_as_soon_as_it_knows_the_answer() {
    // If `or` looked at its right-hand side this would divide by zero and stop.
    assert_eq!(printed("let n = 0\nprint(n == 0 or 1 / n == 1)\n"), "yes");
    assert_eq!(printed("let n = 0\nprint(n != 0 and 1 / n == 1)\n"), "no");
}

#[test]
fn not_turns_a_bool_around() {
    assert_eq!(printed("print(not 1 == 2)\n"), "yes");
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[test]
fn a_function_gives_back_what_it_returns() {
    assert_eq!(printed("to double(n: Int) returns Int = n * 2\nprint(double(21))\n"), "42");
}

#[test]
fn a_function_may_be_called_above_where_it_is_written() {
    assert_eq!(printed("print(later())\nto later() returns Int = 7\n"), "7");
}

#[test]
fn a_default_is_used_when_the_argument_is_left_out() {
    let source = "to greet(name: Text, greeting: Text = \"hello\") returns Text = \
                  \"{greeting}, {name}\"\n\
                  print(greet(\"Ada\"))\nprint(greet(\"Alan\", greeting: \"hi\"))\n";
    assert_eq!(output(source), ["hello, Ada", "hi, Alan"]);
}

#[test]
fn arguments_given_by_name_land_where_they_belong() {
    let source = "to between(low: Int, high: Int) returns Int = high - low\n\
                  print(between(high: 10, low: 4))\n";
    assert_eq!(printed(source), "6");
}

#[test]
fn a_function_with_no_result_gives_nothing() {
    assert_eq!(printed("to announce(what: Text) { print(what) }\nannounce(\"here\")\n"), "here");
}

#[test]
fn a_function_calls_itself_by_pushing_a_frame_not_by_recursing_in_rust() {
    let source = "to countdown(n: Int) returns Int {\n\
                  \x20   if n <= 0 { return 0 }\n\
                  \x20   return countdown(n - 1)\n\
                  }\n\
                  print(countdown(5000))\n";
    assert_eq!(printed(source), "0");
}

#[test]
fn a_function_held_in_a_value_can_be_called() {
    let source = "to double(n: Int) returns Int = n * 2\n\
                  to apply(change: to(Int) returns Int, to_what: Int) returns Int = \
                  change(to_what)\n\
                  print(apply(double, 4))\n";
    assert_eq!(printed(source), "8");
}

// ---------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------

#[test]
fn a_closure_takes_one_value_and_gives_another() {
    assert_eq!(printed("print([1, 2, 3].map(n -> n * n))\n"), "[1, 4, 9]");
}

#[test]
fn a_closure_with_several_parameters_takes_a_pair_apart() {
    assert_eq!(printed("print([(1, 2), (3, 4)].map((a, b) -> a + b))\n"), "[3, 7]");
}

#[test]
fn a_closure_body_may_be_a_block() {
    let source = "[1, 2].each(n -> {\n\x20   print(\"saw {n}\")\n})\n";
    assert_eq!(output(source), ["saw 1", "saw 2"]);
}

#[test]
fn a_closure_sees_the_values_around_where_it_was_written() {
    let source = "to scale(by: Int, numbers: list of Int) returns list of Int = \
                  numbers.map(n -> n * by)\n\
                  print(scale(3, [1, 2]))\n";
    assert_eq!(printed(source), "[3, 6]");
}

#[test]
fn a_closure_changes_the_very_value_it_was_given_not_a_copy_of_it() {
    let source = "to total(numbers: list of Int) returns Int {\n\
                  \x20   let changing sum = 0\n\
                  \x20   numbers.each(n -> { sum = sum + n })\n\
                  \x20   return sum\n\
                  }\n\
                  print(total([1, 2, 3, 4]))\n";
    assert_eq!(printed(source), "10");
}

#[test]
fn closures_made_on_different_turns_of_a_loop_have_a_value_each() {
    let source = "to counted() returns Int {\n\
                  \x20   let changing total = 0\n\
                  \x20   for each n in 1..3 {\n\
                  \x20       let step = n\n\
                  \x20       [0].each(ignored -> { total = total + step })\n\
                  \x20   }\n\
                  \x20   return total\n\
                  }\n\
                  print(counted())\n";
    assert_eq!(printed(source), "6");
}

#[test]
fn a_closure_inside_a_closure_reaches_all_the_way_out() {
    let source = "to spread(base: Int) returns list of Int =\n\
                  \x20   [1, 2].map(step -> [10].map(extra -> base + step + extra)[0])\n\
                  print(spread(100))\n";
    assert_eq!(printed(source), "[111, 112]");
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn an_if_is_an_expression_and_so_can_be_given_a_name() {
    assert_eq!(printed("let bigger = if 7 > 4 { 7 } else { 4 }\nprint(bigger)\n"), "7");
}

#[test]
fn an_if_chain_takes_the_first_branch_that_holds() {
    let source = "let n = 0\n\
                  print(if n > 0 { \"up\" } else if n < 0 { \"down\" } else { \"level\" })\n";
    assert_eq!(printed(source), "level");
}

#[test]
fn an_if_with_no_else_is_run_for_what_it_does() {
    assert_eq!(printed("if 1 < 2 { print(\"yes it is\") }\n"), "yes it is");
}

#[test]
fn a_while_loop_runs_until_its_condition_stops_holding() {
    let source = "let changing n = 3\nwhile n > 0 { print(n)\n n = n - 1 }\n";
    assert_eq!(output(source), ["3", "2", "1"]);
}

#[test]
fn repeat_runs_a_block_a_fixed_number_of_times() {
    assert_eq!(output("repeat 2 times { print(\"knock\") }\n"), ["knock", "knock"]);
}

#[test]
fn repeating_no_times_at_all_does_nothing() {
    assert!(output("repeat 0 times { print(\"never\") }\n").is_empty());
}

#[test]
fn for_each_walks_a_list() {
    assert_eq!(output("for each word in [\"a\", \"b\"] { print(word) }\n"), ["a", "b"]);
}

#[test]
fn for_each_walks_a_range_and_takes_both_ends() {
    assert_eq!(output("for each n in 1..3 { print(n) }\n"), ["1", "2", "3"]);
}

#[test]
fn for_each_hands_a_map_its_keys_and_values_together() {
    let source = "let ages = {\"Ada\": 36}\n\
                  for each (name, age) in ages { print(\"{name} is {age}\") }\n";
    assert_eq!(printed(source), "Ada is 36");
}

#[test]
fn a_range_that_runs_backwards_holds_nothing() {
    assert!(output("for each n in 3..1 { print(n) }\n").is_empty());
}

// ---------------------------------------------------------------------------
// Match
// ---------------------------------------------------------------------------

#[test]
fn a_match_takes_the_first_arm_that_applies() {
    let source = "let answer = match 2 {\n\
                  \x20   when 1 then \"one\"\n\
                  \x20   when 2 then \"two\"\n\
                  \x20   otherwise then \"more\"\n\
                  }\nprint(answer)\n";
    assert_eq!(printed(source), "two");
}

#[test]
fn a_guard_narrows_an_arm_further() {
    let source = "let answer = match -3 {\n\
                  \x20   when 0 then \"zero\"\n\
                  \x20   when n if n < 0 then \"negative\"\n\
                  \x20   otherwise then \"positive\"\n\
                  }\nprint(answer)\n";
    assert_eq!(printed(source), "negative");
}

#[test]
fn a_list_pattern_can_name_the_front_and_ignore_the_rest() {
    let source = "match [10, 20, 30] {\n\
                  \x20   when [first, ...] then print(\"starts with {first}\")\n\
                  \x20   otherwise then print(\"empty\")\n\
                  }\n";
    assert_eq!(printed(source), "starts with 10");
}

#[test]
fn a_list_pattern_of_a_fixed_length_only_matches_that_length() {
    let source = "match [1, 2, 3] {\n\
                  \x20   when [a, b] then print(\"two: {a} {b}\")\n\
                  \x20   otherwise then print(\"not two\")\n\
                  }\n";
    assert_eq!(printed(source), "not two");
}

#[test]
fn a_match_arm_may_have_a_block_body() {
    let source = "match 1 {\n\
                  \x20   when 1 then { print(\"one\")\n print(\"still one\") }\n\
                  \x20   otherwise then { print(\"other\") }\n\
                  }\n";
    assert_eq!(output(source), ["one", "still one"]);
}

#[test]
fn a_tuple_is_taken_apart_by_a_pattern() {
    let source = "match (1, \"a\") {\n\
                  \x20   when (number, letter) then print(\"{number}{letter}\")\n\
                  }\n";
    assert_eq!(printed(source), "1a");
}

// ---------------------------------------------------------------------------
// Missing values and failures
// ---------------------------------------------------------------------------

#[test]
fn a_maybe_is_either_found_or_nothing() {
    let source = "let ages = {\"Ada\": 36}\n\
                  match ages.get(\"Ada\") {\n\
                  \x20   when found value then print(\"Ada is {value}\")\n\
                  \x20   when nothing then print(\"unknown\")\n\
                  }\n";
    assert_eq!(printed(source), "Ada is 36");
}

#[test]
fn nothing_is_what_a_map_gives_for_a_key_it_has_not_got() {
    let source = "let ages = {\"Ada\": 36}\n\
                  match ages.get(\"Grace\") {\n\
                  \x20   when found value then print(\"{value}\")\n\
                  \x20   when nothing then print(\"unknown\")\n\
                  }\n";
    assert_eq!(printed(source), "unknown");
}

#[test]
fn otherwise_supplies_a_fallback_for_a_maybe() {
    let source = "let ages = {\"Ada\": 36}\n\
                  print(ages.get(\"Ada\") otherwise 0)\n\
                  print(ages.get(\"Grace\") otherwise 0)\n";
    assert_eq!(output(source), ["36", "0"]);
}

#[test]
fn otherwise_supplies_a_fallback_for_a_failure() {
    let source = "choice Trouble { Broken }\n\
                  to risky(ok: Bool) returns Int or fails Trouble {\n\
                  \x20   if ok { return success 1 }\n\
                  \x20   return failure Trouble.Broken\n\
                  }\n\
                  print(risky(yes) otherwise 0)\nprint(risky(no) otherwise 0)\n";
    assert_eq!(output(source), ["1", "0"]);
}

#[test]
fn try_hands_a_failure_straight_back_to_the_caller() {
    let source = "choice Trouble { Broken }\n\
                  to inner(ok: Bool) returns Int or fails Trouble {\n\
                  \x20   if ok { return success 1 }\n\
                  \x20   return failure Trouble.Broken\n\
                  }\n\
                  to outer(ok: Bool) returns Int or fails Trouble {\n\
                  \x20   let n = try inner(ok)\n\
                  \x20   return success n + 10\n\
                  }\n\
                  print(outer(yes))\nprint(outer(no))\n";
    assert_eq!(output(source), ["success 11", "failure Trouble.Broken"]);
}

#[test]
fn a_failure_can_carry_what_went_wrong() {
    let source = "choice Trouble { TooSmall(amount: Int) }\n\
                  to check(n: Int) returns Int or fails Trouble {\n\
                  \x20   if n < 0 { return failure Trouble.TooSmall(n) }\n\
                  \x20   return success n\n\
                  }\n\
                  match check(-2) {\n\
                  \x20   when success n then print(\"{n}\")\n\
                  \x20   when failure error then print(\"{error}\")\n\
                  }\n";
    assert_eq!(printed(source), "Trouble.TooSmall(-2)");
}

// ---------------------------------------------------------------------------
// Types, choices and abilities
// ---------------------------------------------------------------------------

const ACCOUNT: &str = "\
type Account {
    owner: Text
    balance: Int = 0

    to deposit(amount: Int) returns Account or fails AccountError {
        if amount <= 0 { return failure AccountError.InvalidAmount(amount) }
        return success self.with(balance: self.balance + amount)
    }
}

choice AccountError {
    InvalidAmount(amount: Int)
    Frozen
}
";

#[test]
fn new_fills_in_the_defaults_a_type_declared() {
    let source = format!("{ACCOUNT}print(Account.new(owner: \"Ada\"))\n");
    assert_eq!(printed(&source), "Account(owner: \"Ada\", balance: 0)");
}

#[test]
fn a_field_is_read_with_a_dot() {
    let source = format!("{ACCOUNT}print(Account.new(owner: \"Ada\", balance: 5).balance)\n");
    assert_eq!(printed(&source), "5");
}

#[test]
fn with_gives_back_a_copy_and_leaves_the_original_alone() {
    let source = format!(
        "{ACCOUNT}let opened = Account.new(owner: \"Ada\")\n\
         let funded = opened.with(balance: 50)\n\
         print(opened.balance)\nprint(funded.balance)\n"
    );
    assert_eq!(output(&source), ["0", "50"]);
}

#[test]
fn a_method_sees_the_value_it_was_called_on() {
    let source = format!(
        "{ACCOUNT}match Account.new(owner: \"Ada\").deposit(25) {{\n\
         \x20   when success account then print(\"{{account.owner}} has {{account.balance}}\")\n\
         \x20   when failure error then print(\"no\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "Ada has 25");
}

#[test]
fn a_method_can_fail_and_say_why() {
    let source = format!("{ACCOUNT}print(Account.new(owner: \"Ada\").deposit(0))\n");
    assert_eq!(printed(&source), "failure AccountError.InvalidAmount(0)");
}

#[test]
fn a_variant_with_no_payload_prints_as_its_own_name() {
    let source = format!("{ACCOUNT}print(AccountError.Frozen)\n");
    assert_eq!(printed(&source), "AccountError.Frozen");
}

#[test]
fn a_validated_constructor_replaces_the_automatic_one() {
    let source = "choice EmailError { Invalid }\n\
                  type Email {\n\
                  \x20   address: Text\n\n\
                  \x20   to new(address: Text) returns Email or fails EmailError {\n\
                  \x20       if not address.contains(\"@\") { return failure EmailError.Invalid }\n\
                  \x20       return success Email.raw(address: address)\n\
                  \x20   }\n\
                  }\n\
                  print(Email.new(\"ada@example.com\"))\nprint(Email.new(\"nope\"))\n";
    assert_eq!(
        output(source),
        ["success Email(address: \"ada@example.com\")", "failure EmailError.Invalid"]
    );
}

#[test]
fn an_ability_picks_the_function_the_value_really_has() {
    let source = "ability Describable {\n\x20   to describe() returns Text\n}\n\
                  type Person can Describable {\n\
                  \x20   name: Text\n\n\
                  \x20   to describe() returns Text = \"{self.name}, a person\"\n\
                  }\n\
                  type Planet can Describable {\n\
                  \x20   name: Text\n\n\
                  \x20   to describe() returns Text = \"{self.name}, a planet\"\n\
                  }\n\
                  to announce_all(things: list of Describable) {\n\
                  \x20   for each thing in things { print(thing.describe()) }\n\
                  }\n\
                  announce_all([Person.new(name: \"Ada\"), Planet.new(name: \"Mars\")])\n";
    assert_eq!(output(source), ["Ada, a person", "Mars, a planet"]);
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

#[test]
fn a_list_is_read_by_position_counting_from_zero() {
    assert_eq!(printed("print([10, 20, 30][0])\n"), "10");
}

#[test]
fn a_map_keeps_the_order_its_keys_were_first_put_in() {
    assert_eq!(printed("print({\"b\": 1, \"a\": 2})\n"), "{\"b\": 1, \"a\": 2}");
}

#[test]
fn a_later_entry_replaces_an_earlier_one_with_the_same_key() {
    assert_eq!(printed("print({\"a\": 1, \"a\": 2})\n"), "{\"a\": 2}");
}

#[test]
fn a_tuple_holds_values_of_different_types_together() {
    assert_eq!(printed("print((1, \"one\", yes))\n"), "(1, \"one\", yes)");
}

#[test]
fn a_range_is_the_list_of_numbers_it_stands_for() {
    assert_eq!(printed("print(1..4)\n"), "[1, 2, 3, 4]");
}

#[test]
fn an_empty_list_and_an_empty_map_print_as_themselves() {
    let source = "let items: list of Int = []\nlet ages: map of Text to Int = {}\n\
                  print(items)\nprint(ages)\n";
    assert_eq!(output(source), ["[]", "{}"]);
}
