use crate::query::DataloaderContext;
use crate::types::schema::DynamicSchemaSpec;
use juniper::graphql_object;
use juniper::meta::Field;
use juniper::meta::MetaType;
use juniper::Arguments;
use juniper::DefaultScalarValue;
use juniper::Executor;
use juniper::FieldError;
use juniper::GraphQLType;
use juniper::GraphQLValue;
use juniper::Registry;
use scalars::bigint::BigInt;
use scalars::hash::Hash;
use scalars::pubkey::PublicKey;
use scalars::slot::Slot;
use surfpool_types::SubgraphDataEntry;
use txtx_addon_kit::hex;
use txtx_addon_kit::types::types::Value;
use txtx_addon_network_svm_types::{SvmValue, SVM_PUBKEY};
use uuid::Uuid;

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
        fields.push(registry.field::<&Uuid>("uuid", &()));
        fields.push(registry.field::<&String>("slot", &()));
        fields.push(registry.field::<&String>("transaction_hash", &()));
        for field_metadata in spec.fields.iter() {
            let field = field_metadata.register_as_scalar(registry);
            fields.push(field);
        }
        registry
            .build_object_type::<[SubgraphSpec]>(&spec, &fields)
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
            "slot" => executor.resolve_with_ctx(&(), &Slot(entry.slot)),
            "transaction_hash" => executor.resolve_with_ctx(&(), &Hash(entry.transaction_hash)),
            field_name => {
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
    pub fn new(name: &String, entry: &SubgraphDataEntry) -> Self {
        Self {
            entry: entry.clone(),
            name: name.clone(),
        }
    }
}

#[graphql_object(context = DataloaderContext)]
impl SubgraphDataEntryUpdate {
    pub fn uuid(&self) -> String {
        self.entry.uuid.to_string()
    }
}
