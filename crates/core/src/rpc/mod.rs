use blake3::Hash;
use crossbeam_channel::Sender;
use jsonrpc_core::{
    futures::future::Either, middleware, BoxFuture, Error, FutureResponse, Metadata, Middleware,
    Request, Response,
};
use solana_clock::Slot;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod accounts_data;
pub mod accounts_scan;
pub mod admin;
pub mod bank_data;
pub mod full;
pub mod minimal;
pub mod svm_tricks;
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
    pub id: Option<Hash>,
    pub surfnet_svm: Arc<RwLock<SurfnetSvm>>,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
}

pub type SvmReadClosure<T> = Box<dyn Fn(&SurfnetSvm) -> T + Send + Sync>;

trait State {
    fn get_svm_locker(&self) -> Result<Arc<RwLock<SurfnetSvm>>, SurfpoolError>;
    fn with_svm_reader<T, F>(&self, reader: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static;
    fn with_svm_writer<T, F>(&self, writer: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&mut SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static;
}

impl State for Option<RunloopContext> {
    fn get_svm_locker(&self) -> Result<Arc<RwLock<SurfnetSvm>>, SurfpoolError> {
        // Retrieve svm state
        let Some(ctx) = self else {
            return Err(SurfpoolError::no_locker());
        };
        Ok(ctx.surfnet_svm.clone())
    }

    fn with_svm_reader<T, F>(&self, reader: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let Some(ctx) = self else {
            return Err(SurfpoolError::no_locker());
        };
        let read_lock = ctx.surfnet_svm.clone();
        let res = tokio::task::block_in_place(move || {
            let read_guard = read_lock.blocking_read();
            reader(&read_guard)
        });
        Ok(res)
    }

    fn with_svm_writer<T, F>(&self, writer: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&mut SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let Some(ctx) = self else {
            return Err(SurfpoolError::no_locker());
        };
        let read_lock = ctx.surfnet_svm.clone();
        let res = tokio::task::block_in_place(move || {
            let mut read_guard = read_lock.blocking_write();
            writer(&mut read_guard)
        });
        Ok(res)
    }
}

impl Metadata for RunloopContext {}

use crate::PluginManagerCommand;
use crate::{error::SurfpoolError, surfnet::SurfnetSvm};
use jsonrpc_core::futures::FutureExt;
use std::future::Future;
use surfpool_types::{types::RpcConfig, SimnetCommand};

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub surfnet_svm: Arc<RwLock<SurfnetSvm>>,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
    pub config: RpcConfig,
}

impl SurfpoolMiddleware {
    pub fn new(
        surfnet_svm: Arc<RwLock<SurfnetSvm>>,
        simnet_commands_tx: &Sender<SimnetCommand>,
        plugin_manager_commands_tx: &Sender<PluginManagerCommand>,
        config: &RpcConfig,
    ) -> Self {
        Self {
            surfnet_svm,
            simnet_commands_tx: simnet_commands_tx.clone(),
            plugin_manager_commands_tx: plugin_manager_commands_tx.clone(),
            config: config.clone(),
        }
    }
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
            id: None,
            surfnet_svm: self.surfnet_svm.clone(),
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
