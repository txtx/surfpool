use juniper::{DefaultScalarValue, RootNode};
use mutation::Mutation;
use query::{Query, SchemaDataSource};
use subscription::DynamicSubscription;

pub mod mutation;
pub mod query;
pub mod subscription;
pub mod types;

pub type GqlDynamicSchema =
    RootNode<'static, Query, Mutation, DynamicSubscription, DefaultScalarValue>;

pub fn new_dynamic_schema(subgraph_index: SchemaDataSource) -> GqlDynamicSchema {
    let schema = GqlDynamicSchema::new_with_info(
        Query::new(),
        Mutation,
        DynamicSubscription,
        subgraph_index,
        (),
        (),
    );
    schema.enable_introspection()
}
