use std::pin::Pin;

use futures::Stream;
use juniper::{FieldError, graphql_subscription};

use crate::{query::DataloaderContext, types::CollectionEntryDataUpdate};

type GqlEntriesStream =
    Pin<Box<dyn Stream<Item = Result<CollectionEntryDataUpdate, FieldError>> + Send>>;

pub struct DynamicSubscription;

#[graphql_subscription(
    name = "Subscription"
    context = DataloaderContext,
)]
impl DynamicSubscription {
    async fn entries_event(_context: &DataloaderContext) -> GqlEntriesStream {
        // let entries_tx: tokio::sync::broadcast::Sender<CollectionEntryDataUpdate> =
        //     context.entries_broadcaster.clone();
        // let mut entries_tx = entries_tx.subscribe();
        // let stream = async_stream::stream! {
        //     loop {
        //       if let Ok(entry_event) = entries_tx.recv().await {
        //         yield Ok(entry_event)
        //       }
        //     }
        // };
        // Box::pin(stream)
        unimplemented!()
    }
}
