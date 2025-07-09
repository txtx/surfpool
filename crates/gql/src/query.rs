use std::{
    collections::{BTreeMap, HashMap},
    pin::Pin,
    sync::{Arc, RwLock},
};

use convert_case::{Case, Casing};
use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry, meta::MetaType,
};
use uuid::Uuid;

use crate::types::{
    SubgraphSpec, filters::SubgraphFilterSpec, scalars::bigint::BigInt, schema::DynamicSchemaSpec,
};

#[derive(Debug)]
pub struct DynamicQuery;

impl GraphQLType<DefaultScalarValue> for DynamicQuery {
    fn name(_spec: &SchemaDataSource) -> Option<&str> {
        Some("Query")
    }

    fn meta<'r>(spec: &SchemaDataSource, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        // BigInt needs to be registered as a primitive type before moving on to more complex types.
        let _ = registry.get_type::<&BigInt>(&());

        let mut fields = vec![];
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, schema_spec) in spec.entries.iter() {
            let filter = registry.arg::<Option<SubgraphFilterSpec>>("where", &schema_spec.filter);
            let field = registry
                .field::<&[DynamicSchemaSpec]>(name, schema_spec)
                .argument(filter);
            fields.push(field);
        }
        registry
            .build_object_type::<DynamicQuery>(spec, &fields)
            .into_meta()
    }
}

pub type MemoryStoreEntry = (Uuid, Vec<SubgraphSpec>);

#[derive(Debug, Clone)]
pub struct MemoryStore {
    /// A map of subgraph UUIDs to their names
    pub subgraph_name_lookup: Arc<RwLock<BTreeMap<Uuid, String>>>,
    /// A map of subgraph names to their entries
    pub entries_store: Arc<RwLock<BTreeMap<String, MemoryStoreEntry>>>,
    // A broadcaster for entry updates
    // pub entries_broadcaster: tokio::sync::broadcast::Sender<SubgraphDataEntryUpdate>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore {
            subgraph_name_lookup: Arc::new(RwLock::new(BTreeMap::new())),
            entries_store: Arc::new(RwLock::new(BTreeMap::new())),
            // entries_broadcaster,
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Dataloader for MemoryStore {
    fn fetch_entries_from_subgraph(
        &self,
        subgraph_name: &str,
        _executor: &Executor<DataloaderContext>,
        _schema: &DynamicSchemaSpec,
    ) -> Result<Vec<SubgraphSpec>, FieldError> {
        let subgraph_db = self
            .entries_store
            .read()
            .map_err(|err| FieldError::new(format!("error: {}", err), juniper::Value::null()))?;

        if let Some((_, entries)) = subgraph_db.get(subgraph_name) {
            Ok(entries.clone())
        } else {
            Err(FieldError::new(
                format!("subgraph {} not found", subgraph_name),
                juniper::Value::null(),
            ))
        }
    }

    fn register_subgraph(&self, subgraph_name: &str, subgraph_uuid: Uuid) -> Result<(), String> {
        let mut entries_store = self.entries_store.write().map_err(|err_ctx| {
            format!("{err_ctx}: Failed to acquire write lock on entries store")
        })?;
        let mut lookup = self.subgraph_name_lookup.write().map_err(|err_ctx| {
            format!("{err_ctx}: Failed to acquire write lock on subgraph name lookup")
        })?;
        lookup.insert(subgraph_uuid, subgraph_name.to_case(Case::Camel));
        entries_store.insert(subgraph_name.to_case(Case::Camel), (subgraph_uuid, vec![]));
        Ok(())
    }

    fn get_subgraph_name(&self, subgraph_uuid: &Uuid) -> Option<String> {
        let lookup = self
            .subgraph_name_lookup
            .write()
            .map_err(|_| "Failed to acquire write lock on subgraph name lookup".to_string())
            .ok()?;
        lookup.get(subgraph_uuid).map(|e| e.to_string())
    }

    fn insert_entry_to_subgraph(
        &self,
        subgraph_name: &str,
        entry: SubgraphSpec,
    ) -> Result<(), String> {
        let mut store = self
            .entries_store
            .write()
            .map_err(|_| "Failed to acquire write lock on subgraph name lookup".to_string())?;
        let (_, entries) = store.get_mut(subgraph_name).unwrap();
        entries.push(entry);
        Ok(())
    }
}

pub trait Dataloader {
    fn fetch_entries_from_subgraph(
        &self,
        subgraph_name: &str,
        executor: &Executor<DataloaderContext>,
        schema: &DynamicSchemaSpec,
    ) -> Result<Vec<SubgraphSpec>, FieldError>;
    fn register_subgraph(&self, subgraph_name: &str, subgraph_uuid: Uuid) -> Result<(), String>;
    fn get_subgraph_name(&self, subgraph_uuid: &Uuid) -> Option<String>;
    fn insert_entry_to_subgraph(
        &self,
        subgraph_name: &str,
        entry: SubgraphSpec,
    ) -> Result<(), String>;
}

pub type DataloaderContext = Box<dyn Dataloader + Sync + Send>;

impl juniper::Context for DataloaderContext {}

impl GraphQLValue<DefaultScalarValue> for DynamicQuery {
    type Context = DataloaderContext;
    type TypeInfo = SchemaDataSource;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicQuery as GraphQLType<DefaultScalarValue>>::name(info)
    }
}

impl GraphQLValueAsync<DefaultScalarValue> for DynamicQuery {
    fn resolve_field_async(
        &self,
        info: &SchemaDataSource,
        field_name: &str,
        _arguments: &Arguments,
        executor: &Executor<DataloaderContext>,
    ) -> Pin<
        Box<(dyn futures::Future<Output = Result<juniper::Value, FieldError>> + std::marker::Send)>,
    > {
        let res = match field_name {
            "apiVersion" => executor.resolve_with_ctx(&(), "1.0"),
            subgraph_name => {
                let database = executor.context();
                if let Some(schema) = info.entries.get(subgraph_name) {
                    match database.fetch_entries_from_subgraph(subgraph_name, executor, schema) {
                        Ok(entries) => executor.resolve_with_ctx(schema, &entries[..]),
                        Err(e) => Err(e),
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
    pub entries: HashMap<String, DynamicSchemaSpec>,
}

impl Default for SchemaDataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaDataSource {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: DynamicSchemaSpec) {
        self.entries.insert(entry.name.to_case(Case::Camel), entry);
    }
}
