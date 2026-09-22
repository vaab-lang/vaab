mod support;

use std::time::{Duration, Instant};

use support::ready;
use vaab_vm::Output;

#[test]
fn tier1_jit_speeds_up_accumulate() {
    let source = "to accumulate(n: Int) returns Int {
    let changing acc = 0
    let changing i = 0
    while i < n {
        acc = acc + i
        i = i + 1
    }
    return acc
}
print(accumulate(200000))
";

    let (module, checked) = ready(source);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());

    vaab_vm::run(&mut world).expect("first run");
    assert_eq!(world.output.lines(), ["19999900000"]);

    world.output = Output::collected();
    let start = Instant::now();
    vaab_vm::run(&mut world).expect("second run");
    let second = start.elapsed();
    assert_eq!(world.output.lines(), ["19999900000"]);

    assert!(
        second < Duration::from_millis(150),
        "expected native-speed accumulate after JIT warmup, got {second:?}"
    );
}
