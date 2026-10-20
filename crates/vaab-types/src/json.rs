//! Which types may be turned into JSON.

use std::collections::HashSet;

use crate::checked::{CastId, Checked, ChoiceId, TypeId};
use crate::types::Type;

pub fn can_json(value: &Type, checked: &Checked) -> bool {
    let mut visited = HashSet::new();
    can_json_inner(value, checked, &mut visited)
}

fn can_json_inner(value: &Type, checked: &Checked, visited: &mut HashSet<String>) -> bool {
    match value {
        Type::Named(name) if name == "Json" => true,
        Type::Int | Type::Float | Type::Bool | Type::Text | Type::Bytes | Type::Nothing => true,
        Type::List(item) | Type::Maybe(item) => can_json_inner(item, checked, visited),
        Type::Map { key, value } => {
            **key == Type::Text && can_json_inner(value, checked, visited)
        }
        Type::Tuple(items) => items.iter().all(|item| can_json_inner(item, checked, visited)),
        Type::Variable(_) => false,
        Type::Named(name) => {
            if !visited.insert(name.clone()) {
                return true;
            }
            if let Some((index, _)) =
                checked.declared_types.iter().enumerate().find(|(_, d)| d.name == *name)
            {
                record_can_json(TypeId(index as u32), checked, visited)
            } else if let Some((index, _)) =
                checked.declared_casts.iter().enumerate().find(|(_, d)| d.name == *name)
            {
                cast_can_json(CastId(index as u32), checked, visited)
            } else if let Some((index, _)) =
                checked.choices.iter().enumerate().find(|(_, c)| c.name == *name)
            {
                choice_can_json(ChoiceId(index as u32), checked, visited)
            } else if let Some((index, _)) =
                checked.declared_types.iter().enumerate().find(|(_, d)| matches_riff_name(&d.name, name))
            {
                record_can_json(TypeId(index as u32), checked, visited)
            } else if let Some((index, _)) =
                checked.declared_casts.iter().enumerate().find(|(_, d)| matches_riff_name(&d.name, name))
            {
                cast_can_json(CastId(index as u32), checked, visited)
            } else if let Some((index, _)) =
                checked.choices.iter().enumerate().find(|(_, c)| matches_riff_name(&c.name, name))
            {
                choice_can_json(ChoiceId(index as u32), checked, visited)
            } else {
                false
            }
        }
        Type::Parameter(_) | Type::Unknown => true,
        Type::Function { .. }
        | Type::Fallible { .. }
        | Type::Task(_)
        | Type::Shared(_)
        | Type::Channel(_)
        | Type::Hub(_)
        | Type::Db
        | Type::Store
        | Type::Logger
        | Type::Socket
        | Type::Query { .. }
        | Type::Ability(_) => false,
    }
}

fn matches_riff_name(declared: &str, written: &str) -> bool {
    declared.rsplit_once("__").is_some_and(|(_, suffix)| suffix == written)
}

fn record_can_json(id: TypeId, checked: &Checked, visited: &mut HashSet<String>) -> bool {
    checked
        .declared_type(id)
        .is_some_and(|d| d.fields.iter().all(|f| can_json_inner(&f.declared, checked, visited)))
}

fn cast_can_json(id: CastId, checked: &Checked, visited: &mut HashSet<String>) -> bool {
    checked
        .declared_cast(id)
        .is_some_and(|d| d.fields.iter().all(|f| can_json_inner(&f.declared, checked, visited)))
}

fn choice_can_json(id: ChoiceId, checked: &Checked, visited: &mut HashSet<String>) -> bool {
    checked.choice(id).is_some_and(|c| {
        c.variants
            .iter()
            .all(|v| v.fields.iter().all(|f| can_json_inner(&f.declared, checked, visited)))
    })
}
