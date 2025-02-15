use crate::Context;
use juniper_codegen::graphql_object;

pub struct Mutation;

#[graphql_object(
    context = Context,
)]
impl Mutation {
    fn api_version() -> &'static str {
        "1.0"
    }
}

pub struct DynamicMutation;

#[graphql_object(
    context = Context,
)]
impl DynamicMutation {
    fn api_version() -> &'static str {
        "1.0"
    }
}
