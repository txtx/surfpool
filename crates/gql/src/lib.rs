use juniper::{DefaultScalarValue, RootNode};
use mutation::Mutation;
use query::{Query, SchemaDataSource};
use std::sync::RwLock;
use std::{collections::BTreeMap, sync::Arc};
use subscription::DynamicSubscription;
use types::{GqlSubgraphDataEntry, SubgraphDataEntryUpdate};
use uuid::Uuid;

pub mod mutation;
pub mod query;
pub mod subscription;
pub mod types;

#[derive(Clone, Debug)]
pub struct Context {
    /// A map of subgraph UUIDs to their names
    pub subgraph_name_lookup: Arc<RwLock<BTreeMap<Uuid, String>>>,
    /// A map of subgraph names to their entries
    pub entries_store: Arc<RwLock<BTreeMap<String, (Uuid, Vec<GqlSubgraphDataEntry>)>>>,
    // A broadcaster for entry updates
    // pub entries_broadcaster: tokio::sync::broadcast::Sender<SubgraphDataEntryUpdate>,
}

impl Context {
    pub fn new() -> Context {
        // let (entries_broadcaster, _) = tokio::sync::broadcast::channel(128);
        Context {
            subgraph_name_lookup: Arc::new(RwLock::new(BTreeMap::new())),
            entries_store: Arc::new(RwLock::new(BTreeMap::new())),
            // entries_broadcaster,
        }
    }
}

impl juniper::Context for Context {}

pub type GqlDynamicSchema =
    RootNode<'static, Query, Mutation, DynamicSubscription, DefaultScalarValue>;

pub fn new_dynamic_schema(subgraph_index: SchemaDataSource) -> GqlDynamicSchema {
    GqlDynamicSchema::new_with_info(
        Query::new(),
        Mutation,
        DynamicSubscription,
        subgraph_index,
        (),
        (),
    )
}
