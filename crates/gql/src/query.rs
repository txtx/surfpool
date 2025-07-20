use std::{collections::HashMap, pin::Pin};

use convert_case::{Case, Casing};
use diesel::prelude::*;
use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry, ScalarValue, meta::MetaType,
};
use surfpool_db::{
    diesel::{
        self, associations::HasTable, r2d2::{ConnectionManager, Pool, PooledConnection}, sql_query, sql_types::{Bool, Integer, Text, Untyped}, Connection, ExpressionMethods, MultiConnection, QueryDsl, RunQueryDsl
    }, diesel_dynamic_schema::{
        dynamic_value::{DynamicRow, NamedField}, table, DynamicSelectClause
    }, schema::collections::dsl as collections_dsl, DynamicValue
};
use surfpool_types::subgraphs::SubgraphDataEntry;
use txtx_addon_kit::{
    hex, serde_json,
    types::types::{Type, Value},
};
use txtx_addon_network_svm_types::subgraph::{IndexedSubgraphSourceTypeName, SubgraphRequest};
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

pub trait Dataloader {
    fn fetch_entries_from_subgraph(
        &self,
        subgraph_name: &str,
        executor: Option<&Executor<DataloaderContext>>,
        schema: &DynamicSchemaSpec,
    ) -> Result<Vec<SubgraphSpec>, FieldError>;
    fn register_collection(
        &self,
        subgraph_uuid: &Uuid,
        schema: &SubgraphRequest,
    ) -> Result<(), String>;
    fn insert_entry_to_subgraph(
        &self,
        subgraph_uuid: &Uuid,
        entry: SubgraphSpec,
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
            subgraph_name => {
                let ctx = executor.context();
                if let Some(schema) = info.entries.get(subgraph_name) {
                    match ctx.pool.fetch_entries_from_subgraph(
                        subgraph_name,
                        Some(executor),
                        schema,
                    ) {
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

#[cfg(feature = "postgres")]
pub fn fetch_dynamic_entries_from_postres(
    pg_conn: &mut diesel::pg::PgConnection,
    schema: &DynamicSchemaSpec,
    dynamic_table_name: &str,
    executor: Option<&Executor<DataloaderContext>>,
) -> Result<Vec<DynamicRow<NamedField<DynamicValue>>>, FieldError> {
    let dynamic_table = table(dynamic_table_name); // This is a DynamicTable<Sqlite>

    let mut select = DynamicSelectClause::new();
    let cols = SubgraphDataEntry::default_columns_from_source_type(&schema.source_type);
    for col in cols {
        select.add_field(dynamic_table.column::<Untyped, _>(col));
    }

    // select.add_field(transaction_signature_col);
    for field in schema.fields.iter() {
        let col = field.data.display_name.to_case(Case::Snake);
        select.add_field(dynamic_table.column::<Untyped, _>(col));
    }

    // Build the query and apply filters immediately to avoid borrow checker issues
    let mut query = dynamic_table.clone().select(select).into_boxed();

    // Isolate filters
    let mut filters_specs = vec![];
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
    }

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

    let actual_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *pg_conn)
        .unwrap();
    Ok(actual_data)
}

#[cfg(feature = "sqlite")]
pub fn fetch_dynamic_entries_from_sqlite(
    sqlite_conn: &mut diesel::sqlite::SqliteConnection,
    schema: &DynamicSchemaSpec,
    dynamic_table_name: &str,
    executor: Option<&Executor<DataloaderContext>>,
) -> Result<Vec<DynamicRow<NamedField<DynamicValue>>>, FieldError> {
    let dynamic_table = table(dynamic_table_name); // This is a DynamicTable<Sqlite>

    let mut select = DynamicSelectClause::new();
    let cols = SubgraphDataEntry::default_columns_with_descriptions(&schema.source_type);
    for (col, _) in cols {
        select.add_field(dynamic_table.column::<Untyped, _>(col));
    }

    for field in schema.fields.iter() {
        let col = field.data.display_name.to_case(Case::Snake);
        select.add_field(dynamic_table.column::<Untyped, _>(col));
    }

    // Build the query and apply filters immediately to avoid borrow checker issues
    let mut query = dynamic_table.clone().select(select).into_boxed();

    // Isolate filters
    let mut filters_specs = vec![];
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
    }

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

    let actual_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *sqlite_conn)
        .unwrap();

    Ok(actual_data)
}

impl Dataloader for Pool<ConnectionManager<DatabaseConnection>> {
    fn fetch_entries_from_subgraph(
        &self,
        subgraph_name: &str,
        executor: Option<&Executor<DataloaderContext>>,
        schema: &DynamicSchemaSpec,
    ) -> Result<Vec<SubgraphSpec>, FieldError> {
        let mut conn = self.get().unwrap();
        // Use Diesel's query DSL to fetch the entries_table
        let entries_table: String = collections_dsl::collections
            .filter(collections_dsl::subgraph_id.eq(schema.subgraph_uuid.to_string()))
            .select(collections_dsl::entries_table)
            .first(&mut *conn)
            .map_err(|e| {
                FieldError::new(
                    format!("No subgraph found for name {}: {e}", subgraph_name),
                    juniper::Value::null(),
                )
            })?;

        let entries = match &mut *conn {
            #[cfg(feature = "sqlite")]
            DatabaseConnection::Sqlite(sqlite_conn) => {
                fetch_dynamic_entries_from_sqlite(sqlite_conn, schema, &entries_table, executor)
            }
            #[cfg(feature = "postgres")]
            DatabaseConnection::Postgresql(pg_conn) => {
                fetch_dynamic_entries_from_postres(pg_conn, schema, &entries_table, executor)
            }
        }?;

        let mut results = Vec::new();
        for row in entries {
            // We have a dynamic number of "default columns" for a row, depending on how the data is sourced
            let num_default_fields = SubgraphDataEntry::default_column_numbers(&schema.source_type);
            // We can extract those default fields from our row
            let default_values = (0..num_default_fields)
                .map(|i| row[i].value.0.clone())
                .collect::<Vec<_>>();

            let mut values = HashMap::new();
            for (i, field) in schema.fields.iter().enumerate() {
                let val = &row[num_default_fields + i].value;
                let value = match &field.data.expected_type {
                    Type::String => val
                        .0
                        .as_string()
                        .map(|s| Value::String(s.to_string()))
                        .unwrap_or(Value::String(String::new())),
                    Type::Integer => val
                        .0
                        .as_integer()
                        .map(Value::Integer)
                        .unwrap_or(Value::Integer(0)),
                    Type::Float => val
                        .0
                        .as_string()
                        .and_then(|s| s.parse().ok())
                        .map(Value::Float)
                        .unwrap_or(Value::Float(0.0)),
                    Type::Bool => val
                        .0
                        .as_string()
                        .map(|s| Value::Bool(s == "true"))
                        .unwrap_or(Value::Bool(false)),
                    _ => val
                        .0
                        .as_string()
                        .map(|s| Value::String(s.to_string()))
                        .unwrap_or(Value::String(String::new())),
                };
                values.insert(field.data.display_name.clone(), value);
            }
            let entry =
                SubgraphDataEntry::from_data_row(values, default_values, &schema.source_type);
            results.push(SubgraphSpec(entry));
        }
        Ok(results)
    }

