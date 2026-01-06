use log::debug;
use serde::{Deserialize, Serialize};
use surfpool_db::diesel::{
    self, QueryableByName, RunQueryDsl,
    connection::SimpleConnection,
    r2d2::{ConnectionManager, Pool},
    sql_query,
    sql_types::Text,
};

use crate::storage::{Storage, StorageConstructor, StorageError, StorageResult};

#[derive(QueryableByName, Debug)]
struct KvRecord {
    #[diesel(sql_type = Text)]
    key: String,
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName, Debug)]
struct ValueRecord {
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName, Debug)]
struct KeyRecord {
    #[diesel(sql_type = Text)]
    key: String,
}

#[derive(Clone)]
pub struct SqliteStorage<K, V> {
    pool: Pool<ConnectionManager<diesel::SqliteConnection>>,
    _phantom: std::marker::PhantomData<(K, V)>,
    table_name: String,
    surfnet_id: u32,
}

const NAME: &str = "SQLite";

impl<K, V> SqliteStorage<K, V>
where
    K: Serialize + for<'de> Deserialize<'de>,
    V: Serialize + for<'de> Deserialize<'de> + Clone,
{
    fn ensure_table_exists(&self) -> StorageResult<()> {
        debug!("Ensuring table '{}' exists", self.table_name);
        let create_table_sql = format!(
            "
            CREATE TABLE IF NOT EXISTS {} (
                surfnet_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (surfnet_id, key)
            )
        ",
            self.table_name
        );

        debug!("Getting connection from pool for table creation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        conn.batch_execute(&create_table_sql)
            .map_err(|e| StorageError::create_table(&self.table_name, NAME, e))?;

        debug!("Successfully ensured table '{}' exists", self.table_name);
        Ok(())
    }

    fn serialize_key(&self, key: &K) -> StorageResult<String> {
        trace!("Serializing key for table '{}'", self.table_name);
        let result =
            serde_json::to_string(key).map_err(|e| StorageError::SerializeKeyError(NAME.into(), e));
        if let Ok(ref serialized) = result {
            trace!("Key serialized successfully: {}", serialized);
        }
        result
    }

    fn serialize_value(&self, value: &V) -> StorageResult<String> {
        trace!("Serializing value for table '{}'", self.table_name);
        let result = serde_json::to_string(value)
            .map_err(|e| StorageError::SerializeValueError(NAME.into(), e));
        if let Ok(ref serialized) = result {
            trace!(
                "Value serialized successfully, length: {} chars",
                serialized.len()
            );
        }
        result
    }

    fn deserialize_value(&self, value_str: &str) -> StorageResult<V> {
        trace!(
            "Deserializing value from table '{}', input length: {} chars",
            self.table_name,
            value_str.len()
        );
        let result = serde_json::from_str(value_str)
            .map_err(|e| StorageError::DeserializeValueError(NAME.into(), e));
        if result.is_ok() {
            trace!("Value deserialized successfully");
        }
        result
    }

    fn load_value_from_db(&self, key_str: &str) -> StorageResult<Option<V>> {
        debug!("Loading value from DB for key: {}", key_str);
        let query = sql_query(format!(
            "SELECT value FROM {} WHERE surfnet_id = ? AND key = ?",
            self.table_name
        ))
        .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32)
        .bind::<Text, _>(key_str);

        trace!("Getting connection from pool for loading value");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        let records = query
            .load::<ValueRecord>(&mut *conn)
            .map_err(|e| StorageError::get(&self.table_name, NAME, key_str, e))?;

        if let Some(record) = records.into_iter().next() {
            debug!("Found record for key: {}", key_str);
            let value = self.deserialize_value(&record.value)?;
            Ok(Some(value))
        } else {
            debug!("No record found for key: {}", key_str);
            Ok(None)
        }
    }
}

