// mod hash_map;
#[cfg(feature = "sqlite")]
mod sqlite;
// pub use hash_map::HashMap as StorageHashMap;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;
use surfpool_db::diesel::ConnectionError;

use crate::error::SurfpoolError;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Failed to get pooled connection for '{0}' database: {1}")]
    PooledConnectionError(String, #[source] surfpool_db::diesel::r2d2::PoolError),
    #[error("Failed to serialize key for '{0}' database: {1}")]
    SerializeKeyError(String, serde_json::Error),
    #[error("Failed to serialize value for '{0}' database: {1}")]
    SerializeValueError(String, serde_json::Error),
    #[error("Failed to deserialize value in '{0}' database: {1}")]
    DeserializeValueError(String, serde_json::Error),
    #[error("Failed to acquire lock for database")]
    LockError,
    #[error("Query failed for table '{0}' in '{1}' database: {2}")]
    QueryError(String, String, #[source] QueryExecuteError),
}

impl StorageError {
    pub fn create_table(
        table_name: &str,
        db_type: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::CreateTableError(e),
        )
    }
    pub fn store(
        table_name: &str,
        db_type: &str,
        store_key: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::StoreError(store_key.to_string(), e),
        )
    }
    pub fn get(
        table_name: &str,
        db_type: &str,
        get_key: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::GetError(get_key.to_string(), e),
        )
    }
    pub fn delete(
        table_name: &str,
        db_type: &str,
        delete_key: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::DeleteError(delete_key.to_string(), e),
        )
    }
    pub fn get_all_keys(
        table_name: &str,
        db_type: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::GetAllKeysError(e),
        )
    }
    pub fn get_all_key_value_pairs(
        table_name: &str,
        db_type: &str,
        e: surfpool_db::diesel::result::Error,
    ) -> Self {
        StorageError::QueryError(
            table_name.to_string(),
            db_type.to_string(),
            QueryExecuteError::GetAllKeyValuePairsError(e),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueryExecuteError {
    #[error("Failed to create table: {0}")]
    CreateTableError(#[source] surfpool_db::diesel::result::Error),
    #[error("Failed to store value for key '{0}': {1}")]
    StoreError(String, #[source] surfpool_db::diesel::result::Error),
    #[error("Failed to get value for key '{0}': {1}")]
    GetError(String, #[source] surfpool_db::diesel::result::Error),
    #[error("Failed to delete value for key '{0}': {1}")]
    DeleteError(String, #[source] surfpool_db::diesel::result::Error),
    #[error("Failed to get all keys: {0}")]
    GetAllKeysError(#[source] surfpool_db::diesel::result::Error),
    #[error("Failed to get all key-value pairs: {0}")]
    GetAllKeyValuePairsError(#[source] surfpool_db::diesel::result::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

impl From<StorageError> for jsonrpc_core::Error {
    fn from(err: StorageError) -> Self {
        SurfpoolError::from(err).into()
    }
}

pub trait Storage<K, V>: Send + Sync {
    fn store(&mut self, key: K, value: V) -> StorageResult<()>;
    fn clear(&mut self) -> StorageResult<()>;
    fn get(&self, key: &K) -> StorageResult<Option<V>>;
    fn take(&mut self, key: &K) -> StorageResult<Option<V>>;
    fn keys(&self) -> StorageResult<Vec<K>>;
    fn into_iter(&self) -> StorageResult<Box<dyn Iterator<Item = (K, V)> + '_>>;
    fn contains_key(&self, key: &K) -> StorageResult<bool> {
        Ok(self.get(key)?.is_some())
    }

    // Enable cloning of boxed trait objects
    fn clone_box(&self) -> Box<dyn Storage<K, V>>;
}

// Implement Clone for Box<dyn Storage<K, V>>
impl<K, V> Clone for Box<dyn Storage<K, V>> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

// Separate trait for construction - this doesn't need to be dyn-compatible
pub trait StorageConstructor<K, V>: Storage<K, V> + Clone {
    fn connect(database_url: &str, table_name: &str) -> StorageResult<Self>
    where
        Self: Sized;
}
