use std::{future::Future, net::SocketAddr, sync::Arc};

use blake3::Hash;
use crossbeam_channel::Sender;
use jsonrpc_core::{
    BoxFuture, Error, FutureResponse, Metadata, Middleware, Request, Response,
    futures::{FutureExt, future::Either},
    middleware,
};
use jsonrpc_pubsub::{PubSubMetadata, Session};
use solana_clock::Slot;
use surfpool_types::{SimnetCommand, SimnetEvent, types::RpcConfig};

use crate::{
    PluginManagerCommand,
    error::{SurfpoolError, SurfpoolResult},
    surfnet::{
        locker::SurfnetSvmLocker,
        remote::{SomeRemoteCtx, SurfnetRemoteClient},
        svm::SurfnetSvm,
    },
};

pub mod accounts_data;
pub mod accounts_scan;
pub mod admin;
pub mod bank_data;
pub mod full;
pub mod minimal;
pub mod surfnet_cheatcodes;
pub mod utils;
pub mod ws;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RpcHealthStatus {
    Ok,
    Behind { num_slots: Slot },
    Unknown,
}

pub struct SurfpoolRpc;

#[derive(Clone)]
pub struct RunloopContext {
    pub id: Option<(Hash, String)>,
    pub svm_locker: SurfnetSvmLocker,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
    pub remote_rpc_client: Option<SurfnetRemoteClient>,
    pub rpc_addr: Option<SocketAddr>,
}

pub struct SurfnetRpcContext<T> {
    pub svm_locker: SurfnetSvmLocker,
    pub remote_ctx: Option<(SurfnetRemoteClient, T)>,
}

trait State {
    fn get_svm_locker(&self) -> SurfpoolResult<SurfnetSvmLocker>;
    fn with_svm_reader<T, F>(&self, reader: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static;
    fn get_rpc_context<T>(&self, input: T) -> SurfpoolResult<SurfnetRpcContext<T>>;
    fn get_surfnet_command_tx(&self) -> SurfpoolResult<Sender<SimnetCommand>>;
}

impl State for Option<RunloopContext> {
    fn get_svm_locker(&self) -> SurfpoolResult<SurfnetSvmLocker> {
        // Retrieve svm state
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.svm_locker.clone())
    }

    fn with_svm_reader<T, F>(&self, reader: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.svm_locker.with_svm_reader(reader))
    }

    fn get_rpc_context<T>(&self, input: T) -> SurfpoolResult<SurfnetRpcContext<T>> {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };

        Ok(SurfnetRpcContext {
            svm_locker: ctx.svm_locker.clone(),
            remote_ctx: ctx.remote_rpc_client.get_remote_ctx(input),
        })
    }

    fn get_surfnet_command_tx(&self) -> SurfpoolResult<Sender<SimnetCommand>> {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.simnet_commands_tx.clone())
    }
}

impl Metadata for RunloopContext {}

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub surfnet_svm: SurfnetSvmLocker,
    pub simnet_commands_tx: Sender<SimnetCommand>,
    pub plugin_manager_commands_tx: Sender<PluginManagerCommand>,
    pub config: RpcConfig,
    pub remote_rpc_client: Option<SurfnetRemoteClient>,
}

impl SurfpoolMiddleware {
    pub fn new(
        surfnet_svm: SurfnetSvmLocker,
        simnet_commands_tx: &Sender<SimnetCommand>,
        plugin_manager_commands_tx: &Sender<PluginManagerCommand>,
        config: &RpcConfig,
        remote_rpc_client: &Option<SurfnetRemoteClient>,
    ) -> Self {
        Self {
            surfnet_svm,
            simnet_commands_tx: simnet_commands_tx.clone(),
            plugin_manager_commands_tx: plugin_manager_commands_tx.clone(),
            config: config.clone(),
            remote_rpc_client: remote_rpc_client.clone(),
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
        let rpc_addr = self.config.get_rpc_base_url().parse::<SocketAddr>().ok();
        let meta = Some(RunloopContext {
            id: None,
            svm_locker: self.surfnet_svm.clone(),
            simnet_commands_tx: self.simnet_commands_tx.clone(),
            plugin_manager_commands_tx: self.plugin_manager_commands_tx.clone(),
            remote_rpc_client: self.remote_rpc_client.clone(),
            rpc_addr,
        });
        Either::Left(Box::pin(next(request, meta).map(move |res| res)))
    }
}

#[derive(Clone)]
pub struct SurfpoolWebsocketMiddleware {
    pub surfpool_middleware: SurfpoolMiddleware,
    pub session: Option<Arc<Session>>,
}

impl SurfpoolWebsocketMiddleware {
    pub fn new(surfpool_middleware: SurfpoolMiddleware, session: Option<Arc<Session>>) -> Self {
        Self {
            surfpool_middleware,
            session,
        }
    }
}

impl Middleware<Option<SurfpoolWebsocketMeta>> for SurfpoolWebsocketMiddleware {
    type Future = FutureResponse;
    type CallFuture = middleware::NoopCallFuture;

