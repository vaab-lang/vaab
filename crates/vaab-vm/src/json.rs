//! Turning runtime values into JSON text, and JSON text back into records.
//!
//! Uses `sonic-rs` (SIMD-accelerated JSON) with streaming `serde::Serialize` for
//! zero-intermediate-allocation encoding, and SIMD value parsing for decoding.

use indexmap::IndexMap;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use sonic_rs::{JsonContainerTrait, JsonNumberTrait, JsonValueTrait, Value as Json, ValueRef};

use crate::error::Fault;
use crate::value::{Captured, Key, Record, RecordLayout, Ref, Value, Variant, VariantLayout};

/// How `from_json(..., as: T)` and `expecting T` should shape the decoded value.
///
/// Nested shapes are indexes into the same table, so a recursive type can point
/// at the slot that is still being filled.
#[derive(Clone, Debug)]
pub enum JsonShape {
    Int,
    Float,
    Bool,
    Text,
    Bytes,
    Nothing,
    /// The prelude `Json` choice: keep the document as a walkable value.
    Json,
    Maybe(u32),
    List(u32),
    Map(u32),
    /// `{Text: Text}` where non-string JSON scalars are coerced to text.
    TextCoercedMap,
    Tuple(Vec<u32>),
    Record { layout: u32, fields: Vec<u32> },
    Choice { choice: String, variants: Vec<JsonChoiceVariant> },
}

#[derive(Clone, Debug)]
pub struct JsonChoiceVariant {
    pub name: String,
    pub fields: Vec<(String, u32)>,
}

struct JsonValueSerializer<'a>(&'a Value);

impl<'a> Serialize for JsonValueSerializer<'a> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Nothing | Value::Maybe(None) => serializer.serialize_none(),
            Value::Maybe(Some(held)) | Value::Success(held) => {
                JsonValueSerializer(held).serialize(serializer)
            }
            Value::Int(number) => serializer.serialize_i64(*number),
            Value::Float(number) => {
                if number.is_finite() {
                    serializer.serialize_f64(*number)
                } else {
                    Err(serde::ser::Error::custom(
                        "a decimal that cannot be written as JSON",
                    ))
                }
            }
            Value::Bool(yes) => serializer.serialize_bool(*yes),
            Value::Text(text) => serializer.serialize_str(text),
            Value::Bytes(bytes) => {
                let encoded = hex::encode(bytes.as_ref());
                serializer.serialize_str(&encoded)
            }
            Value::List(items) | Value::Tuple(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items.iter() {
                    seq.serialize_element(&JsonValueSerializer(item))?;
                }
                seq.end()
            }
            Value::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                if entries.len() <= 1 {
                    for (key, value) in entries.iter() {
                        let Value::Text(text) = &key.0 else {
                            return Err(serde::ser::Error::custom("a map key was not text"));
                        };
                        map.serialize_entry(text.as_ref(), &JsonValueSerializer(value))?;
                    }
                } else {
                    let mut pairs: Vec<(&str, &Value)> = Vec::with_capacity(entries.len());
                    for (key, value) in entries.iter() {
                        let Value::Text(text) = &key.0 else {
                            return Err(serde::ser::Error::custom("a map key was not text"));
                        };
                        pairs.push((text.as_ref(), value));
                    }
                    pairs.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    for (key, value) in pairs {
                        map.serialize_entry(key, &JsonValueSerializer(value))?;
                    }
                }
                map.end()
            }
            Value::Record(record) => {
                let mut map = serializer.serialize_map(Some(record.layout.fields.len()))?;
                for &index in &record.layout.sorted_fields {
                    let Some(name) = record.layout.fields.get(index) else {
                        continue;
                    };
                    let Some(value) = record.field(index) else {
                        continue;
                    };
                    map.serialize_entry(name.as_str(), &JsonValueSerializer(&value))?;
                }
                map.end()
            }
            Value::Variant(variant) => {
                if variant.layout.choice == "Json" {
                    return serialize_json_doc_variant(variant, serializer);
                }
                let mut outer = serializer.serialize_map(Some(1))?;
                outer.serialize_entry(
                    variant.layout.name.as_str(),
                    &VariantFieldsSerializer(variant),
                )?;
                outer.end()
            }
            _ => Err(serde::ser::Error::custom(
                "this value cannot be turned into JSON",
            )),
        }
    }
}

