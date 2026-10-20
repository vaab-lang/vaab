//! Turning runtime values into JSON text, and JSON text back into records.

use serde_json::{Map, Number, Value as Json};

use crate::error::Fault;
use crate::value::{Key, Record, RecordLayout, Ref, Value, Variant, VariantLayout};

/// How `from_json(..., as: T)` should shape the decoded value.
///
/// Nested shapes are indexes into the same table, so a recursive type can point
/// at the slot that is still being filled.
#[derive(Clone, Debug)]
pub enum JsonShape {
    Int,
    Float,
    Bool,
    Text,
    Nothing,
    /// The prelude `Json` choice: keep the document as a walkable value.
    Json,
    Maybe(u32),
    List(u32),
    Map(u32),
    Tuple(Vec<u32>),
    Record { layout: u32, fields: Vec<u32> },
    Choice { variants: Vec<JsonChoiceVariant> },
}

#[derive(Clone, Debug)]
pub struct JsonChoiceVariant {
    pub name: String,
    pub fields: Vec<(String, u32)>,
}

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
    Ok(Value::Record(Ref::new(Record { layout, fields, cells: None })))
}

pub fn decode_map(text: &str) -> Result<Value, Fault> {
    let json: Json = serde_json::from_str(text)
        .map_err(|_| Fault::Confused("the request body is not valid JSON"))?;
    let Json::Object(entries) = json else {
        return Err(Fault::Confused("the request body must be a JSON object"));
    };
    let mut map = indexmap::IndexMap::new();
    for (key, value) in entries {
        let cell = match value {
            Json::Null => Value::text(""),
            Json::Bool(yes) => Value::text(if yes { "true" } else { "false" }),
            Json::Number(number) => Value::text(number.to_string()),
            Json::String(text) => Value::text(text),
            other => Value::text(other.to_string()),
        };
        map.insert(crate::value::Key(Value::text(key)), cell);
    }
    Ok(Value::map(map))
}

/// Typed decode used by `from_json`. Reuses [`json_to_value`] for leaves and the
/// same record-from-object walk as [`decode_record`], with `maybe` fields that
/// may be missing.
pub fn decode_typed(
    text: &str,
    shape: u32,
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
) -> Result<Value, String> {
    let json: Json = serde_json::from_str(text).map_err(|error| {
        format!("this is not valid JSON ({error})")
    })?;
    decode_json(&json, shape, shapes, layouts, variants)
}

pub fn parse_doc(text: &str, variants: &[Ref<VariantLayout>]) -> Result<Value, String> {
    let json: Json = serde_json::from_str(text).map_err(|error| {
        format!("this is not valid JSON ({error})")
    })?;
    let value = json_to_value(&json).map_err(fault_message)?;
    value_to_json_doc(&value, variants)
}

pub fn json_at(receiver: &Value, key: &Value) -> Value {
    let Some(doc) = unwrap_json(receiver) else {
        return Value::absent();
    };
    match (variant_name(doc), key) {
        (Some("Object"), Value::Text(name)) => match object_entries(doc) {
            Some(entries) => match entries.get(&Key(Value::Text(Ref::clone(name)))) {
                Some(found) => Value::found(found.clone()),
                None => Value::absent(),
            },
            None => Value::absent(),
        },
        (Some("List"), Value::Int(index)) => match list_items(doc) {
            Some(items) if *index >= 0 => match items.get(*index as usize) {
                Some(found) => Value::found(found.clone()),
                None => Value::absent(),
            },
            _ => Value::absent(),
        },
        _ => Value::absent(),
    }
}

pub fn json_as_text(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(|doc| variant_field(doc, "Text")) {
        Some(Value::Text(text)) => Value::found(Value::Text(Ref::clone(text))),
        _ => Value::absent(),
    }
}

pub fn json_as_int(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(|doc| variant_field(doc, "Number")) {
        Some(Value::Int(number)) => Value::found(Value::Int(*number)),
        Some(Value::Float(number)) if number.is_finite() && number.fract() == 0.0 => {
            Value::found(Value::Int(*number as i64))
        }
        _ => Value::absent(),
    }
}

pub fn json_as_float(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(|doc| variant_field(doc, "Number")) {
        Some(Value::Float(number)) => Value::found(Value::Float(*number)),
        Some(Value::Int(number)) => Value::found(Value::Float(*number as f64)),
        _ => Value::absent(),
    }
}

pub fn json_as_list(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(list_items) {
        Some(items) => Value::found(Value::list(items.clone())),
        None => Value::absent(),
    }
}

