use crate::{types::collection::CollectionData, Context};
use juniper_codegen::graphql_object;

pub struct Query;

#[graphql_object(
    context = Context,
)]
impl Query {
    fn api_version() -> &'static str {
        "1.0"
    }

    async fn collections(context: &Context) -> Vec<CollectionData> {
        let collections_store = context.collections_store.read().unwrap();
        collections_store
            .values().into_iter().map(|c| c.clone()).collect()
    }
}
