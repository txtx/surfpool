use juniper::{DefaultScalarValue, EmptyMutation, EmptySubscription, RootNode};
use query::{CollectionsMetadataLookup, DynamicQuery};

use crate::query::DataloaderContext;

pub mod mutation;
pub mod query;
pub mod subscription;
pub mod types;

pub use surfpool_db as db;

pub type DynamicSchema = RootNode<
    'static,
    DynamicQuery,
    EmptyMutation<DataloaderContext>,
    EmptySubscription<DataloaderContext>,
    DefaultScalarValue,
>;

pub fn new_dynamic_schema(collections_metadata_lookup: CollectionsMetadataLookup) -> DynamicSchema {
    let schema = DynamicSchema::new_with_info(
        DynamicQuery,
        EmptyMutation::<DataloaderContext>::new(),
        EmptySubscription::<DataloaderContext>::new(),
        collections_metadata_lookup,
        (),
        (),
    );
    schema.enable_introspection()
}
