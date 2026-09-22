//! What a Vaab value is while a program runs.
//!
//! A [`Value`] is one word plus, for anything that does not fit in one, a
//! reference-counted pointer. Values are cheap to clone, which is what lets the
//! machine push and pop them freely.
//!
//! # The pointer, and why it is a single alias
//!
//! Phase 7 puts tasks on a work-stealing pool of operating-system threads, and a
//! value that crosses threads has to be counted atomically. Every heap value in
//! the machine therefore goes behind [`Ref`], and nothing outside this module ever
//! names `Rc`. Turning Vaab multi-threaded is then a change to the two aliases
//! here — `Rc` becomes `Arc`, and [`Captured`]'s cell becomes a lock — rather than
//! a rewrite of the value representation and everything that touches it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use indexmap::IndexMap;

use crate::builtin::Builtin;

/// The reference-counted pointer behind every heap value. See the module note.
pub type Ref<T> = std::sync::Arc<T>;

/// A map, which keeps the order its keys were first put in.
pub type Table = IndexMap<Key, Value>;

/// A Vaab value.
#[derive(Clone, Debug, Default)]
pub enum Value {
    /// The one value of type `Nothing`: what a function with no `returns` gives.
    #[default]
    Nothing,
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Ref<str>),
    List(Ref<Vec<Value>>),
    Map(Ref<Table>),
    Tuple(Ref<Vec<Value>>),
    /// A `maybe T`: `found x`, or absent.
    Maybe(Option<Ref<Value>>),
    /// The happy half of a `T or fails E`.
    Success(Ref<Value>),
    /// The failed half of a `T or fails E`.
    Failure(Ref<Value>),
    /// A value of a declared `type`.
    Record(Ref<Record>),
    /// One variant of a declared `choice`.
    Variant(Ref<Variant>),
    /// A declared function or a closure, with whatever it captured.
    Function(Ref<Closure>),
    /// A function from the prelude, used as a value rather than called outright.
    Builtin(Builtin),
    /// A local an inner closure can see, so it lives in a cell both can reach
    /// rather than in the frame that declared it.
    Captured(Ref<Captured>),
    /// A bounded queue between tasks. Phase 7 shares these with `Arc`.
    Channel(u32),
    /// State several tasks may read and change through `.value` and `.update`.
    Shared(Ref<Captured>),
    /// A handle to a task the scheduler is running.
    Task(usize),
    /// An open SQLite connection living in [`crate::machine::World::databases`].
    Db(u32),
    /// An open embedded KV store living in [`crate::machine::World::stores`].
    Store(u32),
    /// A configured logger living in [`crate::machine::World::loggers`].
    Logger(u32),
    /// A fluent AREL-style relation over a Db or Store.
    Query(Ref<crate::query::Query>),
}

/// A local that more than one frame can see.
///
/// A closure that assigns to a value declared around it has to change the very
/// slot the declaring frame reads, so such a local is boxed here when it is
/// declared. The compiler decides which locals need this; see `compile`.
#[derive(Default)]
pub struct Captured(Mutex<Value>);

impl Captured {
    pub fn new(value: Value) -> Captured {
        Captured(Mutex::new(value))
    }

    pub fn get(&self) -> Value {
        match self.0.lock() {
            Ok(held) => held.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set(&self, value: Value) {
        match self.0.lock() {
            Ok(mut held) => *held = value,
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }
}

impl fmt::Debug for Captured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Captured({:?})", self.get())
    }
}

/// What every value of one declared `type` has in common: its name, its fields in
/// order, and the ability tables a dynamic call dispatches through.
#[derive(Clone, Debug)]
pub struct RecordLayout {
    pub name: String,
    pub fields: Vec<String>,
    /// `true` for a `cast`, whose fields may be changed in place.
    pub mutable: bool,
    /// `tables[ability][slot]` is the body that fills that slot of that ability.
    /// An ability the type does not provide has an empty row.
    pub tables: Vec<Vec<u32>>,
}

/// A value of a declared `type` or `cast`. The layout is shared by every value.
#[derive(Clone, Debug)]
pub struct Record {
    pub layout: Ref<RecordLayout>,
    /// Immutable field storage for a `type`.
    pub fields: Vec<Value>,
    /// Mutable field storage for a `cast`.
    pub cells: Option<Ref<Vec<Captured>>>,
}

impl Record {
    pub fn field(&self, index: usize) -> Option<Value> {
        if let Some(cells) = &self.cells {
            cells.get(index).map(Captured::get)
        } else {
            self.fields.get(index).cloned()
        }
    }

