#[macro_use]
extern crate log;

#[allow(unused_imports)]
#[macro_use]
extern crate serde_derive;

#[allow(unused_imports)]
#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod rpc;
pub mod simnet;
pub mod types;

use crossbeam_channel::{Receiver, Sender};
pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
pub use solana_rpc_client;
pub use solana_sdk;
use surfpool_types::{SimnetCommand, SimnetEvent, SubgraphCommand, SurfpoolConfig};
use txtx_addon_network_svm_types::subgraph::PluginConfig;
use uuid::Uuid;

pub async fn start_simnet(
    config: SurfpoolConfig,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    simnet::start(
        config,
        subgraph_commands_tx,
        simnet_events_tx,
        simnet_commands_tx,
        simnet_commands_rx,
    )
    .await
}

#[derive(Debug)]
pub enum PluginManagerCommand {
    LoadConfig(Uuid, PluginConfig, Sender<String>),
}

#[cfg(test)]
mod tests;
