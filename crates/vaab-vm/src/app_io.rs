//! First-class app I/O: env, SQLite, embedded KV, bearer auth, outbound HTTP,
//! and AREL-style [`crate::query::Query`] over Db/Store.
//!
//! These are the golden-path builtins every vibe-coded backend needs. They live
//! in the language rather than as packages, matching the product decision that
//! Postgres/auth/HTTP/env are not riffs.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use hmac::{Hmac, Mac};
use indexmap::IndexMap;
use redb::{Database, ReadableTable, TableDefinition};
use rusqlite::{params_from_iter, types::ValueRef, Connection};
use sha2::Sha256;

use crate::error::Fault;
use crate::query::Dialect;
use crate::value::{Key, Record, RecordLayout, Ref, Value, Variant, VariantLayout};

type HmacSha256 = Hmac<Sha256>;

/// Open database handles shared by every request machine in a `serve` process.
#[derive(Default)]
pub struct Databases {
    connections: Mutex<Vec<Connection>>,
    dialects: Mutex<Vec<Dialect>>,
    by_path: Mutex<HashMap<String, u32>>,
}

impl Databases {
    pub fn shared() -> Arc<Databases> {
        Arc::new(Databases::default())
    }

    pub fn connect(&self, url: &str) -> Result<u32, String> {
        let dialect = crate::query::dialect_for(url);
        if dialect == Dialect::Postgres {
            return Err(
                "Postgres URLs are recognised for Query SQL, but live connections are still SQLite-only — use sqlite: for now"
                    .into(),
            );
        }
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
        self.dialects
            .lock()
            .map_err(|_| "database lock poisoned".to_string())?
            .push(dialect);
        self.by_path
            .lock()
            .map_err(|_| "database lock poisoned".to_string())?
            .insert(path, index);
        Ok(index)
    }

