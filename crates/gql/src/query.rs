use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use crate::{
    types::{collection::CollectionData, subgraph::Subgraph},
    Context,
};
use convert_case::{Case, Casing};
use juniper::{
    meta::{Field, MetaType, ObjectMeta},
    Arguments, DefaultScalarValue, ExecutionResult, Executor, GraphQLType, GraphQLTypeAsync,
    GraphQLValue, GraphQLValueAsync, Registry,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct DynamicQuery {
    pub subgraphs: HashMap<Uuid, Subgraph>,
}

#[derive(Clone, Debug)]
pub struct SchemaDatasource {
    pub entries: HashMap<String, SchemaDatasourceEntry>,
}

impl SchemaDatasource {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: SchemaDatasourceEntry) {
        self.entries.insert(entry.name.to_case(Case::Camel), entry);
    }
}

#[derive(Clone, Debug)]
pub struct SchemaDatasourceEntry {
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<String>,
}

impl SchemaDatasourceEntry {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_case(Case::Pascal),
            description: None,
            fields: vec![],
        }
    }
}

impl GraphQLType<DefaultScalarValue> for SchemaDatasourceEntry {
    fn name(spec: &SchemaDatasourceEntry) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &SchemaDatasourceEntry, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        fields.push(registry.field::<&String>("uuid", &()));
        for field in spec.fields.iter() {
            fields.push(registry.field::<&String>(field, &()));
        }
        registry
            .build_object_type::<SchemaDatasourceEntry>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SchemaDatasourceEntry {
    type Context = ();
    type TypeInfo = SchemaDatasourceEntry;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <SchemaDatasourceEntry as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        info: &SchemaDatasourceEntry,
        field_name: &str,
        args: &Arguments,
        executor: &Executor<()>,
    ) -> ExecutionResult {
        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        let database = executor.context();
        match field_name {
            // Because scalars are defined with another `Context` associated type, you must use
            // `resolve_with_ctx` here to make the `executor` perform automatic type conversion
            // of its argument.
            // You pass a vector of `User` objects to `executor.resolve`, and it will determine
            // which fields of the sub-objects to actually resolve based on the query.
            // The `executor` instance keeps track of its current position in the query.
            // "friends" => executor.resolve(info,
            //     &self.friend_ids.iter()
            //         .filter_map(|id| database.users.get(id))
            //         .collect::<Vec<_>>()
            // ),
            // We can only reach this panic in two cases: either a mismatch between the defined
            // schema in `meta()` above, or a validation failed because of a this library bug.
            //
            // In either of those two cases, the only reasonable way out is to panic the thread.
            _ => panic!("Field {field_name} not found on type User"),
        }
    }
}

impl DynamicQuery {
    pub fn new() -> Self {
        Self {
            subgraphs: HashMap::new(),
        }
    }
}

impl GraphQLType<DefaultScalarValue> for DynamicQuery {
    fn name(_spec: &SchemaDatasource) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(spec: &SchemaDatasource, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        // First, we need to define all fields and their types on this type.
        //
        // If we need arguments, want to implement interfaces, or want to add documentation
        // strings, we can do it here.
        let mut fields = vec![];
        fields.push(registry.field::<&String>("id", &()));
        fields.push(registry.field::<&String>("name", &()));
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, entry) in spec.entries.iter() {
            fields.push(registry.field::<&SchemaDatasourceEntry>(name, &entry));
        }
        registry
            .build_object_type::<DynamicQuery>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for DynamicQuery {
    type Context = Context;
    type TypeInfo = SchemaDatasource;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicQuery as GraphQLType>::name(&info)
    }

    fn resolve_field(
        &self,
        info: &SchemaDatasource,
        field_name: &str,
        args: &Arguments,
        executor: &Executor<Self::Context>,
    ) -> ExecutionResult {
        // Next, we need to match the queried field name. All arms of this match statement
        // return `ExecutionResult`, which makes it hard to statically verify that the type you
        // pass on to `executor.resolve*` actually matches the one that you defined in `meta()`
        // above.
        match field_name {
            // Because scalars are defined with another `Context` associated type, you must use
            // `resolve_with_ctx` here to make the `executor` perform automatic type conversion
            // of its argument.
            "api_version" => executor.resolve_with_ctx(&(), "1.0"),
            // subgraph => executor.resolve_with_ctx(&(), &self.name),
            // You pass a vector of `User` objects to `executor.resolve`, and it will determine
            // which fields of the sub-objects to actually resolve based on the query.
            // The `executor` instance keeps track of its current position in the query.
            // "friends" => executor.resolve(info,
            //     &self.friend_ids.iter()
            //         .filter_map(|id| database.users.get(id))
            //         .collect::<Vec<_>>()
            // ),
            // We can only reach this panic in two cases: either a mismatch between the defined
            // schema in `meta()` above, or a validation failed because of a this library bug.
            //
            // In either of those two cases, the only reasonable way out is to panic the thread.
            _ => panic!("Field {field_name} not found on type DynamicQuery"),
        }
    }
}

impl GraphQLValueAsync<DefaultScalarValue> for DynamicQuery {}

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
            .values()
            .into_iter()
            .map(|c| c.clone())
            .collect()
    }
}
