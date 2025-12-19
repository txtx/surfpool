use txtx_addon_kit::{
    indexmap::IndexMap,
    types::types::{AddonJsonConverter, Value},
};

use crate::error::{SurfpoolError, SurfpoolResult};

pub fn get_txtx_value_json_converters() -> Vec<AddonJsonConverter<'static>> {
    vec![
        Box::new(move |value: &txtx_addon_kit::types::types::Value| {
            txtx_addon_network_svm_types::SvmValue::to_json(value)
        }) as AddonJsonConverter<'static>,
    ]
}

/// Helper function to apply an override to a decoded account value using dot notation
pub fn apply_override_to_decoded_account(
    decoded_value: &mut Value,
    path: &str,
    value: &serde_json::Value,
) -> SurfpoolResult<()> {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.is_empty() {
        return Err(SurfpoolError::internal("Empty path provided for override"));
    }

    // Navigate to the parent of the target field
    let mut current = decoded_value;
    for part in &parts[..parts.len() - 1] {
        match current {
            Value::Object(map) => {
                current = map.get_mut(&part.to_string()).ok_or_else(|| {
                    SurfpoolError::internal(format!(
                        "Path segment '{}' not found in decoded account",
                        part
                    ))
                })?;
            }
            _ => {
                return Err(SurfpoolError::internal(format!(
                    "Cannot navigate through field '{}' - not an object",
                    part
                )));
            }
        }
    }

    // Set the final field
    let final_key = parts[parts.len() - 1];
    match current {
        Value::Object(map) => {
            // Convert serde_json::Value to txtx Value
            let txtx_value = json_to_txtx_value(value)?;
            map.insert(final_key.to_string(), txtx_value);
            Ok(())
        }
        _ => Err(SurfpoolError::internal(format!(
            "Cannot set field '{}' - parent is not an object",
            final_key
        ))),
    }
}

/// Helper function to convert serde_json::Value to txtx Value
fn json_to_txtx_value(json: &serde_json::Value) -> SurfpoolResult<Value> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i as i128))
            } else if let Some(u) = n.as_u64() {
                Ok(Value::Integer(u as i128))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err(SurfpoolError::internal(format!(
                    "Unable to convert number: {}",
                    n
                )))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let txtx_arr: Result<Vec<Value>, _> = arr.iter().map(json_to_txtx_value).collect();
            Ok(Value::Array(Box::new(txtx_arr?)))
        }
        serde_json::Value::Object(obj) => {
            let mut txtx_obj = IndexMap::new();
            for (k, v) in obj.iter() {
                txtx_obj.insert(k.clone(), json_to_txtx_value(v)?);
            }
            Ok(Value::Object(txtx_obj))
        }
    }
}

/// Helper function to find if the account data starts with the given IDL discriminator
/// ## Inputs
/// - `account_data`: The raw account data bytes
/// - `idl_discriminator`: The IDL discriminator bytes to match against
/// ## Returns
/// - `true` if the account data starts with the IDL discriminator, `false` otherwise
pub fn find_discriminator(account_data: &[u8], idl_discriminator: &[u8]) -> bool {
    let idl_discriminator_len = idl_discriminator.len();
    account_data.len() >= idl_discriminator_len
        && account_data[..idl_discriminator_len].eq(idl_discriminator)
}
