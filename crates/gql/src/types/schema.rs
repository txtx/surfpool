use convert_case::{Case, Casing};
use juniper::{
    meta::{Field, MetaType},
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
};
use serde::{Deserialize, Serialize};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{
    subgraph::{IndexedSubgraphField, SubgraphRequest},
    SVM_PUBKEY,
};
use uuid::Uuid;

use super::{
    filters::SubgraphFilterSpec,
    scalars::{bigint::BigInt, pubkey::PublicKey},
};
use crate::query::DataloaderContext;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicSchemaPayload {
    pub name: String,
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<FieldMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicSchemaSpec {
    pub name: String,
    pub filter: SubgraphFilterSpec,
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<FieldMetadata>,
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
        }
    }

    pub fn from_payload(payload: &DynamicSchemaPayload) -> Self {
        let name = payload.name.to_case(Case::Pascal);
        Self {
            name: name.clone(),
            filter: SubgraphFilterSpec {
                name: format!("{}Filter", name),
                fields: payload.fields.clone(),
            },
            subgraph_uuid: payload.subgraph_uuid,
            description: payload.description.clone(),
            fields: payload.fields.clone(),
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

        fields.push(
            registry
                .field::<&Uuid>("uuid", &())
                .description("The entry's UUID"),
        );
        fields.push(
            registry
                .field::<i32>("slot", &())
                .description("The block height that the entry was processed in"),
        );
        fields.push(
            registry
                .field::<&String>("transactionSignature", &())
                .description("The hash of the transaction that created this entry"),
        );

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
