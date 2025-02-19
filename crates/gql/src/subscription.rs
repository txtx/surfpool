use std::pin::Pin;

use crate::{query::SchemaDatasourceEntry, Context};
use futures::Stream;
use juniper::{
    graphql_subscription,
    meta::{Field, MetaType},
    Arguments, DefaultScalarValue, ExecutionResult, Executor, FieldError, GraphQLType,
    GraphQLValue, Registry,
};
use juniper_codegen::graphql_object;
use surfpool_types::Entry;
use txtx_core::kit::serde_json::Value;

#[derive(Debug, Clone)]
pub struct EntryData {
    pub entry: Entry,
    pub name: String,
}

impl EntryData {
    pub fn new(name: &String, entry: &Entry) -> Self {
        Self {
            entry: entry.clone(),
            name: name.clone(),
        }
    }
}

impl GraphQLType<DefaultScalarValue> for EntryData {
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
            .build_object_type::<[EntryData]>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for EntryData {
    type Context = Context;
    type TypeInfo = SchemaDatasourceEntry;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <EntryData as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        _info: &SchemaDatasourceEntry,
        field_name: &str,
        _args: &Arguments,
        executor: &Executor<Context>,
    ) -> ExecutionResult {
        match field_name {
            "uuid" => executor.resolve_with_ctx(&(), &self.entry.uuid.to_string()),
            field_name => {
                let value = self.entry.values.get(field_name).unwrap();
                executor.resolve_with_ctx(&(), &value.to_string())
            }
        }
    }
}

struct GqlValue(Value);
impl GraphQLValue<DefaultScalarValue> for GqlValue {
    type Context = Context;
    type TypeInfo = ();

    fn type_name<'i>(&self, _info: &'i Self::TypeInfo) -> Option<&'i str> {
        None
    }

    fn resolve_field(
        &self,
        _info: &Self::TypeInfo,
        _field_name: &str,
        _args: &Arguments,
        executor: &Executor<Context>,
    ) -> ExecutionResult {
        match &self.0 {
            Value::Null => todo!(),
            Value::Bool(_) => todo!(),
            Value::Number(number) => {
                if let Some(u) = number.as_u64() {
                    println!("u: {:?}", u);
                    executor.resolve_with_ctx(&(), &(u as i32))
                } else if let Some(i) = number.as_i64() {
                    executor.resolve_with_ctx(&(), &(i as i32))
                } else {
                    todo!()
                }
            }
            Value::String(s) => executor.resolve_with_ctx(&(), &s),
            Value::Array(values) => todo!(),
            Value::Object(map) => todo!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EntryUpdate {
    pub entry: Entry,
    pub name: String,
}

impl EntryUpdate {
    pub fn new(name: &String, entry: &Entry) -> Self {
        Self {
            entry: entry.clone(),
            name: name.clone(),
        }
    }
}

#[graphql_object(context = Context)]
impl EntryUpdate {
    pub fn uuid(&self) -> String {
        self.entry.uuid.to_string()
    }
}

pub struct Subscription;

type GqlEntriesStream = Pin<Box<dyn Stream<Item = Result<EntryUpdate, FieldError>> + Send>>;

#[graphql_subscription(
  context = Context,
)]
impl Subscription {
    async fn entries_event(context: &Context) -> GqlEntriesStream {
        let entries_tx = context.entries_broadcaster.clone();
        let mut entries_tx = entries_tx.subscribe();
        let stream = async_stream::stream! {
            loop {
              if let Ok(entry_event) = entries_tx.recv().await {
                yield Ok(entry_event)
              }
            }
        };
        Box::pin(stream)
    }
}

pub struct DynamicSubscription;

#[graphql_subscription(
  context = Context,

)]
impl DynamicSubscription {
    async fn entries_event(context: &Context) -> GqlEntriesStream {
        let entries_tx = context.entries_broadcaster.clone();
        let mut entries_tx = entries_tx.subscribe();
        let stream = async_stream::stream! {
            loop {
              if let Ok(entry_event) = entries_tx.recv().await {
                yield Ok(entry_event)
              }
            }
        };
        Box::pin(stream)
    }
}
