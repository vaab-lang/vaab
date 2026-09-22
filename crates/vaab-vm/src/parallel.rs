//! Work-stealing worker pool for phase 7.

use std::collections::VecDeque;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
#[cfg(test)]
use std::thread::ThreadId;

use crossbeam_deque::{Injector, Steal, Stealer, Worker};

use crate::app_io::{Databases, Stores};
use crate::bytecode::Program;
use crate::concurrency::{Host, PREEMPT_AFTER, Run, TaskLife, WaitSite};
use crate::error::{Fault, RuntimeError};
use crate::log::Loggers;
use crate::machine::{Budget, Machine, Output, Step, World};
use crate::value::{Ref, Value};

#[cfg(test)]
static WORKER_THREADS: Mutex<Vec<ThreadId>> = Mutex::new(Vec::new());

pub fn worker_count() -> usize {
    if let Ok(raw) = std::env::var("VAAB_WORKERS") {
        if let Ok(count) = raw.parse::<usize>() {
            return count.max(1);
        }
    }
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2)
        .max(1)
}

struct TakenWorld {
    program: Ref<Program>,
    globals: Vec<Value>,
    output: Output,
}

struct Parallel {
    program: Ref<Program>,
    host: Mutex<Host>,
    world: Mutex<World>,
    machines: Mutex<Vec<Mutex<Machine>>>,
    injector: Injector<usize>,
    stealers: Vec<Stealer<usize>>,
    done: AtomicBool,
    outcome: Mutex<Option<Run>>,
    idle_workers: AtomicUsize,
}

pub fn run_pool(
    world: &mut World,
    workers: usize,
    host: Host,
    machines: Vec<Machine>,
) -> Result<Value, RuntimeError> {
    let taken = TakenWorld {
        program: Ref::clone(&world.program),
        globals: std::mem::take(&mut world.globals),
        output: std::mem::replace(&mut world.output, Output::collected()),
    };
    let (run, taken) = run_parallel(taken, workers, host, machines);
    world.globals = taken.globals;
    world.output = taken.output;
    match run {
        Run::Finished(value) => Ok(value),
        Run::Failed(error) => Err(error),
    }
}

fn run_parallel(
    taken: TakenWorld,
    workers: usize,
    host: Host,
    machines: Vec<Machine>,
) -> (Run, TakenWorld) {
    let world = World {
        program: Ref::clone(&taken.program),
        globals: taken.globals,
        output: taken.output,
        response: None,
        databases: Databases::shared(),
        stores: Stores::shared(),
        loggers: Loggers::shared(),
        jit: crate::jit::JitEngine::default(),
    };
    let mut queues: Vec<Worker<usize>> = Vec::with_capacity(workers);
    let mut stealers: Vec<Stealer<usize>> = Vec::with_capacity(workers);
    for _ in 0..workers {
        let queue = Worker::new_fifo();
        stealers.push(queue.stealer());
        queues.push(queue);
    }
    let shared = Arc::new(Parallel {
        program: Ref::clone(&taken.program),
        host: Mutex::new(host),
        world: Mutex::new(world),
        machines: Mutex::new(machines.into_iter().map(Mutex::new).collect()),
        injector: Injector::new(),
        stealers,
        done: AtomicBool::new(false),
        outcome: Mutex::new(None),
        idle_workers: AtomicUsize::new(0),
    });
    queues[0].push(0);
    let handles: Vec<JoinHandle<()>> = queues
        .into_iter()
        .map(|queue| {
            let shared = Arc::clone(&shared);
            thread::spawn(move || worker_loop(queue, shared))
        })
        .collect();
    for handle in handles {
        if handle.join().is_err() {
            set_outcome(
                &shared,
                Run::Failed(confused("a scheduler worker stopped unexpectedly")),
            );
            break;
        }
    }
    let run = match lock(&shared.outcome) {
        Ok(mut outcome) => match outcome.take() {
            Some(run) => run,
            None => Run::Failed(confused("the scheduler finished without an outcome")),
        },
        Err(_) => Run::Failed(confused("the scheduler's outcome lock was poisoned")),
    };
    let taken = match lock(&shared.world) {
        Ok(mut world_guard) => TakenWorld {
            program: Ref::clone(&shared.program),
            globals: std::mem::take(&mut world_guard.globals),
            output: std::mem::replace(&mut world_guard.output, Output::collected()),
        },
        Err(_) => TakenWorld {
            program: Ref::clone(&shared.program),
            globals: Vec::new(),
            output: Output::collected(),
        },
    };
    (run, taken)
}

