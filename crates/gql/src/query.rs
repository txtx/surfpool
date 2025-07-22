use std::{collections::HashMap, pin::Pin};

use convert_case::{Case, Casing};
use diesel::prelude::*;
use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry, meta::MetaType,
};
use surfpool_db::{
    diesel::{
        self, Connection, ExpressionMethods, MultiConnection, QueryDsl, RunQueryDsl,
        r2d2::{ConnectionManager, Pool, PooledConnection},
        sql_query,
        sql_types::{Bool, Integer, Text, Untyped},
    },
    diesel_dynamic_schema::{
        DynamicSelectClause,
        dynamic_value::{DynamicRow, NamedField},
        table,
    },
};
use txtx_addon_kit::{
    types::types::{Type, Value},
};
use uuid::Uuid;
use crate::types::sql::DynamicValue;
use crate::types::{
    CollectionEntry, CollectionEntryData, filters::SubgraphFilterSpec, scalars::hash::Hash,
    schema::CollectionMetadata,
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
        // let _ = registry.get_type::<&BigInt>(&());
        // BigInt needs to be registered as a primitive type before moving on to more complex types.
        let _ = registry.get_type::<&i32>(&());

        let mut fields = vec![];
        fields.push(registry.field::<&String>("apiVersion", &()));

        for (name, schema_spec) in spec.entries.iter() {
            let filter = registry.arg::<Option<SubgraphFilterSpec>>("where", &schema_spec.filter);
            let field = registry
                .field::<&[CollectionMetadata]>(name, schema_spec)
                .argument(filter);
            fields.push(field);
        }
        registry
            .build_object_type::<DynamicQuery>(spec, &fields)
            .into_meta()
    }
}

