//! JSON encoding for Vaab values.

use crate::error::Fault;
use crate::value::Value;

pub fn encode(value: &Value) -> Result<String, Fault> {
    serde_json::to_string(&to_json(value))
        .map_err(|_| Fault::Confused("could not turn this value into json"))
}

fn to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Nothing => serde_json::Value::Null,
        Value::Int(number) => serde_json::Value::from(*number),
        Value::Float(number) => match serde_json::Number::from_f64(*number) {
            Some(number) => serde_json::Value::Number(number),
            None => serde_json::Value::Null,
        },
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Text(text) => serde_json::Value::String(text.to_string()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        Value::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, item) in entries.iter() {
                if let Value::Text(name) = &key.0 {
                    object.insert(name.to_string(), to_json(item));
                }
            }
            serde_json::Value::Object(object)
        }
        Value::Tuple(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        Value::Record(record) => {
            let mut object = serde_json::Map::new();
            for (name, item) in record.layout.fields.iter().zip(record.fields.iter()) {
                object.insert(name.clone(), to_json(item));
            }
            serde_json::Value::Object(object)
        }
        Value::Variant(variant) => {
            if variant.fields.is_empty() {
                return serde_json::Value::String(format!("{}.{}", variant.layout.choice, variant.layout.name));
            }
            let mut object = serde_json::Map::new();
            for (name, item) in variant.layout.fields.iter().zip(variant.fields.iter()) {
                object.insert(name.clone(), to_json(item));
            }
            let mut tagged = serde_json::Map::new();
            tagged.insert(variant.layout.name.to_string(), serde_json::Value::Object(object));
            serde_json::Value::Object(tagged)
        }
        Value::Maybe(maybe) => match maybe {
            Some(held) => to_json(held),
            None => serde_json::Value::Null,
        },
        Value::Success(held) => to_json(held),
        Value::Failure(held) => to_json(held),
        Value::Function(_)
        | Value::Builtin(_)
        | Value::Captured(_)
        | Value::Channel(_)
        | Value::Shared(_)
        | Value::Task(_) => serde_json::Value::Null,
    }
}
