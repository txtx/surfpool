use std::{collections::HashMap, pin::Pin};

use convert_case::{Case, Casing};
use diesel::prelude::*;
use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry, meta::MetaType,
};
use surfpool_db::diesel::{
    self, Connection, MultiConnection,
    r2d2::{ConnectionManager, Pool, PooledConnection},
};
use txtx_addon_network_svm_types::subgraph::SubgraphRequest;

use crate::types::{
    CollectionEntry, CollectionEntryData, collections::CollectionMetadata,
    filters::SubgraphFilterSpec,
};

#[derive(Debug)]
pub struct DynamicQuery;

impl GraphQLType<DefaultScalarValue> for DynamicQuery {
    fn name(_spec: &CollectionsMetadataLookup) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(
        collections_metadata_lookup: &CollectionsMetadataLookup,
        registry: &mut Registry<'r>,
    ) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        // BigInt needs to be registered as a primitive type before moving on to more complex types.
        // let _ = registry.get_type::<&BigInt>(&());
        // BigInt needs to be registered as a primitive type before moving on to more complex types.
        let _ = registry.get_type::<&i32>(&());

        let mut fields = vec![];
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, metadata) in collections_metadata_lookup.entries.iter() {
            let filter = registry.arg::<Option<SubgraphFilterSpec>>("where", &metadata.filters);
            let field = registry
                .field::<&[CollectionMetadata]>(name, metadata)
                .argument(filter);
            fields.push(field);
        }
        registry
            .build_object_type::<DynamicQuery>(collections_metadata_lookup, &fields)
            .into_meta()
    }
}

pub trait Dataloader {
    fn fetch_data_from_collection(
        &self,
        executor: Option<&Executor<DataloaderContext>>,
        metadata: &CollectionMetadata,
    ) -> Result<Vec<CollectionEntry>, FieldError>;
    fn register_collection(
        &self,
        metadata: &CollectionMetadata,
        request: &SubgraphRequest,
    ) -> Result<(), String>;
    fn insert_entries_into_collection(
        &self,
        entries: Vec<CollectionEntryData>,
        metadata: &CollectionMetadata,
    ) -> Result<(), String>;
}

pub struct DataloaderContext {
    pub pool: Pool<ConnectionManager<DatabaseConnection>>,
}

impl juniper::Context for DataloaderContext {}

impl GraphQLValue<DefaultScalarValue> for DynamicQuery {
    type Context = DataloaderContext;
    type TypeInfo = CollectionsMetadataLookup;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicQuery as GraphQLType<DefaultScalarValue>>::name(info)
    }
}

impl GraphQLValueAsync<DefaultScalarValue> for DynamicQuery {
    fn resolve_field_async(
        &self,
        collections_metadata_lookup: &CollectionsMetadataLookup,
        field_name: &str,
        _arguments: &Arguments,
        executor: &Executor<DataloaderContext>,
    ) -> Pin<
        Box<(dyn futures::Future<Output = Result<juniper::Value, FieldError>> + std::marker::Send)>,
    > {
        let res = match field_name {
            "apiVersion" => executor.resolve_with_ctx(&(), "1.0"),
            name => {
                let ctx = executor.context();
                if let Some(schema) = collections_metadata_lookup.entries.get(name) {
                    match ctx.pool.fetch_data_from_collection(Some(executor), schema) {
                        Ok(entries) => executor.resolve_with_ctx(schema, &entries[..]),
                        Err(e) => Err(e),
                    }
                } else {
                    Err(FieldError::new(
                        format!("field {} not found", field_name),
                        juniper::Value::null(),
                    ))
                }
            }
        };
        Box::pin(async move { res })
    }
}

#[derive(Clone, Debug)]
pub struct WorkspacesCache {
    pub entries: HashMap<String, CollectionsMetadataLookup>,
}

impl Default for WorkspacesCache {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspacesCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_workspace(&mut self, workspace_slug: &str, entry: CollectionsMetadataLookup) {
        self.entries.insert(workspace_slug.to_string(), entry);
    }
}

#[derive(Clone, Debug)]
pub struct CollectionsMetadataLookup {
    pub entries: HashMap<String, CollectionMetadata>,
}

impl Default for CollectionsMetadataLookup {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectionsMetadataLookup {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_collection(&mut self, entry: CollectionMetadata) {
        self.entries.insert(entry.name.to_case(Case::Camel), entry);
    }
}

#[derive(MultiConnection)]
pub enum DatabaseConnection {
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection),
    #[cfg(feature = "postgres")]
    Postgresql(PgConnection),
}

pub struct SqlStore {
    pub connection_url: String,
    pub pool: Pool<ConnectionManager<DatabaseConnection>>,
}

impl SqlStore {
    pub fn new_in_memory() -> SqlStore {
        Self::new(":memory:")
    }

    pub fn new(connection_url: &str) -> SqlStore {
        let manager = ConnectionManager::<DatabaseConnection>::new(connection_url);
        let pool = Pool::new(manager).expect("unable to create connection pool");
        SqlStore {
            connection_url: connection_url.to_string(),
            pool,
        }
    }

    pub fn get_conn(&self) -> PooledConnection<ConnectionManager<DatabaseConnection>> {
        self.pool.get().unwrap()
    }
}

pub fn extract_graphql_features<'a>(
    executor: Option<&'a Executor<DataloaderContext>>,
) -> (Vec<(&'a str, &'a str, &'a DefaultScalarValue)>, Vec<String>) {
    let mut filters_specs = vec![];
    let mut fetched_fields = vec!["id".to_string()];

    if let Some(executor) = executor {
        for arg in executor.look_ahead().arguments() {
            if arg.name().eq("where") {
                match arg.value() {
                    juniper::LookAheadValue::Object(obj) => {
                        for (attribute, value) in obj.iter() {
                            match value.item {
                                juniper::LookAheadValue::Object(obj) => {
                                    for (predicate, predicate_value) in obj.iter() {
                                        match predicate_value.item {
                                            juniper::LookAheadValue::Scalar(value) => {
                                                filters_specs.push((
                                                    attribute.item,
                                                    predicate.item,
                                                    value,
                                                ));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }

        for child in executor.look_ahead().children().iter() {
            let field_name = child.field_name();
            fetched_fields.push(field_name.to_string());
        }
    }

    (filters_specs, fetched_fields)
}
