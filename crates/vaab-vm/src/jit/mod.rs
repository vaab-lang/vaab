//! Tier-1 Cranelift JIT — hot pure-`Int` bodies compile to native code at runtime.
//!
//! Vaab uses a **tiered JIT**, not AOT:
//! - **Tier 0**: bytecode interpreter (always available, supports channels/tasks/I/O)
//! - **Tier 1**: Cranelift native code for pure integer bodies with no captures
//!
//! This keeps `vaab repl` and `vaab serve` startup instant while still speeding up
//! numeric hot paths like recursive functions once they get warm.

mod compile;
mod eligible;

use std::collections::HashMap;

use cranelift_codegen::settings::{self, Configurable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use crate::bytecode::Program;
use crate::value::Value;

pub use compile::NativeIntFn;
use compile::NativeIntFn as CompiledFn;
pub use eligible::is_eligible;

const WARMUP_CALLS: u32 = 1;

pub struct JitEngine {
    pub(crate) module: JITModule,
    pub(crate) func_ids: HashMap<usize, FuncId>,
    compiled: HashMap<usize, CompiledFn>,
    failed: HashMap<usize, String>,
    call_counts: HashMap<usize, u32>,
}

impl JitEngine {
    pub fn new() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").map_err(|error| error.to_string())?;
        flag_builder.set("is_pic", "false").map_err(|error| error.to_string())?;
        let isa_builder = cranelift_native::builder().map_err(|error| error.to_string())?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|error| error.to_string())?;
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        Ok(Self {
            module: JITModule::new(builder),
            func_ids: HashMap::new(),
            compiled: HashMap::new(),
            failed: HashMap::new(),
            call_counts: HashMap::new(),
        })
    }

    /// Records a call and compiles the body once it is warm enough.
    pub fn note_call(&mut self, program: &Program, body_id: usize) -> Option<CompiledFn> {
        if let Some(native) = self.compiled.get(&body_id).copied() {
            return Some(native);
        }
        if self.failed.contains_key(&body_id) {
            return None;
        }

        let count = self.call_counts.entry(body_id).or_insert(0);
        *count += 1;
        if *count < WARMUP_CALLS {
            return None;
        }

        let body = program.body(body_id)?;
        if !is_eligible(program, body, body_id) {
            self.failed.insert(body_id, "not eligible".into());
            return None;
        }

        match compile::compile_body(self, program, body_id) {
            Ok(native) => {
                self.compiled.insert(body_id, native);
                Some(native)
            }
            Err(error) => {
                self.failed.insert(body_id, error.clone());
                None
            }
        }
    }

    pub fn get(&self, body_id: usize) -> Option<CompiledFn> {
        self.compiled.get(&body_id).copied()
    }

    pub(crate) fn func_id_for(&mut self, body_id: usize, parameters: usize) -> Result<FuncId, String> {
        if let Some(id) = self.func_ids.get(&body_id) {
            return Ok(*id);
        }
        let mut sig = self.module.make_signature();
        for _ in 0..parameters {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(cranelift_codegen::ir::types::I64));
        }
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(cranelift_codegen::ir::types::I64));
        let name = format!("vaab_body_{body_id}");
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|error| error.to_string())?;
        self.func_ids.insert(body_id, id);
        Ok(id)
    }
}

impl Default for JitEngine {
    fn default() -> Self {
        Self::new().expect("Cranelift JIT initialises on this host")
    }
}

/// Try to run a tier-1 native function. Returns `None` when the call must stay interpreted.
pub fn try_native_call(
    engine: &mut JitEngine,
    program: &Program,
    body_id: usize,
    parameters: usize,
    stack: &[Value],
    arity: usize,
) -> Option<i64> {
    if parameters != arity {
        return None;
    }

    let args_start = stack.len().checked_sub(arity)?;
    for value in &stack[args_start..] {
        if !matches!(value, Value::Int(_)) {
            return None;
        }
    }

    let native = engine.note_call(program, body_id)?;
    let args: Vec<i64> = stack[args_start..]
        .iter()
        .map(|value| match value {
            Value::Int(number) => *number,
            _ => unreachable!(),
        })
        .collect();
    Some(native.call(&args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vaab_syntax::parse;
    use vaab_types::check;

    #[test]
    fn compiles_accumulate_to_native() {
        let source = "to accumulate(n: Int) returns Int {
    let changing acc = 0
    let changing i = 0
    while i < n {
        acc = acc + i
        i = i + 1
    }
    return acc
}
";
        let parsed = parse(source);
        let checked = check(&parsed.module).expect("check");
        let program = crate::value::Ref::new(crate::compile(&parsed.module, &checked));
        let body = program.body(1).expect("accumulate body");
        assert!(is_eligible(&program, body, 1));
        let mut engine = JitEngine::new().expect("jit engine");
        super::compile::compile_body(&mut engine, &program, 1).expect("accumulate should compile");
    }
}

