use std::{fmt, ops::Deref, str::FromStr};

use juniper::{GraphQLScalar, InputValue, ScalarValue, Value};
use serde::{Deserialize, Serialize};
use solana_sdk::blake3::Hash as SolHash;

#[derive(Clone, Debug, Deserialize, Eq, GraphQLScalar, PartialEq, Serialize)]
#[graphql(parse_token(String), description = "A 32-byte hash", name = "Hash")]
pub struct Hash(pub SolHash);

impl Hash {
    fn to_output<S: ScalarValue>(&self) -> Value<S> {
        Value::scalar(self.0.to_string())
    }

    fn from_input<S: ScalarValue>(v: &InputValue<S>) -> Result<Self, String> {
        v.as_string_value()
            .map(str::to_owned)
            .or_else(|| v.as_int_value().as_ref().map(ToString::to_string))
            .map(|s| SolHash::from_str(&s).map_err(|e| e.to_string()).map(Self))
            .ok_or_else(|| format!("Expected `String`, found: {v}"))?
    }
}

impl From<String> for Hash {
    fn from(s: String) -> Hash {
        Hash(SolHash::from_str(&s).expect("invalid hash"))
    }
}

impl Hash {
    /// Construct a new Hash from anything implementing `Into<String>`
    pub fn new<S: Into<String>>(value: S) -> Self {
        Hash::from(value.into())
    }
}

impl Deref for Hash {
    type Target = str;

    fn deref(&self) -> &str {
        std::str::from_utf8(self.0.as_ref()).expect("invalid hash")
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
