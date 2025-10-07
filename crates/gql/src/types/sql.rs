use std::collections::HashMap;

use diesel::prelude::*;
use juniper::{DefaultScalarValue, Executor, FieldError, Value, graphql_value};
use surfpool_db::{
    diesel::{
        self, ExpressionMethods, QueryDsl, RunQueryDsl,
        deserialize::{self, FromSql},
        r2d2::{ConnectionManager, Pool},
        result::DatabaseErrorKind,
        sql_query,
        sql_types::{Bool, Integer, Text, Untyped},
    },
    diesel_dynamic_schema::{
        DynamicSelectClause,
        dynamic_value::{Any, DynamicRow, NamedField},
        table,
    },
};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::subgraph::SubgraphRequest;
use uuid::Uuid;

use crate::{
    query::{DatabaseConnection, Dataloader, DataloaderContext, extract_graphql_features},
    types::{CollectionEntry, CollectionEntryData, collections::CollectionMetadata},
};

#[derive(PartialEq, Debug)]
pub struct DynamicValue(pub Value);

#[cfg(feature = "sqlite")]
impl FromSql<Any, diesel::sqlite::Sqlite> for DynamicValue {
    fn from_sql(value: diesel::sqlite::SqliteValue) -> deserialize::Result<Self> {
        use diesel::sqlite::{Sqlite, SqliteType};
        match value.value_type() {
            Some(SqliteType::Text) => {
                <String as FromSql<diesel::sql_types::Text, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::scalar(s)))
            }
            Some(SqliteType::Long) => {
                use surfpool_db::diesel::deserialize::FromSql;

                <i32 as FromSql<diesel::sql_types::Integer, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::scalar(s)))
            }
            _ => Err("Unknown data type".into()),
        }
    }
}

#[cfg(feature = "postgres")]
impl FromSql<Any, diesel::pg::Pg> for DynamicValue {
    fn from_sql(value: diesel::pg::PgValue) -> deserialize::Result<Self> {
        use std::num::NonZeroU32;

        use diesel::pg::Pg;

        const VARCHAR_OID: NonZeroU32 = NonZeroU32::new(1043).unwrap();
        const TEXT_OID: NonZeroU32 = NonZeroU32::new(25).unwrap();
        const INTEGER_OID: NonZeroU32 = NonZeroU32::new(23).unwrap();

        match value.get_oid() {
            VARCHAR_OID | TEXT_OID => {
                <String as FromSql<diesel::sql_types::Text, Pg>>::from_sql(value)
                    .map(|s| DynamicValue(Value::scalar(s)))
            }
            INTEGER_OID => <i32 as FromSql<diesel::sql_types::Integer, Pg>>::from_sql(value)
                .map(|s| DynamicValue(Value::scalar(s))),
            e => Err(format!("Unknown type: {e}").into()),
        }
    }
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
        match value {
            DefaultScalarValue::String(s) => {
                let col = dynamic_table.column::<Text, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(s)),
                    "not" => query.filter(col.ne(s)),
                    "_contains" => query.filter(col.like(format!("%{}%", s))),
                    "_notContains" => query.filter(col.not_like(format!("%{}%", s))),
                    "_endsWith" => query.filter(col.like(format!("%{}", s))),
                    "_startsWith" => query.filter(col.like(format!("{}%", s))),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported string predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid string predicate"}),
                        ));
                    }
                };
            }
            DefaultScalarValue::Int(i) => {
                let col = dynamic_table.column::<Integer, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(*i)),
                    "not" => query.filter(col.ne(*i)),
                    "gt" => query.filter(col.gt(*i)),
                    "gte" => query.filter(col.ge(*i)),
                    "lt" => query.filter(col.lt(*i)),
                    "lte" => query.filter(col.le(*i)),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported integer predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid integer predicate"}),
                        ));
                    }
                };
            }
            DefaultScalarValue::Boolean(b) => {
                let col = dynamic_table.column::<Bool, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(*b)),
                    "not" => query.filter(col.ne(*b)),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported boolean predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid boolean predicate"}),
                        ));
                    }
                };
            }
            _ => {
                return Err(FieldError::new(
                    format!("Unsupported predicate or value type: {}", predicate),
                    graphql_value!({"invalid_params": "Invalid predicate or value type"}),
                ));
            }
        };
    }

    let fetched_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *pg_conn)
        .map_err(|err| {
            FieldError::new(
                format!("Internal error: unable to fetch data"),
                graphql_value!({"error": err.to_string()}),
            )
        })?;

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
        select.add_field(dynamic_table.column::<Untyped, _>(format!("{}", field_name)));
    }

    // Build the query and apply filters immediately to avoid borrow checker issues
    let mut query = dynamic_table.clone().select(select).into_boxed();

    for (field, predicate, value) in filters_specs {
        match value {
            DefaultScalarValue::String(s) => {
                let col = dynamic_table.column::<Text, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(s)),
                    "not" => query.filter(col.ne(s)),
                    "contains" => query.filter(col.like(format!("%{}%", s))),
                    "notContains" => query.filter(col.not_like(format!("%{}%", s))),
                    "endsWith" => query.filter(col.like(format!("%{}", s))),
                    "startsWith" => query.filter(col.like(format!("{}%", s))),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported string predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid string predicate"}),
                        ));
                    }
                };
            }
            DefaultScalarValue::Int(i) => {
                let col = dynamic_table.column::<Integer, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(*i)),
                    "not" => query.filter(col.ne(*i)),
                    "gt" => query.filter(col.gt(*i)),
                    "gte" => query.filter(col.ge(*i)),
                    "lt" => query.filter(col.lt(*i)),
                    "lte" => query.filter(col.le(*i)),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported integer predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid integer predicate"}),
                        ));
                    }
                };
            }
            DefaultScalarValue::Boolean(b) => {
                let col = dynamic_table.column::<Bool, _>(field);
                query = match predicate {
                    "equals" => query.filter(col.eq(*b)),
                    "not" => query.filter(col.ne(*b)),
                    _ => {
                        return Err(FieldError::new(
                            format!("Unsupported boolean predicate: {}", predicate),
                            graphql_value!({"invalid_params": "Invalid boolean predicate"}),
                        ));
                    }
                };
            }
            _ => {
                return Err(FieldError::new(
                    format!("Unsupported predicate or value type: {}", predicate),
                    graphql_value!({"invalid_params": "Invalid predicate or value type"}),
                ));
            }
        };
    }

    let fetched_data = query
        .load::<DynamicRow<NamedField<DynamicValue>>>(&mut *sqlite_conn)
        .map_err(|err| {
            FieldError::new(
                format!("Internal error: unable to fetch data"),
                graphql_value!({"error": err.to_string()}),
            )
        })?;

    Ok((fetched_fields, fetched_data))
}