struct VariantFieldsSerializer<'a>(&'a Variant);

impl<'a> Serialize for VariantFieldsSerializer<'a> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.layout.fields.len()))?;
        for &index in &self.0.layout.sorted_fields {
            if let (Some(name), Some(value)) = (
                self.0.layout.fields.get(index),
                self.0.fields.get(index),
            ) {
                map.serialize_entry(name.as_str(), &JsonValueSerializer(value))?;
            }
        }
        map.end()
    }
}

fn serialize_json_doc_variant<S: Serializer>(
    variant: &Variant,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match variant.layout.name.as_str() {
        "Null" => serializer.serialize_none(),
        "Bool" => match variant.fields.first() {
            Some(Value::Bool(yes)) => serializer.serialize_bool(*yes),
            _ => serializer.serialize_none(),
        },
        "Number" => match variant.fields.first() {
            Some(Value::Int(n)) => serializer.serialize_i64(*n),
            Some(Value::Float(n)) if n.is_finite() => {
                if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n < (i64::MAX as f64) {
                    serializer.serialize_i64(*n as i64)
                } else {
                    serializer.serialize_f64(*n)
                }
            }
            Some(Value::Float(_)) => Err(serde::ser::Error::custom(
                "a decimal that cannot be written as JSON",
            )),
            _ => serializer.serialize_none(),
        },
        "Text" => match variant.fields.first() {
            Some(Value::Text(text)) => serializer.serialize_str(text),
            _ => serializer.serialize_str(""),
        },
        "List" => match variant.fields.first() {
            Some(Value::List(items)) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items.iter() {
                    seq.serialize_element(&JsonValueSerializer(item))?;
                }
                seq.end()
            }
            _ => serializer.serialize_seq(Some(0))?.end(),
        },
        "Object" => match variant.fields.first() {
            Some(Value::Map(entries)) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                if entries.len() <= 1 {
                    for (key, value) in entries.iter() {
                        let Value::Text(text) = &key.0 else {
                            return Err(serde::ser::Error::custom("a map key was not text"));
                        };
                        map.serialize_entry(text.as_ref(), &JsonValueSerializer(value))?;
                    }
                } else {
                    let mut pairs: Vec<(&str, &Value)> = Vec::with_capacity(entries.len());
                    for (key, value) in entries.iter() {
                        let Value::Text(text) = &key.0 else {
                            return Err(serde::ser::Error::custom("a map key was not text"));
                        };
                        pairs.push((text.as_ref(), value));
                    }
                    pairs.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    for (key, value) in pairs {
                        map.serialize_entry(key, &JsonValueSerializer(value))?;
                    }
                }
                map.end()
            }
            _ => serializer.serialize_map(Some(0))?.end(),
        },
        _ => serializer.serialize_none(),
    }
}

fn check_encodable(value: &Value) -> Result<(), Fault> {
    match value {
        Value::Nothing | Value::Int(_) | Value::Bool(_) | Value::Text(_) | Value::Bytes(_) => {
            Ok(())
        }
        Value::Float(number) => {
            if number.is_finite() {
                Ok(())
            } else {
                Err(Fault::Confused("a decimal that cannot be written as JSON"))
            }
        }
        Value::Maybe(None) => Ok(()),
        Value::Maybe(Some(held)) | Value::Success(held) => check_encodable(held),
        Value::List(items) | Value::Tuple(items) => {
            for item in items.iter() {
                check_encodable(item)?;
            }
            Ok(())
        }
        Value::Map(entries) => {
            for (key, val) in entries.iter() {
                if !matches!(&key.0, Value::Text(_)) {
                    return Err(Fault::Confused("a map key was not text"));
                }
                check_encodable(val)?;
            }
            Ok(())
        }
        Value::Record(record) => {
            for index in 0..record.layout.fields.len() {
                if let Some(val) = record.field(index) {
                    check_encodable(&val)?;
                }
            }
            Ok(())
        }
        Value::Variant(variant) => {
            for val in &variant.fields {
                check_encodable(val)?;
            }
            Ok(())
        }
        _ => Err(Fault::Confused("this value cannot be turned into JSON")),
    }
}

