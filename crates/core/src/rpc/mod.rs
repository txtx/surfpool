use jsonrpc_http_server::{hyper::{Body, Request}, RequestMiddleware, RequestMiddlewareAction};
use solana_sdk::{clock::Slot, commitment_config::CommitmentLevel};
use serde_derive::{Deserialize, Serialize};
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
    Behind { num_slots: Slot }, // Validator is behind its known validators
    Unknown,
}

pub struct Router {

}

impl RequestMiddleware for Router {
    fn on_request(&self, request: Request<Body>) -> RequestMiddlewareAction {
        println!("{:?}", request);
        RequestMiddlewareAction::Proceed { should_continue_on_invalid_cors: true, request: request }
    }

}