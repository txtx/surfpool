use convert_case::{Case, Casing};
use juniper::{
    meta::{Argument, Field, MetaType},
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
};
use serde::{Deserialize, Serialize};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{subgraph::IndexedSubgraphField, SVM_PUBKEY};
use uuid::Uuid;

use crate::query::DataloaderContext;

use super::{
    scalars::{bigint::BigInt, pubkey::PublicKey},
    SubgraphFilterSpec,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicSchemaMetadata {
    pub name: String,
    pub subgraph_uuid: Uuid,
    pub description: Option<String>,
    pub fields: Vec<FieldMetadata>,
}

impl DynamicSchemaMetadata {
    pub fn new(
        uuid: &Uuid,
        name: &str,
        description: &Option<String>,
        fields: &Vec<IndexedSubgraphField>,
    ) -> Self {
        Self {
            name: name.to_case(Case::Pascal),
            subgraph_uuid: uuid.clone(),
            description: description.clone(),
            fields: fields.iter().map(|f| FieldMetadata::new(&f)).collect(),
        }
    }
}

impl GraphQLType<DefaultScalarValue> for DynamicSchemaMetadata {
    fn name(spec: &DynamicSchemaMetadata) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &DynamicSchemaMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
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

        registry.arg::<SubgraphFilterSpec>("filter", &spec);

        let mut object_meta = registry.build_object_type::<Self>(&spec, &fields);
        if let Some(description) = &spec.description {
            object_meta = object_meta.description(description.as_str());
        }

        object_meta.into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for DynamicSchemaMetadata {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicSchemaMetadata as GraphQLType<DefaultScalarValue>>::name(info)
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

pub fn build_bool_filter_argument<'r>(
    field_name: &str,
    suffix: &str,
    description: &str,
) -> Argument<'r, DefaultScalarValue> {
    let filter_name = format!("{}{}", field_name, suffix).to_case(Case::Camel);
    Argument::new(
        &filter_name,
        juniper::Type::Named(std::borrow::Cow::Borrowed("Boolean")),
    )
    .description(&format!(
        "return entities with '{}' {}",
        field_name, description
    ))
}

pub fn build_number_filter_argument<'r>(
    field_name: &str,
    filter: &str,
) -> Argument<'r, DefaultScalarValue> {
    Argument::new(
        &filter,
        juniper::Type::Named(std::borrow::Cow::Borrowed("i128")),
    )
    .description(&format!(
        "Filters in entities with a property '{}' {} a given value",
        field_name,
        filter.to_case(Case::Sentence).to_lowercase()
    ))
}
