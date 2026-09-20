//! Programs the checker accepts, and what it works out about them.
//!
//! These are the positive half of the suite. Every one is a small, whole program,
//! because "this is legal Vaab" is the claim being made.

mod support;

use support::{assert_answer, check};

// ---------------------------------------------------------------------------
// Literals and inference
// ---------------------------------------------------------------------------

#[test]
fn a_whole_number_is_an_int() {
    assert_answer("let answer = 1\n", "Int");
}

#[test]
fn a_number_with_a_point_is_a_float() {
    assert_answer("let answer = 1.5\n", "Float");
}

#[test]
fn text_is_text_even_when_it_interpolates() {
    assert_answer("let name = \"Ada\"\nlet answer = \"hello {name}\"\n", "Text");
}

#[test]
fn a_comparison_is_a_bool() {
    assert_answer("let answer = 1 < 2\n", "Bool");
}

#[test]
fn an_annotation_is_honoured() {
    assert_answer("let answer: Float = 1.5\n", "Float");
}

#[test]
fn a_list_takes_the_type_of_what_is_in_it() {
    assert_answer("let answer = [1, 2, 3]\n", "list of Int");
}

#[test]
fn a_map_takes_the_types_of_its_keys_and_values() {
    assert_answer("let answer = {\"Ada\": 36}\n", "map of Text to Int");
}

#[test]
fn a_tuple_keeps_each_part_separate() {
    assert_answer("let answer = (1, \"Ada\", yes)\n", "(Int, Text, Bool)");
}

#[test]
fn an_empty_list_takes_its_type_from_the_annotation() {
    assert_answer("let answer: list of Text = []\n", "list of Text");
}

#[test]
fn an_empty_map_takes_its_type_from_the_annotation() {
    assert_answer("let answer: map of Text to Int = {}\n", "map of Text to Int");
}

#[test]
fn a_range_is_a_list_of_ints() {
    assert_answer("let answer = 1..10\n", "list of Int");
}

#[test]
fn nothing_takes_its_type_from_the_annotation() {
    assert_answer("let answer: maybe Int = nothing\n", "maybe Int");
}

#[test]
fn found_wraps_what_it_holds() {
    assert_answer("let answer = found 1\n", "maybe Int");
}

