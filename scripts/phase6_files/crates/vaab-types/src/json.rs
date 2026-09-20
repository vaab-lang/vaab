//! Which types may be turned into JSON.

use crate::checked::{Checked, ChoiceId, TypeId};
use crate::types::Type;

pub fn can_json(value: &Type, checked: &Checked) -> bool {
    match value {
        Type::Int | Type::Float | Type::Bool | Type::Text | Type::Nothing => true,
        Type::List(item) | Type::Maybe(item) => can_json(item, checked),
        Type::Map { key, value } => **key == Type::Text && can_json(value, checked),
        Type::Tuple(items) => items.iter().all(|item| can_json(item, checked)),
        Type::Variable(_) => false,
        Type::Named(name) => {
            if let Some((index, _)) = checked
                .declared_types
                .iter()
                .enumerate()
                .find(|(_, declared)| declared.name == *name)
            {
                record_can_json(TypeId(index as u32), checked)
            } else if let Some((index, _)) = checked
                .choices
                .iter()
                .enumerate()
                .find(|(_, choice)| choice.name == *name)
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
        | Type::Ability(_) => false,
    }
}

fn record_can_json(id: TypeId, checked: &Checked) -> bool {
    let Some(declared) = checked.declared_type(id) else {
        return false;
    };
    declared.fields.iter().all(|field| can_json(&field.declared, checked))
}

fn choice_can_json(id: ChoiceId, checked: &Checked) -> bool {
    let Some(choice) = checked.choice(id) else {
        return false;
    };
    choice
        .variants
        .iter()
        .all(|variant| variant.fields.iter().all(|field| can_json(&field.declared, checked)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checked::{DeclaredType, Field};
    use vaab_syntax::ast::NodeId;
    use vaab_syntax::span::Span;

    #[test]
    fn built_in_scalars_can_json() {
        let checked = Checked::default();
        assert!(can_json(&Type::Int, &checked));
    }

    #[test]
    fn a_record_can_json_when_every_field_can() {
        let checked = Checked {
            declared_types: vec![DeclaredType {
                name: "Person".to_string(),
                fields: vec![Field {
                    name: "name".to_string(),
                    declared: Type::Text,
                    has_default: false,
                    span: Span::default(),
                }],
                methods: Vec::new(),
                abilities: Vec::new(),
                constructor: crate::checked::Constructor::Automatic,
                declaration: NodeId(0),
                span: Span::default(),
            }],
            ..Checked::default()
        };
        assert!(can_json(&Type::named("Person"), &checked));
    }
}
