//! AREL-style fluent queries over `Db` and `Store`.
//!
//! The Vaab surface is one `Query` type: `.from`, `.where_eq`, `.order`,
//! `.all`, and friends. SQL backends compile through [`sea_query`]; the
//! embedded KV store evaluates the same AST against key/value (and JSON)
//! rows.

use indexmap::IndexMap;
use sea_query::{
    Alias, Asterisk, Expr, ExprTrait, Func, Order, PostgresQueryBuilder, Query as SeaQuery,
    SqliteQueryBuilder, Value as SeaValue,
};
use serde_json::Value as Json;

use crate::app_io::{Databases, Stores};
use crate::value::{Key, Ref, Value};

/// Which backend a fluent query runs against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySource {
    Db(u32),
    Store(u32),
}

/// Comparison used by `.where_*` methods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compare {
    Eq,
    Not,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Predicate {
    pub column: String,
    pub compare: Compare,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
}

/// One fluent relation, built by chaining methods and finished by a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub source: QuerySource,
    /// Table name for SQL, or key prefix for the store.
    pub table: String,
    pub predicates: Vec<Predicate>,
    pub order: Vec<OrderBy>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// `None` means every column (`*` / full row).
    pub columns: Option<Vec<String>>,
}

impl Query {
    pub fn from_db(handle: u32, table: String) -> Query {
        Query {
            source: QuerySource::Db(handle),
            table,
            predicates: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
            columns: None,
        }
    }

    pub fn from_store(handle: u32, prefix: String) -> Query {
        Query {
            source: QuerySource::Store(handle),
            table: prefix,
            predicates: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
            columns: None,
        }
    }

    pub fn with_predicate(mut self, column: String, compare: Compare, value: String) -> Query {
        self.predicates.push(Predicate { column, compare, value });
        self
    }

    pub fn with_order(mut self, column: String, descending: bool) -> Query {
        self.order.push(OrderBy { column, descending });
        self
    }

    pub fn with_limit(mut self, limit: i64) -> Query {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: i64) -> Query {
        self.offset = Some(offset);
        self
    }

    pub fn with_columns(mut self, columns: Vec<String>) -> Query {
        self.columns = Some(columns);
        self
    }

    pub fn error_name(&self) -> &'static str {
        match self.source {
            QuerySource::Db(_) => "DbError",
            QuerySource::Store(_) => "StoreError",
        }
    }
}

/// How SQL is spelled for the open connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

pub fn dialect_for(url: &str) -> Dialect {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("postgres:")
        || lower.starts_with("postgresql:")
        || lower.starts_with("postgres://")
        || lower.starts_with("postgresql://")
    {
        Dialect::Postgres
    } else {
        Dialect::Sqlite
    }
}

fn sea_to_text(value: &SeaValue) -> String {
    match value {
        SeaValue::Bool(Some(yes)) => if *yes { "1" } else { "0" }.into(),
        SeaValue::TinyInt(Some(n)) => n.to_string(),
        SeaValue::SmallInt(Some(n)) => n.to_string(),
        SeaValue::Int(Some(n)) => n.to_string(),
        SeaValue::BigInt(Some(n)) => n.to_string(),
        SeaValue::TinyUnsigned(Some(n)) => n.to_string(),
        SeaValue::SmallUnsigned(Some(n)) => n.to_string(),
        SeaValue::Unsigned(Some(n)) => n.to_string(),
        SeaValue::BigUnsigned(Some(n)) => n.to_string(),
        SeaValue::Float(Some(n)) => n.to_string(),
        SeaValue::Double(Some(n)) => n.to_string(),
        SeaValue::String(Some(text)) => text.to_string(),
        SeaValue::Char(Some(letter)) => letter.to_string(),
        _ => String::new(),
    }
}

fn bind_texts(values: &[SeaValue]) -> Vec<Value> {
    values.iter().map(|value| Value::text(sea_to_text(value))).collect()
}

fn column_expr(name: &str) -> Expr {
    Expr::col(Alias::new(name.to_string()))
}

fn apply_predicates(mut select: sea_query::SelectStatement, predicates: &[Predicate]) -> sea_query::SelectStatement {
    for predicate in predicates {
        let left = column_expr(&predicate.column);
        let right = Expr::val(predicate.value.clone());
        let condition = match predicate.compare {
            Compare::Eq => left.eq(right),
            Compare::Not => left.ne(right),
            Compare::Gt => left.gt(right),
            Compare::Gte => left.gte(right),
            Compare::Lt => left.lt(right),
            Compare::Lte => left.lte(right),
            Compare::Like => left.like(predicate.value.clone()),
        };
        select = select.and_where(condition).to_owned();
    }
    select
}

