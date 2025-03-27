use juniper_codegen::graphql_object;

use crate::query::DataloaderContext;

pub struct Mutation;

#[graphql_object(
    context = DataloaderContext,
)]
impl Mutation {
    fn api_version() -> &'static str {
        "1.0"
    }
}

pub struct DynamicMutation;

#[graphql_object(
    context = DataloaderContext,
)]
impl DynamicMutation {
    fn api_version() -> &'static str {
        "1.0"
    }
}
