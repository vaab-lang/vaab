//! First-class app I/O: env, SQLite, bearer auth, and outbound HTTP.
//!
//! These are the golden-path builtins every vibe-coded backend needs. They live
//! in the language rather than as packages, matching the product decision that
//! Postgres/auth/HTTP/env are not riffs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use indexmap::IndexMap;
use rusqlite::{params_from_iter, types::ValueRef, Connection};
use sha2::Sha256;

use crate::error::Fault;
use crate::value::{Key, Record, RecordLayout, Ref, Value, Variant, VariantLayout};

type HmacSha256 = Hmac<Sha256>;

/// Open database handles shared by every request machine in a `serve` process.
#[derive(Default)]
pub struct Databases {
    connections: Mutex<Vec<Connection>>,
    by_path: Mutex<HashMap<String, u32>>,
}

impl Databases {
    pub fn shared() -> Arc<Databases> {
        Arc::new(Databases::default())
    }

    pub fn connect(&self, url: &str) -> Result<u32, String> {
        let path = url
            .strip_prefix("sqlite:")
            .or_else(|| url.strip_prefix("sqlite://"))
            .unwrap_or(url)
            .to_string();
        {
            let known = self.by_path.lock().map_err(|_| "database lock poisoned".to_string())?;
            if let Some(handle) = known.get(&path) {
                return Ok(*handle);
            }
        }
        let connection = Connection::open(&path).map_err(|error| error.to_string())?;
        let mut held = self.connections.lock().map_err(|_| "database lock poisoned".to_string())?;
        let index = held.len() as u32;
        held.push(connection);
        drop(held);
        self.by_path
            .lock()
            .map_err(|_| "database lock poisoned".to_string())?
            .insert(path, index);
        Ok(index)
    }

    pub fn execute(&self, handle: u32, sql: &str, args: &[Value]) -> Result<i64, String> {
        let sql = postgres_placeholders(sql);
        let bound = bind_args(args)?;
        let held = self.connections.lock().map_err(|_| "database lock poisoned".to_string())?;
        let connection = held
            .get(handle as usize)
            .ok_or_else(|| "that database handle is gone".to_string())?;
        connection
            .execute(&sql, params_from_iter(bound.iter()))
            .map(|changed| changed as i64)
            .map_err(|error| error.to_string())
    }

    pub fn query(&self, handle: u32, sql: &str, args: &[Value]) -> Result<Value, String> {
        let sql = postgres_placeholders(sql);
        let bound = bind_args(args)?;
        let held = self.connections.lock().map_err(|_| "database lock poisoned".to_string())?;
        let connection = held
            .get(handle as usize)
            .ok_or_else(|| "that database handle is gone".to_string())?;
        let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
        let column_count = statement.column_count();
        let names: Vec<String> = (0..column_count)
            .map(|index| statement.column_name(index).unwrap_or("column").to_string())
            .collect();
        let rows = statement
            .query_map(params_from_iter(bound.iter()), |row| {
                let mut entries = IndexMap::new();
                for (index, name) in names.iter().enumerate() {
                    let value = match row.get_ref(index)? {
                        ValueRef::Null => Value::text(""),
                        ValueRef::Integer(number) => Value::text(number.to_string()),
                        ValueRef::Real(number) => Value::text(number.to_string()),
                        ValueRef::Text(text) => {
                            Value::text(String::from_utf8_lossy(text).into_owned())
                        }
                        ValueRef::Blob(_) => Value::text("<blob>"),
                    };
                    entries.insert(Key(Value::text(name.clone())), value);
                }
                Ok(Value::map(entries))
            })
            .map_err(|error| error.to_string())?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row.map_err(|error| error.to_string())?);
        }
        Ok(Value::list(list))
    }
}

