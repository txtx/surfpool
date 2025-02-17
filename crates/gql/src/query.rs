use std::collections::HashMap;

use crate::Context;
use convert_case::{Case, Casing};
use juniper::{
    meta::{Field, MetaType},
    Arguments, BoxFuture, DefaultScalarValue, ExecutionResult, Executor, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct Query {}

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
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<String>,
}

impl SchemaDatasourceEntry {
    pub fn new(uuid: &Uuid, name: &str) -> Self {
        Self {
            name: name.to_case(Case::Pascal),
            subgraph_uuid: uuid.clone(),
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
    type Context = Context;
    type TypeInfo = SchemaDatasourceEntry;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <SchemaDatasourceEntry as GraphQLType>::name(info)
    }
}

impl Query {
    pub fn new() -> Self {
        Self {}
    }
}

impl GraphQLType<DefaultScalarValue> for Query {
    fn name(_spec: &SchemaDatasource) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(spec: &SchemaDatasource, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields = vec![];
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, entry) in spec.entries.iter() {
            fields.push(registry.field::<&[SchemaDatasourceEntry]>(name, &entry));
        }
        registry
            .build_object_type::<Query>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for Query {
    type Context = Context;
    type TypeInfo = SchemaDatasource;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Query as GraphQLType>::name(&info)
    }
}

impl GraphQLValueAsync<DefaultScalarValue> for Query {
    fn resolve_field_async(
        &self,
        info: &SchemaDatasource,
        field_name: &str,
        _arguments: &Arguments,
        executor: &Executor<Context>,
    ) -> BoxFuture<ExecutionResult> {
        let res = match field_name {
            "apiVersion" => executor.resolve_with_ctx(&(), "1.0"),
            subgraph_name => {
                let database = executor.context();
                let subgraph_db = database.entries_store.read().unwrap();
                let entry = info.entries.get(subgraph_name).unwrap();
                let (_, entries) = subgraph_db.get(subgraph_name).unwrap();
                executor.resolve_with_ctx(entry, &entries[..])
            }
        };
        Box::pin(async { res })
    }
}