fn build_select(query: &Query, dialect: Dialect) -> Result<(String, Vec<Value>), String> {
    let mut select = SeaQuery::select();
    match &query.columns {
        Some(columns) if !columns.is_empty() => {
            for column in columns {
                select = select.column(Alias::new(column.clone())).to_owned();
            }
        }
        _ => {
            select = select.column(Asterisk).to_owned();
        }
    }
    select = select.from(Alias::new(query.table.clone())).to_owned();
    select = apply_predicates(select, &query.predicates);
    for order in &query.order {
        let direction = if order.descending { Order::Desc } else { Order::Asc };
        select = select.order_by(Alias::new(order.column.clone()), direction).to_owned();
    }
    if let Some(limit) = query.limit {
        select = select.limit(limit as u64).to_owned();
    }
    if let Some(offset) = query.offset {
        select = select.offset(offset as u64).to_owned();
    }
    let (sql, values) = match dialect {
        Dialect::Sqlite => select.build(SqliteQueryBuilder),
        Dialect::Postgres => select.build(PostgresQueryBuilder),
    };
    Ok((sql, bind_texts(&values.0)))
}

fn build_count(query: &Query, dialect: Dialect) -> Result<(String, Vec<Value>), String> {
    let mut select = SeaQuery::select()
        .expr(Func::count(Expr::col(Asterisk)))
        .from(Alias::new(query.table.clone()))
        .to_owned();
    select = apply_predicates(select, &query.predicates);
    let (sql, values) = match dialect {
        Dialect::Sqlite => select.build(SqliteQueryBuilder),
        Dialect::Postgres => select.build(PostgresQueryBuilder),
    };
    Ok((sql, bind_texts(&values.0)))
}

fn build_insert(query: &Query, row: &IndexMap<Key, Value>, dialect: Dialect) -> Result<(String, Vec<Value>), String> {
    if row.is_empty() {
        return Err("insert needs at least one column".into());
    }
    let mut columns = Vec::new();
    let mut values = Vec::new();
    for (key, value) in row {
        columns.push(Alias::new(key_as_text(key)?));
        values.push(Expr::val(value_as_text(value)?));
    }
    let (sql, bound) = match dialect {
        Dialect::Sqlite => SeaQuery::insert()
            .into_table(Alias::new(query.table.clone()))
            .columns(columns)
            .values_panic(values)
            .build(SqliteQueryBuilder),
        Dialect::Postgres => SeaQuery::insert()
            .into_table(Alias::new(query.table.clone()))
            .columns(columns)
            .values_panic(values)
            .build(PostgresQueryBuilder),
    };
    Ok((sql, bind_texts(&bound.0)))
}

fn build_update(
    query: &Query,
    values: &IndexMap<Key, Value>,
    dialect: Dialect,
) -> Result<(String, Vec<Value>), String> {
    if values.is_empty() {
        return Err("update needs at least one column".into());
    }
    let mut update = SeaQuery::update().table(Alias::new(query.table.clone())).to_owned();
    for (key, value) in values {
        update = update
            .value(Alias::new(key_as_text(key)?), value_as_text(value)?)
            .to_owned();
    }
    for predicate in &query.predicates {
        let left = column_expr(&predicate.column);
        let right = Expr::val(predicate.value.clone());
        let condition = match predicate.compare {
            Compare::Eq => left.eq(right),
            Compare::Not => left.ne(right),
            Compare::Gt => left.gt(right),
            Compare::Gte => left.gte(right),
            Compare::Lt => left.lt(right),
            Compare::Lte => left.lte(right),
            Compare::Like => left.like(predicate.value.clone()),
        };
        update = update.and_where(condition).to_owned();
    }
    let (sql, bound) = match dialect {
        Dialect::Sqlite => update.build(SqliteQueryBuilder),
        Dialect::Postgres => update.build(PostgresQueryBuilder),
    };
    Ok((sql, bind_texts(&bound.0)))
}