fn decode_json(
    json: &Json,
    shape: u32,
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
) -> Result<Value, String> {
    let Some(wanted) = shapes.get(shape as usize) else {
        return Err("the JSON did not match the type".to_string());
    };
    match wanted {
        JsonShape::Int => match json_to_value(json).map_err(fault_message)? {
            Value::Int(number) => Ok(Value::Int(number)),
            _ => Err("the JSON did not match the type (expected a whole number)".to_string()),
        },
        JsonShape::Float => match json {
            Json::Number(number) => number
                .as_f64()
                .map(Value::Float)
                .ok_or_else(|| "the JSON did not match the type (expected a number)".to_string()),
            _ => Err("the JSON did not match the type (expected a number)".to_string()),
        },
        JsonShape::Bool => match json_to_value(json).map_err(fault_message)? {
            Value::Bool(yes) => Ok(Value::Bool(yes)),
            _ => Err("the JSON did not match the type (expected yes or no)".to_string()),
        },
        JsonShape::Text => match json_to_value(json).map_err(fault_message)? {
            Value::Text(text) => Ok(Value::Text(text)),
            _ => Err("the JSON did not match the type (expected text)".to_string()),
        },
        JsonShape::Nothing => match json {
            Json::Null => Ok(Value::Nothing),
            _ => Err("the JSON did not match the type (expected null)".to_string()),
        },
        JsonShape::Json => {
            let value = json_to_value(json).map_err(fault_message)?;
            value_to_json_doc(&value, variants)
        }
        JsonShape::Maybe(inner) => match json {
            Json::Null => Ok(Value::absent()),
            other => Ok(Value::found(decode_json(other, *inner, shapes, layouts, variants)?)),
        },
        JsonShape::List(inner) => {
            let Json::Array(items) = json else {
                return Err("the JSON did not match the type (expected a list)".to_string());
            };
            let items = items
                .iter()
                .map(|item| decode_json(item, *inner, shapes, layouts, variants))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::list(items))
        }
        JsonShape::Map(inner) => {
            let Json::Object(entries) = json else {
                return Err("the JSON did not match the type (expected an object)".to_string());
            };
            let mut map = indexmap::IndexMap::new();
            for (key, value) in entries {
                map.insert(
                    Key(Value::text(key.clone())),
                    decode_json(value, *inner, shapes, layouts, variants)?,
                );
            }
            Ok(Value::map(map))
        }
        JsonShape::Tuple(parts) => {
            let Json::Array(items) = json else {
                return Err("the JSON did not match the type (expected a list)".to_string());
            };
            if items.len() != parts.len() {
                return Err(format!(
                    "the JSON did not match the type (expected {} values)",
                    parts.len()
                ));
            }
            let items = parts
                .iter()
                .zip(items)
                .map(|(part, item)| decode_json(item, *part, shapes, layouts, variants))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Tuple(Ref::from(items)))
        }
        JsonShape::Record { layout, fields } => {
            decode_record_shaped(json, *layout, fields, shapes, layouts, variants)
        }
        JsonShape::Choice { variants: wanted } => {
            decode_choice(json, wanted, shapes, layouts, variants)
        }
    }
}

fn decode_record_shaped(
    json: &Json,
    layout: u32,
    fields: &[u32],
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
) -> Result<Value, String> {
    let layout = layouts
        .get(layout as usize)
        .cloned()
        .ok_or_else(|| "the JSON did not match the type".to_string())?;
    let Json::Object(object) = json else {
        return Err(format!(
            "the JSON did not match the type (expected an object for {})",
            layout.name
        ));
    };
    let mut values = Vec::with_capacity(layout.fields.len());
    for (index, name) in layout.fields.iter().enumerate() {
        let field_shape = fields.get(index).copied().unwrap_or(0);
        let optional = matches!(shapes.get(field_shape as usize), Some(JsonShape::Maybe(_)));
        match object.get(name) {
            None if optional => values.push(Value::absent()),
            None => {
                return Err(format!(
                    "the JSON did not match the type (missing field `{name}`)"
                ));
            }
            Some(Json::Null) if optional => values.push(Value::absent()),
            Some(value) => {
                values.push(decode_json(value, field_shape, shapes, layouts, variants)?);
            }
        }
    }
    Ok(Value::Record(Ref::new(Record {
        layout,
        fields: values,
        cells: None,
    })))
}