#[test]
fn a_list_of_lists_nests() {
    assert_answer("let answer = [[1], [2]]\n", "list of list of Int");
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_keeps_the_type_it_was_given() {
    assert_answer("let answer = 2 + 3 * 4\n", "Int");
    assert_answer("let answer = 2.0 / 4.0\n", "Float");
}

#[test]
fn text_is_joined_by_interpolating_it() {
    assert_answer("let first = \"a\"\nlet answer = \"{first}b\"\n", "Text");
}

#[test]
fn and_or_and_not_work_on_bools() {
    assert_answer("let answer = yes and not no\n", "Bool");
}

#[test]
fn negation_keeps_the_number_it_was_given() {
    assert_answer("let answer = -1.5\n", "Float");
}

#[test]
fn equality_works_on_anything_as_long_as_both_sides_agree() {
    assert_answer("let answer = \"a\" == \"b\"\n", "Bool");
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[test]
fn a_call_has_the_type_the_signature_promises() {
    assert_answer(
        "to double(n: Int) returns Int = n * 2\n\
         let answer = double(2)\n",
        "Int",
    );
}

#[test]
fn a_function_with_no_returns_gives_nothing() {
    assert_answer(
        "to shout(word: Text) { print(word) }\n\
         let answer = shout(\"hi\")\n",
        "Nothing",
    );
}

#[test]
fn arguments_may_be_named() {
    check(
        "to greet(name: Text, greeting: Text) returns Text = \"{greeting}, {name}\"\n\
         let hello = greet(greeting: \"hi\", name: \"Ada\")\n",
    );
}

#[test]
fn an_argument_with_a_default_may_be_left_out() {
    check(
        "to greet(name: Text, greeting: Text = \"hello\") returns Text = \"{greeting}, {name}\"\n\
         let hello = greet(\"Ada\")\n",
    );
}

#[test]
fn a_function_may_be_called_before_it_is_declared() {
    check(
        "let answer = double(2)\n\
         to double(n: Int) returns Int = n * 2\n",
    );
}

#[test]
fn a_function_may_call_itself() {
    check(
        "to countdown(n: Int) returns Int {\n\
         \x20   if n <= 0 { return 0 }\n\
         \x20   return countdown(n - 1)\n\
         }\n",
    );
}

#[test]
fn a_block_bodied_function_may_end_with_its_value() {
    assert_answer(
        "to double(n: Int) returns Int {\n\
         \x20   n * 2\n\
         }\n\
         let answer = double(2)\n",
        "Int",
    );
}

#[test]
fn a_function_may_be_held_in_a_value_and_called() {
    assert_answer(
        "to double(n: Int) returns Int = n * 2\n\
         let twice = double\n\
         let answer = twice(2)\n",
        "Int",
    );
}

// ---------------------------------------------------------------------------
// Generics
// ---------------------------------------------------------------------------

#[test]
fn a_type_parameter_is_pinned_down_by_the_argument() {
    assert_answer(
        "to wrap(item: T) returns maybe T = found item\n\
         let answer = wrap(1)\n",
        "maybe Int",
    );
}

#[test]
fn a_type_parameter_inside_a_list_is_pinned_down_too() {
    assert_answer(
        "to labels(items: list of T) returns list of Text = items.map(item -> \"{item}\")\n\
         let answer = labels([1, 2, 3])\n",
        "list of Text",
    );
}

#[test]
fn one_call_does_not_fix_a_type_parameter_for_the_next() {
    check(
        "to same(item: T) returns T = item\n\
         let a = same(1)\n\
         let b = same(\"Ada\")\n",
    );
}

#[test]
fn a_type_parameter_may_appear_twice() {
    assert_answer(
        "to pair(left: T, right: T) returns list of T = [left, right]\n\
         let answer = pair(1, 2)\n",
        "list of Int",
    );
}

#[test]
fn two_type_parameters_stay_separate() {
    assert_answer(
        "to entry(key: K, value: V) returns map of K to V = {key: value}\n\
         let answer = entry(\"Ada\", 36)\n",
        "map of Text to Int",
    );
}

// ---------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------

#[test]
fn a_closure_takes_its_parameter_types_from_where_it_is_passed() {
    assert_answer("let answer = [1, 2].map(n -> n * 2)\n", "list of Int");
}

#[test]
fn a_closure_may_change_the_type_it_gives_back() {
    assert_answer("let answer = [1, 2].map(n -> \"{n}\")\n", "list of Text");
}

#[test]
fn a_closure_may_take_a_pair_apart() {
    assert_answer("let answer = [(1, 2)].map((a, b) -> a + b)\n", "list of Int");
}

#[test]
fn a_closure_may_have_a_block_body() {
    check("[1, 2].each(n -> {\n    print(\"{n}\")\n})\n");
}

#[test]
fn a_closure_can_read_the_values_around_it() {
    check(
        "let factor = 3\n\
         let scaled = [1, 2].map(n -> n * factor)\n",
    );
}

#[test]
fn an_annotated_let_gives_a_closure_its_types() {
    assert_answer("let answer: to(Int) returns Int = n -> n * 2\n", "to(Int) returns Int");
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

const ACCOUNT: &str = "type Account {\n\
    \x20   owner: Text\n\
    \x20   balance: Int = 0\n\
    \n\
    \x20   to deposit(amount: Int) returns Account = self.with(balance: self.balance + amount)\n\
    \x20   to describe() returns Text = \"{self.owner} has {self.balance}\"\n\
    }\n";

#[test]
fn a_type_is_built_with_new() {
    assert_answer(&format!("{ACCOUNT}let answer = Account.new(owner: \"Ada\")\n"), "Account");
}

#[test]
fn a_field_with_no_default_has_to_be_given() {
    check(&format!("{ACCOUNT}let account = Account.new(owner: \"Ada\", balance: 10)\n"));
}

#[test]
fn a_field_has_the_type_it_was_declared_with() {
    assert_answer(
        &format!("{ACCOUNT}let answer = Account.new(owner: \"Ada\").balance\n"),
        "Int",
    );
}

#[test]
fn a_method_has_the_type_its_signature_promises() {
    assert_answer(
        &format!("{ACCOUNT}let answer = Account.new(owner: \"Ada\").describe()\n"),
        "Text",
    );
}

#[test]
fn with_gives_back_the_same_type() {
    assert_answer(
        &format!("{ACCOUNT}let answer = Account.new(owner: \"Ada\").with(balance: 5)\n"),
        "Account",
    );
}

#[test]
fn a_method_may_call_another_on_self() {
    check(
        "type Account {\n\
         \x20   balance: Int = 0\n\
         \n\
         \x20   to doubled() returns Int = self.balance * 2\n\
         \x20   to report() returns Text = \"{self.doubled()}\"\n\
         }\n",
    );
}

#[test]
fn a_type_may_mention_one_declared_after_it() {
    check(
        "type Person {\n\
         \x20   home: Address\n\
         }\n\
         type Address {\n\
         \x20   city: Text\n\
         }\n",
    );
}

#[test]
fn a_user_defined_new_replaces_the_automatic_one() {
    assert_answer(
        "type Email {\n\
         \x20   text: Text\n\
         \n\
         \x20   to new(text: Text) returns Email or fails Text {\n\
         \x20       if not text.contains(\"@\") { return failure \"that is not an email\" }\n\
         \x20       return success Email.raw(text: text)\n\
         \x20   }\n\
         }\n\
         let answer = Email.new(text: \"ada@example.com\")\n",
        "Email or fails Text",
    );
}

// ---------------------------------------------------------------------------
// Choices
// ---------------------------------------------------------------------------

const ERRORS: &str = "choice AccountError {\n\
    \x20   InvalidAmount(amount: Int)\n\
    \x20   Frozen\n\
    }\n";

#[test]
fn a_variant_with_no_payload_is_a_value_of_the_choice() {
    assert_answer(&format!("{ERRORS}let answer = AccountError.Frozen\n"), "AccountError");
}

#[test]
fn a_variant_with_a_payload_is_built_by_calling_it() {
    assert_answer(
        &format!("{ERRORS}let answer = AccountError.InvalidAmount(amount: 5)\n"),
        "AccountError",
    );
}

#[test]
fn a_match_over_a_choice_that_names_every_variant_is_accepted() {
    check(&format!(
        "{ERRORS}let error = AccountError.Frozen\n\
         let message = match error {{\n\
         \x20   when AccountError.InvalidAmount(amount) then \"{{amount}} is wrong\"\n\
         \x20   when AccountError.Frozen then \"frozen\"\n\
         }}\n",
    ));
}

#[test]
fn a_match_may_finish_with_otherwise_instead() {
    check(&format!(
        "{ERRORS}let error = AccountError.Frozen\n\
         let message = match error {{\n\
         \x20   when AccountError.Frozen then \"frozen\"\n\
         \x20   otherwise then \"something else\"\n\
         }}\n",
    ));
}

#[test]
fn a_variant_may_be_matched_without_naming_its_choice() {
    check(&format!(
        "{ERRORS}let error = AccountError.Frozen\n\
         let message = match error {{\n\
         \x20   when InvalidAmount(amount) then \"{{amount}}\"\n\
         \x20   when Frozen then \"frozen\"\n\
         }}\n",
    ));
}

// ---------------------------------------------------------------------------
// maybe, fallible, try and otherwise
// ---------------------------------------------------------------------------

#[test]
fn a_map_lookup_gives_back_a_maybe() {
    assert_answer("let ages = {\"Ada\": 36}\nlet answer = ages.get(\"Ada\")\n", "maybe Int");
}

#[test]
fn otherwise_unwraps_a_maybe() {
    assert_answer("let ages = {\"Ada\": 36}\nlet answer = ages.get(\"Ada\") otherwise 0\n", "Int");
}

#[test]
fn a_match_over_a_maybe_covering_both_halves_is_accepted() {
    check(
        "let ages = {\"Ada\": 36}\n\
         let message = match ages.get(\"Ada\") {\n\
         \x20   when found age then \"{age}\"\n\
         \x20   when nothing then \"not listed\"\n\
         }\n",
    );
}

#[test]
fn a_fallible_result_carries_both_halves() {
    assert_answer(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         let answer = parse(\"1\")\n",
        "Int or fails Text",
    );
}

#[test]
fn try_unwraps_a_success_inside_a_function_that_can_fail() {
    check(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         to twice(text: Text) returns Int or fails Text {\n\
         \x20   let n = try parse(text)\n\
         \x20   return success n * 2\n\
         }\n",
    );
}

#[test]
fn otherwise_unwraps_a_fallible_result_too() {
    assert_answer(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         let answer = parse(\"1\") otherwise 0\n",
        "Int",
    );
}

#[test]
fn a_match_over_a_fallible_result_covering_both_halves_is_accepted() {
    check(
        "to parse(text: Text) returns Int or fails Text = success 1\n\
         let message = match parse(\"1\") {\n\
         \x20   when success n then \"{n}\"\n\
         \x20   when failure error then error\n\
         }\n",
    );
}

// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

const DESCRIBABLE: &str = "ability Describable {\n\
    \x20   to describe() returns Text\n\
    }\n\
    type Person can Describable {\n\
    \x20   name: Text\n\
    \n\
    \x20   to describe() returns Text = self.name\n\
    }\n";

#[test]
fn a_type_that_provides_an_ability_is_accepted() {
    check(DESCRIBABLE);
}

#[test]
fn a_type_may_be_passed_where_its_ability_is_wanted() {
    check(&format!(
        "{DESCRIBABLE}to announce(thing: Describable) returns Text = thing.describe()\n\
         let said = announce(Person.new(name: \"Ada\"))\n",
    ));
}

#[test]
fn an_ability_typed_parameter_offers_its_required_functions() {
    assert_answer(&format!(
        "{DESCRIBABLE}to announce(thing: Describable) returns Text = thing.describe()\n\
         let answer = announce(Person.new(name: \"Ada\"))\n",
    ), "Text");
}

#[test]
fn a_type_may_provide_more_than_one_ability() {
    check(
        "ability Describable {\n\
         \x20   to describe() returns Text\n\
         }\n\
         ability Countable {\n\
         \x20   to count() returns Int\n\
         }\n\
         type Basket can Describable, Countable {\n\
         \x20   items: list of Text\n\
         \n\
         \x20   to describe() returns Text = \"a basket\"\n\
         \x20   to count() returns Int = 0\n\
         }\n",
    );
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn an_if_used_as_an_expression_takes_the_type_both_branches_agree_on() {
    assert_answer("let answer = if 1 < 2 { \"yes\" } else { \"no\" }\n", "Text");
}

#[test]
fn an_if_without_an_else_may_still_be_used_for_its_effect() {
    check("if 1 < 2 { print(\"yes\") }\n");
}

#[test]
fn an_else_if_chain_is_one_expression() {
    assert_answer(
        "let n = 5\n\
         let answer = if n < 0 { \"negative\" } else if n == 0 { \"zero\" } else { \"positive\" }\n",
        "Text",
    );
}

#[test]
fn a_for_each_walks_a_list() {
    check("for each n in [1, 2, 3] { print(\"{n}\") }\n");
}

#[test]
fn a_for_each_walks_a_range() {
    check("for each n in 1..3 { print(\"{n}\") }\n");
}

#[test]
fn a_for_each_may_take_a_pair_apart() {
    check("for each (a, b) in [(1, 2)] { print(\"{a} {b}\") }\n");
}

#[test]
fn a_while_loop_takes_a_bool() {
    check(
        "let changing n = 0\n\
         while n < 3 { n = n + 1 }\n",
    );
}

#[test]
fn a_repeat_loop_takes_a_count() {
    check("repeat 3 times { print(\"again\") }\n");
}

#[test]
fn a_changing_binding_may_be_assigned_to() {
    check("let changing total = 0\ntotal = total + 1\n");
}

#[test]
fn an_index_reads_an_item_of_a_list() {
    assert_answer("let answer = [1, 2, 3][0]\n", "Int");
}

// ---------------------------------------------------------------------------
// What the compiler is handed
// ---------------------------------------------------------------------------

#[test]
fn every_expression_has_a_type() {
    let checked = check(
        "to double(n: Int) returns Int = n * 2\n\
         let numbers = [1, 2, 3].map(n -> double(n))\n",
    );
    // Sixteen nodes is not the point; that nothing was skipped is.
    assert!(checked.types.len() > 5, "only {} types recorded", checked.types.len());
    assert!(
        !checked.types.values().any(|declared| declared.to_string() == "_"),
        "an unresolved type reached the tables: {:?}",
        support::recorded_types(&checked)
    );
}

#[test]
fn a_local_gets_a_slot_in_a_frame() {
    let checked = check("let first = 1\nlet second = 2\n");
    let slots: Vec<usize> = checked.locals.iter().map(|local| local.slot).collect();
    assert_eq!(slots, [0, 1]);
    assert!(checked.locals.iter().all(|local| local.frame == vaab_types::Checked::TOP_LEVEL));
}

#[test]
fn a_function_body_gets_a_frame_of_its_own() {
    let checked = check("to double(n: Int) returns Int = n * 2\n");
    let function = checked.functions.first().expect("one function");
    assert_eq!(function.parameters.len(), 1);
    let frame = checked.frame(function.frame).expect("its frame");
    assert_eq!(frame.slots, 1);
    assert_eq!(frame.parent, Some(vaab_types::Checked::TOP_LEVEL));
}

#[test]
fn a_captured_value_says_how_far_out_it_lives() {
    let checked = check(
        "let factor = 3\n\
         let scaled = [1, 2].map(n -> n * factor)\n",
    );
    let hops: Vec<u32> = checked
        .resolutions
        .values()
        .filter_map(|resolution| match resolution {
            vaab_types::Resolution::Local(local) => Some(local.hops),
            _ => None,
        })
        .collect();
    assert!(hops.contains(&0), "the closure's own parameter should be at hand: {hops:?}");
    assert!(hops.contains(&1), "`factor` should be one frame out: {hops:?}");
}

#[test]
fn a_call_records_where_each_argument_comes_from() {
    use vaab_types::ArgumentSource;

    let checked = check(
        "to greet(name: Text, greeting: Text = \"hello\") returns Text = \"{greeting}, {name}\"\n\
         let said = greet(greeting: \"hi\", name: \"Ada\")\n",
    );
    let call = checked.calls.values().next().expect("one call");
    // Written out of order and with the default second: the compiler is handed the
    // parameter order, not the written order.
    assert_eq!(call.arguments, [ArgumentSource::Given(1), ArgumentSource::Given(0)]);
}

#[test]
fn a_left_out_argument_is_recorded_as_its_default() {
    use vaab_types::ArgumentSource;

    let checked = check(
        "to greet(name: Text, greeting: Text = \"hello\") returns Text = \"{greeting}, {name}\"\n\
         let said = greet(\"Ada\")\n",
    );
    let call = checked.calls.values().next().expect("one call");
    assert_eq!(call.arguments, [ArgumentSource::Given(0), ArgumentSource::Default]);
}

#[test]
fn with_keeps_the_fields_it_does_not_mention() {
    use vaab_types::ArgumentSource;

    let checked = check(&format!(
        "{ACCOUNT}let account = Account.new(owner: \"Ada\").with(balance: 5)\n"
    ));
    let kept = checked
        .calls
        .values()
        .find(|call| call.arguments.contains(&ArgumentSource::Kept))
        .expect("`.with` should keep something");
    assert_eq!(kept.arguments, [ArgumentSource::Kept, ArgumentSource::Given(0)]);
}

#[test]
fn a_variant_knows_its_number() {
    use vaab_types::Resolution;

    let checked = check(&format!("{ERRORS}let error = AccountError.Frozen\n"));
    let numbers: Vec<usize> = checked
        .resolutions
        .values()
        .filter_map(|resolution| match resolution {
            Resolution::Variant { variant, .. } => Some(*variant),
            _ => None,
        })
        .collect();
    assert_eq!(numbers, [1], "`Frozen` is the second variant");
}

#[test]
fn an_automatic_new_is_told_apart_from_a_written_one() {
    use vaab_types::{Constructor, Resolution};

    let checked = check(&format!("{ACCOUNT}let account = Account.new(owner: \"Ada\")\n"));
    let declared = checked.declared_types.first().expect("one type");
    assert_eq!(declared.constructor, Constructor::Automatic);
    assert!(checked
        .resolutions
        .values()
        .any(|resolution| matches!(resolution, Resolution::AutomaticNew(_))));
}

#[test]
fn a_written_new_is_recorded_as_a_call_to_it() {
    use vaab_types::{Constructor, Resolution};

    let checked = check(
        "type Email {\n\
         \x20   text: Text\n\
         \n\
         \x20   to new(text: Text) returns Email or fails Text = success Email.raw(text: text)\n\
         }\n\
         let mail = Email.new(text: \"ada@example.com\")\n",
    );
    let declared = checked.declared_types.first().expect("one type");
    assert!(matches!(declared.constructor, Constructor::UserDefined(_)));
    assert!(checked
        .resolutions
        .values()
        .any(|resolution| matches!(resolution, Resolution::UserNew { .. })));
}

#[test]
fn a_method_call_is_told_apart_from_a_free_function_call() {
    use vaab_types::Resolution;

    let checked = check(&format!(
        "{ACCOUNT}to describe(account: Account) returns Text = account.describe()\n"
    ));
    let resolutions: Vec<&Resolution> = checked.resolutions.values().collect();
    assert!(resolutions.iter().any(|resolution| matches!(resolution, Resolution::Method { .. })));
    assert!(resolutions.iter().any(|resolution| matches!(resolution, Resolution::Field { .. })));
}

#[test]
fn an_ability_call_is_left_for_the_value_to_answer() {
    use vaab_types::Resolution;

    let checked = check(&format!(
        "{DESCRIBABLE}to announce(thing: Describable) returns Text = thing.describe()\n"
    ));
    assert!(checked
        .resolutions
        .values()
        .any(|resolution| matches!(resolution, Resolution::AbilityMethod { function: 0, .. })));
}

#[test]
fn an_abilitys_functions_are_numbered_for_a_dispatch_table() {
    let checked = check(
        "ability Shape {\n\
         \x20   to area() returns Int\n\
         \x20   to name() returns Text\n\
         }\n\
         type Square can Shape {\n\
         \x20   side: Int\n\
         \n\
         \x20   to area() returns Int = self.side * self.side\n\
         \x20   to name() returns Text = \"square\"\n\
         }\n",
    );
    let ability = checked.abilities.iter().find(|a| a.name == "Shape").expect("Shape ability");
    let names: Vec<&str> =
        ability.functions.iter().map(|function| function.name.as_str()).collect();
    assert_eq!(names, ["area", "name"]);
}

#[test]
fn a_closure_that_takes_a_pair_apart_says_so() {
    let checked = check("let sums = [(1, 2)].map((a, b) -> a + b)\n");
    let closure = checked.closures.values().next().expect("one closure");
    assert!(closure.unpacks);
    assert_eq!(closure.parameters.len(), 2);
}

#[test]
fn a_binding_in_a_pattern_becomes_a_local() {
    let checked = check(
        "let ages = {\"Ada\": 36}\n\
         let message = match ages.get(\"Ada\") {\n\
         \x20   when found age then \"{age}\"\n\
         \x20   when nothing then \"not listed\"\n\
         }\n",
    );
    let age = checked
        .locals
        .iter()
        .find(|local| local.name == "age")
        .expect("`age` should be a local of its own");
    assert_eq!(age.declared.to_string(), "Int");
    assert!(
        checked.bindings.values().any(|id| checked.local(*id).is_some_and(|l| l.name == "age")),
        "the pattern should say which local it fills"
    );
}

// ---------------------------------------------------------------------------
// Concurrency: what may cross into a task
// ---------------------------------------------------------------------------

#[test]
fn a_fixed_value_may_cross_into_a_task() {
    assert_answer("let name = \"Ada\"\nlet answer = start { name }\n", "task of Text");
}

#[test]
fn a_list_may_cross_into_a_task() {
    assert_answer("let names = [\"Ada\"]\nlet answer = start { names }\n", "task of list of Text");
}

#[test]
fn a_map_may_cross_into_a_task() {
    assert_answer(
        "let ages = {\"Ada\": 36}\nlet answer = start { ages }\n",
        "task of map of Text to Int",
    );
}

#[test]
fn a_shared_may_cross_into_a_task() {
    assert_answer("let counter = Shared.new(0)\nlet answer = start { counter.value }\n", "task of Int");
}

#[test]
fn a_channel_may_cross_into_a_task() {
    assert_answer(
        "let inbox = Channel.new(of: Text, size: 1)\n\
         let answer = start { receive from inbox }\n",
        "task of maybe Text",
    );
}

#[test]
fn a_task_handle_may_cross_into_a_task() {
    assert_answer("let first = start { 1 }\nlet answer = start { first.wait() }\n", "task of Int");
}

#[test]
fn an_immutable_type_may_cross_into_a_task() {
    assert_answer(
        "type Account {\n\
         \x20   owner: Text\n\
         \x20   balance: Int = 0\n\
         }\n\
         let account = Account.new(owner: \"Ada\")\n\
         let answer = start { account.balance }\n",
        "task of Int",
    );
}

#[test]
fn a_choice_may_cross_into_a_task() {
    assert_answer(
        "choice Status {\n\
         \x20   Ready\n\
         \x20   Busy(since: Int)\n\
         }\n\
         let state = Status.Ready\n\
         let answer = start { state }\n",
        "task of Status",
    );
}

#[test]
fn a_type_that_holds_its_own_kind_is_still_sendable() {
    assert_answer(
        "type Node {\n\
         \x20   label: Text\n\
         \x20   next: maybe Node\n\
         }\n\
         let chain = Node.new(label: \"a\", next: nothing)\n\
         let answer = start { chain.label }\n",
        "task of Text",
    );
}

#[test]
fn a_type_holding_a_shared_may_cross_into_a_task() {
    assert_answer(
        "type Tally {\n\
         \x20   total: shared Int\n\
         }\n\
         let tally = Tally.new(total: Shared.new(0))\n\
         let answer = start { tally.total.value }\n",
        "task of Int",
    );
}

#[test]
fn a_value_known_only_by_its_ability_may_cross_when_every_provider_can() {
    check(
        "ability Describable {\n\
         \x20   to describe() returns Text\n\
         }\n\
         type Planet can Describable {\n\
         \x20   name: Text\n\
         \n\
         \x20   to describe() returns Text = self.name\n\
         }\n\
         to announce(thing: Describable) {\n\
         \x20   start { print(thing.describe()) }\n\
         }\n",
    );
}

#[test]
fn a_declared_function_is_called_inside_a_task_without_being_carried_in() {
    check("to fetch(url: Text) returns Text = url\nlet job = start { fetch(\"one\") }\n");
}

#[test]
fn a_loop_variable_may_cross_into_a_task() {
    check(
        "to fetch(url: Text) returns Text = url\n\
         together {\n\
         \x20   for each url in [\"a\", \"b\"] {\n\
         \x20       start { fetch(url) }\n\
         \x20   }\n\
         }\n",
    );
}

#[test]
fn a_changing_value_declared_inside_a_task_belongs_to_that_task() {
    assert_answer(
        "let answer = start {\n\
         \x20   let changing total = 0\n\
         \x20   total = total + 1\n\
         \x20   total\n\
         }\n",
        "task of Int",
    );
}

#[test]
fn a_closure_holding_only_fixed_values_may_cross_into_a_task() {
    assert_answer(
        "let base = 10\n\
         let add: to(Int) returns Int = n -> n + base\n\
         let answer = start { add(1) }\n",
        "task of Int",
    );
}

#[test]
fn a_task_records_what_it_has_to_carry_in() {
    let checked = check(
        "let name = \"Ada\"\n\
         let greeting = \"hello\"\n\
         let job = start { \"{greeting}, {name}\" }\n",
    );
    let task = checked.tasks.values().next().expect("one task");
    let carried: Vec<&str> = task
        .captures
        .iter()
        .filter_map(|held| checked.local(*held))
        .map(|held| held.name.as_str())
        .collect();
    assert_eq!(carried, ["greeting", "name"]);
}

#[test]
fn a_task_carries_nothing_when_it_reaches_for_nothing() {
    let checked = check("let job = start { 1 + 2 }\n");
    let task = checked.tasks.values().next().expect("one task");
    assert!(task.captures.is_empty());
}

#[test]
fn a_task_inside_a_task_carries_what_the_inner_one_needs() {
    let checked = check("let name = \"Ada\"\nlet outer = start { start { name } }\n");
    assert_eq!(checked.tasks.len(), 2);
    for task in checked.tasks.values() {
        assert_eq!(task.captures.len(), 1, "both tasks have to carry `name`");
    }
}

// ---------------------------------------------------------------------------
// Concurrency: shared state, `together` and `select`
// ---------------------------------------------------------------------------

#[test]
fn a_change_that_only_works_out_a_value_is_allowed() {
    check("let counter = Shared.new(0)\ncounter.update(n -> n + 1)\n");
}

#[test]
fn a_shared_value_may_be_read_inside_a_task() {
    check("let counter = Shared.new(0)\nlet job = start { print(counter.value) }\n");
}

#[test]
fn a_shared_value_may_be_changed_inside_a_task() {
    check("let counter = Shared.new(0)\nlet job = start { counter.update(n -> n + 1) }\n");
}

#[test]
fn a_shared_may_hold_a_list() {
    assert_answer("let answer = Shared.new([\"a\"])\n", "shared list of Text");
}

#[test]
fn a_together_holding_a_start_waits_for_something() {
    check("together {\n\x20   start { 1 }\n}\n");
}

#[test]
fn a_together_whose_body_only_calls_is_allowed_to_start_tasks_in_there() {
    check(
        "to spread_the_work() {\n\
         \x20   start { 1 }\n\
         }\n\
         together {\n\
         \x20   spread_the_work()\n\
         }\n",
    );
}

#[test]
fn every_unit_of_time_a_timeout_may_be_written_in_is_accepted() {
    for unit in [
        "millisecond",
        "milliseconds",
        "second",
        "seconds",
        "minute",
        "minutes",
        "hour",
        "hours",
    ] {
        check(&format!(
            "let inbox = Channel.new(of: Int, size: 1)\n\
             select {{\n\
             \x20   when timeout after 2 {unit} {{ print(\"quiet\") }}\n\
             }}\n"
        ));
    }
}

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

#[test]
fn a_pure_function_may_do_arithmetic() {
    check("pure to double(n: Int) returns Int = n * 2\n");
}

#[test]
fn a_pure_function_may_call_another_pure_function() {
    check(
        "pure to square(n: Int) returns Int = n * n\n\
         pure to sum(a: Int, b: Int) returns Int = square(a) + square(b)\n",
    );
}

#[test]
fn a_pure_function_may_change_its_own_changing_locals() {
    check(
        "pure to total(numbers: list of Int) returns Int {\n\
         \x20   let changing sum = 0\n\
         \x20   numbers.each(n -> { sum = sum + n })\n\
         \x20   sum\n\
         }\n",
    );
}

#[test]
fn a_pure_method_on_a_type_is_checked_like_any_other_pure_function() {
    check(
        "type Counter {\n\
         \x20   value: Int\n\
         \n\
         \x20   pure to doubled() returns Int = self.value * 2\n\
         }\n",
    );
}

#[test]
fn a_pure_closure_passed_to_map_may_transform_values() {
    check(
        "pure to squares(numbers: list of Int) returns list of Int = numbers.map(n -> n * n)\n",
    );
}
