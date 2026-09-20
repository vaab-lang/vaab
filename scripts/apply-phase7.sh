#!/bin/sh
set -e
cd "$(dirname "$0")/.."

python3 << 'PY'
from pathlib import Path

v = Path('crates/vaab-vm/src/value.rs').read_text()
v = v.replace('use std::cell::Cell;\n', '')
if 'use std::sync::Mutex;' not in v:
    v = v.replace('use std::hash::{Hash, Hasher};\n\n', 'use std::hash::{Hash, Hasher};\nuse std::sync::Mutex;\n\n')
v = v.replace('pub type Ref<T> = std::rc::Rc<T>;', 'pub type Ref<T> = std::sync::Arc<T>;')
v = v.replace('pub struct Captured(Cell<Value>);', 'pub struct Captured(Mutex<Value>);')
v = v.replace('Captured(Cell::new(value))', 'Captured(Mutex::new(value))')
v = v.replace('''    pub fn get(&self) -> Value {
        // `Cell` gives nothing out by reference, so the value is taken, copied and
        // put straight back. Every alternative in the standard library can panic.
        let held = self.0.take();
        self.0.set(held.clone());
        held
    }

    pub fn set(&self, value: Value) {
        self.0.set(value);
    }''', '''    pub fn get(&self) -> Value {
        match self.0.lock() {
            Ok(held) => held.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set(&self, value: Value) {
        match self.0.lock() {
            Ok(mut held) => *held = value,
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }''')
Path('crates/vaab-vm/src/value.rs').write_text(v)

c = Path('crates/vaab-vm/src/concurrency.rs').read_text()
if 'mod parallel' not in c:
    c = c.replace('//! The single-threaded scheduler tasks and channels run on.\n//!\n//! Phase 7 swaps [`value::Ref`] for `Arc` and the cells here for locks; the shape\n//! of this module stays the same.\n', '//! The work-stealing scheduler tasks and channels run on.\n')
    c = c.replace('use crate::value::{Ref, Value};\n\npub const PREEMPT_AFTER', 'use crate::value::{Ref, Value};\n\n#[path = "parallel.rs"]\nmod parallel;\n\npub const PREEMPT_AFTER')
    c = c.replace('''pub fn run(world: &mut crate::machine::World) -> Result<Value, RuntimeError> {
    let mut scheduler = Scheduler::new(Ref::clone(&world.program));
    match scheduler.run(world) {
        Run::Finished(value) => Ok(value),
        Run::Failed(error) => Err(error),
    }
}''', '''pub fn run(world: &mut crate::machine::World) -> Result<Value, RuntimeError> {
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
}''')
    for o, n in [
        ('    program: Ref<Program>,', '    pub(crate) program: Ref<Program>,'),
        ('    tasks: Vec<TaskMeta>,', '    pub(crate) tasks: Vec<TaskMeta>,'),
        ('    together_groups: Vec<TogetherGroup>,', '    pub(crate) together_groups: Vec<TogetherGroup>,'),
        ('    timeouts: Vec<PendingTimeout>,', '    pub(crate) timeouts: Vec<PendingTimeout>,'),
        ('    current: usize,', '    pub(crate) current: usize,'),
        ('    pending_spawns: VecDeque<(Machine, usize)>,', '    pub(crate) pending_spawns: VecDeque<(Machine, usize)>,'),
        ('    pending_wakeups: VecDeque<usize>,', '    pub(crate) pending_wakeups: VecDeque<usize>,'),
        ('enum TaskLife {', 'pub(crate) enum TaskLife {'),
        ('struct TaskMeta {', 'pub(crate) struct TaskMeta {'),
        ('    state: TaskLife,', '    pub(crate) state: TaskLife,'),
        ('    waiting: Option<WaitSite>,', '    pub(crate) waiting: Option<WaitSite>,'),
        ('    outcome: Option<Result<Value, RuntimeError>>,', '    pub(crate) outcome: Option<Result<Value, RuntimeError>>,'),
        ('    together: Option<usize>,', '    pub(crate) together: Option<usize>,'),
        ('    cancelled: bool,', '    pub(crate) cancelled: bool,'),
        ('struct TogetherGroup {', 'pub(crate) struct TogetherGroup {'),
        ('    tasks: Vec<usize>,', '    pub(crate) tasks: Vec<usize>,'),
        ('    failed: Option<RuntimeError>,', '    pub(crate) failed: Option<RuntimeError>,'),
        ('struct PendingTimeout {', 'pub(crate) struct PendingTimeout {'),
        ('    host: Host,', '    pub(crate) host: Host,'),
        ('    machines: Vec<Machine>,', '    pub(crate) machines: Vec<Machine>,'),
    ]:
        c = c.replace(o, n)
    Path('crates/vaab-vm/src/concurrency.rs').write_text(c)
PY

git show HEAD:crates/vaab-vm/src/compile/stmt.rs > crates/vaab-vm/src/compile/stmt.rs

cp scripts/parallel.rs crates/vaab-vm/src/parallel.rs
echo "Phase 7 patches applied."
