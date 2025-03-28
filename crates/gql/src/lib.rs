use juniper::{DefaultScalarValue, RootNode};
use mutation::Immutable;
use query::{DynamicQuery, SchemaDataSource};
use subscription::DynamicSubscription;

pub mod mutation;
pub mod query;
pub mod subscription;
pub mod types;

pub type DynamicSchema =
    RootNode<'static, DynamicQuery, Immutable, DynamicSubscription, DefaultScalarValue>;

pub fn new_dynamic_schema(subgraph_spec: SchemaDataSource) -> DynamicSchema {
    let schema = DynamicSchema::new_with_info(
        DynamicQuery,
        Immutable,
        DynamicSubscription,
        subgraph_spec,
        (),
        (),
    );
    schema.enable_introspection()
}
