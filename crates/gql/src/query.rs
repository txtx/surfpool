use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex},
};

use convert_case::{Case, Casing};
use diesel::{expression::BoxableExpression, prelude::*};
use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue,
    GraphQLValueAsync, Registry, ScalarValue, meta::MetaType,
};
use surfpool_db::{
    DynamicValue,
    diesel::{
        self, Connection, ExpressionMethods, QueryDsl, RunQueryDsl, sql_query,
        sql_types::{Bool, Integer, Text, Untyped},
    },
    diesel_dynamic_schema::{
        DynamicSelectClause, Table,
        dynamic_value::{DynamicRow, NamedField},
        table,
    },
    schema::collections::dsl as collections_dsl,
};
use surfpool_types::SubgraphDataEntry;
use txtx_addon_kit::{
    hex, serde_json,
    types::types::{Type, Value},
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
        subgraph_name: &str,
        schema: &DynamicSchemaSpec,
    ) -> Result<(), String>;
    fn insert_entry_to_subgraph(
        &self,
        subgraph_uuid: &Uuid,
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
                    match database.fetch_entries_from_subgraph(
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

#[derive(Debug, Clone)]
pub enum DatabaseConfiguration {
    Sqlite(String),
    Postgres(String),
}

#[derive(Clone)]
pub struct SqlStore {
    pub db_conf: DatabaseConfiguration,
    pub conn: Arc<Mutex<surfpool_db::diesel::sqlite::SqliteConnection>>,
}

impl SqlStore {
    pub fn new_in_memory() -> SqlStore {
        let conn = surfpool_db::diesel::sqlite::SqliteConnection::establish(":memory:")
            .expect("Failed to create in-memory sqlite connection");
        SqlStore {
            db_conf: DatabaseConfiguration::Sqlite("surfpool.db".to_string()),
            conn: Arc::new(Mutex::new(conn)),
        }
    }
}

impl Dataloader for SqlStore {
    fn fetch_entries_from_subgraph(
        &self,
        subgraph_name: &str,
        executor: Option<&Executor<DataloaderContext>>,
        schema: &DynamicSchemaSpec,
    ) -> Result<Vec<SubgraphSpec>, FieldError> {
        let mut conn = self.conn.lock().unwrap();
        // Use Diesel's query DSL to fetch the entries_table

        let entries_table: String = collections_dsl::collections
            .filter(collections_dsl::name.eq(subgraph_name))
            .select(collections_dsl::entries_table)
            .first(&mut *conn)
            .map_err(|e| {
                FieldError::new(
                    format!("No subgraph found for name {}: {e}", subgraph_name),
                    juniper::Value::null(),
                )
            })?;

        // Build dynamic select clause
        let subgraph = table(entries_table.clone()); // This is a DynamicTable<Sqlite>
        let uuid_col = subgraph.column::<Untyped, _>("uuid");
        let slot_col = subgraph.column::<Untyped, _>("slot");
        // let transaction_signature_col = users.column::<Untyped, _>("transaction_signature");

        let mut select = DynamicSelectClause::new();
        select.add_field(uuid_col);
        select.add_field(slot_col);

        // select.add_field(transaction_signature_col);
        for field in schema.fields.iter() {
            let col = field.data.display_name.to_case(Case::Snake);
            select.add_field(subgraph.column::<Untyped, _>(col));
        }

        // Build the query and apply filters immediately to avoid borrow checker issues
        let mut query = subgraph.clone().select(select).into_boxed();

        if let Some(executor) = executor {
            // Isolate filters
            let mut filters_specs = vec![];
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

            for (field, predicate, value) in &filters_specs {
                let value = juniper_scalar_to_value(value);
                let pred = build_predicate(&subgraph, field, predicate, value);
                query = query.filter(pred);
            }
        }

        let actual_data: Vec<DynamicRow<NamedField<DynamicValue>>> =
            query.load(&mut *conn).map_err(|e| {
                FieldError::new(
                    format!("Failed to query entries: {e}"),
                    juniper::Value::null(),
                )
            })?;

        let mut results = Vec::new();
        for row in actual_data {
            let uuid = Uuid::parse_str(row[0].value.0.expect_string()).unwrap_or(Uuid::nil());
            let slot = row[1].value.0.expect_integer().try_into().unwrap_or(0);
            // let transaction_signature = row[2].value.as_str().and_then(|s| s.parse().ok()).unwrap_or_else(|| blake3::Hash::from([0u8; 32]));
            let mut values = HashMap::new();
            for (i, field) in schema.fields.iter().enumerate() {
                let val = &row[2 + i].value;
                let value = match &field.data.expected_type {
                    Type::String => val
                        .0
                        .as_string()
                        .map(|s| Value::String(s.to_string()))
                        .unwrap_or(Value::String(String::new())),
                    Type::Integer => val
                        .0
                        .as_string()
                        .and_then(|s| s.parse().ok())
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
            let entry = SubgraphDataEntry {
                uuid,
                values,
                slot,
                transaction_signature: blake3::Hash::from([0u8; 32]),
            };
            results.push(SubgraphSpec(entry));
        }
        Ok(results)
    }

    fn register_collection(
        &self,
        subgraph_uuid: &Uuid,
        subgraph_name: &str,
        schema: &DynamicSchemaSpec,
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();

        // 1. Ensure subgraphs table exists
        sql_query(
            "CREATE TABLE IF NOT EXISTS collections (
                id TEXT PRIMARY KEY,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                name TEXT NOT NULL,
                entries_table TEXT NOT NULL,
                schema TEXT NOT NULL
            )",
        )
        .execute(&mut *conn)
        .map_err(|e| format!("Failed to create collections table: {e}"))?;

        // 2. Create a new entries table for this subgraph, using the schema to determine the fields
        let uuid = subgraph_uuid;
        let entries_table = format!("subgraph_entries_{}", uuid.simple().to_string());
        // Build the SQL for the entries table using the schema fields
        let mut columns = vec![
            "id INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
            "uuid TEXT".to_string(),
            "slot INTEGER".to_string(),
            "transaction_signature TEXT".to_string(),
        ];
        for field in &schema.fields {
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
            entries_table,
            columns.join(",\n    ")
        );
        sql_query(&create_entries_sql)
            .execute(&mut *conn)
            .map_err(|e| format!("Failed to create entries table: {e}"))?;
        let schema_json = serde_json::to_string(schema)
            .map_err(|e| format!("Failed to serialize schema: {e}"))?;
        let now = chrono::Utc::now().naive_utc();

        sql_query(
            "INSERT INTO collections (id, created_at, updated_at, name, entries_table, schema) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind::<diesel::sql_types::Text, _>(uuid.to_string())
        .bind::<diesel::sql_types::Timestamp, _>(now)
        .bind::<diesel::sql_types::Timestamp, _>(now)
        .bind::<diesel::sql_types::Text, _>(subgraph_name)
        .bind::<diesel::sql_types::Text, _>(&entries_table)
        .bind::<diesel::sql_types::Text, _>(schema_json)
        .execute(&mut *conn)
        .map_err(|e| format!("Failed to insert subgraph: {e}"))?;

        Ok(())
    }

    fn insert_entry_to_subgraph(
        &self,
        subgraph_uuid: &Uuid,
        entry: SubgraphSpec,
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();

        let (entries_table, schema_json): (String, String) = collections_dsl::collections
            .filter(collections_dsl::id.eq(subgraph_uuid.to_string()))
            .select((collections_dsl::entries_table, collections_dsl::schema))
            .first(&mut *conn)
            .map_err(|e| format!("No subgraph found for uuid {}: {e}", subgraph_uuid))?;

        let schema: DynamicSchemaSpec = serde_json::from_str(&schema_json)
            .map_err(|e| format!("Failed to parse schema: {e}"))?;

        // 2. Prepare the insert statement using the schema for column order
        let SubgraphSpec(data_entry) = entry;
        let mut columns = vec![
            "uuid".to_string(),
            "slot".to_string(),
            "transaction_signature".to_string(),
        ];
        let mut values: Vec<String> = vec![
            format!("'{}'", data_entry.uuid),
            format!("{}", data_entry.slot),
            format!("'{}'", data_entry.transaction_signature),
        ];

        // Use the schema to determine the order and names of dynamic fields
        // Insert null if the value is missing

        for field in &schema.fields {
            let col = field.data.display_name.to_case(Case::Snake);
            columns.push(col.clone());
            if let Some(val) = data_entry.values.get(&field.data.display_name) {
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

fn build_predicate<'a>(
    table: &'a Table<String, String>,
    field: &'a str,
    predicate: &str,
    value: Value,
) -> Box<
    dyn BoxableExpression<
            Table<String, String>,
            diesel::sqlite::Sqlite,
            SqlType = diesel::sql_types::Bool,
        > + 'a,
> {
    // Try to infer column type from value
    match value {
        Value::String(s) => {
            let col = table.column::<Text, _>(field);
            match predicate {
                "equals" => Box::new(col.eq(s)),
                "notEquals" => Box::new(col.ne(s)),
                "contains" => Box::new(col.like(format!("%{}%", s))),
                "notContains" => Box::new(col.not_like(format!("%{}%", s))),
                "endsWith" => Box::new(col.like(format!("%{}", s))),
                "startsWith" => Box::new(col.like(format!("{}%", s))),
                _ => panic!("Unsupported string predicate: {}", predicate),
            }
        }
        Value::Integer(i) => {
            let i: i64 = i.try_into().unwrap();
            let col = table.column::<Integer, _>(field);
            match predicate {
                "equals" | "isEqual" => Box::new(col.eq(i as i32)),
                "notEquals" => Box::new(col.ne(i as i32)),
                "greaterThan" => Box::new(col.gt(i as i32)),
                "greaterOrEqual" => Box::new(col.ge(i as i32)),
                "lowerThan" => Box::new(col.lt(i as i32)),
                "lowerOrEqual" => Box::new(col.le(i as i32)),
                "between" => panic!("'between' requires a tuple/array value"),
                _ => panic!("Unsupported integer predicate: {}", predicate),
            }
        }
        Value::Bool(b) => {
            let col = table.column::<Bool, _>(field);
            match predicate {
                "true" => Box::new(col.eq(true)),
                "false" => Box::new(col.eq(false)),
                _ => panic!("Unsupported boolean predicate: {}", predicate),
            }
        }
        _ => panic!("Unsupported predicate or value type"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use blake3::Hash as Blake3Hash;
    use txtx_addon_kit::types::types::{Type, Value};
    use uuid::Uuid;

    use super::*;
    use crate::types::schema::{DynamicSchemaSpec, FieldMetadata};

    fn test_schema() -> DynamicSchemaSpec {
        DynamicSchemaSpec {
            name: "TestSubgraph".to_string(),
            filter: SubgraphFilterSpec {
                name: "TestSubgraphFilter".to_string(),
                fields: vec![],
            },
            subgraph_uuid: Uuid::new_v4(),
            description: None,
            fields: vec![FieldMetadata {
                data: txtx_addon_network_svm_types::subgraph::IndexedSubgraphField {
                    display_name: "test_field".to_string(),
                    source_key: "test_field".to_string(),
                    expected_type: Type::String,
                    description: None,
                },
            }],
        }
    }

    fn test_entry(_schema: &DynamicSchemaSpec) -> SubgraphSpec {
        let mut values = HashMap::new();
        values.insert("test_field".to_string(), Value::String("hello".to_string()));
        SubgraphSpec(surfpool_types::SubgraphDataEntry {
            uuid: Uuid::new_v4(),
            values,
            slot: 42,
            transaction_signature: Blake3Hash::from([1u8; 32]),
        })
    }

    #[test]
    fn test_register_insert_fetch() {
        let store = SqlStore::new_in_memory();
        let schema = test_schema();
        let uuid = schema.subgraph_uuid;
        let name = schema.name.clone();
        // Register subgraph
        store
            .register_collection(&uuid, &name, &schema)
            .expect("register_collection");
        // Insert entry
        let entry = test_entry(&schema);
        store
            .insert_entry_to_subgraph(&uuid, entry.clone())
            .expect("insert_entry_to_subgraph");
        // Fetch entries
        let fetched = store
            .fetch_entries_from_subgraph(&name, None, &schema)
            .expect("fetch_entries_from_subgraph");
        assert_eq!(fetched.len(), 1);
        // Check field value
        println!("Fetched entry: {:?}", fetched);
        let fetched_entry = &fetched[0].0;
        assert_eq!(fetched_entry.slot, 42);
        assert_eq!(
            fetched_entry.values.get("test_field"),
            Some(&Value::String("hello".to_string()))
        );
    }
}