pub fn encode(value: &Value) -> Result<String, Fault> {
    check_encodable(value)?;
    sonic_rs::to_string(&JsonValueSerializer(value))
        .map_err(|_| Fault::Confused("could not turn the value into JSON text"))
}

pub fn decode_record(text: &str, layout: Ref<RecordLayout>) -> Result<Value, Fault> {
    let json: Json = sonic_rs::from_str(text)
        .map_err(|_| Fault::Confused("the request body is not valid JSON"))?;
    let Some(object) = json.as_object() else {
        return Err(Fault::Confused("the request body must be a JSON object"));
    };
    let mut fields = Vec::with_capacity(layout.fields.len());
    for name in &layout.fields {
        match object.get(&name.as_str()) {
            Some(value) => fields.push(json_to_value(value)?),
            None => return Err(Fault::Confused("the request body is missing a field")),
        }
    }
    Ok(build_record(layout, fields))
}

pub fn decode_map(text: &str) -> Result<Value, Fault> {
    let json: Json = sonic_rs::from_str(text)
        .map_err(|_| Fault::Confused("the request body is not valid JSON"))?;
    let Some(entries) = json.as_object() else {
        return Err(Fault::Confused("the request body must be a JSON object"));
    };
    let mut map = IndexMap::with_capacity(entries.len());
    for (key, value) in entries.iter() {
        let cell = coerce_json_to_text(value);
        map.insert(Key(Value::text(key)), cell);
    }
    Ok(Value::map(map))
}

fn coerce_json_to_text(value: &Json) -> Value {
    match value.as_ref() {
        ValueRef::Null => Value::text(""),
        ValueRef::Bool(yes) => Value::text(if yes { "true" } else { "false" }),
        ValueRef::Number(number) => {
            if let Some(whole) = number.as_i64() {
                let mut buf = itoa::Buffer::new();
                Value::text(buf.format(whole))
            } else if let Some(whole) = number.as_u64() {
                let mut buf = itoa::Buffer::new();
                Value::text(buf.format(whole))
            } else if let Some(decimal) = number.as_f64() {
                Value::text(decimal.to_string())
            } else {
                Value::text("")
            }
        }
        ValueRef::String(text) => Value::text(text),
        _ => Value::text(sonic_rs::to_string(value).unwrap_or_default()),
    }
}

/// Typed decode used by `from_json` and route `expecting` bodies.
pub fn decode_typed(
    text: &str,
    shape: u32,
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
) -> Result<Value, String> {
    let json: Json = sonic_rs::from_str(text).map_err(|error| {
        format!("this is not valid JSON ({error})")
    })?;
    let cache = VariantCache::from_slice(variants);
    decode_json(&json, shape, shapes, layouts, variants, &cache)
}

