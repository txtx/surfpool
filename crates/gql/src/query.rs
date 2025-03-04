use std::{collections::HashMap, pin::Pin};

use crate::{types::schema::DynamicSchemaMetadata, Context};

use convert_case::{Case, Casing};
use juniper::{
    meta::MetaType, Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry,
};

#[derive(Debug)]
pub struct Query {}

impl Query {
    pub fn new() -> Self {
        Self {}
    }
}

impl GraphQLType<DefaultScalarValue> for Query {
    fn name(_spec: &SchemaDataSource) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(spec: &SchemaDataSource, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields = vec![];
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, entry) in spec.entries.iter() {
            fields.push(registry.field::<&[DynamicSchemaMetadata]>(name, &entry));
        }
        registry
            .build_object_type::<Query>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for Query {
    type Context = Context;
    type TypeInfo = SchemaDataSource;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Query as GraphQLType<DefaultScalarValue>>::name(&info)
    }
}

impl GraphQLValueAsync<DefaultScalarValue> for Query {
    fn resolve_field_async(
        &self,
        info: &SchemaDataSource,
        field_name: &str,
        _arguments: &Arguments,
        executor: &Executor<Context>,
    ) -> Pin<
        Box<(dyn futures::Future<Output = Result<juniper::Value, FieldError>> + std::marker::Send)>,
    > {
        let res = match field_name {
            "apiVersion" => executor.resolve_with_ctx(&(), "1.0"),
            subgraph_name => {
                let database = executor.context();
                let subgraph_db = database
                    .entries_store
                    .read()
                    .expect("failed to read from gql entries store");
                if let Some(entry) = info.entries.get(subgraph_name) {
                    if let Some((_, entries)) = subgraph_db.get(subgraph_name) {
                        executor.resolve_with_ctx(entry, &entries[..])
                    } else {
                        Err(FieldError::new(
                            format!("subgraph {} not found", subgraph_name),
                            juniper::Value::null(),
                        ))
                    }
                } else {
                    Err(FieldError::new(
                        format!("subgraph {} not found", subgraph_name),
                        juniper::Value::null(),
                    ))
                }
            }
        };
        Box::pin(async move { res })
    }
}

#[derive(Clone, Debug)]
pub struct SchemaDataSource {
    pub entries: HashMap<String, DynamicSchemaMetadata>,
}

impl SchemaDataSource {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: DynamicSchemaMetadata) {
        self.entries.insert(entry.name.to_case(Case::Camel), entry);
    }
}
