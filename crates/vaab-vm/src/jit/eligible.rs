use crate::bytecode::{Body, Op, Program};

/// Whether a body is safe to compile to native code with the tier-1 JIT.
///
/// Tier 1 only handles pure integer arithmetic, locals, control flow, and calls
/// to other tier-1 bodies with no captures.
pub fn is_eligible(program: &Program, body: &Body, body_id: usize) -> bool {
    if !body.captures.is_empty() {
        return false;
    }

    if references_self(program, body_id, body) {
        return false;
    }

    if calls_other_body(program, body_id, body) {
        return false;
    }

    body.code.iter().all(|op| is_allowed_op(program, *op))
}

fn references_self(program: &Program, body_id: usize, body: &Body) -> bool {
    body.code.iter().any(|op| {
        matches!(op, Op::Constant(index) if function_target(program, *index) == Some(body_id))
    })
}

fn calls_other_body(program: &Program, body_id: usize, body: &Body) -> bool {
    body.code.iter().any(|op| {
        matches!(
            op,
            Op::Constant(index) if function_target(program, *index).is_some_and(|target| target != body_id)
        )
    })
}

fn function_target(program: &Program, index: u32) -> Option<usize> {
    let value = program.constants.get(index as usize)?;
    let crate::value::Value::Function(closure) = value else {
        return None;
    };
    Some(closure.body as usize)
}

fn is_allowed_op(program: &Program, op: Op) -> bool {
    match op {
        Op::Int(_)
        | Op::LoadLocal(_)
        | Op::StoreLocal(_)
        | Op::Add
        | Op::Subtract
        | Op::Multiply
        | Op::Negate
        | Op::Less
        | Op::LessOrEqual
        | Op::Greater
        | Op::GreaterOrEqual
        | Op::Equal
        | Op::NotEqual
        | Op::Jump(_)
        | Op::JumpIfFalse(_)
        | Op::JumpIfTrue(_)
        | Op::Return
        | Op::Pop
        | Op::Nothing => true,

        Op::Call(arity) => (1..=8).contains(&arity),

        Op::Constant(index) => constant_is_function(program, index),

        _ => false,
    }
}

fn constant_is_function(program: &Program, index: u32) -> bool {
    let Some(value) = program.constants.get(index as usize) else {
        return false;
    };
    let crate::value::Value::Function(closure) = value else {
        return false;
    };
    closure.captures.is_empty()
}
