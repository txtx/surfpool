use crate::query::DataloaderContext;
use crate::types::schema::DynamicSchemaMetadata;
use juniper::graphql_object;
use juniper::meta::EnumValue;
use juniper::meta::Field;
use juniper::meta::MetaType;
use juniper::Arguments;
use juniper::DefaultScalarValue;
use juniper::Executor;
use juniper::FieldError;
use juniper::FromInputValue;
use juniper::GraphQLType;
use juniper::GraphQLValue;
use juniper::InputValue;
use juniper::Registry;
use scalars::bigint::BigInt;
use scalars::pubkey::PublicKey;
use schema::build_number_filter_argument;
use surfpool_types::SubgraphDataEntry;
use txtx_addon_kit::hex;
use txtx_addon_kit::types::types::Value;
use txtx_addon_network_svm_types::{SvmValue, SVM_PUBKEY};
use uuid::Uuid;

pub mod scalars;
pub mod schema;

#[derive(Debug)]
pub struct FieldInfo {
    name: String,
}

impl FieldInfo {
    pub fn new(source: &str) -> FieldInfo {
        FieldInfo {
            name: source.to_string(),
        }
    }
}

#[derive(Debug)]
pub enum NumberFilter {
    IsEqual(i32),
    GreaterThan(i32),
    GreaterOrEqual(i32),
    LowerThan(i32),
    LowerOrEqual(i32),
    Between(i32, i32),
}

impl GraphQLType<DefaultScalarValue> for NumberFilter {
    fn name(_: &FieldInfo) -> Option<&'static str> {
        Some("NumberFilter")
    }

    fn meta<'r>(spec: &FieldInfo, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let args = [
            build_number_filter_argument(&spec.name, "isEqual"),
            build_number_filter_argument(&spec.name, "greaterThan"),
            build_number_filter_argument(&spec.name, "greaterOrEqual"),
            build_number_filter_argument(&spec.name, "lowerThan"),
            build_number_filter_argument(&spec.name, "lowerOrEqual"),
        ];
        registry
            .build_input_object_type::<NumberFilter>(spec, &args)
            .into_meta()
    }
}

impl FromInputValue<DefaultScalarValue> for NumberFilter {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        v: &InputValue<DefaultScalarValue>,
    ) -> Result<NumberFilter, Self::Error> {
        println!("NumberFilter ==> {:?}", v);
        unimplemented!()
    }
}

impl GraphQLValue<DefaultScalarValue> for NumberFilter {
    type Context = DataloaderContext;
    type TypeInfo = FieldInfo;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType>::name(info)
    }
}

#[derive(Debug)]
pub enum BooleanFilter {
    False,
    True,
}

impl GraphQLType<DefaultScalarValue> for BooleanFilter {
    fn name(_: &DynamicSchemaMetadata) -> Option<&'static str> {
        Some("BooleanFilter")
    }

    fn meta<'r>(spec: &DynamicSchemaMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let args = [EnumValue::new("false"), EnumValue::new("true")];
        registry
            .build_enum_type::<BooleanFilter>(spec, &args)
            .into_meta()
    }
}

impl FromInputValue<DefaultScalarValue> for BooleanFilter {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        v: &InputValue<DefaultScalarValue>,
    ) -> Result<BooleanFilter, Self::Error> {
        println!("BooleanFilter ==> {:?}", v);
        unimplemented!()
    }
}

impl GraphQLValue<DefaultScalarValue> for BooleanFilter {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType>::name(info)
    }
}

#[derive(Debug)]
pub enum StringFilter {
    StartsWith(String),
    EndsWith(String),
    Contains(String),
    Equals(String),
}

impl GraphQLType<DefaultScalarValue> for StringFilter {
    fn name(_: &DynamicSchemaMetadata) -> Option<&'static str> {
        Some("StringFilter")
    }

    fn meta<'r>(spec: &DynamicSchemaMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let args = [EnumValue::new("false"), EnumValue::new("true")];
        registry
            .build_enum_type::<StringFilter>(spec, &args)
            .into_meta()
    }
}

impl FromInputValue<DefaultScalarValue> for StringFilter {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        v: &InputValue<DefaultScalarValue>,
    ) -> Result<StringFilter, Self::Error> {
        println!("StringFilter ==> {:?}", v);
        unimplemented!()
    }
}

impl GraphQLValue<DefaultScalarValue> for StringFilter {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType>::name(info)
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct SubgraphFilterSpec {}

impl GraphQLType<DefaultScalarValue> for SubgraphFilterSpec {
    fn name(_: &DynamicSchemaMetadata) -> Option<&'static str> {
        Some("SubgraphFilterSpec")
    }

    fn meta<'r>(spec: &DynamicSchemaMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut args = vec![
            registry.arg::<Option<NumberFilter>>("blockHeight", &FieldInfo::new("blockHeight"))
        ];

        for field in spec.fields.iter() {
            if field.is_bool() {
                args.push(registry.arg::<Option<BooleanFilter>>(&field.name, &spec));
            } else if field.is_string() {
                args.push(registry.arg::<Option<StringFilter>>(&field.name, &spec));
            } else if field.is_number() {
                args.push(
                    registry.arg::<Option<NumberFilter>>(&field.name, &FieldInfo::new(&field.name)),
                );
            }
        }
        registry
            .build_input_object_type::<Self>(spec, &args)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SubgraphFilterSpec {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType>::name(info)
    }
}

impl FromInputValue<DefaultScalarValue> for SubgraphFilterSpec {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        v: &InputValue<DefaultScalarValue>,
    ) -> Result<SubgraphFilterSpec, Self::Error> {
        println!("SubgraphFilterSpec ==> {:?}", v);
        unimplemented!()
    }
}

#[derive(Debug, Clone)]
pub struct SubgraphSpec(pub SubgraphDataEntry);

impl GraphQLType<DefaultScalarValue> for SubgraphSpec {
    fn name(spec: &DynamicSchemaMetadata) -> Option<&str> {
        Some(spec.name.as_str())
    }

    fn meta<'r>(spec: &DynamicSchemaMetadata, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut fields: Vec<Field<'r, DefaultScalarValue>> = vec![];
        fields.push(registry.field::<&Uuid>("uuid", &()));
        for field_metadata in spec.fields.iter() {
            let field = field_metadata.register_as_scalar(registry);
            fields.push(field);
        }

        registry.arg::<SubgraphFilterSpec>("filter", &spec);
        registry
            .build_object_type::<[SubgraphSpec]>(&spec, &fields)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SubgraphSpec {
    type Context = DataloaderContext;
    type TypeInfo = DynamicSchemaMetadata;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <SubgraphSpec as GraphQLType>::name(info)
    }

    fn resolve_field(
        &self,
        _info: &DynamicSchemaMetadata,
        field_name: &str,
        _args: &Arguments,
        executor: &Executor<DataloaderContext>,
    ) -> Result<juniper::Value, FieldError> {
        let entry = &self.0;
        match field_name {
            "uuid" => executor.resolve_with_ctx(&(), &entry.uuid.to_string()),
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
