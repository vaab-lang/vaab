//! Channels, tasks, `together`, `select`, and `shared` state at run time.

mod support;

use insta::assert_snapshot;
use support::{output, printed, stops};

fn run(source: &str) -> Vec<String> {
    output(source)
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

#[test]
fn rendezvous_send_and_receive_meet_in_the_middle() {
    let source = "let lane = Channel.new(of: Int, size: 0)\n\
                  together {\n\
                  \x20   start { send 7 to lane }\n\
                  \x20   start {\n\
                  \x20       let answer = receive from lane\n\
                  \x20       print(answer)\n\
                  \x20   }\n\
                  }\n";
    assert_snapshot!(printed(source));
}

#[test]
fn a_bounded_channel_holds_only_as_many_as_its_size() {
    let source = "let inbox = Channel.new(of: Int, size: 2)\n\
                  send 1 to inbox\n\
                  send 2 to inbox\n\
                  print(receive from inbox)\n\
                  print(receive from inbox)\n";
    assert_snapshot!(run(source).join("\n"));
}

#[test]
fn receive_from_a_closed_empty_channel_is_absent() {
    let source = "let inbox = Channel.new(of: Int, size: 1)\n\
                  close inbox\n\
                  print(receive from inbox)\n";
    assert_snapshot!(printed(source));
}

#[test]
fn iterating_a_channel_reads_until_it_closes() {
    let source = "let inbox = Channel.new(of: Int, size: 2)\n\
                  send 1 to inbox\n\
                  send 2 to inbox\n\
                  close inbox\n\
                  for each item in inbox { print(item) }\n";
    assert_snapshot!(run(source).join("\n"));
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

#[test]
fn a_worker_pool_collects_squares() {
    let source = "let answers = Channel.new(of: Int, size: 4)\n\
                  together {\n\
                  \x20   start { send (2 * 2) to answers }\n\
                  \x20   start { send (3 * 3) to answers }\n\
                  \x20   start { send (5 * 5) to answers }\n\
                  }\n\
                  close answers\n\
                  for each square in answers { print(square) }\n";
    assert_snapshot!(run(source).join("\n"));
}

#[test]
fn a_pipeline_passes_work_between_tasks() {
    let source = "let stage = Channel.new(of: Int, size: 2)\n\
                  start {\n\
                  \x20   send 1 to stage\n\
                  \x20   send 2 to stage\n\
                  \x20   close stage\n\
                  }\n\
                  for each n in stage { print(n + 10) }\n";
    assert_snapshot!(run(source).join("\n"));
}

#[test]
fn fan_out_and_fan_in_adds_every_partial_sum() {
    let source = "let parts = Channel.new(of: Int, size: 4)\n\
                  together {\n\
                  \x20   start { send 10 to parts }\n\
                  \x20   start { send 20 to parts }\n\
                  \x20   start { send 30 to parts }\n\
                  }\n\
                  close parts\n\
                  let changing total = 0\n\
                  for each part in parts { total = total + part }\n\
                  print(total)\n";
    assert_snapshot!(printed(source));
}

#[test]
fn together_cancels_siblings_when_one_fails() {
    let source = "together {\n\
                  \x20   start { print(1 / 0) }\n\
                  \x20   start {\n\
                  \x20       let gate = Channel.new(of: Int, size: 0)\n\
                  \x20       send 1 to gate\n\
                  \x20   }\n\
                  }\n";
    let rendered = stops(source);
    assert!(
        rendered.contains("divides by zero") || rendered.contains("cancelled"),
        "{rendered}"
    );
    assert_snapshot!(rendered);
}

// ---------------------------------------------------------------------------
// Select
// ---------------------------------------------------------------------------

#[test]
fn select_otherwise_runs_without_waiting() {
    let source = "select {\n\
                  \x20   when timeout after 5 seconds { print(\"late\") }\n\
                  \x20   otherwise { print(\"now\") }\n\
                  }\n";
    assert_snapshot!(printed(source));
}

#[test]
fn select_timeout_fires_when_nothing_else_is_ready() {
    let source = "select {\n\
                  \x20   when timeout after 0 seconds { print(\"quiet\") }\n\
                  }\n";
    assert_snapshot!(printed(source));
}

#[test]
fn select_receives_the_first_ready_channel() {
    let source = "let a = Channel.new(of: Int, size: 1)\n\
                  let b = Channel.new(of: Int, size: 1)\n\
                  send 1 to b\n\
                  select {\n\
                  \x20   when receive from a as x { print(x) }\n\
                  \x20   when receive from b as y { print(y) }\n\
                  }\n";
    assert_snapshot!(printed(source));
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

#[test]
fn many_tasks_take_turns_under_preemption() {
    let source = "let tally = Channel.new(of: Int, size: 40)\n\
                  together {\n\
                  \x20   repeat 20 times { start { send 1 to tally } }\n\
                  \x20   repeat 20 times { start { send 1 to tally } }\n\
                  }\n\
                  close tally\n\
                  let changing total = 0\n\
                  for each n in tally { total = total + n }\n\
                  print(total)\n";
    assert_eq!(printed(source), "40");
}

#[test]
fn deadlock_on_a_rendezvous_with_no_partner_is_reported() {
    let source = "let alone = Channel.new(of: Int, size: 0)\n\
                  send 1 to alone\n";
    let rendered = stops(source);
    assert!(rendered.contains("deadlock") || rendered.contains("blocked"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn a_two_task_cycle_is_reported_as_deadlock() {
    let source = "let left = Channel.new(of: Int, size: 0)\n\
                  let right = Channel.new(of: Int, size: 0)\n\
                  let job = start {\n\
                  \x20   send 1 to left\n\
                  \x20   receive from right\n\
                  }\n\
                  send 2 to right\n\
                  receive from left\n\
                  job.wait()\n";
    let rendered = stops(source);
    assert!(rendered.contains("deadlock") || rendered.contains("blocked"), "{rendered}");
    assert_snapshot!(rendered);
}
