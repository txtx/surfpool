use convert_case::{Case, Casing};
use juniper::{
    meta::{Field, MetaType},
    DefaultScalarValue, GraphQLType, GraphQLValue, Registry,
};
use txtx_addon_kit::types::types::Type;
use txtx_addon_network_svm_types::{subgraph::IndexedSubgraphField, SVM_PUBKEY};
use uuid::Uuid;

use crate::Context;

use super::scalars::{bigint::BigInt, pubkey::PublicKey};

#[derive(Clone, Debug)]
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
            fields.push(field_metadata.register_as_scalar(registry));
        }

        let mut object_meta = registry.build_object_type::<DynamicSchemaMetadata>(&spec, &fields);
        if let Some(description) = &spec.description {
            object_meta = object_meta.description(description.as_str());
        }
        object_meta.into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for DynamicSchemaMetadata {
    type Context = Context;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <DynamicSchemaMetadata as GraphQLType<DefaultScalarValue>>::name(info)
    }
}

#[derive(Clone, Debug)]
pub struct FieldMetadata {
    name: String,
    ty: Type,
    description: Option<String>,
}
impl FieldMetadata {
    pub fn new(field: &IndexedSubgraphField) -> Self {
        Self {
            name: field.display_name.clone(),
            ty: field.expected_type.clone(),
            description: field.description.clone(),
        }
    }
    pub fn register_as_scalar<'r>(
        &self,
        registry: &mut Registry<'r>,
    ) -> Field<'r, DefaultScalarValue> {
        let field_name = self.name.as_str();

        let mut field = match &self.ty {
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
