use std::{fmt::Display, future::Future, pin::Pin};

use crossbeam_channel::TrySendError;
use jsonrpc_core::{Error, Result};
use serde::Serialize;
use serde_json::json;
use solana_pubkey::Pubkey;

pub type SurfpoolResult<T> = std::result::Result<T, SurfpoolError>;

#[derive(Debug, Clone)]
pub struct SurfpoolError(Error);

impl From<SurfpoolError> for String {
    fn from(e: SurfpoolError) -> Self {
        e.0.to_string()
    }
}

impl From<SurfpoolError> for Error {
    fn from(e: SurfpoolError) -> Self {
        e.0
    }
}

impl std::error::Error for SurfpoolError {}

impl std::fmt::Display for SurfpoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let Error {
            code,
            message,
            data,
        } = &self.0;

        let core = if code.description().eq(message) {
            code.description()
        } else {
            format!("{}: {}", code.description(), message)
        };

        if let Some(data_value) = data {
            write!(f, "{}: {}", core, data_value.to_string().as_str())
        } else {
            write!(f, "{}", core)
        }
    }
}

impl<T> From<SurfpoolError> for Pin<Box<dyn Future<Output = Result<T>> + Send>> {
    fn from(e: SurfpoolError) -> Self {
        Box::pin(async move { Err(e.into()) })
    }
}

impl<T> From<TrySendError<T>> for SurfpoolError {
    fn from(val: TrySendError<T>) -> Self {
        SurfpoolError::from_try_send_error(val)
    }
}

impl SurfpoolError {
    pub fn from_try_send_error<T>(e: TrySendError<T>) -> Self {
        let mut error = Error::internal_error();
        error.data = Some(json!(format!(
            "Failed to send command on channel: {}",
            e.to_string()
        )));
        Self(error)
    }

    pub fn no_locker() -> Self {
        let mut error = Error::internal_error();
        error.data = Some(json!("Failed to access internal SVM state"));
        Self(error)
    }

    pub fn set_account<T>(pubkey: Pubkey, e: T) -> Self
    where
        T: ToString,
    {
        let mut error = Error::internal_error();
        error.data = Some(json!(format!(
            "Failed to set account {}: {}",
            pubkey,
            e.to_string()
        )));
        Self(error)
    }
    pub fn get_account<T>(pubkey: Pubkey, e: T) -> Self
    where
        T: ToString,
    {
        let mut error = Error::internal_error();
        error.data = Some(json!(format!(
            "Failed to fetch account {} from remote: {}",
            pubkey,
            e.to_string()
        )));
        Self(error)
    }

    pub fn get_multiple_accounts<T>(e: T) -> Self
    where
        T: ToString,
    {
        let mut error = Error::internal_error();
        error.data = Some(json!(format!(
            "Failed to fetch accounts from remote: {}",
            e.to_string()
        )));
        Self(error)
    }

    pub fn invalid_pubkey<D>(pubkey: &str, data: D) -> Self
    where
        D: Serialize,
    {
        let mut error = Error::invalid_params(format!("Invalid pubkey {pubkey}"));
        error.data = Some(json!(data));
        Self(error)
    }

    pub fn invalid_signature<D>(signature: &str, data: D) -> Self
    where
        D: Serialize,
    {
        let mut error = Error::invalid_params(format!("Invalid signature {signature}"));
        error.data = Some(json!(data));
        Self(error)
    }

    pub fn invalid_program_account<P, D>(program_id: P, data: D) -> Self
    where
        P: Display,
        D: Serialize,
    {
        let mut error = Error::invalid_params(format!("Invalid program account {program_id}"));
        error.data = Some(json!(data));
        Self(error)
    }

    pub fn expected_program_account<P>(program_id: P) -> Self
    where
        P: Display,
    {
        let error = Error::invalid_params(format!("Account {program_id} is not a program account"));
        Self(error)
    }

    pub fn account_not_found<P>(pubkey: P) -> Self
    where
        P: Display,
    {
        let error = Error::invalid_params(format!("Account {pubkey} not found"));
        Self(error)
    }

    pub fn invalid_account_data<P, D, M>(pubkey: P, data: D, message: Option<M>) -> Self
    where
        P: Display,
        D: Serialize,
        M: Display,
    {
        let base_msg = format!("invalid account data {pubkey}");
        let full_msg = if let Some(msg) = message {
            format!("{base_msg}: {msg}")
        } else {
            base_msg
        };
        let mut error = Error::invalid_params(full_msg);
        error.data = Some(json!(data));
        Self(error)
    }

    pub fn invalid_account_owner<P, M>(pubkey: P, message: Option<M>) -> Self
    where
        P: Display,
        M: Display,
    {
        let base_msg = format!("invalid account owner {pubkey}");
        let full_msg = if let Some(msg) = message {
            format!("{base_msg}: {msg}")
        } else {
            base_msg
        };
        let mut error = Error::invalid_params(full_msg);
        Self(error)
    }
    pub fn invalid_lookup_index<P>(pubkey: P) -> Self
    where
        P: Display,
    {
        let mut error =
            Error::invalid_params(format!("Address lookup {pubkey} contains an invalid index"));
        Self(error)
    }

    pub fn internal<D>(data: D) -> Self
    where
        D: Serialize,
    {
        let mut error = Error::internal_error();
        error.data = Some(json!(data));
        Self(error)
    }
}
