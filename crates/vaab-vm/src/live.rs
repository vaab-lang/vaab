//! A scheduler that outlives any one machine.
//!
//! `vaab serve` runs top-level initialisation, `on start`, and every request on
//! the work-stealing pool in [`crate::concurrency::LivePool`] so route bodies,
//! `on start` tasks, and socket bodies can run on more than one worker. A pool
//! of one worker keeps the same parking behaviour as the single-thread loop.

use std::sync::mpsc;
use std::sync::Mutex;

use crate::concurrency::LivePool;
use crate::error::RuntimeError;
use crate::machine::{HttpResponse, Machine, World};
use crate::socket::LiveSocket;
use crate::value::Value;

pub type Done = Box<dyn FnOnce(LiveDone) + Send>;

pub struct LiveDone {
    pub value: Result<Value, RuntimeError>,
    pub response: Option<HttpResponse>,
}

pub struct LiveRuntime {
    pool: LivePool,
    stopping: Mutex<bool>,
}

impl LiveRuntime {
    pub fn start(world: World) -> LiveRuntime {
        LiveRuntime {
            pool: LivePool::start(world),
            stopping: Mutex::new(false),
        }
    }

    pub fn spawn(&self, machine: Machine, done: Option<Done>) {
        self.pool.spawn(machine, wrap_done(done));
    }

    pub fn attach_socket_and_spawn(
        &self,
        live: LiveSocket,
        then: impl FnOnce(Value) -> Machine + Send + 'static,
        done: Option<Done>,
    ) {
        self.pool.prepare(
            move |host| {
                let socket = host.attach_socket(live, crate::socket::DEFAULT_BUFFER);
                then(socket)
            },
            wrap_done(done),
        );
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
        if let Ok(mut stopping) = self.stopping.lock() {
            if *stopping {
                return;
            }
            *stopping = true;
        }
        self.pool.shutdown();
    }
}

impl Drop for LiveRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn wrap_done(done: Option<Done>) -> Option<crate::concurrency::Finish> {
    done.map(|done| {
        Box::new(
            move |value: Result<Value, RuntimeError>, response: Option<HttpResponse>| {
                done(LiveDone { value, response });
            },
        ) as crate::concurrency::Finish
    })
}