fn decode_choice(
    json: &Json,
    wanted: &[JsonChoiceVariant],
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
) -> Result<Value, String> {
    let Json::Object(object) = json else {
        return Err("the JSON did not match the type (expected a tagged object)".to_string());
    };
    if object.len() != 1 {
        return Err("the JSON did not match the type (expected a tagged object)".to_string());
    }
    let (name, payload) = object.iter().next().expect("one entry");
    let variant = wanted
        .iter()
        .find(|variant| variant.name == *name)
        .ok_or_else(|| format!("the JSON did not match the type (unknown variant `{name}`)"))?;
    let layout = variants
        .iter()
        .find(|layout| layout.name == variant.name)
        .cloned()
        .ok_or_else(|| "the JSON did not match the type".to_string())?;
    let fields = match payload {
        Json::Object(entries) => {
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field, shape) in &variant.fields {
                let value = entries.get(field).ok_or_else(|| {
                    format!("the JSON did not match the type (missing field `{field}`)")
                })?;
                fields.push(decode_json(value, *shape, shapes, layouts, variants)?);
            }
            fields
        }
        _ if variant.fields.is_empty() => Vec::new(),
        _ => {
            return Err("the JSON did not match the type (expected variant fields)".to_string());
        }
    };
    Ok(Value::Variant(Ref::new(Variant { layout, fields })))
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

fn value_to_json_doc(value: &Value, variants: &[Ref<VariantLayout>]) -> Result<Value, String> {
    match value {
        Value::Nothing | Value::Maybe(None) => json_variant(variants, "Null", Vec::new()),
        Value::Bool(yes) => json_variant(variants, "Bool", vec![Value::Bool(*yes)]),
        Value::Int(number) => json_variant(variants, "Number", vec![Value::Float(*number as f64)]),
        Value::Float(number) => json_variant(variants, "Number", vec![Value::Float(*number)]),
        Value::Text(text) => json_variant(variants, "Text", vec![Value::Text(Ref::clone(text))]),
        Value::List(items) => {
            let items = items
                .iter()
                .map(|item| value_to_json_doc(item, variants))
                .collect::<Result<Vec<_>, _>>()?;
            json_variant(variants, "List", vec![Value::list(items)])
        }
        Value::Map(entries) => {
            let mut map = indexmap::IndexMap::new();
            for (key, value) in entries.iter() {
                map.insert(key.clone(), value_to_json_doc(value, variants)?);
            }
            json_variant(variants, "Object", vec![Value::map(map)])
        }
        Value::Maybe(Some(held)) => value_to_json_doc(held, variants),
        other => Err(format!("the JSON did not match the type ({other} cannot be a Json document)")),
    }
}

fn json_variant(
    variants: &[Ref<VariantLayout>],
    name: &str,
    fields: Vec<Value>,
) -> Result<Value, String> {
    let layout = variants
        .iter()
        .find(|layout| layout.choice == "Json" && layout.name == name)
        .cloned()
        .ok_or_else(|| format!("the Json.{name} variant is missing"))?;
    Ok(Value::Variant(Ref::new(Variant { layout, fields })))
}

fn unwrap_json(value: &Value) -> Option<&Value> {
    match value {
        Value::Maybe(None) => None,
        Value::Maybe(Some(held)) => unwrap_json(held),
        Value::Variant(variant) if variant.layout.choice == "Json" => Some(value),
        _ => None,
    }
}

fn variant_name(value: &Value) -> Option<&str> {
    match value {
        Value::Variant(variant) => Some(variant.layout.name.as_str()),
        _ => None,
    }
}

fn variant_field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Variant(variant) if variant.layout.name == name => variant.fields.first(),
        _ => None,
    }
}

fn object_entries(value: &Value) -> Option<&indexmap::IndexMap<Key, Value>> {
    match variant_field(value, "Object") {
        Some(Value::Map(entries)) => Some(entries),
        _ => None,
    }
}

fn list_items(value: &Value) -> Option<&Vec<Value>> {
    match variant_field(value, "List") {
        Some(Value::List(items)) => Some(items),
        _ => None,
    }
}

fn fault_message(fault: Fault) -> String {
    match fault {
        Fault::Confused(message) => message.to_string(),
        _ => "the JSON did not match the type".to_string(),
    }
}

fn value_to_json(value: &Value) -> Result<Json, Fault> {
    match value {
        Value::Nothing => Ok(Json::Null),
        Value::Int(number) => Ok(Json::Number(Number::from(*number))),
        Value::Float(number) => Number::from_f64(*number).map(Json::Number).ok_or(Fault::Confused("a decimal that cannot be written as JSON")),
        Value::Bool(yes) => Ok(Json::Bool(*yes)),
        Value::Text(text) => Ok(Json::String(text.to_string())),
        Value::Bytes(bytes) => Ok(Json::String(hex::encode(bytes.as_ref()))),
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
            for (index, name) in record.layout.fields.iter().enumerate() {
                let Some(value) = record.field(index) else { continue };
                object.insert(name.clone(), value_to_json(&value)?);
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
