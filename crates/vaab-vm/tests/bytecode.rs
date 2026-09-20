//! What the compiler writes.
//!
//! These snapshots are here to be read rather than to be right: a change that
//! makes a listing longer or stranger is worth noticing, and a listing is the
//! quickest way to see what a piece of Vaab really costs.

mod support;

use insta::assert_snapshot;
use support::listing;

#[test]
fn a_file_of_one_print_is_three_instructions() {
    assert_snapshot!(listing("print(1)\n"));
}

#[test]
fn a_function_gets_a_body_of_its_own() {
    assert_snapshot!(listing("to double(n: Int) returns Int = n * 2\nprint(double(4))\n"));
}

#[test]
fn an_if_jumps_over_the_branch_it_did_not_take() {
    assert_snapshot!(listing("let n = 1\nprint(if n > 0 { \"up\" } else { \"down\" })\n"));
}

#[test]
fn a_while_loop_jumps_backwards_to_test_again() {
    assert_snapshot!(listing("let changing n = 3\nwhile n > 0 { n = n - 1 }\n"));
}

#[test]
fn a_for_each_loop_counts_a_position_the_program_never_sees() {
    assert_snapshot!(listing("for each n in [1, 2] { print(n) }\n"));
}

#[test]
fn map_is_a_loop_rather_than_a_call_into_rust() {
    assert_snapshot!(listing("print([1, 2].map(n -> n * 2))\n"));
}

#[test]
fn a_closure_says_which_values_it_takes_with_it() {
    assert_snapshot!(listing(
        "to scale(by: Int, numbers: list of Int) returns list of Int = \
         numbers.map(n -> n * by)\n"
    ));
}

#[test]
fn a_captured_local_is_put_in_a_box_where_it_is_declared() {
    assert_snapshot!(listing(
        "to total(numbers: list of Int) returns Int {\n\
         \x20   let changing sum = 0\n\
         \x20   numbers.each(n -> { sum = sum + n })\n\
         \x20   return sum\n\
         }\n"
    ));
}

#[test]
fn a_match_tries_each_arm_and_jumps_to_the_end_when_one_fits() {
    assert_snapshot!(listing(
        "let answer = match 2 {\n\
         \x20   when 1 then \"one\"\n\
         \x20   otherwise then \"more\"\n\
         }\n"
    ));
}

#[test]
fn a_record_is_built_from_a_layout_the_program_carries() {
    assert_snapshot!(listing(
        "type Account {\n\x20   owner: Text\n\x20   balance: Int = 0\n}\n\
         let account = Account.new(owner: \"Ada\")\n"
    ));
}

#[test]
fn a_method_is_called_by_name_when_the_type_is_known() {
    assert_snapshot!(listing(
        "type Counter {\n\
         \x20   count: Int = 0\n\n\
         \x20   to next() returns Counter = self.with(count: self.count + 1)\n\
         }\n\
         print(Counter.new().next().count)\n"
    ));
}

#[test]
fn an_ability_is_looked_up_on_the_value_rather_than_decided_in_advance() {
    assert_snapshot!(listing(
        "ability Greets {\n\x20   to greet() returns Text\n}\n\
         type Dog can Greets {\n\
         \x20   name: Text\n\n\
         \x20   to greet() returns Text = \"woof\"\n\
         }\n\
         to say(who: Greets) { print(who.greet()) }\n\
         say(Dog.new(name: \"Rex\"))\n"
    ));
}

#[test]
fn a_feature_a_later_phase_brings_compiles_to_one_instruction() {
    assert_snapshot!(listing("print(read_file(\"notes.txt\"))\n"));
}
