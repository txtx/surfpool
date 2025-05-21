use std::str::FromStr;

use juniper::{GraphQLScalar, InputValue, ScalarValue, Value};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, GraphQLScalar, PartialEq)]
#[graphql(parse_token(String), description = "A 128-bit integer", name = "i128")]
pub struct BigInt(pub i128);

impl BigInt {
    fn to_output<S: ScalarValue>(&self) -> Value<S> {
        Value::scalar(self.0.to_string())
    }

    fn from_input<S: ScalarValue>(v: &InputValue<S>) -> Result<Self, String> {
        v.as_string_value()
            .ok_or_else(|| "Expected a string".to_string())
            .and_then(|s| {
                i128::from_str(s)
                    .map(BigInt)
                    .map_err(|_| "Invalid i128".to_string())
            })
    }
}