fn bind_args(args: &[Value]) -> Result<Vec<String>, String> {
    args.iter()
        .map(|value| match value {
            Value::Text(text) => Ok(text.to_string()),
            Value::Int(number) => Ok(number.to_string()),
            Value::Float(number) => Ok(number.to_string()),
            Value::Bool(yes) => Ok(if *yes { "1".into() } else { "0".into() }),
            _ => Err("database arguments must be text, numbers, or yes/no".to_string()),
        })
        .collect()
}

/// Turns `$1`-style placeholders into the `?1` form SQLite expects.
fn postgres_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(letter) = chars.next() {
        if letter == '$' {
            let mut digits = String::new();
            while let Some(digit) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
                digits.push(digit);
                chars.next();
            }
            if digits.is_empty() {
                out.push('$');
            } else {
                out.push('?');
                out.push_str(&digits);
            }
        } else {
            out.push(letter);
        }
    }
    out
}

pub fn env_get(name: &str) -> Value {
    match std::env::var(name) {
        Ok(value) => Value::found(Value::text(value)),
        Err(_) => Value::absent(),
    }
}

pub fn env_required(name: &str, missing_layout: Ref<VariantLayout>) -> Value {
    match std::env::var(name) {
        Ok(value) => Value::success(Value::text(value)),
        Err(_) => {
            let variant = Value::Variant(Ref::new(Variant {
                layout: missing_layout,
                fields: vec![Value::text(name)],
            }));
            Value::failure(variant)
        }
    }
}

pub fn http_get(url: &str) -> Result<String, String> {
    ureq::get(url)
        .call()
        .map_err(|error| error.to_string())?
        .into_string()
        .map_err(|error| error.to_string())
}

pub fn http_post(url: &str, body: &str) -> Result<String, String> {
    ureq::post(url)
        .set("content-type", "application/json")
        .send_string(body)
        .map_err(|error| error.to_string())?
        .into_string()
        .map_err(|error| error.to_string())
}

/// Verifies `Authorization: Bearer <id>|<email>|<mac>` where mac is hex(HMAC-SHA256).
pub fn verify_bearer(
    headers: &HashMap<String, String>,
    user_layout: Ref<RecordLayout>,
) -> Result<Value, ()> {
    let secret = std::env::var("AUTH_SECRET").map_err(|_| ())?;
    let header = headers
        .get("authorization")
        .or_else(|| headers.get("Authorization"))
        .ok_or(())?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .ok_or(())?
        .trim();
    // `id|email|mac` — emails contain dots, so `|` is the separator.
    let mut parts = token.splitn(3, '|');
    let id = parts.next().ok_or(())?;
    let email = parts.next().ok_or(())?;
    let mac = parts.next().ok_or(())?;
    if id.is_empty() || email.is_empty() || mac.is_empty() {
        return Err(());
    }
    let expected = sign_token(&secret, id, email);
    if !constant_time_eq(mac.as_bytes(), expected.as_bytes()) {
        return Err(());
    }
    Ok(Value::Record(Ref::new(Record {
        layout: user_layout,
        fields: vec![Value::text(id), Value::text(email)],
    })))
}

pub fn sign_token(secret: &str, id: &str, email: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(email.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub fn failure_message(layout: Ref<VariantLayout>, message: String) -> Value {
    Value::failure(Value::Variant(Ref::new(Variant {
        layout,
        fields: vec![Value::text(message)],
    })))
}

pub fn failure_unit(layout: Ref<VariantLayout>) -> Value {
    Value::failure(Value::Variant(Ref::new(Variant {
        layout,
        fields: Vec::new(),
    })))
}

pub fn headers_from_value(value: &Value) -> Result<HashMap<String, String>, Fault> {
    let Value::Map(entries) = value else {
        return Err(Fault::Confused("request headers must be a map"));
    };
    let mut headers = HashMap::new();
    for (key, value) in entries.iter() {
        let Value::Text(name) = &key.0 else {
            return Err(Fault::Confused("a header name was not text"));
        };
        let Value::Text(text) = value else {
            return Err(Fault::Confused("a header value was not text"));
        };
        headers.insert(name.to_string(), text.to_string());
    }
    Ok(headers)
}
