use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crossbeam_channel::Sender;
use jsonrpc_core::{
    futures::future::Either, middleware, BoxFuture, Error, FutureResponse, Metadata, Middleware,
    Request, Response,
};
use solana_client::rpc_custom_error::RpcCustomError;
use solana_sdk::{blake3::Hash, clock::Slot};

pub mod accounts_data;
pub mod accounts_scan;
pub mod admin;
pub mod bank_data;
pub mod full;
pub mod minimal;
pub mod utils;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RpcHealthStatus {
    Ok,
    Behind { num_slots: Slot },
    Unknown,
}

pub struct SurfpoolRpc;

#[derive(Clone)]
pub struct RunloopContext {
    pub id: Hash,
    pub state: Arc<RwLock<GlobalState>>,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
}

trait State {
    fn get_state<'a>(&'a self) -> Result<RwLockReadGuard<'a, GlobalState>, RpcCustomError>;
    fn get_state_mut<'a>(&'a self) -> Result<RwLockWriteGuard<'a, GlobalState>, RpcCustomError>;
}

impl State for Option<RunloopContext> {
    fn get_state<'a>(&'a self) -> Result<RwLockReadGuard<'a, GlobalState>, RpcCustomError> {
        // Retrieve svm state
        let Some(ctx) = self else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        // Lock read access
        ctx.state.read().map_err(|_| RpcCustomError::NodeUnhealthy {
            num_slots_behind: None,
        })
    }

    fn get_state_mut<'a>(&'a self) -> Result<RwLockWriteGuard<'a, GlobalState>, RpcCustomError> {
        // Retrieve svm state
        let Some(ctx) = self else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        // Lock write access to get a mutable reference
        ctx.state
            .write()
            .map_err(|_| RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            })
    }
}

impl Metadata for RunloopContext {}

use crate::types::GlobalState;
use jsonrpc_core::futures::FutureExt;
use std::future::Future;
use surfpool_types::{
    types::{PluginManagerCommand, RpcConfig},
    SimnetCommand,
};

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub context: Arc<RwLock<GlobalState>>,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
    pub config: RpcConfig,
}

impl Middleware<Option<RunloopContext>> for SurfpoolMiddleware {
    type Future = FutureResponse;
    type CallFuture = middleware::NoopCallFuture;

    fn on_request<F, X>(
        &self,
        request: Request,
        _meta: Option<RunloopContext>,
        next: F,
    ) -> Either<Self::Future, X>
    where
        F: FnOnce(Request, Option<RunloopContext>) -> X + Send,
        X: Future<Output = Option<Response>> + Send + 'static,
    {
        let meta = Some(RunloopContext {
            id: Hash::new_unique(),
            state: self.context.clone(),
            simnet_commands_tx: self.simnet_commands_tx.clone(),
            plugin_manager_commands_tx: self.plugin_manager_commands_tx.clone(),
        });
        Either::Left(Box::pin(next(request, meta).map(move |res| res)))
    }
}

pub const NOT_IMPLEMENTED_CODE: i64 = -32051; // -32000 to -32099 are reserved by the json-rpc spec for custom errors
pub const NOT_IMPLEMENTED_MSG: &str = "Method not yet implemented. If this endpoint is a priority for you, please open an issue here so we can prioritize: https://github.com/txtx/surfpool/issues";
/// Helper function to return a `NotImplemented` JSON RPC error
pub fn not_implemented_err<T>() -> Result<T, Error> {
    Err(Error {
        code: jsonrpc_core::types::ErrorCode::ServerError(NOT_IMPLEMENTED_CODE),
        message: NOT_IMPLEMENTED_MSG.to_string(),
        data: None,
    })
}

pub fn not_implemented_err_async<T>() -> BoxFuture<Result<T, Error>> {
    Box::pin(async {
        Err(Error {
            code: jsonrpc_core::types::ErrorCode::ServerError(NOT_IMPLEMENTED_CODE),
            message: NOT_IMPLEMENTED_MSG.to_string(),
            data: None,
        })
    })
}
