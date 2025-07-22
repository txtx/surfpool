use convert_case::{Case, Casing};
use juniper::{
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
    meta::{Field, MetaType},
};
use serde::{Deserialize, Serialize};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{
    SVM_PUBKEY,
    subgraph::{IndexedSubgraphField, IndexedSubgraphSourceType, SubgraphRequest},
};
use uuid::Uuid;

use super::{
    filters::SubgraphFilterSpec,
    scalars::pubkey::PublicKey,
};
use crate::{
    query::DataloaderContext,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub table_name: String,
    pub filters: SubgraphFilterSpec,
    pub fields: Vec<FieldMetadata>,
    pub source_type: IndexedSubgraphSourceType,
}

impl CollectionMetadata {
    pub fn from_request(uuid: &Uuid, request: &SubgraphRequest) -> Self {
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
            table_name: format!("entries_{}", uuid.simple().to_string()),
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
            let registration = match &field.data.expected_type {
                Type::String => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Integer => registry
                    .field::<&i32>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Bool => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Float => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Null => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Array(_array) => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Buffer => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Addon(_addon) => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Object(_object) => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
                Type::Map(_map) => registry
                    .field::<&String>(&field.data.display_name, &())
                    .description(
                        &field
                            .data
                            .description
                            .as_ref()
                            .map(|d| d.as_str())
                            .unwrap_or(""),
                    ),
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
            Type::Integer => registry.field::<&i32>(field_name, &()),
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
