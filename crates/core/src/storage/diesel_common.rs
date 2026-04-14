//! Shared helpers for the diesel-backed `Storage` implementations.
//!
//! Both `SqliteStorage` and `PostgresStorage` speak the same wire shape:
//! the key/value codec is pure `serde_json`, and every row they load is one
//! of four fixed `QueryableByName` shapes (three `TEXT` columns or a single
//! `BIGINT` for counts). This module owns those pieces so the per-backend
//! modules only carry the dialect-specific bits (connection type, SQL
//! placeholders `?` vs `$N`, upsert syntax, WAL management).
//!
//! The codec helpers take a `backend_name: &'static str` (e.g. `"SQLite"` /
//! `"PostgreSQL"`) so `StorageError` variants surfaced by the caller name
//! the correct backend.

use serde::{Deserialize, Serialize};
// The `QueryableByName` derive expands to paths rooted at `diesel::`, so we
// also re-bind the re-exported crate under that name at the module level.
use surfpool_db::diesel::{self, QueryableByName, sql_types::Text};

use crate::storage::{StorageError, StorageResult};

#[derive(QueryableByName, Debug)]
pub(crate) struct KvRecord {
    #[diesel(sql_type = Text)]
    pub key: String,
    #[diesel(sql_type = Text)]
    pub value: String,
}

#[derive(QueryableByName, Debug)]
pub(crate) struct ValueRecord {
    #[diesel(sql_type = Text)]
    pub value: String,
}

#[derive(QueryableByName, Debug)]
pub(crate) struct KeyRecord {
    #[diesel(sql_type = Text)]
    pub key: String,
}

#[derive(QueryableByName, Debug)]
pub(crate) struct CountRecord {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}

pub(crate) fn serialize_key<K: Serialize>(
    backend_name: &'static str,
    table_name: &str,
    key: &K,
) -> StorageResult<String> {
    trace!("Serializing key for table '{}'", table_name);
    let result = serde_json::to_string(key)
        .map_err(|e| StorageError::SerializeKeyError(backend_name.into(), e));
    if let Ok(ref serialized) = result {
        trace!("Key serialized successfully: {}", serialized);
    }
    result
}

pub(crate) fn serialize_value<V: Serialize>(
    backend_name: &'static str,
    table_name: &str,
    value: &V,
) -> StorageResult<String> {
    trace!("Serializing value for table '{}'", table_name);
    let result = serde_json::to_string(value)
        .map_err(|e| StorageError::SerializeValueError(backend_name.into(), e));
    if let Ok(ref serialized) = result {
        trace!(
            "Value serialized successfully, length: {} chars",
            serialized.len()
        );
    }
    result
}

pub(crate) fn deserialize_value<V: for<'de> Deserialize<'de>>(
    backend_name: &'static str,
    table_name: &str,
    value_str: &str,
) -> StorageResult<V> {
    trace!(
        "Deserializing value from table '{}', input length: {} chars",
        table_name,
        value_str.len()
    );
    let result = serde_json::from_str(value_str)
        .map_err(|e| StorageError::DeserializeValueError(backend_name.into(), e));
    if result.is_ok() {
        trace!("Value deserialized successfully");
    }
    result
}
