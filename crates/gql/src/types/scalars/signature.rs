use std::{fmt, ops::Deref, str::FromStr};

use juniper::{GraphQLScalar, InputValue, ScalarValue, Value};
use solana_sdk::signature::Signature as SolSignature;

#[derive(Clone, Debug, Eq, GraphQLScalar, PartialEq)]
#[graphql(
    parse_token(String),
    description = "A 64-byte Transaction Signature",
    name = "Signature"
)]
pub struct Signature(pub SolSignature);

impl Signature {
    fn to_output<S: ScalarValue>(&self) -> Value<S> {
        Value::scalar(self.0.to_string())
    }

    fn from_input<S: ScalarValue>(v: &InputValue<S>) -> Result<Self, String> {
        v.as_string_value()
            .map(str::to_owned)
            .or_else(|| v.as_int_value().as_ref().map(ToString::to_string))
            .map(|s| {
                SolSignature::from_str(&s)
                    .map_err(|e| e.to_string())
                    .map(Self)
            })
            .ok_or_else(|| format!("Expected `String`, found: {v}"))?
    }
}

impl From<String> for Signature {
    fn from(s: String) -> Signature {
        Signature(SolSignature::from_str(&s).expect("invalid Signature"))
    }
}

impl Signature {
    /// Construct a new Signature from anything implementing `Into<String>`
    pub fn new<S: Into<String>>(value: S) -> Self {
        Signature::from(value.into())
    }
}

impl Deref for Signature {
    type Target = SolSignature;

    fn deref(&self) -> &SolSignature {
        &self.0
    }
}

impl Signature {
    /// Returns the string representation of the signature.
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
