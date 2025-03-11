use std::{fmt, ops::Deref, str::FromStr};

use juniper::{GraphQLScalar, InputValue, ScalarValue, Value};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

#[derive(Clone, Debug, Deserialize, Eq, GraphQLScalar, PartialEq, Serialize)]
#[graphql(
    parse_token(String),
    description = "A public key in the SVM network",
    name = "Pubkey"
)]
pub struct PublicKey(pub Pubkey);

impl PublicKey {
    fn to_output<S: ScalarValue>(&self) -> Value<S> {
        Value::scalar(self.0.to_string())
    }

    fn from_input<S: ScalarValue>(v: &InputValue<S>) -> Result<Self, String> {
        v.as_string_value()
            .map(str::to_owned)
            .or_else(|| v.as_int_value().as_ref().map(ToString::to_string))
            .map(|s| Pubkey::from_str(&s).map_err(|e| e.to_string()).map(Self))
            .ok_or_else(|| format!("Expected `String`, found: {v}"))?
    }
}

impl From<String> for PublicKey {
    fn from(s: String) -> PublicKey {
        PublicKey(Pubkey::from_str(&s).expect("invalid pubkey"))
    }
}

impl PublicKey {
    /// Construct a new PublicKey from anything implementing `Into<String>`
    pub fn new<S: Into<String>>(value: S) -> Self {
        PublicKey::from(value.into())
    }
}

impl Deref for PublicKey {
    type Target = str;

    fn deref(&self) -> &str {
        std::str::from_utf8(self.0.as_ref()).expect("invalid pubkey")
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
