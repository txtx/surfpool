use convert_case::{Case, Casing};
use juniper::{
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
    meta::{Field, MetaType},
};
use serde::{Deserialize, Serialize};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{
    SVM_F32, SVM_F64, SVM_I8, SVM_I16, SVM_I32, SVM_I64, SVM_I128, SVM_I256, SVM_PUBKEY,
    SVM_SIGNATURE, SVM_U8, SVM_U16, SVM_U32, SVM_U64, SVM_U128, SVM_U256,
    subgraph::{IndexedSubgraphField, IndexedSubgraphSourceType, SubgraphRequest},
};
use uuid::Uuid;

use super::{
    filters::SubgraphFilterSpec,
    scalars::{bigint::BigInt, pubkey::PublicKey, signature::Signature},
};
use crate::query::DataloaderContext;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub table_name: String,
    pub workspace_slug: String,
    pub filters: SubgraphFilterSpec,
    pub fields: Vec<FieldMetadata>,
    pub source_type: IndexedSubgraphSourceType,
}

impl CollectionMetadata {
    pub fn from_request(uuid: &Uuid, request: &SubgraphRequest, workspace_slug: &str) -> Self {
        let SubgraphRequest::V0(request) = request;
        let name = request.subgraph_name.to_case(Case::Pascal);
        let mut fields: Vec<_> = request
            .intrinsic_fields
            .iter()
            .map(FieldMetadata::new)
            .collect();
        let mut defined: Vec<_> = request
            .defined_fields
            .iter()
            .map(FieldMetadata::new)
            .collect();
        fields.append(&mut defined);
        Self {
            id: *uuid,
            name: name.clone(),
            table_name: format!("entries_{}", uuid.simple()),
            workspace_slug: workspace_slug.to_string(),
            filters: SubgraphFilterSpec {
                name: format!("{}Filter", name),
                fields: fields.clone(),
            },
            description: request.subgraph_description.clone(),
            fields,
            source_type: request.data_source.clone(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl GraphQLType<DefaultScalarValue> for CollectionMetadata {
    fn name(metadata: &CollectionMetadata) -> Option<&str> {
        Some(metadata.get_name())
    }

    fn meta<'r>(metadata: &CollectionMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        for field in metadata.fields.iter() {
            let registration = {
                let description = field.data.description.as_deref().unwrap_or("");
                match &field.data.expected_type {
                    Type::String => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Integer => registry
                        .field::<&BigInt>(&field.data.display_name, &())
                        .description(description),
                    Type::Bool => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Float => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Null => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Array(_array) => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Buffer => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Addon(addon_id) => match addon_id.as_str() {
                        SVM_PUBKEY => registry
                            .field::<&PublicKey>(&field.data.display_name, &())
                            .description(description),
                        SVM_SIGNATURE => registry
                            .field::<&Signature>(&field.data.display_name, &())
                            .description(description),
                        SVM_U8 | SVM_U16 | SVM_I8 | SVM_I16 | SVM_I32 => registry
                            .field::<&i32>(&field.data.display_name, &())
                            .description(description),
                        SVM_U32 | SVM_U64 | SVM_I64 | SVM_I128 => registry
                            .field::<&BigInt>(&field.data.display_name, &())
                            .description(description),
                        SVM_U128 | SVM_U256 | SVM_I256 => registry
                            .field::<&String>(&field.data.display_name, &())
                            .description(description),
                        SVM_F32 | SVM_F64 => registry
                            .field::<&f64>(&field.data.display_name, &())
                            .description(description),
                        _ => registry
                            .field::<&String>(&field.data.display_name, &())
                            .description(description),
                    },
                    Type::Object(_object) => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                    Type::Map(_map) => registry
                        .field::<&String>(&field.data.display_name, &())
                        .description(description),
                }
            };
            fields.push(registration);
        }

        let mut object_meta = registry.build_object_type::<Self>(metadata, &fields);
        if let Some(description) = &metadata.description {
            object_meta = object_meta.description(description.as_str());
        }
        object_meta.into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for CollectionMetadata {
    type Context = DataloaderContext;
    type TypeInfo = CollectionMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <CollectionMetadata as GraphQLType<DefaultScalarValue>>::name(info)
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
            Type::Addon(addon_id) => match addon_id.as_str() {
                SVM_PUBKEY => registry.field::<&PublicKey>(field_name, &()),
                SVM_SIGNATURE => registry.field::<&Signature>(field_name, &()),
                SVM_U8 | SVM_U16 | SVM_I8 | SVM_I16 | SVM_I32 => {
                    registry.field::<&i32>(field_name, &())
                }
                SVM_U32 | SVM_U64 | SVM_I64 | SVM_I128 => {
                    registry.field::<&BigInt>(field_name, &())
                }
                SVM_U128 | SVM_U256 | SVM_I256 => registry.field::<&String>(field_name, &()),
                SVM_F32 | SVM_F64 => registry.field::<&f64>(field_name, &()),
                _ => registry.field::<&String>(field_name, &()),
            },
            unsupported => unimplemented!("unsupported type: {:?}", unsupported),
        };
        if let Some(description) = &self.data.description {
            field = field.description(description.as_str());
        }
        field
    }
}
