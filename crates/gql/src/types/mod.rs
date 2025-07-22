use std::collections::HashMap;

use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue, Registry,
    Value, graphql_object,
    meta::{Field, MetaType},
};
use scalars::{bigint::BigInt, pubkey::PublicKey, slot::Slot};
use surfpool_types::{CpiEventTableDefaults, CollectionEntriesPack, PdaTableDefaults};
use uuid::Uuid;
        use txtx_addon_kit::types::types::Value as TxtxValue;

use crate::{
    query::DataloaderContext,
    types::{
        scalars::{hash::Hash, signature::Signature},
        schema::CollectionMetadata,
    },
};

pub mod filters;
pub mod scalars;
pub mod schema;
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
        fields.push(registry.field::<&Uuid>("uuid", &()));
        fields.push(registry.field::<i32>("slot", &()));
        fields.push(registry.field::<&String>("transactionSignature", &()));
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
            "uuid" => executor.resolve_with_ctx(&(), &entry.uuid.to_string()),
            "slot" => executor.resolve_with_ctx(&(), &Slot(entry.slot)),
            "transactionSignature" => executor.resolve_with_ctx(&(), &entry.transaction_signature),
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
        self.entry.uuid.to_string()
    }
}

use solana_pubkey::Pubkey;
use txtx_addon_network_svm_types::subgraph::IndexedSubgraphSourceTypeName;

#[derive(Debug, Clone)]
pub struct CollectionEntryData {
    // The UUID of the entry
    pub uuid: Uuid,
    // A map of field names and their values
    pub values: HashMap<String, Value>,
    // The slot that the transaction that created this entry was processed in
    pub slot: u64,
    // The transaction hash that created this entry
    pub transaction_signature: Hash,
}

fn convert_txtx_values_to_juniper_values(src: HashMap<String, TxtxValue>) -> HashMap<String, Value> {
    let mut dst = HashMap::new();

    for (k, old) in src.into_iter() {
        let new = match old {
            TxtxValue::Bool(b) => Value::scalar(b),
            TxtxValue::String(s) => Value::scalar(s),
            TxtxValue::Integer(n) => Value::scalar(i32::try_from(n).expect("i32 overflow")),
            TxtxValue::Float(f) => Value::scalar(f),
            TxtxValue::Buffer(bytes) => unimplemented!(),
            TxtxValue::Addon(addon) => unimplemented!(),
            TxtxValue::Null => unimplemented!(),
            TxtxValue::Array(_arr) => unimplemented!(),
            TxtxValue::Object(_obj) => unimplemented!(),
        };

        dst.insert(k, new);
    }
    dst
}

impl CollectionEntryData {
    pub fn from_entries_pack(
        subgraph_uuid: &Uuid,
        entries_pack: CollectionEntriesPack,
    ) -> Result<Vec<Self>, String> {

        let err_ctx = "Failed to apply new database entry to subgraph";
        let mut result = vec![];
        match entries_pack {
            CollectionEntriesPack::CpiEvent(cpi) => {
                let entries: Vec<HashMap<String, TxtxValue>> = serde_json::from_slice(&cpi.data).map_err(|e| {
                    format!("{err_ctx}: Failed to deserialize new cpi event database entry for subgraph {}: {}", subgraph_uuid, e)
                })?;

                for entry in entries.into_iter() {
                    result.push(Self::cpi_event(
                        Uuid::new_v4(),
                        convert_txtx_values_to_juniper_values(entry),
                        cpi.slot,
                        cpi.transaction_signature.into(),
                    ));
                }
                println!("{:?}", result);
            }
            CollectionEntriesPack::Pda(pda_entry) => {
                // let entries: Vec<HashMap<String, Value>> = serde_json::from_slice(&pda_entry.data).map_err(|e| {
                //     format!("{err_ctx}: Failed to deserialize new pda database entry for subgraph {}: {}", subgraph_uuid, e)
                // })?;
                // for entry in entries.into_iter() {
                //     result.push(Self::pda(
                //         Uuid::new_v4(),
                //         entry,
                //         pda_entry.slot,
                //         pda_entry.transaction_signature.into(),
                //         pda_entry.pubkey.into(),
                //         pda_entry.owner.into(),
                //         pda_entry.lamports,
                //         pda_entry.write_version,
                //     ));
                // }
            }
        }
        Ok(result)
    }

    // pub fn from_data_row(
    //     dynamic_values: HashMap<String, Value>,
    //     default_values: Vec<Value>,
    //     source_type: &IndexedSubgraphSourceTypeName,
    // ) -> Self {
    //     let uuid = Uuid::parse_str(default_values[0].expect_string()).unwrap_or(Uuid::nil());

    //     match source_type {
    //         IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
    //         IndexedSubgraphSourceTypeName::Event => {
    //             let slot = default_values[1].expect_integer().try_into().unwrap_or(0);
    //             let transaction_signature = default_values[2]
    //                 .as_string()
    //                 .and_then(|s| s.parse().ok())
    //                 .unwrap_or_else(|| Signature::from([0u8; 64]));
    //             Self::cpi_event(uuid, dynamic_values, slot, transaction_signature)
    //         }
    //         IndexedSubgraphSourceTypeName::Pda => {
    //             let slot = default_values[1].expect_integer().try_into().unwrap_or(0);
    //             let transaction_signature = default_values[2]
    //                 .as_string()
    //                 .and_then(|s| s.parse().ok())
    //                 .unwrap_or_else(|| Signature::from([0u8; 64]));
    //             let pubkey = SvmValue::to_pubkey(&default_values[3]).unwrap_or(Pubkey::default());
    //             let owner = SvmValue::to_pubkey(&default_values[4]).unwrap_or(Pubkey::default());
    //             let lamports = default_values[5].expect_integer().try_into().unwrap_or(0);
    //             let write_version = default_values[6].expect_integer().try_into().unwrap_or(0);
    //             Self::pda(
    //                 uuid,
    //                 dynamic_values,
    //                 slot,
    //                 transaction_signature,
    //                 pubkey,
    //                 owner,
    //                 lamports,
    //                 write_version,
    //             )
    //         }
    //     }
    // }

    pub fn cpi_event(
        uuid: Uuid,
        values: HashMap<String, Value>,
        slot: u64,
        transaction_signature: solana_signature::Signature,
    ) -> Self {
        Self {
            uuid,
            slot,
            transaction_signature: Hash(blake3::Hash::from_bytes([1u8; 32])),
            values,
        }
    }

    pub fn default_columns(&self) -> Vec<String> {
        vec![]
    }

    pub fn column_metadata(source_type: &IndexedSubgraphSourceTypeName) -> Vec<String> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => CpiEventTableDefaults::column_metadata(),
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::column_metadata(),
        }
    }

    pub fn default_column_numbers(source_type: &IndexedSubgraphSourceTypeName) -> usize {
        Self::column_metadata(source_type).len() - 1 // -1 to omit the id field
    }

    pub fn default_columns_with_descriptions(
        source_type: &IndexedSubgraphSourceTypeName,
    ) -> Vec<(String, String)> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => {
                CpiEventTableDefaults::columns_with_descriptions()
            }
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::columns_with_descriptions(),
        }
    }
    pub fn default_columns_with_types(source_type: &IndexedSubgraphSourceTypeName) -> Vec<String> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => CpiEventTableDefaults::column_metadata(),
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::column_metadata(),
        }
    }
}
