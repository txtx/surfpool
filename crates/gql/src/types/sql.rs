use juniper::Value;
use surfpool_db::diesel::deserialize::*;
#[cfg(feature = "sqlite")]
use surfpool_db::{
    diesel::{self, deserialize::FromSql},
    diesel_dynamic_schema::dynamic_value::Any,
};

#[derive(PartialEq, Debug)]
pub struct DynamicValue(pub Value);

#[cfg(feature = "sqlite")]
impl FromSql<Any, diesel::sqlite::Sqlite> for DynamicValue {
    fn from_sql(value: diesel::sqlite::SqliteValue) -> Result<Self> {
        use diesel::sqlite::{Sqlite, SqliteType};
        match value.value_type() {
            Some(SqliteType::Text) => {
                <String as FromSql<diesel::sql_types::Text, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::scalar(s)))
            }
            Some(SqliteType::Long) => {
                <i32 as FromSql<diesel::sql_types::Integer, Sqlite>>::from_sql(value)
                    .map(|s| DynamicValue(Value::scalar(s)))
            }
            _ => Err("Unknown data type".into()),
        }
    }
}

#[cfg(feature = "postgres")]
impl FromSql<Any, diesel::pg::Pg> for DynamicValue {
    fn from_sql(value: diesel::pg::PgValue) -> Result<Self> {
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
