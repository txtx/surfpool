pub use diesel;
use diesel::deserialize::*;
pub use diesel_dynamic_schema;
use diesel_dynamic_schema::dynamic_value::*;
use txtx_addon_kit::types::types::Value;

pub mod schema;

#[derive(PartialEq, Debug)]
pub struct DynamicValue(pub Value);

impl FromSql<Any, diesel::sqlite::Sqlite> for DynamicValue {
    fn from_sql(value: diesel::sqlite::SqliteValue) -> Result<Self> {
        use diesel::sqlite::{Sqlite, SqliteType};
        match value.value_type() {
            Some(SqliteType::Text) => {
                <String as FromSql<diesel::sql_types::Text, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::string(s)))
            }
            Some(SqliteType::Long) => {
                <i32 as FromSql<diesel::sql_types::Integer, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::integer(s.into())))
            }
            _ => Err("Unknown data type".into()),
        }
    }
}