impl<K, V> Storage<K, V> for SqliteStorage<K, V>
where
    K: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
    V: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
{
    fn store(&mut self, key: K, value: V) -> StorageResult<()> {
        debug!("Storing value in table '{}", self.table_name);
        let key_str = self.serialize_key(&key)?;
        let value_str = self.serialize_value(&value)?;

        // Use prepared statement with sql_query for better safety
        let query = sql_query(format!(
            "INSERT OR REPLACE INTO {} (surfnet_id, key, value, updated_at) VALUES (?, ?, ?, CURRENT_TIMESTAMP)",
            self.table_name
        ))
        .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32)
        .bind::<Text, _>(&key_str)
        .bind::<Text, _>(&value_str);

        trace!("Getting connection from pool for store operation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        query
            .execute(&mut *conn)
            .map_err(|e| StorageError::store(&self.table_name, NAME, &key_str, e))?;

        debug!("Value stored successfully in table '{}'", self.table_name);
        Ok(())
    }

    fn get(&self, key: &K) -> StorageResult<Option<V>> {
        debug!("Getting value from table '{}", self.table_name);
        let key_str = self.serialize_key(key)?;

        self.load_value_from_db(&key_str)
    }

    fn take(&mut self, key: &K) -> StorageResult<Option<V>> {
        debug!("Taking value from table '{}'", self.table_name);
        let key_str = self.serialize_key(key)?;

        // If not in cache, try to load from database
        if let Some(value) = self.load_value_from_db(&key_str)? {
            debug!("Value found, removing from database");
            // Remove from database
            let delete_query = sql_query(format!(
                "DELETE FROM {} WHERE surfnet_id = ? AND key = ?",
                self.table_name
            ))
            .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32)
            .bind::<Text, _>(&key_str);

            trace!("Getting connection from pool for delete operation");
            let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

            delete_query
                .execute(&mut *conn)
                .map_err(|e| StorageError::delete(&self.table_name, NAME, &key_str, e))?;

            debug!(
                "Value taken and removed successfully from table '{}'",
                self.table_name
            );
            Ok(Some(value))
        } else {
            debug!("No value found to take from table '{}'", self.table_name);
            Ok(None)
        }
    }

    fn clear(&mut self) -> StorageResult<()> {
        debug!("Clearing all data from table '{}'", self.table_name);
        let delete_query = sql_query(format!(
            "DELETE FROM {} WHERE surfnet_id = ?",
            self.table_name
        ))
        .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32);

        trace!("Getting connection from pool for clear operation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        delete_query
            .execute(&mut *conn)
            .map_err(|e| StorageError::delete(&self.table_name, NAME, "*all*", e))?;

        debug!("Table '{}' cleared successfully", self.table_name);
        Ok(())
    }

    fn keys(&self) -> StorageResult<Vec<K>> {
        debug!("Fetching all keys from table '{}'", self.table_name);
        let query = sql_query(format!(
            "SELECT key FROM {} WHERE surfnet_id = ?",
            self.table_name
        ))
        .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32);

        trace!("Getting connection from pool for keys operation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        let records = query
            .load::<KeyRecord>(&mut *conn)
            .map_err(|e| StorageError::get_all_keys(&self.table_name, NAME, e))?;

        let mut keys = Vec::new();
        for record in records {
            let key: K = serde_json::from_str(&record.key)
                .map_err(|e| StorageError::DeserializeValueError(NAME.into(), e))?;
            keys.push(key);
        }

        debug!(
            "Retrieved {} keys from table '{}'",
            keys.len(),
            self.table_name
        );
        Ok(keys)
    }

    fn clone_box(&self) -> Box<dyn Storage<K, V>> {
        Box::new(self.clone())
    }

    fn into_iter(&self) -> StorageResult<Box<dyn Iterator<Item = (K, V)> + '_>> {
        debug!(
            "Creating iterator for all key-value pairs in table '{}'",
            self.table_name
        );
        let query = sql_query(format!(
            "SELECT key, value FROM {} WHERE surfnet_id = ?",
            self.table_name
        ))
        .bind::<diesel::sql_types::Integer, _>(self.surfnet_id as i32);

        trace!("Getting connection from pool for into_iter operation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        let records = query
            .load::<KvRecord>(&mut *conn)
            .map_err(|e| StorageError::get_all_key_value_pairs(&self.table_name, NAME, e))?;

        let iter = records.into_iter().filter_map(move |record| {
            let key: K = match serde_json::from_str(&record.key) {
                Ok(k) => k,
                Err(e) => {
                    debug!("Failed to deserialize key: {}", e);
                    return None;
                }
            };
            let value: V = match serde_json::from_str(&record.value) {
                Ok(v) => v,
                Err(e) => {
                    debug!("Failed to deserialize value: {}", e);
                    return None;
                }
            };
            Some((key, value))
        });

        debug!(
            "Iterator created successfully for table '{}'",
            self.table_name
        );
        Ok(Box::new(iter))
    }
}

impl<K, V> StorageConstructor<K, V> for SqliteStorage<K, V>
where
    K: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
    V: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
{
    fn connect(database_url: &str, table_name: &str, surfnet_id: u32) -> StorageResult<Self> {
        debug!(
            "Connecting to SQLite database: {} with table: {} and surfnet_id: {}",
            database_url, table_name, surfnet_id
        );

        let connection_string = if database_url != ":memory:" {
            // Add connection string parameters to avoid readonly issues
            if database_url.contains('?') {
                format!("{}&mode=rwc", database_url)
            } else {
                format!("{}?mode=rwc", database_url)
            }
        } else {
            database_url.to_string()
        };

        let manager = ConnectionManager::<diesel::SqliteConnection>::new(connection_string);
        trace!("Creating connection pool");
        let pool =
            Pool::new(manager).map_err(|e| StorageError::PooledConnectionError(NAME.into(), e))?;

        let storage = SqliteStorage {
            pool,
            _phantom: std::marker::PhantomData,
            table_name: table_name.to_string(),
            surfnet_id,
        };

        storage.ensure_table_exists()?;
        debug!(
            "SQLite storage connected successfully for table: {}",
            table_name
        );
        Ok(storage)
    }
}
