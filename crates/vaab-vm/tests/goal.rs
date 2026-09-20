//! The program phase 3 was asked for, run end to end.

mod support;

use support::output;

const GOAL: &str = "\
to fib(n: Int) returns Int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}
print(fib(25))
print([1, 2, 3].map(n -> n * 2))
";

#[test]
fn the_goal_program_prints_what_it_should() {
    assert_eq!(output(GOAL), ["75025", "[2, 4, 6]"]);
}
