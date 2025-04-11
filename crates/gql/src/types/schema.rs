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

use crate::query::DataloaderContext;

use super::{
    filters::SubgraphFilterSpec,
    scalars::{bigint::BigInt, pubkey::PublicKey},
};

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
        let fields: Vec<_> = request
            .fields
            .iter()
            .map(|f| FieldMetadata::new(&f))
            .collect();
        Self {
            name: name.clone(),
            filter: SubgraphFilterSpec {
                name: format!("{}Filter", name),
                fields: fields.clone(),
            },
            subgraph_uuid: uuid.clone(),
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
            subgraph_uuid: payload.subgraph_uuid.clone(),
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

        for field_metadata in spec.fields.iter() {
            let field = field_metadata.register_as_scalar(registry);
            fields.push(field);
        }

        let mut object_meta = registry.build_object_type::<Self>(&spec, &fields);
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
    pub name: String,
    pub typing: Type,
    description: Option<String>,
}

impl FieldMetadata {
    pub fn new(field: &IndexedSubgraphField) -> Self {
        Self {
            name: field.display_name.clone(),
            typing: field.expected_type.clone(),
            description: field.description.clone(),
        }
    }

    pub fn is_bool(&self) -> bool {
        self.typing.eq(&Type::Bool)
    }

    pub fn is_string(&self) -> bool {
        self.typing.eq(&Type::String)
    }

    pub fn is_number(&self) -> bool {
        self.typing.eq(&Type::Float) || self.typing.eq(&Type::Integer)
    }

    pub fn register_as_scalar<'r>(
        &self,
        registry: &mut Registry<'r>,
    ) -> Field<'r, DefaultScalarValue> {
        let field_name = self.name.as_str();
        let mut field = match &self.typing {
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
        if let Some(description) = &self.description {
            field = field.description(description.as_str());
        }
        field
    }
}
