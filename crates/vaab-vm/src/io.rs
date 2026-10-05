//! A small pool that runs blocking I/O off the scheduler thread.
//!
//! Builtins that wait park with [`WaitSite::Io`](crate::concurrency::WaitSite::Io)
//! and are woken when the worker finishes. The scheduler thread itself never
//! blocks on the network.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

struct IoJob {
    ticket: u64,
    work: IoWork,
    reply: Sender<(u64, Result<String, String>)>,
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
                    let result = job.work.run();
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
    tx: Sender<(u64, Result<String, String>)>,
    rx: Receiver<(u64, Result<String, String>)>,
    waiters: HashMap<u64, usize>,
    results: HashMap<u64, Result<String, String>>,
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

    pub fn submit(&mut self, work: IoWork) -> u64 {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let job = IoJob {
            ticket,
            work,
            reply: self.tx.clone(),
            notify: self.notify.clone(),
        };
        let _ = pool().send(job);
        ticket
    }

    pub fn take(&mut self, ticket: u64) -> Option<Result<String, String>> {
        self.results.remove(&ticket)
    }

    pub fn has_pending(&self) -> bool {
        !self.waiters.is_empty()
    }

    pub fn register(&mut self, ticket: u64, task: usize) {
        self.waiters.insert(ticket, task);
    }

    pub fn drain(&mut self, wake: &mut impl FnMut(usize)) {
        while let Ok((ticket, result)) = self.rx.try_recv() {
            self.results.insert(ticket, result);
            if let Some(task) = self.waiters.remove(&ticket) {
                wake(task);
            }
        }
    }

    pub fn wait(&mut self, deadline: Option<Instant>, wake: &mut impl FnMut(usize)) {
        let timeout = match deadline {
            Some(at) => at.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(3600),
        };
        match self.rx.recv_timeout(timeout) {
            Ok((ticket, result)) => {
                self.results.insert(ticket, result);
                if let Some(task) = self.waiters.remove(&ticket) {
                    wake(task);
                }
            }
            Err(_) => {}
        }
        self.drain(wake);
    }
}
