nexuse convert_case::{Case, Casing};
use juniper::{
    DefaultScalarValue, FieldError, FromInputValue, GraphQLType, GraphQLValue, InputValue,
    Registry,
    meta::{Argument, EnumValue, MetaType},
};
use serde::{Deserialize, Serialize};

use super::collections::FieldMetadata;
use crate::query::DataloaderContext;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubgraphFilterSpec {
    pub name: String,
    pub fields: Vec<FieldMetadata>,
}

impl GraphQLType<DefaultScalarValue> for SubgraphFilterSpec {
    fn name(spec: &SubgraphFilterSpec) -> Option<&str> {
        Some(&spec.name)
    }

    fn meta<'r>(spec: &SubgraphFilterSpec, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let mut args = vec![];
        for field in spec.fields.iter() {
            if !field.data.is_indexed {
                continue;
            }
            if field.is_bool() {
                args.push(registry.arg::<Option<BooleanFilter>>(&field.data.display_name, spec));
            } else if field.is_string() {
                args.push(registry.arg::<Option<StringFilter>>(&field.data.display_name, spec));
            } else if field.is_number() {
                args.push(registry.arg::<Option<NumericFilter>>(
                    &field.data.display_name,
                    &FieldInfo::new(&field.data.display_name),
                ));
            }
        }

        registry
            .build_input_object_type::<Self>(spec, &args)
            .into_meta()
    }
}

impl GraphQLValue<DefaultScalarValue> for SubgraphFilterSpec {
    type Context = ();
    type TypeInfo = SubgraphFilterSpec;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType<DefaultScalarValue>>::name(info)
    }
}

impl FromInputValue<DefaultScalarValue> for SubgraphFilterSpec {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        _: &InputValue<DefaultScalarValue>,
    ) -> Result<SubgraphFilterSpec, Self::Error> {
        unimplemented!()
    }
}

#[derive(Debug)]
pub struct FieldInfo {
    pub name: String,
}

impl FieldInfo {
    pub fn new(source: &str) -> FieldInfo {
        FieldInfo {
            name: source.to_string(),
        }
    }
}

#[derive(Debug)]
pub enum NumericFilter {
    Equals(i32),
    GreaterThan(i32),
    GreaterOrEqual(i32),
    LowerThan(i32),
    LowerOrEqual(i32),
    Between(i32, i32),
    NotEquals(i32),
}

impl GraphQLType<DefaultScalarValue> for NumericFilter {
    fn name(_: &FieldInfo) -> Option<&'static str> {
        Some("NumericFilter")
    }

    fn meta<'r>(spec: &FieldInfo, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let args = [
            build_number_filter_argument(&spec.name, "equals"),
            build_number_filter_argument(&spec.name, "not"),
            build_number_filter_argument(&spec.name, "gt"),
            build_number_filter_argument(&spec.name, "gte"),
            build_number_filter_argument(&spec.name, "lt"),
            build_number_filter_argument(&spec.name, "lte"),
        ];
        registry
            .build_input_object_type::<Self>(spec, &args)
            .into_meta()
    }
}

impl FromInputValue<DefaultScalarValue> for NumericFilter {
    type Error = FieldError<DefaultScalarValue>;

    fn from_input_value<'a>(
        v: &InputValue<DefaultScalarValue>,
    ) -> Result<NumericFilter, Self::Error> {
        println!("NumericFilter ==> {:?}", v);
        unimplemented!()
    }
}

impl GraphQLValue<DefaultScalarValue> for NumericFilter {
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
    fn name(_: &SubgraphFilterSpec) -> Option<&'static str> {
        Some("BooleanFilter")
    }

    fn meta<'r>(spec: &SubgraphFilterSpec, registry: &mut Registry<'r>) -> MetaType<'r>
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
    type TypeInfo = SubgraphFilterSpec;

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
    NotEquals(String),
    NotContains(String),
}

impl GraphQLType<DefaultScalarValue> for StringFilter {
    fn name(_: &SubgraphFilterSpec) -> Option<&'static str> {
        Some("StringFilter")
    }

    fn meta<'r>(spec: &SubgraphFilterSpec, registry: &mut Registry<'r>) -> MetaType<'r>
    where
        DefaultScalarValue: 'r,
    {
        let args = [
            build_string_filter_argument(&spec.name, "startsWith"),
            build_string_filter_argument(&spec.name, "endsWith"),
            build_string_filter_argument(&spec.name, "contains"),
            build_string_filter_argument(&spec.name, "equals"),
            build_string_filter_argument(&spec.name, "notEquals"),
            build_string_filter_argument(&spec.name, "notContains"),
        ];
        registry
            .build_input_object_type::<Self>(spec, &args)
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
    type TypeInfo = SubgraphFilterSpec;

    fn type_name<'i>(&self, info: &'i Self::TypeInfo) -> Option<&'i str> {
        <Self as GraphQLType>::name(info)
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
    _field_name: &str,
    filter: &str,
) -> Argument<'r, DefaultScalarValue> {
    Argument::new(
        filter,
        juniper::Type::Named(std::borrow::Cow::Borrowed("Int")),
    )
    .description(&format!(
        "Keep entries with a value {} to another value",
        filter.to_case(Case::Sentence).to_lowercase()
    ))
}

pub fn build_string_filter_argument<'r>(
    _field_name: &str,
    filter: &str,
) -> Argument<'r, DefaultScalarValue> {
    Argument::new(
        filter,
        juniper::Type::Named(std::borrow::Cow::Borrowed("String")),
    )
    .description(&format!(
        "Keep entries which {} a given substring",
        filter.to_case(Case::Sentence).to_lowercase()
    ))
}