    pub fn dialect(&self, handle: u32) -> Result<Dialect, String> {
        let held = self.dialects.lock().map_err(|_| "database lock poisoned".to_string())?;
        held.get(handle as usize)
            .copied()
            .ok_or_else(|| "that database handle is gone".to_string())
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

pub fn http_send(
    method: &str,
    url: &str,
    body: &str,
    headers: &HashMap<String, String>,
) -> Result<String, String> {
    let mut request = match method.to_ascii_uppercase().as_str() {
        "GET" => ureq::get(url),
        "POST" => ureq::post(url),
        "PUT" => ureq::put(url),
        "PATCH" => ureq::patch(url),
        "DELETE" => ureq::delete(url),
        other => return Err(format!("http.send does not know the method `{other}`")),
    };
    for (name, value) in headers {
        request = request.set(name, value);
    }
    let response = if body.is_empty() {
        request.call().map_err(|error| error.to_string())?
    } else {
        request.send_string(body).map_err(|error| error.to_string())?
    };
    response.into_string().map_err(|error| error.to_string())
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
        cells: None,
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

const KV: TableDefinition<&str, &str> = TableDefinition::new("kv");
/// Persist pending writes after this many dirty keys — one redb commit per batch.
const STORE_FLUSH_BATCH: usize = 32;

#[derive(Default)]
struct StoreOverlay {
    values: HashMap<String, String>,
    removed: HashSet<String>,
    dirty: HashSet<String>,
}

struct StoreState {
    database: Arc<Database>,
    overlay: Mutex<StoreOverlay>,
}

impl StoreState {
    fn open(path: &str) -> Result<Self, String> {
        let database = Database::create(path).map_err(|error| error.to_string())?;
        {
            let write = database.begin_write().map_err(|error| error.to_string())?;
            {
                let _ = write.open_table(KV).map_err(|error| error.to_string())?;
            }
            write.commit().map_err(|error| error.to_string())?;
        }
        Ok(Self {
            database: Arc::new(database),
            overlay: Mutex::new(StoreOverlay::default()),
        })
    }

    fn get(&self, key: &str) -> Result<Value, String> {
        let overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
        if overlay.removed.contains(key) {
            return Ok(Value::absent());
        }
        if let Some(value) = overlay.values.get(key) {
            return Ok(Value::found(Value::text(value.clone())));
        }
        drop(overlay);

        let read = self.database.begin_read().map_err(|error| error.to_string())?;
        let table = read.open_table(KV).map_err(|error| error.to_string())?;
        match table.get(key).map_err(|error| error.to_string())? {
            Some(value) => {
                let text = value.value().to_string();
                let mut overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
                if !overlay.removed.contains(key) {
                    overlay.values.insert(key.to_string(), text.clone());
                }
                Ok(Value::found(Value::text(text)))
            }
            None => Ok(Value::absent()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        let mut overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
        overlay.removed.remove(key);
        overlay.values.insert(key.to_string(), value.to_string());
        overlay.dirty.insert(key.to_string());
        let should_flush = overlay.dirty.len() >= STORE_FLUSH_BATCH;
        if should_flush {
            Self::flush_locked(&self.database, &mut overlay)?;
        }
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<bool, String> {
        // Drop the overlay lock before any disk read — holding it across a
        // second lock() deadlocks the request worker (Clear / delete hung).
        let existed = {
            let mut overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
            if overlay.values.remove(key).is_some() {
                true
            } else {
                drop(overlay);
                self.read_disk(key)?.is_some()
            }
        };
        let mut overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
        overlay.removed.insert(key.to_string());
        overlay.values.remove(key);
        overlay.dirty.insert(key.to_string());
        let should_flush = overlay.dirty.len() >= STORE_FLUSH_BATCH;
        if should_flush {
            Self::flush_locked(&self.database, &mut overlay)?;
        }
        Ok(existed)
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>, String> {
        let mut overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
        Self::flush_locked(&self.database, &mut overlay)?;
        drop(overlay);

        let mut keys: HashSet<String> = HashSet::new();
        {
            let overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
            for key in overlay.values.keys() {
                if key.starts_with(prefix) {
                    keys.insert(key.clone());
                }
            }
        }

        let read = self.database.begin_read().map_err(|error| error.to_string())?;
        let table = read.open_table(KV).map_err(|error| error.to_string())?;
        let overlay = self.overlay.lock().map_err(|_| "store lock poisoned".to_string())?;
        for item in table.iter().map_err(|error| error.to_string())? {
            let (key, _) = item.map_err(|error| error.to_string())?;
            let key = key.value();
            if key.starts_with(prefix) && !overlay.removed.contains(key) {
                keys.insert(key.to_string());
            }
        }

        let mut keys: Vec<String> = keys.into_iter().collect();
        keys.sort();
        Ok(keys)
    }

    fn read_disk(&self, key: &str) -> Result<Option<String>, String> {
        let read = self.database.begin_read().map_err(|error| error.to_string())?;
        let table = read.open_table(KV).map_err(|error| error.to_string())?;
        Ok(table
            .get(key)
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string()))
    }

    fn flush_locked(database: &Database, overlay: &mut StoreOverlay) -> Result<(), String> {
        if overlay.dirty.is_empty() {
            return Ok(());
        }
        let pending: Vec<String> = overlay.dirty.drain().collect();
        let write = database.begin_write().map_err(|error| error.to_string())?;
        {
            let mut table = write.open_table(KV).map_err(|error| error.to_string())?;
            for key in pending {
                if overlay.removed.contains(&key) {
                    table.remove(key.as_str()).map_err(|error| error.to_string())?;
                } else if let Some(value) = overlay.values.get(&key) {
                    table
                        .insert(key.as_str(), value.as_str())
                        .map_err(|error| error.to_string())?;
                } else {
                    table.remove(key.as_str()).map_err(|error| error.to_string())?;
                }
            }
        }
        write.commit().map_err(|error| error.to_string())?;
        Ok(())
    }
}

/// Embedded key-value stores opened by `Store.open`, shared across request machines.
#[derive(Default)]
pub struct Stores {
    stores: Mutex<Vec<Arc<StoreState>>>,
    by_path: Mutex<HashMap<String, u32>>,
}

impl Stores {
    pub fn shared() -> Arc<Stores> {
        static STORES: OnceLock<Arc<Stores>> = OnceLock::new();
        Arc::clone(STORES.get_or_init(|| Arc::new(Stores::default())))
    }

    pub fn open(&self, path: &str) -> Result<u32, String> {
        {
            let known = self.by_path.lock().map_err(|_| "store lock poisoned".to_string())?;
            if let Some(handle) = known.get(path) {
                return Ok(*handle);
            }
        }
        let store = Arc::new(StoreState::open(path)?);
        let mut held = self.stores.lock().map_err(|_| "store lock poisoned".to_string())?;
        let index = held.len() as u32;
        held.push(store);
        drop(held);
        self.by_path
            .lock()
            .map_err(|_| "store lock poisoned".to_string())?
            .insert(path.to_string(), index);
        Ok(index)
    }

    pub fn get(&self, handle: u32, key: &str) -> Result<Value, String> {
        self.store(handle)?.get(key)
    }

    pub fn set(&self, handle: u32, key: &str, value: &str) -> Result<(), String> {
        self.store(handle)?.set(key, value)
    }

    pub fn remove(&self, handle: u32, key: &str) -> Result<bool, String> {
        self.store(handle)?.remove(key)
    }

    pub fn keys(&self, handle: u32, prefix: &str) -> Result<Vec<String>, String> {
        self.store(handle)?.keys(prefix)
    }

    fn store(&self, handle: u32) -> Result<Arc<StoreState>, String> {
        let held = self.stores.lock().map_err(|_| "store lock poisoned".to_string())?;
        held.get(handle as usize)
            .cloned()
            .ok_or_else(|| format!("store {handle} is not open"))
    }
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