impl Dataloader for Pool<ConnectionManager<DatabaseConnection>> {
    fn fetch_data_from_collection(
        &self,
        executor: Option<&Executor<DataloaderContext>>,
        metadata: &CollectionMetadata,
    ) -> Result<Vec<CollectionEntry>, FieldError> {
        let mut conn = self.get().expect("unable to connect to db");
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
                id: Uuid::new_v4(),
                values,
            }));
        }
        Ok(results)
    }

    fn register_collection(
        &self,
        metadata: &CollectionMetadata,
        request: &SubgraphRequest,
        worker_id: &Uuid,
    ) -> Result<(), String> {
        let SubgraphRequest::V0(request_v0) = request;
        let mut conn = self.get().expect("unable to connect to db");

        // 2. Create a new entries table for this subgraph, using the schema to determine the fields
        // Build the SQL for the entries table using the schema fields
        let mut columns = vec!["id TEXT PRIMARY KEY".to_string()];
        for field in &metadata.fields {
            // Map field types to SQLite types (expand as needed)
            let sql_type = match &field.data.expected_type {
                Type::String => "TEXT",
                Type::Integer => "INTEGER",
                Type::Float => "REAL",
                Type::Bool => "BOOLEAN",
                _ => "TEXT", // fallback for unknown types
            };
            let col = format!("\"{}\" {}", field.data.display_name, sql_type);
            columns.push(col);
        }
        let create_entries_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n    {}\n)",
            metadata.table_name,
            columns.join(",\n    ")
        );

        match sql_query(&create_entries_sql).execute(&mut *conn) {
            Ok(_)
            | Err(diesel::result::Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {
                Ok(())
            }
            Err(e) => Err(format!("Failed to create entries table: {}", e)),
        }?;
        // let schema_json = serde_json::to_string(request)
        //     .map_err(|e| format!("Failed to serialize schema: {e}"))?;
        let schema_json = serde_json::to_string(request).expect("Failed to serialize schema");
        let now = chrono::Utc::now().naive_utc();

        let sql = format!(
            "INSERT INTO collections (id, created_at, updated_at, table_name, workspace_slug, source, latest_slot_successfully_processed, worker_id) VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}')",
            metadata.id,
            now,
            now,
            &metadata.table_name,
            &metadata.workspace_slug,
            schema_json,
            request_v0.slot,
            worker_id
        );

        match sql_query(&sql).execute(&mut *conn) {
            Ok(_)
            | Err(diesel::result::Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {
                Ok(())
            }
            Err(e) => Err(format!("Failed to create entries table: {}", e)),
        }?;

        Ok(())
    }

    fn insert_entries_into_collection(
        &self,
        entries: Vec<CollectionEntryData>,
        metadata: &CollectionMetadata,
    ) -> Result<(), String> {
        let mut conn = self.get().expect("unable to connect to db");

        // 2. Prepare the insert statement using the schema for column order
        // let CollectionEntry(data_entry) = entry;

        for entry in entries {
            let mut columns = vec![];
            let mut values: Vec<String> = vec![];
            columns.push("\"id\"".to_string());
            values.push(format!("'{}'", entry.id));

            // Use the schema to determine the order and names of dynamic fields
            // Insert null if the value is missing
            for field in &metadata.fields {
                let col = field.data.display_name.as_str();
                if let Some(val) = entry.values.get(col) {
                    let val_str = match val {
                        juniper::Value::Scalar(DefaultScalarValue::String(value)) => {
                            format!("'{}'", value)
                        }
                        juniper::Value::Scalar(value) => value.to_string(),
                        _ => unimplemented!(),
                    };
                    values.push(val_str);
                    columns.push(format!("\"{}\"", col));
                }
            }

            // 3. Build and execute the insert
            let sql: String = format!(
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

#[cfg(test)]
mod tests {
    use solana_pubkey::Pubkey;
    use txtx_addon_kit::types::{
        ConstructDid, Did,
        types::{Type, Value},
    };
    use txtx_addon_network_svm_types::{
        anchor::types::{Idl, IdlEvent, IdlMetadata, IdlSerialization, IdlTypeDef, IdlTypeDefTy},
        subgraph::{
            EventSubgraphSource, IndexedSubgraphField, IndexedSubgraphSourceType, SubgraphRequest,
            SubgraphRequestV0,
        },
    };
    use uuid::Uuid;

    use super::*;
    use crate::{query::SqlStore, types::collections::CollectionMetadata};

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
                ty: IdlTypeDefTy::Enum { variants: vec![] },
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
        SubgraphRequest::V0(SubgraphRequestV0 {
            program_id: program_id.clone(),
            slot: 0,
            subgraph_name: "TestSubgraph".to_string(),
            subgraph_description: None,
            data_source: IndexedSubgraphSourceType::Event(
                EventSubgraphSource::new(&event_name, &idl).unwrap(),
            ),
            construct_did: ConstructDid(Did::zero()),
            network: "localnet".into(),
            defined_fields: vec![IndexedSubgraphField {
                display_name: "test_field".to_string(),
                source_key: "test_field".to_string(),
                expected_type: Type::String,
                description: None,
                is_indexed: false,
            }],
            intrinsic_fields: vec![],
            idl_types: vec![],
        })
    }

    fn test_entry(_schema: &CollectionMetadata) -> Vec<u8> {
        let mut values = HashMap::new();
        values.insert("test_field".to_string(), Value::String("hello".to_string()));
        let bytes = serde_json::to_vec(&vec![values]).unwrap();
        bytes
    }

    #[test]
    fn test_register_insert_fetch() {
        // Prepare dataset
        let store = SqlStore::new_in_memory();
        store
            .init_subgraph_tables()
            .expect("unable to initialize tables");

        let request = test_request();
        let uuid = Uuid::new_v4();
        let metadata = CollectionMetadata::from_request(&uuid, &request, "test");

        // Register subgraph
        store
            .pool
            .register_collection(&metadata, &request, &Uuid::default())
            .expect("register_collection");

        // Insert entry
        let entries_pack = test_entry(&metadata);
        let entries = CollectionEntryData::from_entries_bytes(&uuid, entries_pack).unwrap();

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
        let _fetched_entry = &fetched[0].0;
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
