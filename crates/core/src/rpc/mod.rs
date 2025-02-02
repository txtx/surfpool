use std::{
    sync::{Arc, RwLock, RwLockReadGuard},
    time::Instant,
};

use jsonrpc_core::{
    futures::future::Either, middleware, FutureResponse, Metadata, Middleware, Request, Response,
};
use solana_client::rpc_custom_error::RpcCustomError;
use solana_sdk::{clock::Slot, transaction::VersionedTransaction};
use tokio::sync::broadcast;

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
}

trait State {
    fn get_state<'a>(&'a self) -> Result<RwLockReadGuard<'a, GlobalState>, RpcCustomError>;
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
        let Ok(state_reader) = ctx.state.try_read() else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        Ok(state_reader)
    }
}

impl Metadata for RunloopContext {}

use crate::simnet::GlobalState;
use jsonrpc_core::futures::FutureExt;
use std::future::Future;

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub context: Arc<RwLock<GlobalState>>,
    pub mempool_tx: broadcast::Sender<VersionedTransaction>,
    pub config: Config,
}

#[derive(Clone)]
pub struct Config {}

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
        let start = Instant::now();

        let meta = Some(RunloopContext {
            state: self.context.clone(),
            mempool_tx: self.mempool_tx.clone(),
        });
        // println!("Processing request {}: {:?}, {:?}", request_number, request, meta);

        Either::Left(Box::pin(next(request, meta).map(move |res| {
            println!("Processing took: {:?}", start.elapsed());
            res
        })))
    }
}
