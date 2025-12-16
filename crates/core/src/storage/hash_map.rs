// pub use std::collections::HashMap;
// use std::hash::Hash;

// impl<K, V> super::Storage<K, V> for HashMap<K, V>
// where
//     K: Eq + Hash,
// {
//     fn connect(database_url: Option<&str>) -> super::StorageResult<Self>
//     where
//         Self: Sized,
//     {
//         if database_url.is_some() {
//             return Err(super::StorageError::InvalidConfiguration(
//                 "HashMap storage does not support database URLs".to_string(),
//             ));
//         }
//         Ok(HashMap::new())
//     }

//     fn store(&mut self, key: K, value: V) -> super::StorageResult<()> {
//         self.insert(key, value);
//         Ok(())
//     }

//     fn get(&self, key: &K) -> super::StorageResult<Option<&V>> {
//         Ok(self.get(key))
//     }

//     fn take(&mut self, key: &K) -> super::StorageResult<Option<V>> {
//         Ok(self.remove(key))
//     }
// }
