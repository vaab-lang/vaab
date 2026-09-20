//! The work-stealing scheduler tasks and channels run on.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use vaab_syntax::span::Span;

use crate::bytecode::{Program, SelectArm, SelectDescriptor};
use crate::error::{Fault, RuntimeError};
use crate::machine::{Budget, Machine, Step};
use crate::value::{Ref, Value};

#[path = "parallel.rs"]
mod parallel;

pub const PREEMPT_AFTER: u64 = 10_000;

pub fn run(world: &mut crate::machine::World) -> Result<Value, RuntimeError> {
    let workers = parallel::worker_count();
    let scheduler = Scheduler::new(Ref::clone(&world.program));
    if workers <= 1 {
        let mut scheduler = scheduler;
        match scheduler.run(world) {
            Run::Finished(value) => Ok(value),
            Run::Failed(error) => Err(error),
        }
    } else {
        parallel::run_pool(world, workers, scheduler.host, scheduler.machines)
    }
}

/// Channel, task and `together` state the machine reaches into while it runs.
pub struct Host {
    pub(crate) program: Ref<Program>,
    channels: HashMap<u32, ChannelState>,
    next_channel: u32,
    pub(crate) tasks: Vec<TaskMeta>,
    pub(crate) together_groups: Vec<TogetherGroup>,
    active_together: Vec<usize>,
    pub(crate) timeouts: Vec<PendingTimeout>,
    pub(crate) current: usize,
    pub(crate) pending_spawns: VecDeque<(Machine, usize)>,
    pub(crate) pending_wakeups: VecDeque<usize>,
    completed_sends: HashSet<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskLife {
    Running,
    Runnable,
    Parked,
    Done,
}

#[derive(Clone, Debug)]
pub enum WaitSite {
    Send { channel: u32, span: Span },
    Receive { channel: u32, span: Span },
    WaitTask { task: usize, span: Span },
    Together { group: usize },
    Select { descriptor: usize, resume_pc: usize },
}

pub(crate) struct TaskMeta {
    pub(crate) state: TaskLife,
    pub(crate) waiting: Option<WaitSite>,
    pub(crate) outcome: Option<Result<Value, RuntimeError>>,
    pub(crate) together: Option<usize>,
    pub(crate) cancelled: bool,
    select_fire: Option<usize>,
}

struct ChannelState {
    buffer: Vec<Value>,
    capacity: usize,
    closed: bool,
    send_waiters: Vec<usize>,
    recv_waiters: Vec<usize>,
    /// Values from senders waiting for a rendezvous partner.
    rendezvous_sends: VecDeque<(usize, Value)>,
    /// Values handed to a receiver that has not woken yet.
    rendezvous_to: HashMap<usize, Value>,
}

pub(crate) struct TogetherGroup {
    pub(crate) tasks: Vec<usize>,
    waiter: Option<usize>,
    pub(crate) failed: Option<RuntimeError>,
}

#[derive(Clone)]
pub(crate) struct PendingTimeout {
    at: Instant,
    task: usize,
    select_arm: usize,
}

pub enum Run {
    Finished(Value),
    Failed(RuntimeError),
}

pub enum OpResult<T = ()> {
    Done,
    Ready(T),
    Park(WaitSite),
    Stop(Fault),
    Failed(RuntimeError),
}

pub enum SelectStep {
    Arm { arm: usize, value: Option<Value> },
    Otherwise,
    Park { resume_pc: usize },
}

pub struct Scheduler {
    pub(crate) host: Host,
    pub(crate) machines: Vec<Machine>,
    runnable: VecDeque<usize>,
}

impl Scheduler {
    pub fn new(program: Ref<Program>) -> Scheduler {
        Scheduler {
            host: Host {
                program: Ref::clone(&program),
                channels: HashMap::new(),
                next_channel: 0,
                tasks: vec![TaskMeta {
                    state: TaskLife::Runnable,
                    waiting: None,
                    outcome: None,
                    together: None,
                    cancelled: false,
                    select_fire: None,
                }],
                together_groups: Vec::new(),
                active_together: Vec::new(),
                timeouts: Vec::new(),
                current: 0,
                pending_spawns: VecDeque::new(),
                pending_wakeups: VecDeque::new(),
                completed_sends: HashSet::new(),
            },
            machines: vec![Machine::start(&program)],
            runnable: VecDeque::from([0]),
        }
    }

