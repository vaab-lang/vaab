use std::collections::{HashMap, HashSet, VecDeque};

use cranelift_codegen::ir::{types, Block, FuncRef, InstBuilder, Value};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::Module;

use crate::bytecode::Op;
use crate::bytecode::Program;
use crate::value::Value as RuntimeValue;

use super::JitEngine;

#[derive(Clone, Copy)]
pub enum NativeIntFn {
    Int0(fn() -> i64),
    Int1(fn(i64) -> i64),
    Int2(fn(i64, i64) -> i64),
    Int3(fn(i64, i64, i64) -> i64),
    Int4(fn(i64, i64, i64, i64) -> i64),
}

impl NativeIntFn {
    pub fn call(self, args: &[i64]) -> i64 {
        match self {
            NativeIntFn::Int0(function) => function(),
            NativeIntFn::Int1(function) => function(args[0]),
            NativeIntFn::Int2(function) => function(args[0], args[1]),
            NativeIntFn::Int3(function) => function(args[0], args[1], args[2]),
            NativeIntFn::Int4(function) => function(args[0], args[1], args[2], args[3]),
        }
    }
}

enum Sim {
    Value(Value),
    Callee(usize),
}

pub fn compile_body(
    engine: &mut JitEngine,
    program: &Program,
    body_id: usize,
) -> Result<NativeIntFn, String> {
    let body = program
        .body(body_id)
        .ok_or_else(|| format!("body {body_id} is missing"))?;

    let func_id = engine.func_id_for(body_id, body.parameters)?;
    let mut ctx = engine.module.make_context();
    ctx.func.signature = engine.module.declarations().get_function_decl(func_id).signature.clone();

    let self_ref = engine.module.declare_func_in_func(func_id, &mut ctx.func);

    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);

        let params: Vec<Value> = builder.block_params(entry).to_vec();
        let mut locals = Vec::with_capacity(body.slots);
        for slot in 0..body.slots {
            let var = Variable::from_u32(slot as u32);
            builder.declare_var(var, types::I64);
            if slot < body.parameters {
                builder.def_var(var, params[slot]);
            } else {
                let zero = builder.ins().iconst(types::I64, 0);
                builder.def_var(var, zero);
            }
            locals.push(var);
        }

        let start = builder.create_block();
        builder.ins().jump(start, &[]);

        let mut blocks: HashMap<usize, Block> = HashMap::new();
        blocks.insert(0, start);
        let mut call_targets: HashMap<usize, FuncRef> = HashMap::new();

        let block_at = |pc: usize, builder: &mut FunctionBuilder, blocks: &mut HashMap<usize, Block>| {
            *blocks.entry(pc).or_insert_with(|| builder.create_block())
        };

        let mut stack: Vec<Sim> = Vec::new();
        let mut queue = VecDeque::from([0usize]);
        let mut visited = HashSet::new();

        while let Some(pc) = queue.pop_front() {
            if !visited.insert(pc) {
                continue;
            }

            let current = block_at(pc, &mut builder, &mut blocks);
            builder.switch_to_block(current);
            stack.clear();

            let mut index = pc;
            loop {
                if index >= body.code.len() {
                    let zero = builder.ins().iconst(types::I64, 0);
                    builder.ins().return_(&[zero]);
                    break;
                }

                match body.code[index] {
                    Op::Int(number) => stack.push(Sim::Value(builder.ins().iconst(types::I64, number))),
                    Op::LoadLocal(slot) => stack.push(Sim::Value(builder.use_var(locals[slot as usize]))),
                    Op::StoreLocal(slot) => {
                        let Sim::Value(value) = stack.pop().ok_or("store with empty stack")? else {
                            return Err("store with callee marker on stack".into());
                        };
                        builder.def_var(locals[slot as usize], value);
                    }
                    Op::Add | Op::Subtract | Op::Multiply | Op::Divide | Op::Remainder => {
                        let Sim::Value(right) = stack.pop().ok_or("binary op with empty stack")? else {
                            return Err("binary op with callee marker on stack".into());
                        };
                        let Sim::Value(left) = stack.pop().ok_or("binary op with empty stack")? else {
                            return Err("binary op with callee marker on stack".into());
                        };
                        stack.push(Sim::Value(match body.code[index] {
                            Op::Add => builder.ins().iadd(left, right),
                            Op::Subtract => builder.ins().isub(left, right),
                            Op::Multiply => builder.ins().imul(left, right),
                            Op::Divide => builder.ins().sdiv(left, right),
                            Op::Remainder => builder.ins().srem(left, right),
                            _ => unreachable!(),
                        }));
                    }
                    Op::Negate => {
                        let Sim::Value(value) = stack.pop().ok_or("negate with empty stack")? else {
                            return Err("negate with callee marker on stack".into());
                        };
                        stack.push(Sim::Value(builder.ins().ineg(value)));
                    }
                    Op::Less | Op::LessOrEqual | Op::Greater | Op::GreaterOrEqual => {
                        let Sim::Value(right) = stack.pop().ok_or("compare with empty stack")? else {
                            return Err("compare with callee marker on stack".into());
                        };
                        let Sim::Value(left) = stack.pop().ok_or("compare with empty stack")? else {
                            return Err("compare with callee marker on stack".into());
                        };
                        let cmp = match body.code[index] {
                            Op::Less => builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::SignedLessThan, left, right),
                            Op::LessOrEqual => builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual, left, right),
                            Op::Greater => builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan, left, right),
                            Op::GreaterOrEqual => builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual, left, right),
                            _ => unreachable!(),
                        };
                        stack.push(Sim::Value(builder.ins().uextend(types::I64, cmp)));
                    }
                    Op::Equal | Op::NotEqual => {
                        let Sim::Value(right) = stack.pop().ok_or("compare with empty stack")? else {
                            return Err("compare with callee marker on stack".into());
                        };
                        let Sim::Value(left) = stack.pop().ok_or("compare with empty stack")? else {
                            return Err("compare with callee marker on stack".into());
                        };
                        let same = builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, left, right);
                        let value = if body.code[index] == Op::Equal {
                            same
                        } else {
                            builder.ins().bnot(same)
                        };
                        stack.push(Sim::Value(builder.ins().uextend(types::I64, value)));
                    }
                    Op::Jump(target) => {
                        let target_pc = target as usize;
                        let target_block = block_at(target_pc, &mut builder, &mut blocks);
                        builder.ins().jump(target_block, &[]);
                        queue.push_back(target_pc);
                        break;
                    }
                    Op::JumpIfFalse(target) => {
                        let Sim::Value(condition) = stack.pop().ok_or("jump with empty stack")? else {
                            return Err("jump with callee marker on stack".into());
                        };
                        let is_false = builder.ins().icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, condition, 0);
                        let taken_pc = target as usize;
                        let fallthrough_pc = index + 1;
                        let taken = block_at(taken_pc, &mut builder, &mut blocks);
                        let fallthrough = block_at(fallthrough_pc, &mut builder, &mut blocks);
                        builder.ins().brif(is_false, taken, &[], fallthrough, &[]);
                        queue.push_back(taken_pc);
                        queue.push_back(fallthrough_pc);
                        break;
                    }
                    Op::JumpIfTrue(target) => {
                        let Sim::Value(condition) = stack.pop().ok_or("jump with empty stack")? else {
                            return Err("jump with callee marker on stack".into());
                        };
                        let is_true = builder.ins().icmp_imm(cranelift_codegen::ir::condcodes::IntCC::NotEqual, condition, 0);
                        let taken_pc = target as usize;
                        let fallthrough_pc = index + 1;
                        let taken = block_at(taken_pc, &mut builder, &mut blocks);
                        let fallthrough = block_at(fallthrough_pc, &mut builder, &mut blocks);
                        builder.ins().brif(is_true, taken, &[], fallthrough, &[]);
                        queue.push_back(taken_pc);
                        queue.push_back(fallthrough_pc);
                        break;
                    }
                    Op::Return => {
                        let value = match stack.pop() {
                            Some(Sim::Value(value)) => value,
                            _ => builder.ins().iconst(types::I64, 0),
                        };
                        builder.ins().return_(&[value]);
                        break;
                    }
                    Op::Pop => {
                        stack.pop();
                    }
                    Op::Nothing => {}
                    Op::Constant(index) => {
                        if let Some(target) = function_target(program, index) {
                            stack.push(Sim::Callee(target));
                        } else {
                            stack.push(Sim::Value(builder.ins().iconst(types::I64, 0)));
                        }
                    }
                    Op::Call(arity) => {
                        let mut args = Vec::with_capacity(arity as usize);
                        for _ in 0..arity {
                            match stack.pop().ok_or("call with too few stack values")? {
                                Sim::Value(value) => args.push(value),
                                Sim::Callee(_) => return Err("call argument was a callee marker".into()),
                            }
                        }
                        args.reverse();
                        let target = match stack.pop().ok_or("call with no callee")? {
                            Sim::Callee(target) => target,
                            Sim::Value(_) => body_id,
                        };
                        let func_ref = if target == body_id {
                            self_ref
                        } else {
                            *call_targets.entry(target).or_insert_with(|| {
                                let parameters = program.body(target).map(|shape| shape.parameters).unwrap_or(0);
                                let id = engine.func_id_for(target, parameters).expect("func id");
                                engine.module.declare_func_in_func(id, builder.func)
                            })
                        };
                        let call = builder.ins().call(func_ref, &args);
                        stack.push(Sim::Value(builder.inst_results(call)[0]));
                    }
                    op => return Err(format!("unsupported op in tier-1 JIT: {op:?}")),
                }

                index += 1;
            }
        }

        builder.seal_all_blocks();
        builder.finalize();
    }

    if let Err(error) = engine.module.define_function(func_id, &mut ctx) {
        return Err(format!("{error:?}"));
    }
    engine.module.clear_context(&mut ctx);
    engine.module.finalize_definitions().map_err(|error| error.to_string())?;
    let code = engine.module.get_finalized_function(func_id);
    Ok(match body.parameters {
        0 => NativeIntFn::Int0(unsafe { std::mem::transmute(code) }),
        1 => NativeIntFn::Int1(unsafe { std::mem::transmute(code) }),
        2 => NativeIntFn::Int2(unsafe { std::mem::transmute(code) }),
        3 => NativeIntFn::Int3(unsafe { std::mem::transmute(code) }),
        4 => NativeIntFn::Int4(unsafe { std::mem::transmute(code) }),
        _ => return Err(format!("tier-1 JIT supports up to 4 parameters, not {}", body.parameters)),
    })
}

fn function_target(program: &Program, index: u32) -> Option<usize> {
    let value = program.constants.get(index as usize)?;
    let RuntimeValue::Function(closure) = value else {
        return None;
    };
    Some(closure.body as usize)
}
