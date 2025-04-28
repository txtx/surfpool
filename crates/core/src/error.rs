use std::{future::Future, pin::Pin};

use crossbeam_channel::TrySendError;
use serde_json::json;

use jsonrpc_core::{Error, Result};
use solana_pubkey::Pubkey;

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
            format!("{}", code.description())
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

impl<T> Into<SurfpoolError> for TrySendError<T> {
    fn into(self) -> SurfpoolError {
        SurfpoolError::from_try_send_error(self)
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
}
