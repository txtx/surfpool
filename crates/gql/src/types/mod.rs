use juniper::{
    Arguments, DefaultScalarValue, Executor, FieldError, GraphQLType, GraphQLValue, Registry,
    graphql_object,
    meta::{Field, MetaType},
};
use scalars::{bigint::BigInt, pubkey::PublicKey, slot::Slot};
use surfpool_types::subgraphs::SubgraphDataEntry;
use txtx_addon_kit::{hex, types::types::Value};
use txtx_addon_network_svm_types::{SVM_PUBKEY, SvmValue};
use uuid::Uuid;

use crate::{
    query::DataloaderContext,
    types::{scalars::signature::Signature, schema::DynamicSchemaSpec},
};

pub mod filters;
pub mod scalars;
pub mod schema;

#[derive(Debug, Clone)]
pub struct SubgraphSpec(pub SubgraphDataEntry);

impl GraphQLType<DefaultScalarValue> for SubgraphSpec {
    fn name(spec: &DynamicSchemaSpec) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &DynamicSchemaSpec, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        let cols = SubgraphDataEntry::default_columns_with_descriptions(&spec.source_type);
        for (col, description) in cols {
            if col == "pubkey" || col == "owner" {
                fields.push(
                    registry
                        .field::<&PublicKey>(&col, &())
                        .description(&description),
                );
            } else if col == "lamports" || col == "writeVersion" {
                fields.push(
                    registry
                        .field::<&BigInt>(&col, &())
                        .description(&description),
                );
            } else if col == "slot" {
                fields.push(registry.field::<&Slot>(&col, &()).description(&description));
            } else if col == "data" {
                fields.push(
                    registry
                        .field::<&String>(&col, &())
                        .description(&description),
                );
            } else if col == "transactionSignature" {
                fields.push(
                    registry
                        .field::<&Signature>(&col, &())
                        .description(&description),
                );
            } else if col == "uuid" {
                fields.push(registry.field::<&Uuid>(&col, &()).description(&description));
            } else {
                fields.push(
                    registry
                        .field::<&String>(&col, &())
                        .description(&description),
                );
            }
        }

        for field_metadata in spec.fields.iter() {
            let field = field_metadata.register_as_scalar(registry);
            fields.push(field);
        }
        registry
            .build_object_type::<[SubgraphSpec]>(spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SubgraphSpec {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaSpec;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <SubgraphSpec as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        _info: &DynamicSchemaSpec,
        field_name: &str,
        _args: &Arguments,
        executor: &Executor<DataloaderContext>,
    ) -> Result<juniper::Value, FieldError> {
        let entry = &self.0;
        match field_name {
            "uuid" => executor.resolve_with_ctx(&(), &entry.uuid.to_string()),
            "slot" => executor.resolve_with_ctx(&(), &Slot(entry.table_defaults.slot())),
            "transactionSignature" => executor.resolve_with_ctx(
                &(),
                &Signature(entry.table_defaults.transaction_signature()),
            ),
            field_name => {
                match field_name {
                    "pubkey" => {
                        if let Some(pubkey) = entry.table_defaults.pubkey() {
                            return executor.resolve_with_ctx(&(), &PublicKey(pubkey));
                        }
                    }
                    "owner" => {
                        if let Some(owner) = entry.table_defaults.owner() {
                            return executor.resolve_with_ctx(&(), &PublicKey(owner));
                        }
                    }
                    "lamports" => {
                        if let Some(lamports) = entry.table_defaults.lamports() {
                            return executor.resolve_with_ctx(&(), &BigInt(lamports as i128));
                        }
                    }
                    "writeVersion" => {
                        if let Some(write_version) = entry.table_defaults.write_version() {
                            return executor.resolve_with_ctx(&(), &BigInt(write_version as i128));
                        }
                    }
                    _ => {}
                }
                let value = entry.values.get(field_name).unwrap();
                match value {
                    Value::Bool(b) => executor.resolve_with_ctx(&(), b),
                    Value::String(s) => executor.resolve_with_ctx(&(), s),
                    Value::Integer(n) => executor.resolve_with_ctx(&(), &BigInt(*n)),
                    Value::Float(f) => executor.resolve_with_ctx(&(), f),
                    Value::Buffer(bytes) => executor.resolve_with_ctx(&(), &hex::encode(bytes)),
                    Value::Addon(addon_data) => {
                        if addon_data.id == SVM_PUBKEY {
                            let pubkey = SvmValue::to_pubkey(value).map_err(|e| {
                                FieldError::new(
                                    format!("invalid pubkey in database: {}", e),
                                    juniper::Value::Null,
                                )
                            })?;
                            executor.resolve_with_ctx(&(), &PublicKey(pubkey))
                        } else {
                            executor.resolve_with_ctx(&(), &addon_data.to_string())
                        }
                    }
                    other => executor.resolve_with_ctx(&(), &other.to_string()),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubgraphDataEntryUpdate {
    /// The name of the subgraph that had an entry updated
    pub name: String,
    /// The updated entry
    pub entry: SubgraphDataEntry,
}

impl SubgraphDataEntryUpdate {
    pub fn new(name: &str, entry: &SubgraphDataEntry) -> Self {
        Self {
            entry: entry.clone(),
            name: name.to_owned(),
        }
    }
}

#[graphql_object(context = DataloaderContext)]
impl SubgraphDataEntryUpdate {
    pub fn uuid(&self) -> String {
        self.entry.uuid.to_string()
    }
}
