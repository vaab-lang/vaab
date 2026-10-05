//! A scheduler that outlives any one machine.
//!
//! `vaab serve` runs top-level initialisation, `on start`, and every request on
//! this loop so tasks started from `on start` stay alive and HTTP parks instead
//! of blocking the worker.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::concurrency::{Host, PREEMPT_AFTER, TaskLife};
use crate::error::RuntimeError;
use crate::machine::{Budget, HttpResponse, Machine, Step, World};
use crate::value::{Ref, Value};

pub type Done = Box<dyn FnOnce(LiveDone) + Send>;

pub struct LiveDone {
    pub value: Result<Value, RuntimeError>,
    pub response: Option<HttpResponse>,
}

enum Job {
    Spawn { machine: Machine, done: Option<Done> },
    Shutdown,
}

#[derive(Clone)]
struct Pulse {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl Pulse {
    fn new() -> Pulse {
        Pulse {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    fn ping(&self) {
        if let Ok(mut ready) = self.inner.0.lock() {
            *ready = true;
            self.inner.1.notify_one();
        }
    }

    fn wait(&self, deadline: Option<Instant>) {
        let Ok(mut ready) = self.inner.0.lock() else {
            return;
        };
        while !*ready {
            if let Some(at) = deadline {
                let now = Instant::now();
                if at <= now {
                    break;
                }
                let (guard, _) = match self.inner.1.wait_timeout(ready, at - now) {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                ready = guard;
                break;
            }
            ready = match self.inner.1.wait(ready) {
                Ok(guard) => guard,
                Err(_) => return,
            };
        }
        *ready = false;
    }
}

pub struct LiveRuntime {
    jobs: Sender<Job>,
    pulse: Pulse,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LiveRuntime {
    pub fn start(world: World) -> LiveRuntime {
        let (tx, rx) = mpsc::channel();
        let pulse = Pulse::new();
        let notify = pulse.clone();
        let thread = thread::Builder::new()
            .name("vaab-serve".into())
            .spawn(move || run_loop(world, rx, notify))
            .expect("the server scheduler could start");
        LiveRuntime {
            jobs: tx,
            pulse,
            thread: Mutex::new(Some(thread)),
        }
    }

    pub fn spawn(&self, machine: Machine, done: Option<Done>) {
        let _ = self.jobs.send(Job::Spawn { machine, done });
        self.pulse.ping();
    }

    pub fn run_to_completion(&self, machine: Machine) -> LiveDone {
        let (tx, rx) = mpsc::channel();
        self.spawn(
            machine,
            Some(Box::new(move |done| {
                let _ = tx.send(done);
            })),
        );
        rx.recv().unwrap_or(LiveDone {
            value: Err(RuntimeError {
                fault: crate::error::Fault::Confused("the server scheduler stopped"),
                trace: Vec::new(),
            }),
            response: None,
        })
    }

    pub fn shutdown(&self) {
        let _ = self.jobs.send(Job::Shutdown);
        self.pulse.ping();
        if let Ok(mut thread) = self.thread.lock() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for LiveRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_loop(mut world: World, jobs: Receiver<Job>, pulse: Pulse) {
    let program = Ref::clone(&world.program);
    let mut host = Host::empty(&program);
    let notify = pulse.clone();
    host.io.set_notify(Arc::new(move || notify.ping()));

    let mut machines: Vec<Machine> = Vec::new();
    let mut callbacks: HashMap<usize, Done> = HashMap::new();
    let mut runnable: VecDeque<usize> = VecDeque::new();
    let mut stopping = false;

    loop {
        while let Ok(job) = jobs.try_recv() {
            match job {
                Job::Spawn { machine, done } => {
                    let id = push_task(&mut host, &mut machines, machine);
                    if let Some(done) = done {
                        callbacks.insert(id, done);
                    }
                    runnable.push_back(id);
                }
                Job::Shutdown => {
                    stopping = true;
                    host.cancel_all();
                }
            }
        }

        host.fire_timeouts(&mut runnable);
        host.drain_io(&mut runnable);
        while let Some(woken) = host.pending_wakeups.pop_front() {
            if !runnable.iter().any(|id| *id == woken) {
                runnable.push_back(woken);
            }
        }

        if stopping {
            finish_remaining(&mut machines, &mut callbacks);
            return;
        }

        let Some(task) = runnable.pop_front() else {
            pulse.wait(host.next_deadline());
            continue;
        };

        if host.tasks.get(task).is_some_and(|entry| entry.cancelled || entry.state == TaskLife::Done)
        {
            continue;
        }

        host.current = task;
        host.tasks[task].state = TaskLife::Running;
        let step = machines[task].resume(&mut world, &mut host, Budget::of(PREEMPT_AFTER));

        while let Some((machine, id)) = host.pending_spawns.pop_front() {
            while machines.len() < id {
                machines.push(Machine::entering(&program, 0, 0));
            }
            if id == machines.len() {
                machines.push(machine);
            } else {
                machines[id] = machine;
            }
            if !runnable.iter().any(|seen| *seen == id) {
                runnable.push_back(id);
            }
        }
        while let Some(woken) = host.pending_wakeups.pop_front() {
            runnable.push_back(woken);
        }

        match step {
            Step::Yielded => {
                host.tasks[task].state = TaskLife::Runnable;
                runnable.push_back(task);
            }
            Step::Parked(site) => {
                host.tasks[task].state = TaskLife::Parked;
                host.tasks[task].waiting = Some(site.clone());
                host.register_wait(task, &site, &mut runnable);
            }
            Step::Finished(value) => {
                host.tasks[task].state = TaskLife::Done;
                host.tasks[task].outcome = Some(Ok(value.clone()));
                runnable.retain(|id| *id != task);
                host.wake_task_waiters(task, &mut runnable);
                host.finish_together_task(task, &mut runnable);
                if let Some(done) = callbacks.remove(&task) {
                    done(LiveDone {
                        value: Ok(value),
                        response: machines[task].response.take(),
                    });
                }
            }
            Step::Failed(error) => {
                let error = *error;
                host.tasks[task].state = TaskLife::Done;
                host.tasks[task].outcome = Some(Err(error.clone()));
                host.wake_task_waiters(task, &mut runnable);
                if let Some(done) = callbacks.remove(&task) {
                    done(LiveDone {
                        value: Err(error),
                        response: machines[task].response.take(),
                    });
                }
            }
        }
    }
}

fn push_task(host: &mut Host, machines: &mut Vec<Machine>, machine: Machine) -> usize {
    let id = host.tasks.len();
    host.tasks.push(crate::concurrency::TaskMeta {
        state: TaskLife::Runnable,
        waiting: None,
        outcome: None,
        together: None,
        cancelled: false,
        select_fire: None,
    });
    debug_assert_eq!(id, machines.len());
    machines.push(machine);
    id
}

fn finish_remaining(machines: &mut [Machine], callbacks: &mut HashMap<usize, Done>) {
    for (id, done) in callbacks.drain() {
        done(LiveDone {
            value: Err(RuntimeError {
                fault: crate::error::Fault::Confused("the server is shutting down"),
                trace: Vec::new(),
            }),
            response: machines.get_mut(id).and_then(|machine| machine.response.take()),
        });
    }
}
