#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod rpc;
pub mod simnet;

use std::sync::mpsc::Sender;

pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
use simnet::SimnetEvent;
pub use solana_rpc_client;
pub use solana_sdk;

pub async fn start_simnet(
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    simnet::start(simnet_events_tx).await?;
    Ok(())
}
