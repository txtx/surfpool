use std::collections::HashMap;

use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue, Registry,
    Value, graphql_object,
    meta::{Field, MetaType},
};
use txtx_addon_kit::{
    hex,
    types::types::{AddonData, Value as TxtxValue},
};
use txtx_addon_network_svm_types::{
    SVM_F32, SVM_F64, SVM_I8, SVM_I16, SVM_I32, SVM_I64, SVM_I128, SVM_I256, SVM_PUBKEY,
    SVM_SIGNATURE, SVM_U8, SVM_U16, SVM_U32, SVM_U64, SVM_U128, SVM_U256, SvmValue, U256,
};
use uuid::Uuid;

use crate::{
    query::DataloaderContext,
    types::{
        collections::CollectionMetadata,
        scalars::{bigint::BigInt, pubkey::PublicKey, signature::Signature},
    },
};

pub mod collections;
pub mod filters;
pub mod scalars;
pub mod sql;

#[derive(Debug, Clone)]
pub struct CollectionEntry(pub CollectionEntryData);

impl GraphQLType<DefaultScalarValue> for CollectionEntry {
    fn name(spec: &CollectionMetadata) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &CollectionMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        fields.push(registry.field::<&Uuid>("id", &()));
        for field_metadata in spec.fields.iter() {
            let field = field_metadata.register_as_scalar(registry);
            fields.push(field);
        }
        registry
            .build_object_type::<[CollectionEntry]>(spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for CollectionEntry {
    type Context = DataloaderContext;
    type TypeInfo = CollectionMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <CollectionEntry as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        _info: &CollectionMetadata,
        field_name: &str,
        _args: &Arguments,
        executor: &Executor<DataloaderContext>,
    ) -> Result<juniper::Value, FieldError> {
        let entry = &self.0;
        match field_name {
            "id" => executor.resolve_with_ctx(&(), &entry.id.to_string()),
            field_name => {
                if let Some(schema) = entry.values.get(field_name) {
                    return Ok(schema.clone());
                } else {
                    Err(FieldError::new(
                        format!("field {} not found", field_name),
                        juniper::Value::null(),
                    ))
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CollectionEntryDataUpdate {
    /// The name of the subgraph that had an entry updated
    pub name: String,
    /// The updated entry
    pub entry: CollectionEntryData,
}

impl CollectionEntryDataUpdate {
    pub fn new(name: &str, entry: &CollectionEntryData) -> Self {
        Self {
            entry: entry.clone(),
            name: name.to_owned(),
        }
    }
}

#[graphql_object(context = DataloaderContext)]
impl CollectionEntryDataUpdate {
    pub fn uuid(&self) -> String {
        self.entry.id.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct CollectionEntryData {
    // The UUID of the entry
    pub id: Uuid,
    // A map of field names and their values
    pub values: HashMap<String, Value>,
}

fn convert_txtx_values_to_juniper_values(
    src: HashMap<String, TxtxValue>,
) -> HashMap<String, Value> {
    let mut dst = HashMap::new();

    for (k, old) in src.into_iter() {
        let new = match &old {
            TxtxValue::Bool(b) => Value::scalar(*b),
            TxtxValue::String(s) => Value::scalar(s.to_owned()),
            TxtxValue::Integer(n) => BigInt(*n).to_output(),
            TxtxValue::Float(f) => Value::scalar(*f),
            TxtxValue::Buffer(_bytes) => unimplemented!(),
            TxtxValue::Addon(AddonData { bytes, id }) => match id.as_str() {
                SVM_SIGNATURE => {
                    let bytes: [u8; 64] = bytes[0..64]
                        .try_into()
                        .expect("could not convert value to signature");
                    let signature = solana_signature::Signature::from(bytes);
                    Signature(signature).to_output()
                }
                SVM_PUBKEY => {
                    let bytes: [u8; 32] = bytes[0..32]
                        .try_into()
                        .expect("could not convert value to pubkey");
                    let pubkey = solana_pubkey::Pubkey::new_from_array(bytes);
                    PublicKey(pubkey).to_output()
                }
                SVM_U8 => {
                    let num =
                        SvmValue::to_number::<u8>(&old).expect("could not convert value to u8");
                    Value::scalar(num as i32)
                }
                SVM_U16 => {
                    let num =
                        SvmValue::to_number::<u16>(&old).expect("could not convert value to u16");
                    Value::scalar(num as i32)
                }
                SVM_U32 => {
                    let num =
                        SvmValue::to_number::<u32>(&old).expect("could not convert value to u32");
                    BigInt(num as i128).to_output()
                }
                SVM_U64 => {
                    let num =
                        SvmValue::to_number::<u64>(&old).expect("could not convert value to u64");
                    BigInt(num as i128).to_output()
                }
                SVM_U128 => {
                    let num =
                        SvmValue::to_number::<u128>(&old).expect("could not convert value to u128");
                    let bytes = num.to_le_bytes();
                    Value::scalar(hex::encode(bytes))
                }
                SVM_U256 => {
                    let num =
                        SvmValue::to_number::<U256>(&old).expect("could not convert value to u256");
                    let bytes = num.0;
                    Value::scalar(hex::encode(bytes))
                }
                SVM_I8 => {
                    let num =
                        SvmValue::to_number::<i8>(&old).expect("could not convert value to i8");
                    Value::scalar(num as i32)
                }
                SVM_I16 => {
                    let num =
                        SvmValue::to_number::<i16>(&old).expect("could not convert value to i16");
                    Value::scalar(num as i32)
                }
                SVM_I32 => {
                    let num =
                        SvmValue::to_number::<i32>(&old).expect("could not convert value to i32");
                    Value::scalar(num as i32)
                }
                SVM_I64 => {
                    let num =
                        SvmValue::to_number::<i64>(&old).expect("could not convert value to i64");
                    BigInt(num as i128).to_output()
                }
                SVM_I128 => {
                    let num =
                        SvmValue::to_number::<i128>(&old).expect("could not convert value to i128");
                    BigInt(num as i128).to_output()
                }
                SVM_I256 => {
                    let num =
                        SvmValue::to_number::<U256>(&old).expect("could not convert value to i256");
                    let bytes = num.0;
                    Value::scalar(hex::encode(bytes))
                }
                SVM_F32 => {
                    let num =
                        SvmValue::to_number::<f32>(&old).expect("could not convert value to f32");
                    Value::scalar(num as f64)
                }
                SVM_F64 => {
                    let num =
                        SvmValue::to_number::<f64>(&old).expect("could not convert value to f64");
                    Value::scalar(num)
                }
                _ => {
                    Value::scalar(String::from_utf8(bytes.to_vec()).expect("addon data not utf-8"))
                }
            },
            TxtxValue::Null => unimplemented!(),
            TxtxValue::Array(_arr) => unimplemented!(),
            TxtxValue::Object(_obj) => unimplemented!(),
        };

        dst.insert(k, new);
    }
    dst
}

impl CollectionEntryData {
    pub fn from_entries_bytes(
        subgraph_uuid: &Uuid,
        entry_bytes: Vec<u8>,
    ) -> Result<Vec<Self>, String> {
        let err_ctx = "Failed to apply new database entry to subgraph";
        let mut result = vec![];
        let entries: Vec<HashMap<String, TxtxValue>> = serde_json::from_slice(&entry_bytes)
            .map_err(|e| {
                format!(
                    "{err_ctx}: Failed to deserialize new database entry for subgraph {}: {}",
                    subgraph_uuid, e
                )
            })?;
        for entry in entries.into_iter() {
            result.push(Self::new(
                Uuid::new_v4(),
                convert_txtx_values_to_juniper_values(entry),
            ));
        }
        Ok(result)
    }

    pub fn new(id: Uuid, values: HashMap<String, Value>) -> Self {
        Self { id, values }
    }
}
