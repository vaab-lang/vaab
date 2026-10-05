//! A small pool that runs blocking I/O off the scheduler thread.
//!
//! Builtins that wait park with [`WaitSite::Io`](crate::concurrency::WaitSite::Io)
//! and are woken when the worker finishes. The scheduler thread itself never
//! blocks on the network. Socket work uses the same tickets and notify path, but
//! runs on the dedicated Tokio runtime in [`crate::socket`].

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::socket::LiveSocket;

const WORKERS: usize = 4;

pub enum IoWork {
    HttpGet { url: String },
    HttpPost { url: String, body: String },
    HttpSend {
        method: String,
        url: String,
        body: String,
        headers: HashMap<String, String>,
    },
}

impl IoWork {
    fn run(self) -> Result<String, String> {
        match self {
            IoWork::HttpGet { url } => crate::app_io::http_get(&url),
            IoWork::HttpPost { url, body } => crate::app_io::http_post(&url, &body),
            IoWork::HttpSend {
                method,
                url,
                body,
                headers,
            } => crate::app_io::http_send(&method, &url, &body, &headers),
        }
    }
}

pub enum IoOutcome {
    Http(Result<String, String>),
    SocketConnect(Result<LiveSocket, String>),
    Unit(Result<(), String>),
    Pulse,
}

#[derive(Clone)]
pub struct IoCompleter {
    tx: Sender<(u64, IoOutcome)>,
}

impl IoCompleter {
    pub fn complete(&self, ticket: u64, outcome: IoOutcome) {
        let _ = self.tx.send((ticket, outcome));
    }

    pub fn pulse(&self) {
        let _ = self.tx.send((0, IoOutcome::Pulse));
    }
}

struct IoJob {
    ticket: u64,
    work: IoWork,
    reply: Sender<(u64, IoOutcome)>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

fn pool() -> Sender<IoJob> {
    static POOL: OnceLock<Sender<IoJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<IoJob>();
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..WORKERS {
            let rx = Arc::clone(&rx);
            let _ = std::thread::Builder::new()
                .name(format!("vaab-io-{index}"))
                .spawn(move || loop {
                    let job = {
                        let lock = match rx.lock() {
                            Ok(lock) => lock,
                            Err(_) => return,
                        };
                        match lock.recv() {
                            Ok(job) => job,
                            Err(_) => return,
                        }
                    };
                    let result = IoOutcome::Http(job.work.run());
                    let _ = job.reply.send((job.ticket, result));
                    if let Some(notify) = job.notify {
                        notify();
                    }
                });
        }
        tx
    })
    .clone()
}

/// Outbound I/O tickets and their results, owned by a [`Host`](crate::concurrency::Host).
pub struct IoHub {
    tx: Sender<(u64, IoOutcome)>,
    rx: Receiver<(u64, IoOutcome)>,
    waiters: HashMap<u64, usize>,
    results: HashMap<u64, IoOutcome>,
    next_ticket: u64,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl IoHub {
    pub fn new() -> IoHub {
        let (tx, rx) = mpsc::channel();
        IoHub {
            tx,
            rx,
            waiters: HashMap::new(),
            results: HashMap::new(),
            next_ticket: 1,
            notify: None,
        }
    }

    pub fn set_notify(&mut self, notify: Arc<dyn Fn() + Send + Sync>) {
        self.notify = Some(notify);
    }

    pub fn notify(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        self.notify.clone()
    }

    pub fn completer(&self) -> IoCompleter {
        IoCompleter { tx: self.tx.clone() }
    }

    pub fn reserve(&mut self) -> u64 {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        ticket
    }

    pub fn submit(&mut self, work: IoWork) -> u64 {
        let ticket = self.reserve();
        let job = IoJob {
            ticket,
            work,
            reply: self.tx.clone(),
            notify: self.notify.clone(),
        };
        let _ = pool().send(job);
        ticket
    }

    pub fn take(&mut self, ticket: u64) -> Option<IoOutcome> {
        self.results.remove(&ticket)
    }

    pub fn has_pending(&self) -> bool {
        !self.waiters.is_empty()
    }

    pub fn register(&mut self, ticket: u64, task: usize) {
        self.waiters.insert(ticket, task);
    }

    fn wake_ready(&mut self, wake: &mut impl FnMut(usize)) {
        let ready: Vec<u64> = self
            .waiters
            .keys()
            .copied()
            .filter(|ticket| self.results.contains_key(ticket))
            .collect();
        for ticket in ready {
            if let Some(task) = self.waiters.remove(&ticket) {
                wake(task);
            }
        }
    }

    pub fn drain(&mut self, wake: &mut impl FnMut(usize)) {
        while let Ok((ticket, result)) = self.rx.try_recv() {
            if ticket == 0 {
                continue;
            }
            self.results.insert(ticket, result);
            if let Some(task) = self.waiters.remove(&ticket) {
                wake(task);
            }
        }
        self.wake_ready(wake);
    }

    pub fn wait(&mut self, deadline: Option<Instant>, wake: &mut impl FnMut(usize)) {
        let timeout = match deadline {
            Some(at) => at.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(3600),
        };
        match self.rx.recv_timeout(timeout) {
            Ok((ticket, result)) => {
                if ticket != 0 {
                    self.results.insert(ticket, result);
                    if let Some(task) = self.waiters.remove(&ticket) {
                        wake(task);
                    }
                }
            }
            Err(_) => {}
        }
        self.drain(wake);
    }
}
