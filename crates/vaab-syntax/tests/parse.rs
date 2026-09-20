//! Snapshot tests for the shape of the parsed tree.
//!
//! One test per feature, each as small as it can be while still being a real
//! program. Review the snapshots by reading them: they are the parser's
//! documentation.

mod support;

use insta::assert_snapshot;
use support::tree;

// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

#[test]
fn serve_block() {
    assert_snapshot!(tree(
        r#"
serve on port 8080 {
    before every request { print("{request.method}") }
    route get "/hello" { reply with "hello" }
    when anything fails with ApiError as error { reply explain(error) }
}
"#
    ));
}

// ---------------------------------------------------------------------------
// Values and bindings
// ---------------------------------------------------------------------------

#[test]
fn fixed_and_changeable_bindings() {
    assert_snapshot!(tree(
        r#"
let name = "world"
let changing count = 0
count = count + 1
"#
    ));
}

#[test]
fn an_annotated_binding() {
    assert_snapshot!(tree("let total: Int = 0\n"));
}

#[test]
fn literals_of_every_kind() {
    assert_snapshot!(tree(
        r#"
let whole = 42
let big = 1_000_000
let fraction = 3.5
let truth = yes
let falsehood = no
let missing = nothing
let items = [1, 2, 3]
let ages = {"Ada": 36, "Alan": 41}
let pair = (1, "a")
let span = 1..10
"#
    ));
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn text_always_interpolates() {
    assert_snapshot!(tree(r#"print("{greeting}, {name}!")"#));
}

#[test]
fn a_literal_brace_is_escaped() {
    assert_snapshot!(tree(r#"print("a \{ brace, and a } brace")"#));
}

#[test]
fn a_hole_can_hold_a_whole_expression() {
    assert_snapshot!(tree(r#"print("Ada is {ages.get("Ada") otherwise 0}")"#));
}

#[test]
fn escapes_are_decoded() {
    assert_snapshot!(tree(r#"print("line\nbreak\ttab\\slash\"quote")"#));
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[test]
fn a_function_with_a_default_argument() {
    assert_snapshot!(tree(
        r#"
to greet(name: Text, greeting: Text = "hello") returns Text {
    return "{greeting}, {name}"
}
"#
    ));
}

#[test]
fn the_one_line_function_form() {
    assert_snapshot!(tree("to double(n: Int) returns Int = n * 2\n"));
}

#[test]
fn a_function_returning_maybe() {
    assert_snapshot!(tree(
        r#"
to first_of(items: list of T) returns maybe T {
    if items.is_empty { return nothing }
    return found items[0]
}
"#
    ));
}

#[test]
fn a_function_with_no_result_returns_nothing() {
    assert_snapshot!(tree("to announce(message: Text) {\n    print(message)\n}\n"));
}

#[test]
fn a_pure_function() {
    assert_snapshot!(tree("pure to square(n: Int) returns Int = n * n\n"));
}

#[test]
fn named_arguments_at_the_call_site() {
    assert_snapshot!(tree(r#"greet("Ada", greeting: "hi")"#));
}

// ---------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------

#[test]
fn closures_take_one_or_several_parameters() {
    assert_snapshot!(tree(
        r#"
let squares = numbers.map(n -> n * n)
let sums = pairs.map((a, b) -> a + b)
items.each(item -> { print(item) })
"#
    ));
}

// ---------------------------------------------------------------------------
// Types, choices and abilities
// ---------------------------------------------------------------------------

#[test]
fn a_type_with_a_default_and_a_method() {
    assert_snapshot!(tree(
        r#"
type Account {
    owner: Text
    balance: Int = 0

    to deposit(amount: Int) returns Account or fails AccountError {
        if amount <= 0 { return failure AccountError.InvalidAmount(amount) }
        return success self.with(balance: self.balance + amount)
    }
}
"#
    ));
}

#[test]
fn a_type_with_its_own_validated_constructor() {
    assert_snapshot!(tree(
        r#"
type Email {
    address: Text

    to new(address: Text) returns Email or fails EmailError {
        if not address.contains("@") { return failure EmailError.Invalid }
        return success Email.raw(address: address)
    }
}
"#
    ));
}

#[test]
fn a_choice_with_and_without_payloads() {
    assert_snapshot!(tree(
        r#"
choice AccountError {
    InvalidAmount(amount: Int)
    Frozen
}
"#
    ));
}

#[test]
fn an_ability_and_a_type_that_can_do_it() {
    assert_snapshot!(tree(
        r#"
ability Describable {
    to describe() returns Text
}

type Account can Describable {
    to describe() returns Text = "{self.owner} has {self.balance}"
}
"#
    ));
}

#[test]
fn construction_always_goes_through_new() {
    assert_snapshot!(tree(
        r#"
let account = Account.new(owner: "Ada")
let funded = Account.new(owner: "Ada", balance: 50)
let inbox = Channel.new(of: Text, size: 10)
let counter = Shared.new(0)
"#
    ));
}

// ---------------------------------------------------------------------------
// Types as written
// ---------------------------------------------------------------------------

#[test]
fn every_way_of_writing_a_type() {
    assert_snapshot!(tree(
        r#"
to shapes(
    a: Int,
    b: list of Text,
    c: map of Text to Int,
    d: maybe Int,
    e: channel of Text,
    f: task of Int,
    g: shared Int,
    h: (Int, Text),
    i: to(Int, Text) returns Bool,
    j: list of maybe list of Int
) returns Text or fails ShapeError = "ok"
"#
    ));
}

// ---------------------------------------------------------------------------
// Missing values and errors
// ---------------------------------------------------------------------------

#[test]
fn try_and_otherwise() {
    assert_snapshot!(tree(
        r#"
to read_config(path: Text) returns Config or fails FileError {
    let text = try read_file(path)
    let config = try parse_config(text)
    return success config
}

let safe = ages.get("Ada") otherwise 0
"#
    ));
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn if_is_an_expression_and_chains() {
    assert_snapshot!(tree(
        r#"
let bigger = if a > b { a } else if a == b { a } else { b }
"#
    ));
}

#[test]
fn every_loop() {
    assert_snapshot!(tree(
        r#"
for each item in items { print(item) }
for each n in 1..10 { print(n) }
while ready { work() }
repeat 4 times { knock() }
"#
    ));
}

#[test]
fn match_with_guards_lists_and_a_catch_all() {
    assert_snapshot!(tree(
        r#"
let description = match value {
    when 0            then "zero"
    when n if n < 0   then "negative"
    when [first, ...] then "starts with {first}"
    when found x      then "got {x}"
    otherwise         then "something else"
}
"#
    ));
}

#[test]
fn match_on_a_choice_is_written_with_variant_names() {
    assert_snapshot!(tree(
        r#"
match error {
    when AccountError.InvalidAmount(amount) then report(amount)
    when AccountError.Frozen then freeze()
}
"#
    ));
}

#[test]
fn match_arms_can_have_block_bodies() {
    assert_snapshot!(tree(
        r#"
match age {
    when found value then { print("Ada is {value}") }
    when nothing     then { print("unknown") }
}
"#
    ));
}

#[test]
fn boolean_operators_are_words() {
    assert_snapshot!(tree("let ok = ready and not tired or forced\n"));
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn channels_are_sent_to_received_from_and_closed() {
    assert_snapshot!(tree(
        r#"
let inbox = Channel.new(of: Text, size: 10)
send "ping" to inbox
let message = receive from inbox
close inbox
for each item in inbox { print(item) }
"#
    ));
}

#[test]
fn tasks_start_and_are_waited_on() {
    assert_snapshot!(tree(
        r#"
let job = start { slow_calculation(42) }
let answer: Int = job.wait()
"#
    ));
}

#[test]
fn together_waits_for_every_task_inside() {
    assert_snapshot!(tree(
        r#"
together {
    for each url in urls {
        start { fetch(url) }
    }
}
"#
    ));
}

#[test]
fn select_waits_on_whichever_is_ready_first() {
    assert_snapshot!(tree(
        r#"
select {
    when receive from inbox as message { handle(message) }
    when receive from quit             { return }
    when timeout after 2 seconds       { print("quiet") }
    otherwise                          { print("nothing ready") }
}
"#
    ));
}

#[test]
fn shared_values_are_updated_atomically() {
    assert_snapshot!(tree(
        r#"
let counter = Shared.new(0)
counter.update(n -> n + 1)
print(counter.value)
"#
    ));
}

// ---------------------------------------------------------------------------
// Layout rules
// ---------------------------------------------------------------------------

#[test]
fn a_line_continues_after_an_operator() {
    assert_snapshot!(tree("let total = 1 +\n    2 +\n    3\n"));
}

#[test]
fn a_line_continues_when_the_next_one_starts_with_a_dot() {
    assert_snapshot!(tree(
        r#"
let shouted = names
    .map(name -> name.upper())
    .join(", ")
"#
    ));
}

#[test]
fn arguments_may_be_spread_over_several_lines() {
    assert_snapshot!(tree(
        r#"
greet(
    "Ada",
    greeting: "hi"
)
"#
    ));
}

#[test]
fn comments_are_ignored() {
    assert_snapshot!(tree("# a note\nlet a = 1 # another note\n"));
}
