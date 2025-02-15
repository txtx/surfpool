use juniper::RootNode;
use mutation::{DynamicMutation, Mutation};
use query::{DynamicQuery, Query, SchemaDatasource};
use std::sync::RwLock;
use std::{collections::BTreeMap, sync::Arc};
use subscription::{DynamicSubscription, Subscription};
use types::{collection::CollectionData, entry::EntryData};
use uuid::Uuid;

pub mod mutation;
pub mod query;
pub mod subscription;
pub mod types;

#[derive(Clone, Debug)]
pub struct Context {
    pub collections_store: Arc<RwLock<BTreeMap<Uuid, CollectionData>>>,
    pub entries_broadcaster: tokio::sync::broadcast::Sender<EntryData>,
}

impl Context {
    pub fn new() -> Context {
        let (entries_broadcaster, _) = tokio::sync::broadcast::channel(128);
        Context {
            collections_store: Arc::new(RwLock::new(BTreeMap::new())),
            entries_broadcaster,
        }
    }
}

impl juniper::Context for Context {}

pub type GqlStaticSchema = RootNode<'static, Query, Mutation, Subscription>;
pub type GqlDynamicSchema = RootNode<'static, DynamicQuery, Mutation, DynamicSubscription>;

pub fn new_static_schema() -> GqlStaticSchema {
    GqlStaticSchema::new(Query, Mutation, Subscription)
}

pub fn new_dynamic_schema(subgraph_index: SchemaDatasource) -> GqlDynamicSchema {
    GqlDynamicSchema::new_with_info(
        DynamicQuery::new(),
        Mutation,
        DynamicSubscription,
        subgraph_index,
        (),
        (),
    )
}
