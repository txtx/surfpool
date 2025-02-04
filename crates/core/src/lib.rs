#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod rpc;
pub mod simnet;
pub mod types;

use crossbeam_channel::Sender;
pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
use simnet::SimnetEvent;
pub use solana_rpc_client;
pub use solana_sdk;
use types::SurfpoolConfig;

pub async fn start_simnet(
    config: &SurfpoolConfig,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    simnet::start(config, simnet_events_tx).await?;
    Ok(())
}
