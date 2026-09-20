//! Turning runtime values into JSON text.

use serde_json::{Map, Number, Value as Json};

use crate::error::Fault;
use crate::value::Value;

pub fn encode(value: &Value) -> Result<String, Fault> {
    let json = value_to_json(value)?;
    serde_json::to_string(&json).map_err(|_| Fault::Confused("could not turn the value into JSON text"))
}

fn value_to_json(value: &Value) -> Result<Json, Fault> {
    match value {
        Value::Nothing => Ok(Json::Null),
        Value::Int(number) => Ok(Json::Number(Number::from(*number))),
        Value::Float(number) => Number::from_f64(*number).map(Json::Number).ok_or(Fault::Confused("a decimal that cannot be written as JSON")),
        Value::Bool(yes) => Ok(Json::Bool(*yes)),
        Value::Text(text) => Ok(Json::String(text.to_string())),
        Value::List(items) => Ok(Json::Array(items.iter().map(value_to_json).collect::<Result<Vec<_>, _>>()?)),
        Value::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries.iter() {
                let Value::Text(text) = &key.0 else { return Err(Fault::Confused("a map key was not text")); };
                object.insert(text.to_string(), value_to_json(value)?);
            }
            Ok(Json::Object(object))
        }
        Value::Tuple(parts) => Ok(Json::Array(parts.iter().map(value_to_json).collect::<Result<Vec<_>, _>>()?)),
        Value::Maybe(None) => Ok(Json::Null),
        Value::Maybe(Some(held)) => value_to_json(held),
        Value::Record(record) => {
            let mut object = Map::new();
            for (name, value) in record.layout.fields.iter().zip(&record.fields) {
                object.insert(name.clone(), value_to_json(value)?);
            }
            Ok(Json::Object(object))
        }
        Value::Variant(variant) => {
            let mut fields = Map::new();
            for (name, value) in variant.layout.fields.iter().zip(&variant.fields) {
                fields.insert(name.clone(), value_to_json(value)?);
            }
            let mut object = Map::new();
            object.insert(variant.layout.name.clone(), Json::Object(fields));
            Ok(Json::Object(object))
        }
        Value::Success(held) => value_to_json(held),
        _ => Err(Fault::Confused("this value cannot be turned into JSON")),
    }
}