fn worker_loop(queue: Worker<usize>, shared: Arc<Parallel>) {
    while !shared.done.load(Ordering::Acquire) {
        if let Some(task) = find_work(&queue, &shared) {
            run_task(task, &queue, &shared);
            continue;
        }
        shared.idle_workers.fetch_add(1, Ordering::AcqRel);
        if try_finish_idle(&shared) {
            shared.idle_workers.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        shared.idle_workers.fetch_sub(1, Ordering::AcqRel);
        thread::yield_now();
    }
}

fn find_work(queue: &Worker<usize>, shared: &Parallel) -> Option<usize> {
    queue
        .pop()
        .or_else(|| steal_from(&shared.stealers))
        .or_else(|| match shared.injector.steal() {
            Steal::Success(task) => Some(task),
            _ => None,
        })
}

fn steal_from(stealers: &[Stealer<usize>]) -> Option<usize> {
    for stealer in stealers {
        if let Steal::Success(task) = stealer.steal() {
            return Some(task);
        }
    }
    None
}

fn run_task(task: usize, queue: &Worker<usize>, shared: &Arc<Parallel>) {
    if shared.done.load(Ordering::Acquire) {
        return;
    }
    let skip = match lock(&shared.host) {
        Ok(host) => {
            host.tasks
                .get(task)
                .is_some_and(|entry| entry.cancelled || entry.state == TaskLife::Done)
        }
        Err(_) => {
            set_outcome(shared, Run::Failed(confused("the scheduler's host lock was poisoned")));
            return;
        }
    };
    if skip {
        return;
    }
    #[cfg(test)]
    {
        if let Ok(mut seen) = WORKER_THREADS.lock() {
            let id = thread::current().id();
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
    }
    let (yielded, pending, wakeups) = {
        let mut host = match lock(&shared.host) {
            Ok(host) => host,
            Err(_) => {
                set_outcome(shared, Run::Failed(confused("the scheduler's host lock was poisoned")));
                return;
            }
        };
        let mut world = match lock(&shared.world) {
            Ok(world) => world,
            Err(_) => {
                set_outcome(shared, Run::Failed(confused("the scheduler's world lock was poisoned")));
                return;
            }
        };
        let mut machines = match lock(&shared.machines) {
            Ok(machines) => machines,
            Err(_) => {
                set_outcome(
                    shared,
                    Run::Failed(confused("the scheduler's machine lock was poisoned")),
                );
                return;
            }
        };
        if task >= machines.len() {
            return;
        }
        let mut machine = match lock(&machines[task]) {
            Ok(machine) => machine,
            Err(_) => {
                set_outcome(shared, Run::Failed(confused("a task's machine lock was poisoned")));
                return;
            }
        };
        host.current = task;
        host.tasks[task].state = TaskLife::Running;
        let step = machine.resume(&mut *world, &mut *host, Budget::of(PREEMPT_AFTER));
        drop(machine);
        let mut pending = Vec::new();
        drain_pending(&mut *host, &mut *machines, &mut pending);
        let mut wakeups = VecDeque::new();
        let yielded = matches!(&step, Step::Yielded);
        match step {
            Step::Yielded => host.tasks[task].state = TaskLife::Runnable,
            Step::Parked(site) => {
                host.tasks[task].state = TaskLife::Parked;
                host.tasks[task].waiting = Some(site.clone());
                host.register_wait(task, &site, &mut wakeups);
            }
            Step::Finished(value) => {
                if task == 0 {
                    set_outcome(shared, Run::Finished(value));
                    return;
                }
                host.tasks[task].state = TaskLife::Done;
                host.tasks[task].outcome = Some(Ok(value.clone()));
                host.wake_task_waiters(task, &mut wakeups);
                host.finish_together_task(task, &mut wakeups);
            }
            Step::Failed(error) => {
                let error = *error;
                if task == 0 {
                    set_outcome(shared, Run::Failed(error));
                    return;
                }
                host.tasks[task].state = TaskLife::Done;
                host.tasks[task].outcome = Some(Err(error.clone()));
                host.wake_task_waiters(task, &mut wakeups);
                if let Some(group) = host.tasks[task].together {
                    if let Some(entry) = host.together_groups.get_mut(group) {
                        if entry.failed.is_none() {
                            entry.failed = Some(error.clone());
                        }
                    }
                    fail_together(&mut *host, group, &mut wakeups, machines.as_slice());
                }
                host.finish_together_task(task, &mut wakeups);
            }
        }
        (yielded, pending, wakeups)
    };
    for id in pending {
        if id == task {
            queue.push(id);
        } else {
            shared.injector.push(id);
        }
    }
    for id in wakeups {
        shared.injector.push(id);
    }
    if yielded {
        queue.push(task);
    }
}

fn drain_pending(host: &mut Host, machines: &mut Vec<Mutex<Machine>>, pending: &mut Vec<usize>) {
    while let Some((machine, id)) = host.pending_spawns.pop_front() {
        machines.push(Mutex::new(machine));
        pending.push(id);
    }
    while let Some(id) = host.pending_wakeups.pop_front() {
        pending.push(id);
    }
}

fn fail_together(
    host: &mut Host,
    group: usize,
    schedule: &mut VecDeque<usize>,
    machines: &[Mutex<Machine>],
) {
    let victims = host
        .together_groups
        .get(group)
        .map(|entry| entry.tasks.clone())
        .unwrap_or_default();
    for victim in victims {
        if victim != 0 {
            cancel_task(host, victim, schedule, machines);
        }
    }
}

fn cancel_task(
    host: &mut Host,
    task: usize,
    schedule: &mut VecDeque<usize>,
    machines: &[Mutex<Machine>],
) {
    if let Some(entry) = host.tasks.get_mut(task) {
        if entry.outcome.is_some() {
            return;
        }
        entry.cancelled = true;
        entry.state = TaskLife::Done;
        entry.outcome = Some(Err(RuntimeError {
            fault: Fault::Confused(
                "a task was cancelled because another in the same `together` failed",
            ),
            trace: trace_machine(machines, &host.program, task),
        }));
        entry.waiting = None;
    }
    schedule.retain(|id| *id != task);
    host.wake_task_waiters(task, schedule);
}

fn trace_machine(
    machines: &[Mutex<Machine>],
    program: &Ref<Program>,
    task: usize,
) -> Vec<crate::error::Level> {
    let machine = match machines.get(task) {
        Some(machine) => machine,
        None => return Vec::new(),
    };
    let machine = match lock(machine) {
        Ok(machine) => machine,
        Err(_) => return Vec::new(),
    };
    machine.trace(program)
}

fn try_finish_idle(shared: &Parallel) -> bool {
    if shared.done.load(Ordering::Acquire) {
        return true;
    }
    let mut pending = VecDeque::new();
    if let Ok(mut host) = lock(&shared.host) {
        host.fire_timeouts(&mut pending);
    }
    for task in pending {
        shared.injector.push(task);
        return false;
    }
    if shared.idle_workers.load(Ordering::Acquire) < shared.stealers.len() {
        return false;
    }
    if !shared.injector.is_empty() || shared.stealers.iter().any(|stealer| !stealer.is_empty()) {
        return false;
    }
    if let Ok(host) = lock(&shared.host) {
        if host.timeouts.is_empty() && host.all_parked() {
            set_outcome(shared, Run::Failed(deadlock_parallel(&host, &shared.machines)));
            return true;
        }
    }
    false
}

fn deadlock_parallel(host: &Host, machines: &Mutex<Vec<Mutex<Machine>>>) -> RuntimeError {
    let machines = match lock(machines) {
        Ok(machines) => machines,
        Err(_) => return confused("the scheduler's machine lock was poisoned"),
    };
    let mut lines = Vec::new();
    for (id, entry) in host.tasks.iter().enumerate() {
        if entry.state != TaskLife::Parked {
            continue;
        }
        let name = trace_machine(machines.as_slice(), &host.program, id)
            .first()
            .map(|level| level.name.to_string())
            .unwrap_or_else(|| format!("task {id}"));
        let site = match &entry.waiting {
            Some(WaitSite::Send { channel, .. }) => format!("sending to channel {channel}"),
            Some(WaitSite::Receive { channel, .. }) => format!("receiving from channel {channel}"),
            Some(WaitSite::WaitTask { task, .. }) => format!("waiting for task {task}"),
            Some(WaitSite::Together { .. }) => "waiting in `together`".to_string(),
            Some(WaitSite::Select { .. }) => "waiting in `select`".to_string(),
            None => "blocked".to_string(),
        };
        lines.push(format!("  task {id} ({name}): {site}"));
    }
    let detail = if lines.is_empty() {
        "every task is blocked and none can be woken".to_string()
    } else {
        format!("every task is blocked:\n{}", lines.join("\n"))
    };
    RuntimeError {
        fault: Fault::Deadlock { detail },
        trace: trace_machine(machines.as_slice(), &host.program, 0),
    }
}

fn set_outcome(shared: &Parallel, run: Run) {
    shared.done.store(true, Ordering::Release);
    if let Ok(mut outcome) = lock(&shared.outcome) {
        if outcome.is_none() {
            *outcome = Some(run);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, ()> {
    match mutex.lock() {
        Ok(guard) => Ok(guard),
        Err(_) => Err(()),
    }
}

fn confused(detail: &'static str) -> RuntimeError {
    RuntimeError {
        fault: Fault::Confused(detail),
        trace: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn worker_threads_seen() -> usize {
    WORKER_THREADS
        .lock()
        .map(|seen| {
            seen.iter().copied().collect::<HashSet<_>>().len()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Output;
    use vaab_syntax::parse;
    use vaab_types::check;

    #[test]
    fn two_tasks_run_on_different_worker_threads() {
        if let Ok(mut seen) = WORKER_THREADS.lock() {
            seen.clear();
        }
        let source = "let lane = Channel.new(of: Int, size: 2)\n\
                      together {\n\
                      \x20   start {\n\
                      \x20\x20   repeat 20000 times {\n\
                      \x20\x20\x20   let changing n = 0\n\
                      \x20\x20\x20   n = n + 1\n\
                      \x20\x20   }\n\
                      \x20\x20   send 1 to lane\n\
                      \x20   }\n\
                      \x20   start {\n\
                      \x20\x20   repeat 20000 times {\n\
                      \x20\x20\x20   let changing n = 0\n\
                      \x20\x20\x20   n = n + 1\n\
                      \x20\x20   }\n\
                      \x20\x20   send 2 to lane\n\
                      \x20   }\n\
                      }\n\
                      receive from lane\n\
                      receive from lane\n";
        let parsed = parse(source);
        let checked = check(&parsed.module).expect("source should type-check");
        let mut world = World::new(
            Ref::new(crate::compile::compile(&parsed.module, &checked)),
            Output::collected(),
        );
        let scheduler = crate::concurrency::Scheduler::new(Ref::clone(&world.program));
        let host = scheduler.host;
        let machines = scheduler.machines;
        let result = run_pool(&mut world, 4, host, machines);
        assert!(result.is_ok(), "run failed: {:?}", result.err());
        assert!(
            worker_threads_seen() >= 2,
            "expected work to run on at least two worker threads, saw {}",
            worker_threads_seen()
        );
    }
}
