use juniper_codegen::graphql_object;

use crate::query::DataloaderContext;

pub struct Immutable;

#[graphql_object(
    context = DataloaderContext,
)]
impl Immutable {
    fn api_version() -> &'static str {
        "1.0"
    }
}