    pub fn run(&mut self, world: &mut crate::machine::World) -> Run {
        loop {
            self.host.fire_timeouts(&mut self.runnable);

            if self.runnable.is_empty() {
                if self.host.timeouts.is_empty() && self.host.all_parked() {
                    return Run::Failed(self.host.deadlock(&self.machines));
                }
                if self.host.timeouts.is_empty() {
                    return Run::Failed(self.host.deadlock(&self.machines));
                }
                continue;
            }

            let task = match self.runnable.pop_front() {
                Some(task) => task,
                None => continue,
            };

            if self.host.tasks.get(task).is_some_and(|entry| entry.cancelled) {
                continue;
            }
            if self.host.tasks.get(task).is_some_and(|entry| entry.state == TaskLife::Done) {
                continue;
            }

            self.host.current = task;
            self.host.tasks[task].state = TaskLife::Running;
            let step = self.machines[task].resume(world, &mut self.host, Budget::of(PREEMPT_AFTER));

            while let Some((machine, runnable)) = self.host.pending_spawns.pop_front() {
                self.machines.push(machine);
                if !self.runnable.iter().any(|id| *id == runnable) {
                    self.runnable.push_back(runnable);
                }
            }
            while let Some(woken) = self.host.pending_wakeups.pop_front() {
                self.runnable.push_back(woken);
            }

            match step {
                Step::Yielded => {
                    self.host.tasks[task].state = TaskLife::Runnable;
                    self.runnable.push_back(task);
                }
                Step::Parked(site) => {
                    self.host.tasks[task].state = TaskLife::Parked;
                    self.host.tasks[task].waiting = Some(site.clone());
                    self.host.register_wait(task, &site, &mut self.runnable);
                }
                Step::Finished(value) => {
                    if task == 0 {
                        return Run::Finished(value);
                    }
                    self.host.tasks[task].state = TaskLife::Done;
                    self.host.tasks[task].outcome = Some(Ok(value.clone()));
                    self.runnable.retain(|id| *id != task);
                    self.host.wake_task_waiters(task, &mut self.runnable);
                    self.host.finish_together_task(task, &mut self.runnable);
                }
                Step::Failed(error) => {
                    let error = *error;
                    if task == 0 {
                        return Run::Failed(error);
                    }
                    self.host.tasks[task].state = TaskLife::Done;
                    self.host.tasks[task].outcome = Some(Err(error.clone()));
                    self.host.wake_task_waiters(task, &mut self.runnable);
                    if let Some(group) = self.host.tasks[task].together {
                        if let Some(entry) = self.host.together_groups.get_mut(group) {
                            if entry.failed.is_none() {
                                entry.failed = Some(error.clone());
                            }
                        }
                        self.host.fail_together(group, &mut self.runnable, &self.machines);
                    }
                    self.host.finish_together_task(task, &mut self.runnable);
                }
            }
        }
    }
}

impl Host {
    pub fn scratch(program: &Ref<Program>) -> Host {
        Host {
            program: Ref::clone(program),
            channels: HashMap::new(),
            next_channel: 0,
            tasks: vec![TaskMeta {
                state: TaskLife::Runnable,
                waiting: None,
                outcome: None,
                together: None,
                cancelled: false,
                select_fire: None,
            }],
            together_groups: Vec::new(),
            active_together: Vec::new(),
            timeouts: Vec::new(),
            current: 0,
            pending_spawns: VecDeque::new(),
            pending_wakeups: VecDeque::new(),
            completed_sends: HashSet::new(),
        }
    }

    pub fn take_completed_send(&mut self, task: usize) -> bool {
        self.completed_sends.remove(&task)
    }

    pub fn current(&self) -> usize {
        self.current
    }

    fn wake(&mut self, task: usize) {
        self.pending_wakeups.push_back(task);
    }

    pub fn channel_new(&mut self, capacity: i64) -> Value {
        let id = self.next_channel;
        self.next_channel += 1;
        let capacity = usize::try_from(capacity.max(0)).unwrap_or(0);
        self.channels.insert(
            id,
            ChannelState {
                buffer: Vec::new(),
                capacity,
                closed: false,
                send_waiters: Vec::new(),
                recv_waiters: Vec::new(),
                rendezvous_sends: VecDeque::new(),
                rendezvous_to: HashMap::new(),
            },
        );
        Value::Channel(id)
    }

    pub fn shared_new(&mut self, value: Value) -> Value {
        Value::Shared(Ref::new(crate::value::Captured::new(value)))
    }