    fn register_collection(
        &self,
        subgraph_uuid: &Uuid,
        request: &SubgraphRequest,
    ) -> Result<(), String> {
        let mut conn = self.get().unwrap();

        // 1. Ensure subgraphs table exists
        let cols = collections_dsl::collections::all_columns();
        sql_query(
            "CREATE TABLE IF NOT EXISTS collections (
                id TEXT PRIMARY KEY,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                subgraph_id TEXT NOT NULL,
                entries_table TEXT NOT NULL,
                schema TEXT NOT NULL
            )",
        )
        .execute(&mut *conn)
        .map_err(|e| format!("Failed to create collections table: {e}"))?;

        // 2. Create a new entries table for this subgraph, using the schema to determine the fields
        let uuid = subgraph_uuid;
        let entries_table = format!("entries_{}", uuid.simple().to_string());
        // Build the SQL for the entries table using the schema fields
        let data_source_type = IndexedSubgraphSourceTypeName::from(&request.data_source);
        let mut columns = SubgraphDataEntry::column_metadata(&data_source_type);
        for field in &request.fields {
            // Map field types to SQLite types (expand as needed)
            let sql_type = match &field.expected_type {
                Type::String => "TEXT",
                Type::Integer => "INTEGER",
                Type::Float => "REAL",
                Type::Bool => "BOOLEAN",
                _ => "TEXT", // fallback for unknown types
            };
            let col = format!(
                "{} {}",
                field.display_name.to_case(Case::Snake),
                sql_type
            );
            columns.push(col);
        }
        let create_entries_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n    {}\n)",
            entries_table,
            columns.join(",\n    ")
        );
        sql_query(&create_entries_sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to create entries table: {e}"))?;
        let schema_json = serde_json::to_string(request)
            .map_err(|e| format!("Failed to serialize schema: {e}"))?;
        let now = chrono::Utc::now().naive_utc();

        let sql = format!(
            "INSERT INTO collections (id, created_at, updated_at, subgraph_id, entries_table, schema) VALUES ('{}', '{}', '{}', '{}', '{}', X'{}')",
            uuid.to_string(),
            now,
            now,
            subgraph_uuid,
            &entries_table,
            hex::encode(schema_json.as_bytes())
        );
        
        sql_query(&sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to insert subgraph: {e}"))?;

        Ok(())
    }

    fn insert_entry_to_subgraph(
        &self,
        subgraph_uuid: &Uuid,
        entry: SubgraphSpec,
    ) -> Result<(), String> {
        let mut conn = self.get().unwrap();

        let (entries_table, request_src): (String, String) = collections_dsl::collections
            .filter(collections_dsl::id.eq(subgraph_uuid.to_string()))
            .select((collections_dsl::entries_table, collections_dsl::schema))
            .first(&mut *conn)
            .map_err(|e| format!("No subgraph found for uuid {}: {e}", subgraph_uuid))?;

        let request: SubgraphRequest = serde_json::from_str(&request_src)
            .map_err(|e| format!("Failed to parse schema: {e}"))?;

        // 2. Prepare the insert statement using the schema for column order
        let SubgraphSpec(data_entry) = entry;
        let mut columns = data_entry.default_columns();

        let (mut values, dynamic_values) = data_entry.values();

        // Use the schema to determine the order and names of dynamic fields
        // Insert null if the value is missing
        for field in &request.fields {
            let col = field.display_name.to_case(Case::Snake);
            columns.push(col.clone());
            if let Some(val) = dynamic_values.get(&field.display_name) {
                let val_str = match val {
                    Value::Bool(b) => b.to_string(),
                    Value::String(s) => format!("'{}'", s.replace("'", "''")),
                    Value::Integer(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Buffer(bytes) => format!("'{}'", hex::encode(bytes)),
                    Value::Addon(addon) => format!("'{}'", format!("{:?}", addon)),
                    Value::Null => "NULL".to_string(),
                    Value::Array(_arr) => unimplemented!(),
                    Value::Object(_obj) => unimplemented!(),
                };
                values.push(val_str);
            } else {
                values.push("NULL".to_string());
            }
        }

        // 3. Build and execute the insert
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            entries_table,
            columns.join(", "),
            values.join(", ")
        );
        sql_query(&sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to insert entry: {e}"))?;

        Ok(())
    }
}

fn juniper_scalar_to_value(scalar: &DefaultScalarValue) -> Value {
    match scalar.as_string() {
        Some(s) => Value::String(s.to_string()),
        None => panic!("Only string scalars are supported in filters for now"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use solana_sdk::signature::Signature;
    use surfpool_types::subgraphs::SubgraphDataEntryTableDefaults;
    use txtx_addon_kit::types::types::{Type, Value};
    use solana_pubkey::Pubkey;
    use txtx_addon_kit::types::{ConstructDid, Did};
    use txtx_addon_network_svm_types::{anchor::types::{Idl, IdlMetadata}, subgraph::{EventSubgraphSource, IndexedSubgraphField, IndexedSubgraphSourceType}};
    use uuid::Uuid;

    use super::*;
    use crate::types::schema::DynamicSchemaSpec;

    fn test_request() -> SubgraphRequest {
        let program_id = Pubkey::new_unique();
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
            types: vec![],
            constants: vec![],
            instructions: vec![],
            accounts: vec![],
            events: vec![],
            errors: vec![],
        };
        SubgraphRequest {
            program_id: program_id.clone(),
            block_height: 0,
            subgraph_name: "TestSubgraph".to_string(),
            subgraph_description: None,
            data_source: IndexedSubgraphSourceType::Event(EventSubgraphSource::new("TestEvent", &idl).unwrap()),
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

    fn test_entry(_schema: &DynamicSchemaSpec) -> SubgraphSpec {
        let mut values = HashMap::new();
        values.insert("test_field".to_string(), Value::String("hello".to_string()));
        SubgraphSpec(surfpool_types::subgraphs::SubgraphDataEntry::cpi_event(
            Uuid::new_v4(),
            values,
            42,
            Signature::from([1u8; 64]),
        ))
    }

    #[test]
    fn test_register_insert_fetch() {
        let store = SqlStore::new_in_memory();
        let request = test_request();
        let uuid = Uuid::now_v7();
        let schema = DynamicSchemaSpec::from_request(&uuid, &request);
        let name = schema.name.clone();
        // Register subgraph
        store
            .pool
            .register_collection(&uuid, &request)
            .expect("register_collection");
        // Insert entry
        let entry = test_entry(&schema);
        store
            .pool
            .insert_entry_to_subgraph(&uuid, entry.clone())
            .expect("insert_entry_to_subgraph");
        // Fetch entries
        let fetched = store
            .pool
            .fetch_entries_from_subgraph(&name, None, &schema)
            .expect("fetch_entries_from_subgraph");
        assert_eq!(fetched.len(), 1);
        // Check field value
        println!("Fetched entry: {:?}", fetched);
        let fetched_entry = &fetched[0].0;
        let SubgraphDataEntryTableDefaults::CpiEvent(ref default) = fetched_entry.table_defaults
        else {
            panic!("Unexpected subgraph data entry type");
        };
        assert_eq!(default.slot, 42);
        assert_eq!(
            fetched_entry.values.get("test_field"),
            Some(&Value::String("hello".to_string()))
        );
    }
}
