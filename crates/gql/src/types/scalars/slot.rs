use std::str::FromStr;

use juniper::{GraphQLScalar, InputValue, ScalarValue, Value};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, GraphQLScalar, PartialEq)]
#[graphql(
    parse_token(String),
    description = "The slot at which a block is produced"
)]
pub struct Slot(pub u64);

impl Slot {
    fn to_output<S: ScalarValue>(&self) -> Value<S> {
        Value::scalar(self.0.to_string())
    }

    fn from_input<S: ScalarValue>(v: &InputValue<S>) -> Result<Self, String> {
        v.as_string_value()
            .ok_or_else(|| "Expected a string".to_string())
            .and_then(|s| {
                u64::from_str(s)
                    .map(Slot)
                    .map_err(|_| "Invalid slot".to_string())
            })
    }
}
