use convert_case::{Case, Casing};
use juniper::{
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
    meta::{Field, MetaType},
};
use serde::{Deserialize, Serialize};
use surfpool_types::subgraphs::SubgraphDataEntry;
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{
    SVM_PUBKEY,
    subgraph::{IndexedSubgraphField, IndexedSubgraphSourceTypeName, SubgraphRequest},
};
use uuid::Uuid;

use super::{
    filters::SubgraphFilterSpec,
    scalars::{bigint::BigInt, pubkey::PublicKey},
};
use crate::{
    query::DataloaderContext,
    types::scalars::{signature::Signature, slot::Slot},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicSchemaSpec {
    pub name: String,
    pub filter: SubgraphFilterSpec,
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<FieldMetadata>,
    pub source_type: IndexedSubgraphSourceTypeName,
}

impl DynamicSchemaSpec {
    pub fn from_request(uuid: &Uuid, request: &SubgraphRequest) -> Self {
        let name = request.subgraph_name.to_case(Case::Pascal);
        let fields: Vec<_> = request.fields.iter().map(FieldMetadata::new).collect();
        Self {
            name: name.clone(),
            filter: SubgraphFilterSpec {
                name: format!("{}Filter", name),
                fields: fields.clone(),
            },
            subgraph_uuid: *uuid,
            description: request.subgraph_description.clone(),
            fields,
            source_type: (&request.data_source).into(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl GraphQLType<DefaultScalarValue> for DynamicSchemaSpec {
    fn name(spec: &DynamicSchemaSpec) -> Option<&str> {
        Some(spec.get_name())
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

        let mut object_meta = registry.build_object_type::<Self>(spec, &fields);
        if let Some(description) = &spec.description {
            object_meta = object_meta.description(description.as_str());
        }

        object_meta.into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for DynamicSchemaSpec {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaSpec;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicSchemaSpec as GraphQLType<DefaultScalarValue>>::name(info)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldMetadata {
    pub data: IndexedSubgraphField,
}

impl FieldMetadata {
    pub fn new(field: &IndexedSubgraphField) -> Self {
        Self {
            data: field.clone(),
        }
    }

    pub fn is_bool(&self) -> bool {
        self.data.expected_type.eq(&Type::Bool)
    }

    pub fn is_string(&self) -> bool {
        self.data.expected_type.eq(&Type::String)
    }

    pub fn is_number(&self) -> bool {
        self.data.expected_type.eq(&Type::Float) || self.data.expected_type.eq(&Type::Integer)
    }

    pub fn register_as_scalar<'r>(
        &self,
        registry: &mut Registry<'r>,
    ) -> Field<'r, DefaultScalarValue> {
        let field_name = self.data.display_name.as_str();
        let mut field = match &self.data.expected_type {
            Type::Bool => registry.field::<&bool>(field_name, &()),
            Type::String => registry.field::<&String>(field_name, &()),
            Type::Integer => registry.field::<&BigInt>(field_name, &()),
            Type::Float => registry.field::<&f64>(field_name, &()),
            Type::Buffer => registry.field::<&String>(field_name, &()),
            Type::Addon(addon_id) => {
                if addon_id == SVM_PUBKEY {
                    registry.field::<&PublicKey>(field_name, &())
                } else {
                    registry.field::<&String>(field_name, &())
                }
            }
            unsupported => unimplemented!("unsupported type: {:?}", unsupported),
        };
        if let Some(description) = &self.data.description {
            field = field.description(description.as_str());
        }
        field
    }
}
