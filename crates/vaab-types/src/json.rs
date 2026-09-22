//! Which types may be turned into JSON.

use crate::checked::{CastId, Checked, ChoiceId, TypeId};
use crate::types::Type;

pub fn can_json(value: &Type, checked: &Checked) -> bool {
    match value {
        Type::Int | Type::Float | Type::Bool | Type::Text | Type::Nothing => true,
        Type::List(item) | Type::Maybe(item) => can_json(item, checked),
        Type::Map { key, value } => **key == Type::Text && can_json(value, checked),
        Type::Tuple(items) => items.iter().all(|item| can_json(item, checked)),
        Type::Variable(_) => false,
        Type::Named(name) => {
            if let Some((index, _)) =
                checked.declared_types.iter().enumerate().find(|(_, d)| d.name == *name)
            {
                record_can_json(TypeId(index as u32), checked)
            } else if let Some((index, _)) =
                checked.declared_casts.iter().enumerate().find(|(_, d)| d.name == *name)
            {
                cast_can_json(CastId(index as u32), checked)
            } else if let Some((index, _)) =
                checked.choices.iter().enumerate().find(|(_, c)| c.name == *name)
            {
                choice_can_json(ChoiceId(index as u32), checked)
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
        | Type::Db
        | Type::Store
        | Type::Query { .. }
        | Type::Ability(_) => false,
    }
}

fn record_can_json(id: TypeId, checked: &Checked) -> bool {
    checked.declared_type(id).is_some_and(|d| d.fields.iter().all(|f| can_json(&f.declared, checked)))
}

fn cast_can_json(id: CastId, checked: &Checked) -> bool {
    checked
        .declared_cast(id)
        .is_some_and(|d| d.fields.iter().all(|f| can_json(&f.declared, checked)))
}

fn choice_can_json(id: ChoiceId, checked: &Checked) -> bool {
    checked.choice(id).is_some_and(|c| c.variants.iter().all(|v| v.fields.iter().all(|f| can_json(&f.declared, checked))))
}
