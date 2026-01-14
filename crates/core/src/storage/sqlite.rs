use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

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

/// Track which database files have already been checkpointed during shutdown.
/// This prevents multiple SqliteStorage instances sharing the same file from
/// conflicting when each tries to checkpoint and delete WAL files.
fn checkpointed_databases() -> &'static Mutex<HashSet<String>> {
    static CHECKPOINTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CHECKPOINTED.get_or_init(|| Mutex::new(HashSet::new()))
}

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

#[derive(QueryableByName, Debug)]
struct CountRecord {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

#[derive(Clone)]
pub struct SqliteStorage<K, V> {
    pool: Pool<ConnectionManager<diesel::SqliteConnection>>,
    _phantom: std::marker::PhantomData<(K, V)>,
    table_name: String,
    surfnet_id: String,
    /// Whether this is a file-based database (not :memory:)
    /// Used to determine if WAL checkpoint should be performed on drop
    is_file_based: bool,
    /// The connection string for creating direct connections during cleanup
    connection_string: String,
}

const NAME: &str = "SQLite";

// Checkpoint implementation that doesn't require K, V bounds
impl<K, V> SqliteStorage<K, V> {
    /// Checkpoint the WAL and truncate it to consolidate into the main database file,
    /// then remove the -wal and -shm files.
    /// Only runs for file-based databases (not :memory:).
    /// Uses a static set to track which databases have been checkpointed to avoid
    /// conflicts when multiple SqliteStorage instances share the same database file.
    fn checkpoint(&self) {
        if !self.is_file_based {
            return;
        }

        // Extract the file path from the connection string
        // Connection string is like "file:/path/to/db.sqlite?mode=rwc"
        let db_path = self
            .connection_string
            .strip_prefix("file:")
            .and_then(|s| s.split('?').next())
            .unwrap_or(&self.connection_string)
            .to_string();

        // Check if this database has already been checkpointed by another storage instance
        {
            let mut checkpointed = checkpointed_databases().lock().unwrap();
            if checkpointed.contains(&db_path) {
                debug!(
                    "Database {} already checkpointed, skipping for table '{}'",
                    db_path, self.table_name
                );
                return;
            }
            checkpointed.insert(db_path.clone());
        }

        debug!(
            "Checkpointing WAL for database '{}' (table '{}')",
            db_path, self.table_name
        );

        // Use pool connection to checkpoint - this flushes WAL to main database
        if let Ok(mut conn) = self.pool.get() {
            if let Err(e) = conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE);") {
                debug!("WAL checkpoint failed: {}", e);
                return;
            }
        }

        // Remove the -wal and -shm files
        let wal_path = format!("{}-wal", db_path);
        let shm_path = format!("{}-shm", db_path);

        if std::path::Path::new(&wal_path).exists() {
            if let Err(e) = std::fs::remove_file(&wal_path) {
                debug!("Failed to remove WAL file {}: {}", wal_path, e);
            } else {
                debug!("Removed WAL file: {}", wal_path);
            }
        }

        if std::path::Path::new(&shm_path).exists() {
            if let Err(e) = std::fs::remove_file(&shm_path) {
                debug!("Failed to remove SHM file {}: {}", shm_path, e);
            } else {
                debug!("Removed SHM file: {}", shm_path);
            }
        }
    }
}

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
                surfnet_id TEXT NOT NULL,
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
        .bind::<Text, _>(&self.surfnet_id)
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
        .bind::<Text, _>(&self.surfnet_id)
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
            .bind::<Text, _>(&self.surfnet_id)
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
        .bind::<Text, _>(&self.surfnet_id);

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
        .bind::<Text, _>(&self.surfnet_id);

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

    fn shutdown(&self) {
        self.checkpoint();
    }

    fn count(&self) -> StorageResult<u64> {
        debug!("Counting entries in table '{}'", self.table_name);
        let query = sql_query(format!(
            "SELECT COUNT(*) as count FROM {} WHERE surfnet_id = ?",
            self.table_name
        ))
        .bind::<Text, _>(&self.surfnet_id);

        trace!("Getting connection from pool for count operation");
        let mut conn = self.pool.get().map_err(|_| StorageError::LockError)?;

        let records = query
            .load::<CountRecord>(&mut *conn)
            .map_err(|e| StorageError::count(&self.table_name, NAME, e))?;

        let count = records.first().map(|r| r.count as u64).unwrap_or(0);
        debug!("Table '{}' has {} entries", self.table_name, count);
        Ok(count)
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
        .bind::<Text, _>(&self.surfnet_id);

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
    fn connect(database_url: &str, table_name: &str, surfnet_id: &str) -> StorageResult<Self> {
        debug!(
            "Connecting to SQLite database: {} with table: {} and surfnet_id: {}",
            database_url, table_name, surfnet_id
        );

        let connection_string = if database_url == ":memory:" {
            database_url.to_string()
        } else if database_url.starts_with("file:") {
            // Already a URI, just add mode if needed
            if database_url.contains('?') {
                format!("{}&mode=rwc", database_url)
            } else {
                format!("{}?mode=rwc", database_url)
            }
        } else {
            // Convert plain path to file: URI format for proper parameter handling
            format!("file:{}?mode=rwc", database_url)
        };

        let manager = ConnectionManager::<diesel::SqliteConnection>::new(connection_string.clone());
        trace!("Creating connection pool");
        let pool =
            Pool::new(manager).map_err(|e| StorageError::PooledConnectionError(NAME.into(), e))?;

        let is_file_based = database_url != ":memory:";
        let storage = SqliteStorage {
            pool,
            _phantom: std::marker::PhantomData,
            table_name: table_name.to_string(),
            surfnet_id: surfnet_id.to_string(),
            is_file_based,
            connection_string,
        };

        // Set SQLite pragmas for performance and reliability
        {
            let mut conn = storage.pool.get().map_err(|_| StorageError::LockError)?;

            // Different pragma sets for file-based vs in-memory databases
            let pragmas = if database_url == ":memory:" {
                // In-memory database pragmas (WAL not supported)
                "
                PRAGMA synchronous=OFF;
                PRAGMA temp_store=MEMORY;
                PRAGMA cache_size=-64000;
                PRAGMA busy_timeout=5000;
                "
            } else {
                // File-based database pragmas
                "
                PRAGMA journal_mode=WAL;
                PRAGMA synchronous=NORMAL;
                PRAGMA temp_store=MEMORY;
                PRAGMA mmap_size=268435456;
                PRAGMA cache_size=-64000;
                PRAGMA busy_timeout=5000;
                PRAGMA wal_autocheckpoint=1000;
                "
                // Pragma explanations:
                // - journal_mode=WAL: Write-Ahead Logging for better concurrency and crash recovery
                // - synchronous=NORMAL: Safe with WAL mode, good performance/durability balance
                // - temp_store=MEMORY: Store temp tables in memory for speed
                // - mmap_size=268435456: 256MB memory-mapped I/O for faster reads
                // - cache_size=-64000: 64MB page cache (negative = KB)
                // - busy_timeout=5000: Wait 5s for locks instead of failing immediately
                // - wal_autocheckpoint=1000: Checkpoint WAL after 1000 pages (~4MB with default page size)
            };

            conn.batch_execute(pragmas)
                .map_err(|e| StorageError::create_table(table_name, NAME, e))?;
        }

        storage.ensure_table_exists()?;
        debug!(
            "SQLite storage connected successfully for table: {}",
            table_name
        );
        Ok(storage)
    }
}
