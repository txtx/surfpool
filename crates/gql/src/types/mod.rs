use std::collections::HashMap;

use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue, Registry,
    Value, graphql_object,
    meta::{Field, MetaType},
};
use solana_signature::Signature;
use surfpool_types::CollectionEntriesPack;
use txtx_addon_kit::types::types::{AddonData, Value as TxtxValue};
use txtx_addon_network_svm_types::SVM_SIGNATURE;
use uuid::Uuid;

use crate::{
    query::DataloaderContext,
    types::{collections::CollectionMetadata, scalars::hash::Hash},
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
        let new = match old {
            TxtxValue::Bool(b) => Value::scalar(b),
            TxtxValue::String(s) => Value::scalar(s),
            TxtxValue::Integer(n) => Value::scalar(i32::try_from(n).expect("i32 overflow")),
            TxtxValue::Float(f) => Value::scalar(f),
            TxtxValue::Buffer(_bytes) => unimplemented!(),
            TxtxValue::Addon(AddonData { bytes, id }) => match id.as_str() {
                SVM_SIGNATURE => Value::scalar(
                    Signature::try_from(bytes)
                        .expect("signature malformed")
                        .to_string(),
                ),
                _ => unimplemented!("convert_txtx_values_to_juniper_values for {id}"),
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
                    ));
                }
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

    pub fn cpi_event(id: Uuid, values: HashMap<String, Value>) -> Self {
        Self { id, values }
    }
}