    fn on_request<F, X>(
        &self,
        request: Request,
        meta: Option<SurfpoolWebsocketMeta>,
        next: F,
    ) -> Either<Self::Future, X>
    where
        F: FnOnce(Request, Option<SurfpoolWebsocketMeta>) -> X + Send,
        X: Future<Output = Option<Response>> + Send + 'static,
    {
        let rpc_addr = self.surfpool_middleware.config.get_rpc_base_url().parse::<SocketAddr>().ok();
        let runloop_context = RunloopContext {
            id: None,
            svm_locker: self.surfpool_middleware.surfnet_svm.clone(),
            simnet_commands_tx: self.surfpool_middleware.simnet_commands_tx.clone(),
            plugin_manager_commands_tx: self.surfpool_middleware.plugin_manager_commands_tx.clone(),
            remote_rpc_client: self.surfpool_middleware.remote_rpc_client.clone(),
            rpc_addr,
        };
        let session = meta
            .as_ref()
            .and_then(|m| m.session.clone())
            .or(self.session.clone());
        let meta = Some(SurfpoolWebsocketMeta::new(runloop_context, session));
        Either::Left(Box::pin(next(request, meta).map(move |res| res)))
    }
}

#[derive(Clone)]
pub struct SurfpoolWebsocketMeta {
    pub runloop_context: RunloopContext,
    pub session: Option<Arc<Session>>,
}

impl SurfpoolWebsocketMeta {
    pub fn new(runloop_context: RunloopContext, session: Option<Arc<Session>>) -> Self {
        Self {
            runloop_context,
            session,
        }
    }

    pub fn log_debug(&self, msg: &str) {
        let _ = self
            .runloop_context
            .svm_locker
            .simnet_events_tx()
            .send(SimnetEvent::debug(msg));
    }

    pub fn log_warn(&self, msg: &str) {
        let _ = self
            .runloop_context
            .svm_locker
            .simnet_events_tx()
            .send(SimnetEvent::warn(msg));
    }
}

impl State for Option<SurfpoolWebsocketMeta> {
    fn get_svm_locker(&self) -> SurfpoolResult<SurfnetSvmLocker> {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.runloop_context.svm_locker.clone())
    }

    fn with_svm_reader<T, F>(&self, reader: F) -> Result<T, SurfpoolError>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.runloop_context.svm_locker.with_svm_reader(reader))
    }

    fn get_rpc_context<T>(&self, input: T) -> SurfpoolResult<SurfnetRpcContext<T>> {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };

        Ok(SurfnetRpcContext {
            svm_locker: ctx.runloop_context.svm_locker.clone(),
            remote_ctx: ctx.runloop_context.remote_rpc_client.get_remote_ctx(input),
        })
    }

    fn get_surfnet_command_tx(&self) -> SurfpoolResult<Sender<SimnetCommand>> {
        let Some(ctx) = self else {
            return Err(SurfpoolError::missing_context());
        };
        Ok(ctx.runloop_context.simnet_commands_tx.clone())
    }
}

impl Metadata for SurfpoolWebsocketMeta {}
impl PubSubMetadata for SurfpoolWebsocketMeta {
    fn session(&self) -> Option<Arc<jsonrpc_pubsub::Session>> {
        self.session.clone()
    }
}

pub const NOT_IMPLEMENTED_CODE: i64 = -32051; // -32000 to -32099 are reserved by the json-rpc spec for custom errors
pub const NOT_IMPLEMENTED_MSG: &str = "Method not yet implemented. If this endpoint is a priority for you, please open an issue here so we can prioritize: https://github.com/txtx/surfpool/issues";
fn not_implemented_msg(method: &str) -> String {
    format!(
        "Method `{}` is not yet implemented. If this endpoint is a priority for you, please open an issue here so we can prioritize: https://github.com/txtx/surfpool/issues",
        method
    )
}
/// Helper function to return a `NotImplemented` JSON RPC error
pub fn not_implemented_err<T>(method: &str) -> Result<T, Error> {
    Err(Error {
        code: jsonrpc_core::types::ErrorCode::ServerError(NOT_IMPLEMENTED_CODE),
        message: not_implemented_msg(method),
        data: None,
    })
}

pub fn not_implemented_err_async<T>(method: &str) -> BoxFuture<Result<T, Error>> {
    let method = method.to_string();
    Box::pin(async move {
        Err(Error {
            code: jsonrpc_core::types::ErrorCode::ServerError(NOT_IMPLEMENTED_CODE),
            message: not_implemented_msg(&method),
            data: None,
        })
    })
}