fn build_delete(query: &Query, dialect: Dialect) -> Result<(String, Vec<Value>), String> {
    let mut delete = SeaQuery::delete()
        .from_table(Alias::new(query.table.clone()))
        .to_owned();
    for predicate in &query.predicates {
        let left = column_expr(&predicate.column);
        let right = Expr::val(predicate.value.clone());
        let condition = match predicate.compare {
            Compare::Eq => left.eq(right),
            Compare::Not => left.ne(right),
            Compare::Gt => left.gt(right),
            Compare::Gte => left.gte(right),
            Compare::Lt => left.lt(right),
            Compare::Lte => left.lte(right),
            Compare::Like => left.like(predicate.value.clone()),
        };
        delete = delete.and_where(condition).to_owned();
    }
    let (sql, bound) = match dialect {
        Dialect::Sqlite => delete.build(SqliteQueryBuilder),
        Dialect::Postgres => delete.build(PostgresQueryBuilder),
    };
    Ok((sql, bind_texts(&bound.0)))
}

fn key_as_text(key: &Key) -> Result<String, String> {
    match &key.0 {
        Value::Text(text) => Ok(text.to_string()),
        _ => Err("query maps need text keys".into()),
    }
}

fn value_as_text(value: &Value) -> Result<String, String> {
    match value {
        Value::Text(text) => Ok(text.to_string()),
        Value::Int(number) => Ok(number.to_string()),
        Value::Float(number) => Ok(number.to_string()),
        Value::Bool(yes) => Ok(if *yes { "1".into() } else { "0".into() }),
        _ => Err("query values must be text, numbers, or yes/no".into()),
    }
}

/// Run a finished select against the right backend.
pub fn run_all(databases: &Databases, stores: &Stores, query: &Query) -> Result<Value, String> {
    match query.source {
        QuerySource::Db(handle) => {
            let dialect = databases.dialect(handle)?;
            let (sql, args) = build_select(query, dialect)?;
            databases.query(handle, &sql, &args)
        }
        QuerySource::Store(handle) => store_all(stores, handle, query),
    }
}

pub fn run_first(databases: &Databases, stores: &Stores, query: &Query) -> Result<Value, String> {
    let mut limited = query.clone();
    limited.limit = Some(1);
    match run_all(databases, stores, &limited)? {
        Value::List(rows) => Ok(match rows.first() {
            Some(row) => Value::found(row.clone()),
            None => Value::absent(),
        }),
        _ => Err("query.all did not return a list".into()),
    }
}

pub fn run_count(databases: &Databases, stores: &Stores, query: &Query) -> Result<i64, String> {
    match query.source {
        QuerySource::Db(handle) => {
            let dialect = databases.dialect(handle)?;
            let (sql, args) = build_count(query, dialect)?;
            let rows = databases.query(handle, &sql, &args)?;
            count_from_rows(rows)
        }
        QuerySource::Store(handle) => {
            let rows = store_all(stores, handle, query)?;
            match rows {
                Value::List(items) => Ok(items.len() as i64),
                _ => Err("store query did not return a list".into()),
            }
        }
    }
}

pub fn run_insert(
    databases: &Databases,
    stores: &Stores,
    query: &Query,
    row: &IndexMap<Key, Value>,
) -> Result<i64, String> {
    match query.source {
        QuerySource::Db(handle) => {
            let dialect = databases.dialect(handle)?;
            let (sql, args) = build_insert(query, row, dialect)?;
            databases.execute(handle, &sql, &args)
        }
        QuerySource::Store(handle) => store_insert(stores, handle, query, row),
    }
}

pub fn run_update(
    databases: &Databases,
    stores: &Stores,
    query: &Query,
    values: &IndexMap<Key, Value>,
) -> Result<i64, String> {
    match query.source {
        QuerySource::Db(handle) => {
            let dialect = databases.dialect(handle)?;
            let (sql, args) = build_update(query, values, dialect)?;
            databases.execute(handle, &sql, &args)
        }
        QuerySource::Store(handle) => store_update(stores, handle, query, values),
    }
}

pub fn run_delete(databases: &Databases, stores: &Stores, query: &Query) -> Result<i64, String> {
    match query.source {
        QuerySource::Db(handle) => {
            let dialect = databases.dialect(handle)?;
            let (sql, args) = build_delete(query, dialect)?;
            databases.execute(handle, &sql, &args)
        }
        QuerySource::Store(handle) => store_delete(stores, handle, query),
    }
}