pub trait Dataloader {
    fn fetch_data_from_collection(
        &self,
        executor: Option<&Executor<DataloaderContext>>,
        metadata: &CollectionMetadata,
    ) -> Result<Vec<CollectionEntry>, FieldError>;
    fn register_collection(&self, metadata: &CollectionMetadata) -> Result<(), String>;
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
            field_name => {
                let ctx = executor.context();
                if let Some(schema) = info.entries.get(field_name) {
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
pub struct SchemaDataSource {
    pub entries: HashMap<String, CollectionMetadata>,
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

    pub fn add_entry(&mut self, entry: CollectionMetadata) {
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

pub fn extract_graphql_features<'a>(executor: Option<&'a Executor<DataloaderContext>>) -> (Vec<(&'a str, &'a str, &'a DefaultScalarValue)>, Vec<String>) {
    let mut filters_specs = vec![];
    let mut fetched_fields = vec![];

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

#[cfg(feature = "postgres")]
pub fn fetch_dynamic_entries_from_postres(
    pg_conn: &mut diesel::pg::PgConnection,
    metadata: &CollectionMetadata,
    executor: Option<&Executor<DataloaderContext>>,
) -> Result<(Vec<String>, Vec<DynamicRow<NamedField<DynamicValue>>>), FieldError> {

    let mut select = DynamicSelectClause::new();
    let dynamic_table = table(metadata.table_name.as_str());
    let (filters_specs, fetched_fields) = extract_graphql_features(executor);
    for field_name in fetched_fields.iter() {
        select.add_field(dynamic_table.column::<Untyped, _>(field_name.to_string()));
    }
    
    let mut query = dynamic_table.clone().select(select).into_boxed();

    for (field, predicate, value) in filters_specs {
        let value = juniper_scalar_to_value(value);
        match value {
            Value::String(s) => {
                let col = dynamic_table.column::<Text, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(s)),
                    "notEquals" => query.filter(col.ne(s)),
                    "contains" => query.filter(col.like(format!("%{}%", s))),
                    "notContains" => query.filter(col.not_like(format!("%{}%", s))),
                    "endsWith" => query.filter(col.like(format!("%{}", s))),
                    "startsWith" => query.filter(col.like(format!("{}%", s))),
                    _ => panic!("Unsupported string predicate: {}", predicate),
                };
            }
            Value::Integer(i) => {
                let i: i64 = i.try_into().unwrap();
                let col = dynamic_table.column::<Integer, _>(field);
                query = match predicate {
                    "equals" | "isEqual" => query.filter(col.eq(i as i32)),
                    "notEquals" => query.filter(col.ne(i as i32)),
                    "greaterThan" => query.filter(col.gt(i as i32)),
                    "greaterOrEqual" => query.filter(col.ge(i as i32)),
                    "lowerThan" => query.filter(col.lt(i as i32)),
                    "lowerOrEqual" => query.filter(col.le(i as i32)),
                    "between" => panic!("'between' requires a tuple/array value"),
                    _ => panic!("Unsupported integer predicate: {}", predicate),
                };
            }
            Value::Bool(_b) => {
                let col = dynamic_table.column::<Bool, _>(field);
                query = match predicate {
                    "true" => query.filter(col.eq(true)),
                    "false" => query.filter(col.eq(false)),
                    _ => panic!("Unsupported boolean predicate: {}", predicate),
                };
            }
            _ => panic!("Unsupported predicate or value type"),
        };
    }

    let fetched_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *pg_conn)
        .unwrap();

    Ok((fetched_fields, fetched_data))
}

#[cfg(feature = "sqlite")]
pub fn fetch_dynamic_entries_from_sqlite(
    sqlite_conn: &mut diesel::sqlite::SqliteConnection,
    metadata: &CollectionMetadata,
    executor: Option<&Executor<DataloaderContext>>,
) -> Result<(Vec<String>, Vec<DynamicRow<NamedField<DynamicValue>>>), FieldError> {

    // Isolate filters
    let mut select = DynamicSelectClause::new();
    let dynamic_table = table(metadata.table_name.as_str());
    let (filters_specs, fetched_fields) = extract_graphql_features(executor);
    for field_name in fetched_fields.iter() {
        select.add_field(dynamic_table.column::<Untyped, _>(field_name.to_string()));
    }

    // Build the query and apply filters immediately to avoid borrow checker issues
    let mut query = dynamic_table.clone().select(select).into_boxed();

    for (field, predicate, value) in filters_specs {
        let value = juniper_scalar_to_value(value);
        match value {
            Value::String(s) => {
                let col = dynamic_table.column::<Text, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(s)),
                    "notEquals" => query.filter(col.ne(s)),
                    "contains" => query.filter(col.like(format!("%{}%", s))),
                    "notContains" => query.filter(col.not_like(format!("%{}%", s))),
                    "endsWith" => query.filter(col.like(format!("%{}", s))),
                    "startsWith" => query.filter(col.like(format!("{}%", s))),
                    _ => panic!("Unsupported string predicate: {}", predicate),
                };
            }
            Value::Integer(i) => {
                let i: i64 = i.try_into().unwrap();
                let col = dynamic_table.column::<Integer, _>(field);
                query = match predicate {
                    "equals" | "isEqual" => query.filter(col.eq(i as i32)),
                    "notEquals" => query.filter(col.ne(i as i32)),
                    "greaterThan" => query.filter(col.gt(i as i32)),
                    "greaterOrEqual" => query.filter(col.ge(i as i32)),
                    "lowerThan" => query.filter(col.lt(i as i32)),
                    "lowerOrEqual" => query.filter(col.le(i as i32)),
                    "between" => panic!("'between' requires a tuple/array value"),
                    _ => panic!("Unsupported integer predicate: {}", predicate),
                };
            }
            Value::Bool(_b) => {
                let col = dynamic_table.column::<Bool, _>(field);
                query = match predicate {
                    "true" => query.filter(col.eq(true)),
                    "false" => query.filter(col.eq(false)),
                    _ => panic!("Unsupported boolean predicate: {}", predicate),
                };
            }
            _ => panic!("Unsupported predicate or value type"),
        };
    }

    let fetched_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *sqlite_conn)
        .unwrap();

    Ok((fetched_fields, fetched_data))
}

impl Dataloader for Pool<ConnectionManager<DatabaseConnection>> {
    fn fetch_data_from_collection(
        &self,
        executor: Option<&Executor<DataloaderContext>>,
        metadata: &CollectionMetadata,
    ) -> Result<Vec<CollectionEntry>, FieldError> {
        let mut conn = self.get().unwrap();
        // Use Diesel's query DSL to fetch the table_name

        let (fetch_fields, fetched_data) = match &mut *conn {
            #[cfg(feature = "sqlite")]
            DatabaseConnection::Sqlite(sqlite_conn) => {
                fetch_dynamic_entries_from_sqlite(sqlite_conn, metadata, executor)
            }
            #[cfg(feature = "postgres")]
            DatabaseConnection::Postgresql(pg_conn) => {
                fetch_dynamic_entries_from_postres(pg_conn, metadata, executor)
            }
        }?;

        let mut results = Vec::new();
        for row in fetched_data {
            let mut values = HashMap::new();
            for (i, field) in fetch_fields.iter().enumerate() {
                values.insert(field.clone(), row[i].value.0.clone()); // FIXME
            }
            results.push(CollectionEntry(CollectionEntryData {
                uuid: Uuid::new_v4(),
                slot: 0,
                transaction_signature: Hash(blake3::Hash::from_bytes([0u8; 32])),
                values,
            }));
        }
        Ok(results)
    }