    pub fn set_field(&self, index: usize, value: Value) -> bool {
        if let Some(cells) = &self.cells {
            if let Some(cell) = cells.get(index) {
                cell.set(value);
                return true;
            }
        }
        false
    }
}

/// What every value of one variant of one `choice` has in common.
#[derive(Clone, Debug)]
pub struct VariantLayout {
    /// This variant's place in the flattened table, which is what a pattern tests.
    pub index: u32,
    pub choice: String,
    pub name: String,
    pub fields: Vec<String>,
}

/// One variant of a `choice`, with its payload.
#[derive(Clone, Debug)]
pub struct Variant {
    pub layout: Ref<VariantLayout>,
    pub fields: Vec<Value>,
}

/// A function as a value: which body to run, and what it took with it.
#[derive(Clone, Debug)]
pub struct Closure {
    pub body: u32,
    /// What to call it in a trace or when it is printed.
    pub name: Ref<str>,
    /// One entry per [`Capture`](crate::bytecode::Capture) of the body, each a
    /// [`Value::Captured`].
    pub captures: Vec<Value>,
}

impl Value {
    pub fn text(text: impl AsRef<str>) -> Value {
        Value::Text(Ref::from(text.as_ref()))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Ref::new(items))
    }

    pub fn map(entries: Table) -> Value {
        Value::Map(Ref::new(entries))
    }

    pub fn found(value: Value) -> Value {
        Value::Maybe(Some(Ref::new(value)))
    }

    pub fn absent() -> Value {
        Value::Maybe(None)
    }

    pub fn success(value: Value) -> Value {
        Value::Success(Ref::new(value))
    }

    pub fn failure(value: Value) -> Value {
        Value::Failure(Ref::new(value))
    }

    /// Whether this is the value a statement that works nothing out leaves behind.
    ///
    /// A REPL uses this to tell a line worth answering from one that has already
    /// said everything it had to say.
    pub fn is_nothing(&self) -> bool {
        matches!(self, Value::Nothing)
    }

    /// Whether `==` holds between two values.
    ///
    /// The type checker has already agreed that both sides are the same type, so
    /// this only has to compare like with like. Floats follow IEEE here, which is
    /// why a `NaN` is not equal to itself; map keys use [`Key`] instead.
    pub fn equals(&self, other: &Value) -> bool {
        compare(self, other, Floats::AsNumbers)
    }

    /// What `print` shows.
    ///
    /// Text is shown as it is when it *is* the value, and in quotes when it sits
    /// inside something else, so that `["a", "b"]` does not read as two bare words.
    pub fn show(&self) -> String {
        match self {
            Value::Text(text) => text.to_string(),
            other => other.quoted(),
        }
    }

    /// The same, for a value inside a list, a map, a tuple or a record.
    pub(crate) fn quoted(&self) -> String {
        match self {
            Value::Nothing => "nothing".to_string(),
            Value::Int(number) => number.to_string(),
            // `{:?}` is the shortest text that reads back as the same float, and it
            // keeps the point: `1.0` rather than `1`.
            Value::Float(number) => format!("{number:?}"),
            Value::Bool(true) => "yes".to_string(),
            Value::Bool(false) => "no".to_string(),
            Value::Text(text) => quote(text),
            Value::List(items) => format!("[{}]", joined(items)),
            Value::Map(entries) => {
                if entries.is_empty() {
                    return "{}".to_string();
                }
                let shown: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| format!("{}: {}", key.0.quoted(), value.quoted()))
                    .collect();
                format!("{{{}}}", shown.join(", "))
            }
            Value::Tuple(parts) => format!("({})", joined(parts)),
            Value::Maybe(None) => "nothing".to_string(),
            Value::Maybe(Some(held)) => format!("found {}", held.quoted()),
            Value::Success(held) => format!("success {}", held.quoted()),
            Value::Failure(held) => format!("failure {}", held.quoted()),
            Value::Record(record) => {
                let shown: Vec<String> = record
                    .layout
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        format!(
                            "{name}: {}",
                            record.field(index).unwrap_or(Value::Nothing).quoted()
                        )
                    })
                    .collect();
                format!("{}({})", record.layout.name, shown.join(", "))
            }
            Value::Variant(variant) => {
                let path = format!("{}.{}", variant.layout.choice, variant.layout.name);
                if variant.fields.is_empty() {
                    return path;
                }
                format!("{path}({})", joined(&variant.fields))
            }
            Value::Function(closure) => closure.name.to_string(),
            Value::Builtin(builtin) => format!("to {}", builtin.name()),
            // A cell is never handed to anything that shows a value; it is read
            // through first. Showing what is inside is still the honest answer.
            Value::Captured(cell) => cell.get().quoted(),
            Value::Channel(_) => "channel".to_string(),
            Value::Shared(held) => format!("shared {}", held.get().quoted()),
            Value::Task(id) => format!("task {id}"),
            Value::Db(handle) => format!("db {handle}"),
            Value::Store(handle) => format!("store {handle}"),
            Value::Logger(handle) => format!("logger {handle}"),
            Value::Query(_) => "query".to_string(),
        }
    }
}

