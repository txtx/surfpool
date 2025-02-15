use super::entry::EntryData;
use crate::Context;
use juniper_codegen::graphql_object;
use surfpool_core::types::{Collection, Entry};

#[derive(Debug, Clone)]
pub struct CollectionData {
    pub collection: Collection,
}

impl CollectionData {
    pub fn new(collection: &Collection) -> Self {
        Self {
            collection: collection.clone(),
        }
    }
}

#[graphql_object(context = Context)]
impl CollectionData {
    pub fn uuid(&self) -> String {
        self.collection.uuid.to_string()
    }

    pub fn name(&self) -> String {
        self.collection.name.clone()
    }

    pub fn entries(&self) -> Vec<EntryData> {
        self.collection
            .entries
            .iter()
            .map(|e| EntryData::new(e))
            .collect()
    }
}
