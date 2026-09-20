//! Turning runtime values into JSON text, and JSON text back into records.

use serde_json::{Map, Number, Value as Json};

use crate::error::Fault;
use crate::value::{Record, RecordLayout, Ref, Value};

pub fn encode(value: &Value) -> Result<String, Fault> {
    let json = value_to_json(value)?;
    serde_json::to_string(&json).map_err(|_| Fault::Confused("could not turn the value into JSON text"))
}

pub fn decode_record(text: &str, layout: Ref<RecordLayout>) -> Result<Value, Fault> {
    let json: Json = serde_json::from_str(text)
        .map_err(|_| Fault::Confused("the request body is not valid JSON"))?;
    let Json::Object(object) = json else {
        return Err(Fault::Confused("the request body must be a JSON object"));
    };
    let mut fields = Vec::with_capacity(layout.fields.len());
    for name in &layout.fields {
        let value = object
            .get(name)
            .ok_or_else(|| Fault::Confused("the request body is missing a field"))?;
        fields.push(json_to_value(value)?);
    }
    Ok(Value::Record(Ref::new(Record { layout, fields })))
}

fn json_to_value(value: &Json) -> Result<Value, Fault> {
    match value {
        Json::Null => Ok(Value::absent()),
        Json::Bool(yes) => Ok(Value::Bool(*yes)),
        Json::Number(number) => {
            if let Some(whole) = number.as_i64() {
                Ok(Value::Int(whole))
            } else if let Some(decimal) = number.as_f64() {
                Ok(Value::Float(decimal))
            } else {
                Err(Fault::Confused("a JSON number could not be read"))
            }
        }
        Json::String(text) => Ok(Value::text(text.clone())),
        Json::Array(items) => Ok(Value::list(
            items.iter().map(json_to_value).collect::<Result<Vec<_>, _>>()?,
        )),
        Json::Object(entries) => {
            let mut map = indexmap::IndexMap::new();
            for (key, value) in entries {
                map.insert(crate::value::Key(Value::text(key.clone())), json_to_value(value)?);
            }
            Ok(Value::map(map))
        }
    }
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