/// Renders a value the way `print` does.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.show())
    }
}

fn joined(values: &[Value]) -> String {
    values.iter().map(Value::quoted).collect::<Vec<String>>().join(", ")
}

/// Text inside a container, with the escapes Vaab knows put back in.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for letter in text.chars() {
        match letter {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// How to treat a float when two values are compared.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Floats {
    /// IEEE, so a `NaN` equals nothing, not even itself. This is what `==` means.
    AsNumbers,
    /// By the bits, so that a key put into a map can always be found again.
    AsBits,
}

fn compare(left: &Value, right: &Value, floats: Floats) -> bool {
    match (left, right) {
        (Value::Nothing, Value::Nothing) => true,
        (Value::Int(left), Value::Int(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => match floats {
            Floats::AsNumbers => left == right,
            Floats::AsBits => left.to_bits() == right.to_bits(),
        },
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Text(left), Value::Text(right)) => left == right,
        (Value::List(left), Value::List(right)) | (Value::Tuple(left), Value::Tuple(right)) => {
            left.len() == right.len()
                && left.iter().zip(right.iter()).all(|(a, b)| compare(a, b, floats))
        }
        (Value::Map(left), Value::Map(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right.get(key).is_some_and(|other| compare(value, other, floats))
                })
        }
        (Value::Maybe(None), Value::Maybe(None)) => true,
        (Value::Maybe(Some(left)), Value::Maybe(Some(right)))
        | (Value::Success(left), Value::Success(right))
        | (Value::Failure(left), Value::Failure(right)) => compare(left, right, floats),
        (Value::Record(left), Value::Record(right)) => {
            Ref::ptr_eq(&left.layout, &right.layout)
                && left
                    .fields
                    .iter()
                    .zip(&right.fields)
                    .all(|(a, b)| compare(a, b, floats))
        }
        (Value::Variant(left), Value::Variant(right)) => {
            left.layout.index == right.layout.index
                && left
                    .fields
                    .iter()
                    .zip(&right.fields)
                    .all(|(a, b)| compare(a, b, floats))
        }
        // Two functions are the same function only when they are the same value.
        // There is no way to look inside one and ask whether it would agree.
        (Value::Function(left), Value::Function(right)) => Ref::ptr_eq(left, right),
        (Value::Builtin(left), Value::Builtin(right)) => left == right,
        (Value::Channel(left), Value::Channel(right)) => left == right,
        (Value::Shared(left), Value::Shared(right)) => Ref::ptr_eq(left, right),
        (Value::Task(left), Value::Task(right)) => left == right,
        (Value::Db(left), Value::Db(right)) => left == right,
        (Value::Store(left), Value::Store(right)) => left == right,
        (Value::Logger(left), Value::Logger(right)) => left == right,
        (Value::Query(left), Value::Query(right)) => left.as_ref() == right.as_ref(),
        _ => false,
    }
}

/// A value used as a key in a map.
///
/// Vaab lets anything be a key, so this is a wrapper rather than a smaller type.
/// It differs from `==` in one place: floats are compared by their bits, because a
/// key that cannot be found again is worse than a surprising `NaN`.
#[derive(Clone, Debug)]
pub struct Key(pub Value);

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        compare(&self.0, &other.0, Floats::AsBits)
    }
}

impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_value(&self.0, state);
    }
}

fn hash_value<H: Hasher>(value: &Value, state: &mut H) {
    // The tag keeps `1` and `yes` apart even though neither hashes anything else.
    std::mem::discriminant(value).hash(state);
    match value {
        Value::Nothing | Value::Maybe(None) => {}
        Value::Int(number) => number.hash(state),
        Value::Float(number) => number.to_bits().hash(state),
        Value::Bool(held) => held.hash(state),
        Value::Text(text) => text.hash(state),
        Value::List(items) | Value::Tuple(items) => {
            items.len().hash(state);
            for item in items.iter() {
                hash_value(item, state);
            }
        }
        Value::Map(entries) => {
            // A map's order is not part of what it is, so the entries' hashes are
            // added together rather than fed in one after another.
            entries.len().hash(state);
            let mut total = 0u64;
            for (key, value) in entries.iter() {
                let mut each = std::collections::hash_map::DefaultHasher::new();
                key.hash(&mut each);
                hash_value(value, &mut each);
                total = total.wrapping_add(each.finish());
            }
            total.hash(state);
        }
        Value::Maybe(Some(held)) | Value::Success(held) | Value::Failure(held) => {
            hash_value(held, state)
        }
        Value::Record(record) => {
            record.layout.name.hash(state);
            for index in 0..record.layout.fields.len() {
                if let Some(field) = record.field(index) {
                    hash_value(&field, state);
                }
            }
        }
        Value::Variant(variant) => {
            variant.layout.index.hash(state);
            for field in &variant.fields {
                hash_value(field, state);
            }
        }
        Value::Function(closure) => closure.body.hash(state),
        Value::Builtin(builtin) => builtin.name().hash(state),
        Value::Captured(cell) => hash_value(&cell.get(), state),
        Value::Channel(id) => id.hash(state),
        Value::Shared(held) => hash_value(&held.get(), state),
        Value::Task(id) => id.hash(state),
        Value::Db(id) => id.hash(state),
        Value::Store(id) => id.hash(state),
        Value::Logger(id) => id.hash(state),
        Value::Query(query) => {
            // Identity by pointer is enough for map keys; queries are not keyed.
            Ref::as_ptr(query).hash(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_bare_on_its_own_and_quoted_inside_a_list() {
        assert_eq!(Value::text("hello").show(), "hello");
        assert_eq!(Value::list(vec![Value::text("hello")]).show(), "[\"hello\"]");
    }

    #[test]
    fn numbers_keep_the_point_that_says_they_are_floats() {
        assert_eq!(Value::Int(1).show(), "1");
        assert_eq!(Value::Float(1.0).show(), "1.0");
        assert_eq!(Value::Float(0.75).show(), "0.75");
    }

    #[test]
    fn booleans_are_shown_in_the_words_vaab_uses() {
        assert_eq!(Value::Bool(true).show(), "yes");
        assert_eq!(Value::Bool(false).show(), "no");
    }

    #[test]
    fn a_maybe_and_a_result_are_shown_the_way_they_are_written() {
        assert_eq!(Value::absent().show(), "nothing");
        assert_eq!(Value::found(Value::Int(3)).show(), "found 3");
        assert_eq!(Value::success(Value::Int(3)).show(), "success 3");
        assert_eq!(Value::failure(Value::text("no")).show(), "failure \"no\"");
    }

    #[test]
    fn a_nan_equals_nothing_but_still_finds_its_own_key() {
        let nan = Value::Float(f64::NAN);
        assert!(!nan.equals(&nan));
        assert_eq!(Key(nan.clone()), Key(nan));
    }

    #[test]
    fn a_captured_local_can_be_read_more_than_once() {
        let cell = Captured::new(Value::Int(1));
        assert!(cell.get().equals(&Value::Int(1)));
        assert!(cell.get().equals(&Value::Int(1)));
        cell.set(Value::Int(2));
        assert!(cell.get().equals(&Value::Int(2)));
    }

    #[test]
    fn escapes_come_back_when_text_is_shown_inside_a_list() {
        let held = Value::list(vec![Value::text("a\"b\nc")]);
        assert_eq!(held.show(), "[\"a\\\"b\\nc\"]");
    }
}
