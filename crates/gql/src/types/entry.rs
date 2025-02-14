use crate::Context;
use juniper_codegen::graphql_object;
use surfpool_core::types::Entry;

#[derive(Debug, Clone)]
pub struct EntryData {
    pub entry: Entry,
}

impl EntryData {
    pub fn new(entry: &Entry) -> Self {
        Self {
            entry: entry.clone(),
        }
    }
}

#[graphql_object(context = Context)]
impl EntryData {
    pub fn uuid(&self) -> String {
        self.entry.uuid.to_string()
    }

    pub fn value(&self) -> String {
        self.entry.value.clone()
    }
}
