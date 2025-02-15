use std::pin::Pin;

use crate::Context;
use futures::Stream;
use juniper::{graphql_subscription, FieldError};
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

    pub fn values(&self) -> String {
      "values".to_string()
    }
}

pub struct Subscription;

type GqlEntriesStream = Pin<Box<dyn Stream<Item = Result<EntryData, FieldError>> + Send>>;

#[graphql_subscription(
  context = Context,
)]
impl Subscription {
    async fn entries_event(context: &Context) -> GqlEntriesStream {
        let entries_tx = context.entries_broadcaster.clone();
        let mut entries_tx = entries_tx.subscribe();
        let stream = async_stream::stream! {
            loop {
              if let Ok(entry_event) = entries_tx.recv().await {
                yield Ok(entry_event)
              }
            }
        };
        Box::pin(stream)
    }
}

pub struct DynamicSubscription;

#[graphql_subscription(
  context = Context,

)]
impl DynamicSubscription {
    async fn entries_event(context: &Context) -> GqlEntriesStream {
        let entries_tx = context.entries_broadcaster.clone();
        let mut entries_tx = entries_tx.subscribe();
        let stream = async_stream::stream! {
            loop {
              if let Ok(entry_event) = entries_tx.recv().await {
                yield Ok(entry_event)
              }
            }
        };
        Box::pin(stream)
    }
}