fn count_from_rows(rows: Value) -> Result<i64, String> {
    let Value::List(items) = rows else {
        return Err("count did not return a list".into());
    };
    let Some(Value::Map(row)) = items.first() else {
        return Ok(0);
    };
    for value in row.values() {
        if let Value::Text(text) = value {
            if let Ok(number) = text.parse::<i64>() {
                return Ok(number);
            }
        }
    }
    Ok(0)
}

fn store_all(stores: &Stores, handle: u32, query: &Query) -> Result<Value, String> {
    let keys = stores.keys(handle, &query.table)?;
    let mut rows = Vec::new();
    for key in keys {
        let value = match stores.get(handle, &key)? {
            Value::Maybe(Some(held)) => match held.as_ref() {
                Value::Text(text) => text.to_string(),
                _ => continue,
            },
            Value::Maybe(None) => continue,
            _ => continue,
        };
        let row = store_row(&key, &value);
        if !matches_predicates(&row, &query.predicates) {
            continue;
        }
        rows.push(project_row(row, query.columns.as_ref()));
    }
    rows = order_rows(rows, &query.order);
    if let Some(offset) = query.offset {
        let skip = std::cmp::Ord::max(offset, 0) as usize;
        if skip >= rows.len() {
            rows.clear();
        } else {
            rows = rows.split_off(skip);
        }
    }
    if let Some(limit) = query.limit {
        rows.truncate(std::cmp::Ord::max(limit, 0) as usize);
    }
    Ok(Value::list(rows))
}

fn store_row(key: &str, value: &str) -> IndexMap<Key, Value> {
    let mut row = IndexMap::new();
    row.insert(Key(Value::text("key")), Value::text(key));
    row.insert(Key(Value::text("value")), Value::text(value));
    if let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(value) {
        for (name, field) in fields {
            row.insert(Key(Value::text(name)), json_to_text(field));
        }
    }
    row
}

fn json_to_text(value: Json) -> Value {
    match value {
        Json::Null => Value::text(""),
        Json::Bool(yes) => Value::text(if yes { "yes" } else { "no" }),
        Json::Number(number) => Value::text(number.to_string()),
        Json::String(text) => Value::text(text),
        other => Value::text(other.to_string()),
    }
}

fn project_row(row: IndexMap<Key, Value>, columns: Option<&Vec<String>>) -> Value {
    match columns {
        Some(columns) if !columns.is_empty() => {
            let mut projected = IndexMap::new();
            for column in columns {
                let key = Key(Value::text(column.clone()));
                if let Some(value) = row.get(&key) {
                    projected.insert(key, value.clone());
                }
            }
            Value::map(projected)
        }
        _ => Value::map(row),
    }
}

fn matches_predicates(row: &IndexMap<Key, Value>, predicates: &[Predicate]) -> bool {
    predicates.iter().all(|predicate| {
        let key = Key(Value::text(predicate.column.clone()));
        let Some(Value::Text(held)) = row.get(&key) else {
            return false;
        };
        let left = held.as_ref();
        let right = predicate.value.as_str();
        match predicate.compare {
            Compare::Eq => left == right,
            Compare::Not => left != right,
            Compare::Gt => compare_ordered(left, right) == std::cmp::Ordering::Greater,
            Compare::Gte => compare_ordered(left, right) != std::cmp::Ordering::Less,
            Compare::Lt => compare_ordered(left, right) == std::cmp::Ordering::Less,
            Compare::Lte => compare_ordered(left, right) != std::cmp::Ordering::Greater,
            Compare::Like => like_match(left, right),
        }
    })
}

fn compare_ordered(left: &str, right: &str) -> std::cmp::Ordering {
    match (left.parse::<f64>(), right.parse::<f64>()) {
        (Ok(left), Ok(right)) => left.partial_cmp(&right).unwrap_or(std::cmp::Ordering::Equal),
        _ => left.cmp(right),
    }
}

