use std::{future::Future, pin::Pin};

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

impl From<SurfpoolError> for Box<dyn std::error::Error + Send + Sync> {
    fn from(e: SurfpoolError) -> Self {
        Box::new(e.0)
    }
}

impl<T> From<SurfpoolError> for Pin<Box<dyn Future<Output = Result<T>> + Send>> {
    fn from(e: SurfpoolError) -> Self {
        Box::pin(async move { Err(e.into()) })
    }
}

impl SurfpoolError {
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
