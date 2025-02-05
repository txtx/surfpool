use std::{
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::Instant,
};

use jsonrpc_core::{
    futures::future::Either, middleware, FutureResponse, Metadata, Middleware, Request, Response,
};
use solana_client::{rpc_client::RpcClient, rpc_custom_error::RpcCustomError};
use solana_sdk::{clock::Slot, transaction::VersionedTransaction};
use tokio::sync::broadcast;

pub mod accounts_data;
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
    pub state: Arc<RwLock<GlobalState>>,
    pub mempool_tx: broadcast::Sender<VersionedTransaction>,
    pub rpc_config: RpcConfig,
}

impl RunloopContext {
    pub fn new(
        state: Arc<RwLock<GlobalState>>,
        mempool_tx: broadcast::Sender<VersionedTransaction>,
        rpc_config: RpcConfig,
    ) -> Self {
        Self {
            state,
            mempool_tx,
            rpc_config,
        }
    }
}

trait State {
    fn get_state<'a>(&'a self) -> Result<RwLockReadGuard<'a, GlobalState>, RpcCustomError>;
    fn get_state_mut<'a>(&'a self) -> Result<RwLockWriteGuard<'a, GlobalState>, RpcCustomError>;
    fn get_remote_rpc_client(&self) -> Result<RpcClient, RpcCustomError>;
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

    fn get_remote_rpc_client(&self) -> Result<RpcClient, RpcCustomError> {
        let Some(ctx) = self else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };
        Ok(RpcClient::new(&ctx.rpc_config.remote_rpc_url))
    }
}

impl Metadata for RunloopContext {}

use crate::{simnet::GlobalState, types::RpcConfig};
use jsonrpc_core::futures::FutureExt;
use std::future::Future;

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub context: Arc<RwLock<GlobalState>>,
    pub mempool_tx: broadcast::Sender<VersionedTransaction>,
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
        let meta = Some(RunloopContext::new(
            self.context.clone(),
            self.mempool_tx.clone(),
            self.config.clone(),
        ));
        // println!("Processing request {}: {:?}, {:?}", request_number, request, meta);

        Either::Left(Box::pin(next(request, meta).map(move |res| {
            // println!("Processing took: {:?}", start.elapsed());
            res
        })))
    }
}
