use std::{
    sync::{
        atomic::{self, AtomicUsize},
        Arc, Mutex, RwLock,
    },
    time::Instant,
};

use jsonrpc_core::{
    futures::future::Either, middleware, FutureResponse, Middleware, Request, Response,
};
use litesvm::LiteSVM;
use minimal::RunloopChannel;
use serde_derive::{Deserialize, Serialize};
use solana_sdk::{clock::Slot, commitment_config::CommitmentLevel, transaction::Transaction};
use tokio::sync::broadcast;
pub mod minimal;
pub mod utils;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentConfig {
    pub commitment: CommitmentLevel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcContextConfig {
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    pub min_context_slot: Option<Slot>,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RpcHealthStatus {
    Ok,
    Behind { num_slots: Slot },
    Unknown,
}

// pub struct Router {

// }

// impl RequestMiddleware for Router {
//     fn on_request(&self, request: Request<Body>) -> RequestMiddlewareAction {
//         println!("{:?}", request);
//         RequestMiddlewareAction::Proceed { should_continue_on_invalid_cors: true, request: request }
//     }
// }

use jsonrpc_core::futures::FutureExt;
use std::future::Future;

#[derive(Clone)]
pub struct SurfpoolMiddleware {
    pub svm: Arc<RwLock<LiteSVM>>,
    pub mempool_tx: broadcast::Sender<Transaction>,
}

impl Middleware<Option<RunloopChannel>> for SurfpoolMiddleware {
    type Future = FutureResponse;
    type CallFuture = middleware::NoopCallFuture;

    fn on_request<F, X>(
        &self,
        request: Request,
        _meta: Option<RunloopChannel>,
        next: F,
    ) -> Either<Self::Future, X>
    where
        F: FnOnce(Request, Option<RunloopChannel>) -> X + Send,
        X: Future<Output = Option<Response>> + Send + 'static,
    {
        let start = Instant::now();

        let meta = Some(RunloopChannel {
            state: self.svm.clone(),
            mempool_tx: self.mempool_tx.clone(),
        });
        // println!("Processing request {}: {:?}, {:?}", request_number, request, meta);

        Either::Left(Box::pin(next(request, meta).map(move |res| {
            println!("Processing took: {:?}", start.elapsed());
            res
        })))
    }
}
