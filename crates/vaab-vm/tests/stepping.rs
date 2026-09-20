//! The part of the machine phase 4 builds on.
//!
//! A scheduler needs three things from this crate: to run a machine for a while
//! and get it back unfinished, to look at where it is, and to own several of them
//! against one world. These tests hold those three promises still.

mod support;

use support::ready;
use vaab_vm::{Budget, Machine, Output, Step, Value, World};

const COUNTING: &str = "\
let changing total = 0
for each n in 1..200 {
    total = total + n
}
print(total)
";

#[test]
fn a_machine_that_runs_out_of_budget_hands_itself_back_unfinished() {
    let (module, checked) = ready(COUNTING);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    let mut machine = Machine::start(&world.program);

    assert!(matches!(machine.resume(&mut world, Budget::of(20)), Step::Yielded));
    assert!(!machine.is_finished());
    assert!(world.output.lines().is_empty());
}

#[test]
fn a_machine_picked_up_again_carries_on_from_where_it_stopped() {
    let (module, checked) = ready(COUNTING);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    let mut machine = Machine::start(&world.program);

    let mut turns = 0;
    loop {
        match machine.resume(&mut world, Budget::of(16)) {
            Step::Yielded => turns += 1,
            Step::Finished(_) => break,
            Step::Failed(problem) => panic!("stopped: {:?}", problem.fault),
        }
        assert!(turns < 10_000, "this should have finished by now");
    }

    assert!(turns > 1, "a budget of 16 should not have been enough in one go");
    assert_eq!(world.output.lines(), ["20100"]);
}

#[test]
fn a_machine_can_be_parked_halfway_down_a_call() {
    let source = "to deep(n: Int) returns Int {\n\
                  \x20   if n <= 0 { return 0 }\n\
                  \x20   return deep(n - 1)\n\
                  }\nprint(deep(20))\n";
    let (module, checked) = ready(source);
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    let mut machine = Machine::start(&world.program);

    // Far enough in to be several calls deep, nowhere near far enough to finish.
    assert!(matches!(machine.resume(&mut world, Budget::of(40)), Step::Yielded));
    assert!(machine.depth() > 1, "expected to be inside a call, not at the top level");

    assert!(matches!(machine.resume(&mut world, Budget::unlimited()), Step::Finished(_)));
    assert_eq!(world.output.lines(), ["0"]);
}

#[test]
fn a_finished_machine_says_so() {
    let (module, checked) = ready("print(1)\n");
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    let mut machine = Machine::start(&world.program);

    assert!(!machine.is_finished());
    assert!(matches!(machine.resume(&mut world, Budget::unlimited()), Step::Finished(_)));
    assert!(machine.is_finished());
}

#[test]
fn several_machines_may_share_one_world() {
    let (module, checked) = ready("print(\"from a machine\")\n");
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());

    let mut first = Machine::start(&world.program);
    let mut second = Machine::start(&world.program);

    // Stepped in turn, the way a scheduler would, they both get through.
    while !first.is_finished() || !second.is_finished() {
        first.resume(&mut world, Budget::of(3));
        second.resume(&mut world, Budget::of(3));
    }

    assert_eq!(world.output.lines(), ["from a machine", "from a machine"]);
}

#[test]
fn the_values_a_file_keeps_outlive_the_machine_that_made_them() {
    let (module, checked) = ready("let greeting = \"hello\"\n");
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    assert!(vaab_vm::run(&mut world).is_ok());

    assert!(
        world.globals.iter().any(|value| matches!(value, Value::Text(text) if &**text == "hello")),
        "the file's own values should still be in the world"
    );
}

#[test]
fn a_world_takes_a_new_program_without_losing_what_it_holds() {
    let (module, checked) = ready("let greeting = \"hello\"\n");
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    assert!(vaab_vm::run(&mut world).is_ok());

    let more = "let greeting = \"hello\"\nprint(greeting)\n";
    let (module, checked) = ready(more);
    world.reload(vaab_vm::Ref::new(vaab_vm::compile(&module, &checked)));

    // Only the statement the second version added is run, and it can still see
    // the value the first one worked out.
    assert!(vaab_vm::run_from(&mut world, 1).is_ok());
    assert_eq!(world.output.lines(), ["hello"]);
}

#[test]
fn a_machine_started_with_no_budget_at_all_does_nothing_and_stays_ready() {
    let (module, checked) = ready("print(1)\n");
    let mut world = vaab_vm::prepare(&module, &checked, Output::collected());
    let mut machine = Machine::start(&world.program);

    assert!(matches!(machine.resume(&mut world, Budget::of(0)), Step::Yielded));
    assert!(!machine.is_finished());
    assert!(matches!(machine.resume(&mut world, Budget::unlimited()), Step::Finished(_)));
}

#[test]
fn a_world_can_be_told_to_keep_what_was_printed_instead_of_showing_it() {
    let (module, checked) = ready("print(\"kept\")\n");
    let mut world = World::new(
        vaab_vm::Ref::new(vaab_vm::compile(&module, &checked)),
        Output::collected(),
    );
    assert!(vaab_vm::run(&mut world).is_ok());
    assert_eq!(world.output.lines(), ["kept"]);
}