pub fn parse_doc(text: &str, variants: &[Ref<VariantLayout>]) -> Result<Value, String> {
    let json: Json = sonic_rs::from_str(text).map_err(|error| {
        format!("this is not valid JSON ({error})")
    })?;
    let cache = VariantCache::from_slice(variants);
    json_to_doc(&json, &cache)
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

pub fn json_as_bool(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(|doc| variant_field(doc, "Bool")) {
        Some(Value::Bool(yes)) => Value::found(Value::Bool(*yes)),
        _ => Value::absent(),
    }
}

pub fn json_as_list(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(list_items) {
        Some(items) => Value::found(Value::list(items.clone())),
        None => Value::absent(),
    }
}

pub fn json_as_map(receiver: &Value) -> Value {
    match unwrap_json(receiver).and_then(object_entries) {
        Some(entries) => Value::found(Value::map(entries.clone())),
        None => Value::absent(),
    }
}

struct VariantCache {
    null: Option<Ref<VariantLayout>>,
    bool_: Option<Ref<VariantLayout>>,
    number: Option<Ref<VariantLayout>>,
    text: Option<Ref<VariantLayout>>,
    list: Option<Ref<VariantLayout>>,
    object: Option<Ref<VariantLayout>>,
}

impl VariantCache {
    fn from_slice(variants: &[Ref<VariantLayout>]) -> Self {
        let mut cache = Self {
            null: None,
            bool_: None,
            number: None,
            text: None,
            list: None,
            object: None,
        };
        for layout in variants {
            if layout.choice == "Json" {
                match layout.name.as_str() {
                    "Null" => cache.null = Some(Ref::clone(layout)),
                    "Bool" => cache.bool_ = Some(Ref::clone(layout)),
                    "Number" => cache.number = Some(Ref::clone(layout)),
                    "Text" => cache.text = Some(Ref::clone(layout)),
                    "List" => cache.list = Some(Ref::clone(layout)),
                    "Object" => cache.object = Some(Ref::clone(layout)),
                    _ => {}
                }
            }
        }
        cache
    }

    fn get(&self, name: &str) -> Result<Ref<VariantLayout>, String> {
        let found = match name {
            "Null" => &self.null,
            "Bool" => &self.bool_,
            "Number" => &self.number,
            "Text" => &self.text,
            "List" => &self.list,
            "Object" => &self.object,
            _ => &None,
        };
        found
            .clone()
            .ok_or_else(|| format!("the Json.{name} variant is missing"))
    }
}

fn json_to_doc(json: &Json, cache: &VariantCache) -> Result<Value, String> {
    match json.as_ref() {
        ValueRef::Null => Ok(Value::Variant(Ref::new(Variant {
            layout: cache.get("Null")?,
            fields: Vec::new(),
        }))),
        ValueRef::Bool(yes) => Ok(Value::Variant(Ref::new(Variant {
            layout: cache.get("Bool")?,
            fields: vec![Value::Bool(yes)],
        }))),
        ValueRef::Number(number) => {
            let float = if let Some(whole) = number.as_i64() {
                whole as f64
            } else if let Some(decimal) = number.as_f64() {
                decimal
            } else {
                return Err("a JSON number could not be read".to_string());
            };
            Ok(Value::Variant(Ref::new(Variant {
                layout: cache.get("Number")?,
                fields: vec![Value::Float(float)],
            })))
        }
        ValueRef::String(text) => Ok(Value::Variant(Ref::new(Variant {
            layout: cache.get("Text")?,
            fields: vec![Value::text(text)],
        }))),
        ValueRef::Array(items) => {
            let mut list = Vec::with_capacity(items.len());
            for item in items.iter() {
                list.push(json_to_doc(item, cache)?);
            }
            Ok(Value::Variant(Ref::new(Variant {
                layout: cache.get("List")?,
                fields: vec![Value::list(list)],
            })))
        }
        ValueRef::Object(entries) => {
            let mut map = IndexMap::with_capacity(entries.len());
            for (key, value) in entries.iter() {
                map.insert(Key(Value::text(key)), json_to_doc(value, cache)?);
            }
            Ok(Value::Variant(Ref::new(Variant {
                layout: cache.get("Object")?,
                fields: vec![Value::map(map)],
            })))
        }
    }
}

fn decode_json(
    json: &Json,
    shape: u32,
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
    cache: &VariantCache,
) -> Result<Value, String> {
    let Some(wanted) = shapes.get(shape as usize) else {
        return Err("the JSON did not match the type".to_string());
    };
    match wanted {
        JsonShape::Int => match json.as_ref() {
            ValueRef::Number(number) => {
                if let Some(whole) = number.as_i64() {
                    Ok(Value::Int(whole))
                } else if let Some(decimal) = number.as_f64() {
                    if decimal.is_finite()
                        && decimal.fract() == 0.0
                        && decimal >= (i64::MIN as f64)
                        && decimal < (i64::MAX as f64)
                    {
                        Ok(Value::Int(decimal as i64))
                    } else {
                        Err("the JSON did not match the type (expected a whole number)".to_string())
                    }
                } else {
                    Err("the JSON did not match the type (expected a whole number)".to_string())
                }
            }
            _ => Err("the JSON did not match the type (expected a whole number)".to_string()),
        },
        JsonShape::Float => match json.as_ref() {
            ValueRef::Number(number) => number
                .as_f64()
                .map(Value::Float)
                .ok_or_else(|| "the JSON did not match the type (expected a number)".to_string()),
            _ => Err("the JSON did not match the type (expected a number)".to_string()),
        },
        JsonShape::Bool => match json.as_ref() {
            ValueRef::Bool(yes) => Ok(Value::Bool(yes)),
            _ => Err("the JSON did not match the type (expected yes or no)".to_string()),
        },
        JsonShape::Text => match json.as_ref() {
            ValueRef::String(text) => Ok(Value::text(text)),
            _ => Err("the JSON did not match the type (expected text)".to_string()),
        },
        JsonShape::Bytes => match json.as_ref() {
            ValueRef::String(text) => {
                let bytes = hex::decode(text).map_err(|_| {
                    "the JSON did not match the type (expected hex bytes)".to_string()
                })?;
                Ok(Value::bytes(bytes))
            }
            _ => Err("the JSON did not match the type (expected hex bytes)".to_string()),
        },
        JsonShape::Nothing => match json.as_ref() {
            ValueRef::Null => Ok(Value::Nothing),
            _ => Err("the JSON did not match the type (expected null)".to_string()),
        },
        JsonShape::Json => json_to_doc(json, cache),
        JsonShape::Maybe(inner) => match json.as_ref() {
            ValueRef::Null => Ok(Value::absent()),
            _ => Ok(Value::found(decode_json(
                json, *inner, shapes, layouts, variants, cache,
            )?)),
        },
        JsonShape::List(inner) => {
            let Some(items) = json.as_array() else {
                return Err("the JSON did not match the type (expected a list)".to_string());
            };
            let mut list = Vec::with_capacity(items.len());
            for item in items.iter() {
                list.push(decode_json(
                    item, *inner, shapes, layouts, variants, cache,
                )?);
            }
            Ok(Value::list(list))
        }
        JsonShape::Map(inner) => {
            let Some(entries) = json.as_object() else {
                return Err("the JSON did not match the type (expected an object)".to_string());
            };
            let mut map = IndexMap::with_capacity(entries.len());
            for (key, value) in entries.iter() {
                map.insert(
                    Key(Value::text(key)),
                    decode_json(value, *inner, shapes, layouts, variants, cache)?,
                );
            }
            Ok(Value::map(map))
        }
        JsonShape::TextCoercedMap => {
            let Some(entries) = json.as_object() else {
                return Err("the JSON did not match the type (expected an object)".to_string());
            };
            let mut map = IndexMap::with_capacity(entries.len());
            for (key, value) in entries.iter() {
                map.insert(Key(Value::text(key)), coerce_json_to_text(value));
            }
            Ok(Value::map(map))
        }
        JsonShape::Tuple(parts) => {
            let Some(items) = json.as_array() else {
                return Err("the JSON did not match the type (expected a list)".to_string());
            };
            if items.len() != parts.len() {
                return Err(format!(
                    "the JSON did not match the type (expected {} values)",
                    parts.len()
                ));
            }
            let mut list = Vec::with_capacity(parts.len());
            for (part, item) in parts.iter().zip(items.iter()) {
                list.push(decode_json(item, *part, shapes, layouts, variants, cache)?);
            }
            Ok(Value::Tuple(Ref::from(list)))
        }
        JsonShape::Record { layout, fields } => {
            decode_record_shaped(json, *layout, fields, shapes, layouts, variants, cache)
        }
        JsonShape::Choice {
            choice,
            variants: wanted,
        } => decode_choice(json, choice, wanted, shapes, layouts, variants, cache),
    }
}

fn decode_record_shaped(
    json: &Json,
    layout: u32,
    fields: &[u32],
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
    cache: &VariantCache,
) -> Result<Value, String> {
    let layout = layouts
        .get(layout as usize)
        .cloned()
        .ok_or_else(|| "the JSON did not match the type".to_string())?;
    let Some(object) = json.as_object() else {
        return Err(format!(
            "the JSON did not match the type (expected an object for {})",
            layout.name
        ));
    };
    let mut values = Vec::with_capacity(layout.fields.len());
    for (index, name) in layout.fields.iter().enumerate() {
        let field_shape = fields.get(index).copied().unwrap_or(0);
        let optional = matches!(shapes.get(field_shape as usize), Some(JsonShape::Maybe(_)));
        match object.get(&name.as_str()) {
            None if optional => values.push(Value::absent()),
            None => {
                return Err(format!(
                    "the JSON did not match the type (missing field `{name}`)"
                ));
            }
            Some(value) if optional && value.is_null() => values.push(Value::absent()),
            Some(value) => {
                values.push(decode_json(
                    value,
                    field_shape,
                    shapes,
                    layouts,
                    variants,
                    cache,
                )?);
            }
        }
    }
    Ok(build_record(layout, values))
}

fn build_record(layout: Ref<RecordLayout>, values: Vec<Value>) -> Value {
    if layout.mutable {
        Value::Record(Ref::new(Record {
            layout,
            fields: Vec::new(),
            cells: Some(Ref::new(values.into_iter().map(Captured::new).collect())),
        }))
    } else {
        Value::Record(Ref::new(Record {
            layout,
            fields: values,
            cells: None,
        }))
    }
}

fn decode_choice(
    json: &Json,
    choice: &str,
    wanted: &[JsonChoiceVariant],
    shapes: &[JsonShape],
    layouts: &[Ref<RecordLayout>],
    variants: &[Ref<VariantLayout>],
    cache: &VariantCache,
) -> Result<Value, String> {
    if let ValueRef::String(name) = json.as_ref() {
        let variant_name = name
            .strip_prefix(choice)
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(name);
        if let Some(variant) = wanted
            .iter()
            .find(|v| v.name == variant_name && v.fields.is_empty())
        {
            let layout = variants
                .iter()
                .find(|l| l.choice == choice && l.name == variant.name)
                .or_else(|| variants.iter().find(|l| l.name == variant.name))
                .cloned()
                .ok_or_else(|| "the JSON did not match the type".to_string())?;
            return Ok(Value::Variant(Ref::new(Variant {
                layout,
                fields: Vec::new(),
            })));
        }
    }

    let Some(object) = json.as_object() else {
        return Err("the JSON did not match the type (expected a tagged object)".to_string());
    };
    if object.len() != 1 {
        return Err("the JSON did not match the type (expected a tagged object)".to_string());
    }
    let (name, payload) = object.iter().next().expect("one entry");
    let variant = wanted
        .iter()
        .find(|variant| variant.name == name)
        .ok_or_else(|| format!("the JSON did not match the type (unknown variant `{name}`)"))?;
    let layout = variants
        .iter()
        .find(|layout| layout.choice == choice && layout.name == variant.name)
        .or_else(|| variants.iter().find(|layout| layout.name == variant.name))
        .cloned()
        .ok_or_else(|| "the JSON did not match the type".to_string())?;
    let fields = match payload.as_ref() {
        ValueRef::Object(entries) => {
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field, shape) in &variant.fields {
                let field_opt = matches!(shapes.get(*shape as usize), Some(JsonShape::Maybe(_)));
                match entries.get(&field.as_str()) {
                    Some(value) if field_opt && value.is_null() => fields.push(Value::absent()),
                    Some(value) => {
                        fields.push(decode_json(value, *shape, shapes, layouts, variants, cache)?);
                    }
                    None if field_opt => fields.push(Value::absent()),
                    None => {
                        return Err(format!(
                            "the JSON did not match the type (missing field `{field}`)"
                        ));
                    }
                }
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
    match value.as_ref() {
        ValueRef::Null => Ok(Value::absent()),
        ValueRef::Bool(yes) => Ok(Value::Bool(yes)),
        ValueRef::Number(number) => {
            if let Some(whole) = number.as_i64() {
                Ok(Value::Int(whole))
            } else if let Some(decimal) = number.as_f64() {
                Ok(Value::Float(decimal))
            } else {
                Err(Fault::Confused("a JSON number could not be read"))
            }
        }
        ValueRef::String(text) => Ok(Value::text(text)),
        ValueRef::Array(items) => {
            let mut list = Vec::with_capacity(items.len());
            for item in items.iter() {
                list.push(json_to_value(item)?);
            }
            Ok(Value::list(list))
        }
        ValueRef::Object(entries) => {
            let mut map = IndexMap::with_capacity(entries.len());
            for (key, value) in entries.iter() {
                map.insert(Key(Value::text(key)), json_to_value(value)?);
            }
            Ok(Value::map(map))
        }
    }
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

fn object_entries(value: &Value) -> Option<&IndexMap<Key, Value>> {
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