    pub fn send(&mut self, channel: u32, value: Value, span: Span) -> OpResult {
        let Some(state) = self.channels.get_mut(&channel) else {
            return OpResult::Stop(Fault::Confused("a send named a channel that is not there"));
        };
        if state.closed {
            return OpResult::Stop(Fault::Confused("a send reached a channel that is closed"));
        }
        if let Some(receiver) = state.recv_waiters.first().copied() {
            state.recv_waiters.remove(0);
            state.rendezvous_to.insert(receiver, value);
            self.wake(receiver);
            return OpResult::Done;
        }
        if state.buffer.len() < state.capacity {
            state.buffer.push(value);
            let waiters = state.recv_waiters.clone();
            for waiter in waiters {
                self.try_wake_receiver(waiter, channel);
            }
            return OpResult::Done;
        }
        state.rendezvous_sends.push_back((self.current, value));
        OpResult::Park(WaitSite::Send { channel, span })
    }

    pub fn receive(&mut self, channel: u32, span: Span) -> OpResult<Value> {
        let Some(state) = self.channels.get_mut(&channel) else {
            return OpResult::Stop(Fault::Confused(
                "a receive named a channel that is not there",
            ));
        };
        if let Some(value) = state.rendezvous_to.remove(&self.current) {
            return OpResult::Ready(Value::found(value));
        }
        if let Some((sender, value)) = state.rendezvous_sends.pop_front() {
            state.send_waiters.retain(|task| *task != sender);
            self.completed_sends.insert(sender);
            self.wake(sender);
            return OpResult::Ready(Value::found(value));
        }
        if let Some(value) = state.buffer.first().cloned() {
            state.buffer.remove(0);
            let waiters = state.send_waiters.clone();
            for waiter in waiters {
                self.try_wake_sender(waiter, channel);
            }
            return OpResult::Ready(Value::found(value));
        }
        if state.closed {
            return OpResult::Ready(Value::absent());
        }
        OpResult::Park(WaitSite::Receive { channel, span })
    }

    pub fn close(&mut self, channel: u32) -> OpResult {
        let Some(state) = self.channels.get_mut(&channel) else {
            return OpResult::Stop(Fault::Confused("a close named a channel that is not there"));
        };
        state.closed = true;
        let waiters = state.recv_waiters.clone();
        for waiter in waiters {
            self.try_wake_receiver(waiter, channel);
        }
        OpResult::Done
    }

    pub fn channel_id(value: &Value) -> Option<u32> {
        match value {
            Value::Channel(id) => Some(*id),
            _ => None,
        }
    }

    pub fn spawn(&mut self, function: Value) -> Result<Value, RuntimeError> {
        let Value::Function(closure) = function else {
            return Err(confused("a task was started from something that is not a function"));
        };
        let body = closure.body as usize;
        let machine = match Machine::for_body(&self.program, body, 0, Some(Ref::clone(&closure))) {
            Ok(machine) => machine,
            Err(fault) => return Err(RuntimeError { fault, trace: Vec::new() }),
        };

        let together = self.active_together.last().copied();
        let id = self.tasks.len();
        self.tasks.push(TaskMeta {
            state: TaskLife::Runnable,
            waiting: None,
            outcome: None,
            together,
            cancelled: false,
            select_fire: None,
        });
        if let Some(group) = together {
            if let Some(entry) = self.together_groups.get_mut(group) {
                entry.tasks.push(id);
            }
        }
        self.pending_spawns.push_back((machine, id));
        Ok(Value::Task(id))
    }

    pub fn wait_task(&mut self, target: usize, span: Span) -> OpResult<Value> {
        let Some(entry) = self.tasks.get(target) else {
            return OpResult::Stop(Fault::Confused("a wait named a task that is not there"));
        };
        if entry.cancelled {
            return OpResult::Stop(Fault::Confused("a wait reached a task that was cancelled"));
        }
        match &entry.outcome {
            Some(Ok(value)) => OpResult::Ready(value.clone()),
            Some(Err(error)) => OpResult::Failed(error.clone()),
            None => OpResult::Park(WaitSite::WaitTask { task: target, span }),
        }
    }

    pub fn begin_together(&mut self) -> usize {
        let group = self.together_groups.len();
        self.together_groups.push(TogetherGroup {
            tasks: Vec::new(),
            waiter: None,
            failed: None,
        });
        self.active_together.push(group);
        group
    }

    pub fn end_together(&mut self, group: usize, waiter: usize) -> OpResult {
        let Some(entry) = self.together_groups.get_mut(group) else {
            return OpResult::Stop(Fault::Confused("a together named a group that is not there"));
        };
        entry.waiter = Some(waiter);
        if let Some(error) = entry.failed.clone() {
            self.active_together.pop();
            return OpResult::Failed(error);
        }
        if entry.tasks.iter().all(|id| self.tasks.get(*id).is_some_and(|t| t.outcome.is_some())) {
            self.active_together.pop();
            return OpResult::Done;
        }
        OpResult::Park(WaitSite::Together { group })
    }