    fn register_collection(&self, metadata: &CollectionMetadata) -> Result<(), String> {
        let mut conn = self.get().unwrap();

        // 1. Ensure subgraphs table exists
        sql_query(
            "CREATE TABLE IF NOT EXISTS collections (
                id TEXT PRIMARY KEY,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                table_name TEXT NOT NULL,
                schema TEXT NOT NULL
            )",
        )
        .execute(&mut *conn)
        .map_err(|e| format!("Failed to create collections table: {e}"))?;

        // 2. Create a new entries table for this subgraph, using the schema to determine the fields
        // Build the SQL for the entries table using the schema fields
        let mut columns = vec![
            "id TEXT PRIMARY KEY".to_string(),
            "slot INTEGER".to_string(),
            "transactionSignature TEXT".to_string(),
        ];
        for field in &metadata.fields {
            // Map field types to SQLite types (expand as needed)
            let sql_type = match &field.data.expected_type {
                Type::String => "TEXT",
                Type::Integer => "INTEGER",
                Type::Float => "REAL",
                Type::Bool => "BOOLEAN",
                _ => "TEXT", // fallback for unknown types
            };
            let col = format!(
                "{} {}",
                field.data.display_name.to_case(Case::Snake),
                sql_type
            );
            columns.push(col);
        }
        let create_entries_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n    {}\n)",
            metadata.table_name,
            columns.join(",\n    ")
        );

        sql_query(&create_entries_sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to create entries table: {e}"))?;
        // let schema_json = serde_json::to_string(request)
        //     .map_err(|e| format!("Failed to serialize schema: {e}"))?;
        let schema_json = String::new();
        let now = chrono::Utc::now().naive_utc();

        let sql = format!(
            "INSERT INTO collections (id, created_at, updated_at, table_name, schema) VALUES ('{}', '{}', '{}', '{}', '{}')",
            metadata.id, now, now, &metadata.table_name, schema_json
        );

        sql_query(&sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to insert subgraph: {e}"))?;

        Ok(())
    }

    fn insert_entries_into_collection(
        &self,
        entries: Vec<CollectionEntryData>,
        metadata: &CollectionMetadata,
    ) -> Result<(), String> {
        let mut conn = self.get().unwrap();

        // 2. Prepare the insert statement using the schema for column order
        // let CollectionEntry(data_entry) = entry;

        let mut columns = vec![];
        let mut values: Vec<String> = vec![];

        for entry in entries {

            columns.push("id".to_string());
            columns.push("slot".to_string());
            columns.push("transactionSignature".to_string());
            values.push(format!("'{}'", Uuid::new_v4().to_string()));
            values.push(entry.slot.to_string());
            values.push(format!("'{}'", entry.transaction_signature.to_string()));

            // Use the schema to determine the order and names of dynamic fields
            // Insert null if the value is missing
            for field in &metadata.fields {
                let col = field.data.display_name.to_case(Case::Snake);
                columns.push(col.clone());
                if let Some(val) = entry.values.get(&field.data.display_name) {
                    let val_str = match val {
                        juniper::Value::Scalar(DefaultScalarValue::String(value)) => format!("'{}'", value),
                        juniper::Value::Scalar(value) => value.to_string(),
                        _ => unimplemented!(),
                    };
                    values.push(val_str);
                } else {
                    values.push("NULL".to_string());
                }
            }

            // 3. Build and execute the insert
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                metadata.table_name,
                columns.join(", "),
                values.join(", ")
            );

            sql_query(&sql)
                .execute(&mut *conn)
                .map_err(|e| format!("Failed to insert entry: {e}"))?;
        }
        Ok(())
    }
}