fn like_match(value: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('%').collect();
    if parts.len() == 1 {
        return value == pattern;
    }
    let mut rest = value;
    if !pattern.starts_with('%') {
        let first = parts[0];
        if !rest.starts_with(first) {
            return false;
        }
        rest = &rest[first.len()..];
    }
    if !pattern.ends_with('%') {
        let last = parts[parts.len() - 1];
        if !rest.ends_with(last) {
            return false;
        }
        rest = &rest[..rest.len() - last.len()];
    }
    let middle = if pattern.starts_with('%') {
        &parts[1..parts.len().saturating_sub(if pattern.ends_with('%') { 1 } else { 0 })]
    } else {
        &parts[1..parts.len().saturating_sub(if pattern.ends_with('%') { 1 } else { 0 })]
    };
    for part in middle {
        if part.is_empty() {
            continue;
        }
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    true
}

fn order_rows(mut rows: Vec<Value>, order: &[OrderBy]) -> Vec<Value> {
    if order.is_empty() {
        return rows;
    }
    rows.sort_by(|left, right| {
        let (Value::Map(left), Value::Map(right)) = (left, right) else {
            return std::cmp::Ordering::Equal;
        };
        for rule in order {
            let key = Key(Value::text(rule.column.clone()));
            let left_text = match left.get(&key) {
                Some(Value::Text(text)) => text.as_ref(),
                _ => "",
            };
            let right_text = match right.get(&key) {
                Some(Value::Text(text)) => text.as_ref(),
                _ => "",
            };
            let cmp = compare_ordered(left_text, right_text);
            if cmp != std::cmp::Ordering::Equal {
                return if rule.descending { cmp.reverse() } else { cmp };
            }
        }
        std::cmp::Ordering::Equal
    });
    rows
}

fn store_insert(
    stores: &Stores,
    handle: u32,
    query: &Query,
    row: &IndexMap<Key, Value>,
) -> Result<i64, String> {
    let key = row
        .get(&Key(Value::text("key")))
        .and_then(|value| match value {
            Value::Text(text) => Some(text.to_string()),
            _ => None,
        })
        .ok_or_else(|| "store insert needs a `key` field".to_string())?;
    let full_key = if query.table.is_empty() || key.starts_with(&query.table) {
        key
    } else {
        format!("{}{key}", query.table)
    };
    let value = if let Some(Value::Text(text)) = row.get(&Key(Value::text("value"))) {
        text.to_string()
    } else {
        let mut object = serde_json::Map::new();
        for (field, held) in row {
            let name = key_as_text(field)?;
            if name == "key" {
                continue;
            }
            object.insert(name, Json::String(value_as_text(held)?));
        }
        Json::Object(object).to_string()
    };
    stores.set(handle, &full_key, &value)?;
    Ok(1)
}

fn store_update(
    stores: &Stores,
    handle: u32,
    query: &Query,
    values: &IndexMap<Key, Value>,
) -> Result<i64, String> {
    let matched = match store_all(stores, handle, query)? {
        Value::List(rows) => rows.as_ref().clone(),
        _ => return Err("store update could not list rows".into()),
    };
    let mut changed = 0i64;
    for row in matched {
        let Value::Map(row) = row else { continue };
        let Some(Value::Text(key)) = row.get(&Key(Value::text("key"))) else { continue };
        let mut next = (*row).clone();
        for (field, value) in values {
            next.insert(field.clone(), value.clone());
        }
        let encoded = if let Some(Value::Text(text)) = values.get(&Key(Value::text("value"))) {
            text.to_string()
        } else {
            let mut object = serde_json::Map::new();
            for (field, held) in &next {
                let name = key_as_text(field)?;
                if name == "key" || name == "value" {
                    continue;
                }
                object.insert(name, Json::String(value_as_text(held)?));
            }
            if object.is_empty() {
                match next.get(&Key(Value::text("value"))) {
                    Some(Value::Text(text)) => text.to_string(),
                    _ => String::new(),
                }
            } else {
                Json::Object(object).to_string()
            }
        };
        stores.set(handle, key, &encoded)?;
        changed += 1;
    }
    Ok(changed)
}

fn store_delete(stores: &Stores, handle: u32, query: &Query) -> Result<i64, String> {
    let matched = match store_all(stores, handle, query)? {
        Value::List(rows) => rows.as_ref().clone(),
        _ => return Err("store delete could not list rows".into()),
    };
    let mut changed = 0i64;
    for row in matched {
        let Value::Map(row) = row else { continue };
        let Some(Value::Text(key)) = row.get(&Key(Value::text("key"))) else { continue };
        stores.remove(handle, key)?;
        changed += 1;
    }
    Ok(changed)
}

/// Turn a Vaab argument into the text a predicate holds.
pub fn argument_text(value: &Value) -> Result<String, String> {
    value_as_text(value)
}

pub fn as_query(value: &Value) -> Result<Query, String> {
    match value {
        Value::Query(query) => Ok(query.as_ref().clone()),
        _ => Err("expected a Query".into()),
    }
}

pub fn wrap(query: Query) -> Value {
    Value::Query(Ref::new(query))
}