    pub fn try_select(
        &mut self,
        task: usize,
        descriptor: &SelectDescriptor,
        channels: &[Value],
        resume_pc: usize,
    ) -> SelectStep {
        let mut channel_index = 0usize;
        for (arm, compiled) in descriptor.arms.iter().enumerate() {
            match compiled {
                SelectArm::Receive { .. } => {
                    let Some(channel_value) = channels.get(channel_index) else { continue };
                    channel_index += 1;
                    let Some(channel) = Self::channel_id(channel_value) else { continue };
                    let Some(state) = self.channels.get_mut(&channel) else { continue };
                    if let Some(value) = state.rendezvous_to.remove(&task) {
                        return SelectStep::Arm { arm, value: Some(value) };
                    }
                    if let Some((sender, value)) = state.rendezvous_sends.pop_front() {
                        state.send_waiters.retain(|waiter| *waiter != sender);
                        self.completed_sends.insert(sender);
                        self.wake(sender);
                        return SelectStep::Arm { arm, value: Some(value) };
                    }
                    if let Some(value) = state.buffer.first().cloned() {
                        state.buffer.remove(0);
                        return SelectStep::Arm { arm, value: Some(value) };
                    }
                    // A closed, drained channel is not ready for `select`; use
                    // `receive` when you want `nothing` from one.
                }
                SelectArm::Timeout { milliseconds, .. } => {
                    if *milliseconds <= 0 {
                        return SelectStep::Arm { arm, value: None };
                    }
                    if self.tasks.get(task).and_then(|entry| entry.select_fire) == Some(arm) {
                        if let Some(entry) = self.tasks.get_mut(task) {
                            entry.select_fire = None;
                        }
                        return SelectStep::Arm { arm, value: None };
                    }
                }
            }
        }

        if descriptor.otherwise.is_some() {
            return SelectStep::Otherwise;
        }

        let now = Instant::now();
        channel_index = 0;
        for (arm, compiled) in descriptor.arms.iter().enumerate() {
            match compiled {
                SelectArm::Receive { .. } => {
                    if let Some(channel_value) = channels.get(channel_index) {
                        channel_index += 1;
                        if let Some(channel) = Self::channel_id(channel_value) {
                            if let Some(state) = self.channels.get_mut(&channel) {
                                if !state.closed && !state.recv_waiters.contains(&task) {
                                    state.recv_waiters.push(task);
                                }
                            }
                        }
                    }
                }
                SelectArm::Timeout { milliseconds, .. } => {
                    if *milliseconds > 0 {
                        self.timeouts.push(PendingTimeout {
                            at: now + Duration::from_millis(*milliseconds as u64),
                            task,
                            select_arm: arm,
                        });
                    }
                }
            }
        }

        SelectStep::Park { resume_pc }
    }

    pub fn run_shared_update(
        &mut self,
        shared: &Value,
        change: Value,
        world: &mut crate::machine::World,
    ) -> Result<Value, RuntimeError> {
        let Value::Shared(held) = shared else {
            return Err(confused("`.update` reached something that is not shared"));
        };
        let Value::Function(closure) = change else {
            return Err(confused("`.update` was not given a function"));
        };
        let body = closure.body as usize;
        let current = held.get().clone();
        let mut machine = match Machine::for_body(&self.program, body, 0, Some(Ref::clone(&closure))) {
            Ok(machine) => machine,
            Err(fault) => return Err(RuntimeError { fault, trace: Vec::new() }),
        };
        if let Err(fault) = machine.set_argument(0, current) {
            return Err(RuntimeError { fault, trace: Vec::new() });
        }
        let mut scratch = Host::scratch(&self.program);
        match machine.run_alone(world, &mut scratch) {
            Ok(answer) => {
                held.set(answer);
                Ok(Value::Nothing)
            }
            Err(error) => Err(error),
        }
    }

    fn all_parked(&self) -> bool {
        self.tasks.iter().all(|entry| {
            matches!(entry.state, TaskLife::Parked | TaskLife::Done) || entry.cancelled
        })
    }

    fn register_wait(&mut self, task: usize, site: &WaitSite, _runnable: &mut VecDeque<usize>) {
        match site {
            WaitSite::Send { channel, .. } => {
                if let Some(state) = self.channels.get_mut(channel) {
                    if !state.send_waiters.contains(&task) {
                        state.send_waiters.push(task);
                    }
                }
            }
            WaitSite::Receive { channel, .. } => {
                if let Some(state) = self.channels.get_mut(channel) {
                    if !state.recv_waiters.contains(&task) {
                        state.recv_waiters.push(task);
                    }
                }
            }
            WaitSite::WaitTask { .. } | WaitSite::Together { .. } | WaitSite::Select { .. } => {}
        }
    }