fn juniper_scalar_to_value(scalar: &DefaultScalarValue) -> Value {
    match scalar {
        DefaultScalarValue::String(s) => Value::String(s.to_string()),
        DefaultScalarValue::Int(i) => Value::Integer(i128::from(*i)),
        DefaultScalarValue::Boolean(b) => Value::bool(*b),
        _ => panic!("Only string, int and boolean are supported in filters for now"),
    }
}

#[cfg(test)]
mod tests {
    use solana_pubkey::Pubkey;
    use solana_sdk::signature::Signature;
    use surfpool_types::CollectionEntriesPack;
    use txtx_addon_kit::types::{ConstructDid, Did, types::Type};
    use txtx_addon_network_svm_types::{
        anchor::types::{Idl, IdlEvent, IdlMetadata, IdlSerialization, IdlTypeDef, IdlTypeDefTy},
        subgraph::{
            EventSubgraphSource, IndexedSubgraphField, IndexedSubgraphSourceType, SubgraphRequest,
        },
    };
    use uuid::Uuid;

    use super::*;
    use crate::types::schema::CollectionMetadata;

    fn test_request() -> SubgraphRequest {
        let program_id = Pubkey::new_unique();
        let event_name = "TestEvent".to_string();
        let idl: Idl = Idl {
            address: program_id.clone().to_string(),
            metadata: IdlMetadata {
                name: "TestProgram".to_string(),
                version: "1.0.0".to_string(),
                spec: "1.0.0".to_string(),
                description: None,
                repository: None,
                deployments: None,
                dependencies: vec![],
                contact: None,
            },
            docs: vec![],
            types: vec![IdlTypeDef {
                name: event_name.clone(),
                docs: vec![],
                serialization: IdlSerialization::Borsh, 
                repr: None,
                generics: vec![],
                ty: IdlTypeDefTy::Enum { variants: vec![] }

            }],
            constants: vec![],
            instructions: vec![],
            accounts: vec![],
            events: vec![IdlEvent {
                name: event_name.clone(),
                discriminator: vec![],
            }],
            errors: vec![],
        };
        SubgraphRequest {
            program_id: program_id.clone(),
            block_height: 0,
            subgraph_name: "TestSubgraph".to_string(),
            subgraph_description: None,
            data_source: IndexedSubgraphSourceType::Event(
                EventSubgraphSource::new(&event_name, &idl).unwrap(),
            ),
            construct_did: ConstructDid(Did::zero()),
            network: "localnet".into(),
            fields: vec![IndexedSubgraphField {
                display_name: "test_field".to_string(),
                source_key: "test_field".to_string(),
                expected_type: Type::String,
                description: None,
            }],
        }
    }

    fn test_entry(_schema: &CollectionMetadata) -> CollectionEntriesPack {
        let mut values = HashMap::new();
        values.insert("test_field".to_string(), Value::String("hello".to_string()));
        let bytes = serde_json::to_vec(&vec![values]).unwrap();
        CollectionEntriesPack::cpi_event(bytes, 42, Signature::from([1u8; 64]))
    }

    #[test]
    fn test_register_insert_fetch() {
        // Prepare dataset
        let store = SqlStore::new_in_memory();
        let request = test_request();
        let uuid = Uuid::new_v4();
        let metadata = CollectionMetadata::from_request(&uuid, &request);
        
        // Register subgraph
        store
            .pool
            .register_collection(&metadata)
            .expect("register_collection");

        // Insert entry
        let entries_pack = test_entry(&metadata);
        let entries = CollectionEntryData::from_entries_pack(&uuid, entries_pack).unwrap();

        store
            .pool
            .insert_entries_into_collection(entries, &metadata)
            .expect("insert_entry_to_subgraph");

        // Fetch entries
        let fetched = store
            .pool
            .fetch_data_from_collection(None, &metadata)
            .expect("fetch_entries_from_subgraph");
        assert_eq!(fetched.len(), 1);

        // Check field value
        println!("Fetched entry: {:?}", fetched);
        let fetched_entry = &fetched[0].0;
        // let CollectionEntryDataTableDefaults::CpiEvent(ref default) = fetched_entry.table_defaults
        // else {
        //     panic!("Unexpected subgraph data entry type");
        // };
        // assert_eq!(default.slot, 42);
        // assert_eq!(
        //     fetched_entry.values.get("test_field"),
        //     Some(&juniper::Value::scalar("hello"))
        // );
    }
}