    fn try_wake_receiver(&mut self, task: usize, channel: u32) -> bool {
        let Some(entry) = self.tasks.get_mut(task) else { return false };
        if !matches!(entry.waiting, Some(WaitSite::Receive { channel: c, .. }) if c == channel) {
            return false;
        }
        entry.waiting = None;
        entry.state = TaskLife::Runnable;
        self.wake(task);
        true
    }

    fn try_wake_sender(&mut self, task: usize, channel: u32) -> bool {
        let Some(entry) = self.tasks.get_mut(task) else { return false };
        if !matches!(entry.waiting, Some(WaitSite::Send { channel: c, .. }) if c == channel) {
            return false;
        }
        entry.waiting = None;
        entry.state = TaskLife::Runnable;
        self.wake(task);
        true
    }

    fn wake_task_waiters(&mut self, finished: usize, runnable: &mut VecDeque<usize>) {
        for (id, entry) in self.tasks.iter_mut().enumerate() {
            if matches!(entry.waiting, Some(WaitSite::WaitTask { task, .. }) if task == finished) {
                entry.waiting = None;
                entry.state = TaskLife::Runnable;
                runnable.push_back(id);
            }
        }
    }

    fn finish_together_task(&mut self, task: usize, runnable: &mut VecDeque<usize>) {
        let group = self.tasks.get(task).and_then(|entry| entry.together);
        let Some(group) = group else { return };
        let Some(entry) = self.together_groups.get(group) else { return };
        if !entry.tasks.iter().all(|id| self.tasks.get(*id).is_some_and(|t| t.outcome.is_some())) {
            return;
        }
        let Some(waiter) = entry.waiter else { return };
        if let Some(waiting) = self.tasks.get_mut(waiter) {
            if matches!(waiting.waiting, Some(WaitSite::Together { group: g }) if g == group) {
                waiting.waiting = None;
                waiting.state = TaskLife::Runnable;
                runnable.push_back(waiter);
            }
        }
    }

    fn fail_together(&mut self, group: usize, runnable: &mut VecDeque<usize>, machines: &[Machine]) {
        let Some(entry) = self.together_groups.get_mut(group) else { return };
        let victims: Vec<usize> = entry.tasks.clone();
        for victim in victims {
            if victim == 0 {
                continue;
            }
            self.cancel_task(victim, runnable, machines);
        }
    }

    fn cancel_task(&mut self, task: usize, runnable: &mut VecDeque<usize>, machines: &[Machine]) {
        if let Some(entry) = self.tasks.get_mut(task) {
            if entry.outcome.is_some() {
                return;
            }
            entry.cancelled = true;
            entry.state = TaskLife::Done;
            entry.outcome = Some(Err(RuntimeError {
                fault: Fault::Confused(
                    "a task was cancelled because another in the same `together` failed",
                ),
                trace: machines.get(task).map(|m| m.trace(&self.program)).unwrap_or_default(),
            }));
            entry.waiting = None;
        }
        runnable.retain(|id| *id != task);
        self.wake_task_waiters(task, runnable);
    }

    fn fire_timeouts(&mut self, runnable: &mut VecDeque<usize>) {
        let now = Instant::now();
        let due: Vec<PendingTimeout> = self
            .timeouts
            .iter()
            .filter(|pending| pending.at <= now)
            .cloned()
            .collect();
        if due.is_empty() {
            return;
        }
        self.timeouts.retain(|pending| pending.at > now);
        for pending in due {
            if let Some(entry) = self.tasks.get_mut(pending.task) {
                if entry.state == TaskLife::Parked {
                    entry.select_fire = Some(pending.select_arm);
                    entry.waiting = None;
                    entry.state = TaskLife::Runnable;
                    runnable.push_back(pending.task);
                }
            }
        }
    }

    fn deadlock(&self, machines: &[Machine]) -> RuntimeError {
        let mut lines: Vec<String> = Vec::new();
        for (id, entry) in self.tasks.iter().enumerate() {
            if entry.state != TaskLife::Parked {
                continue;
            }
            let name = machines
                .get(id)
                .map(|machine| machine.trace(&self.program))
                .and_then(|trace| trace.first().map(|level| level.name.to_string()))
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
            trace: machines.first().map(|m| m.trace(&self.program)).unwrap_or_default(),
        }
    }
}

fn confused(detail: &'static str) -> RuntimeError {
    RuntimeError { fault: Fault::Confused(detail), trace: Vec::new() }
}
