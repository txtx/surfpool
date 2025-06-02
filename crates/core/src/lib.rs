#[macro_use]
extern crate log;

#[allow(unused_imports)]
#[macro_use]
extern crate serde_derive;

#[allow(unused_imports)]
#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod error;
pub mod rpc;
pub mod runloops;
pub mod surfnet;
pub mod types;

use crossbeam_channel::{Receiver, Sender};
pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
pub use solana_rpc_client;
use surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm, GeyserEvent};
use surfpool_types::{SimnetCommand, SubgraphCommand, SurfpoolConfig};
use txtx_addon_network_svm_types::subgraph::PluginConfig;
use uuid::Uuid;

pub async fn start_local_surfnet(
    surfnet_svm: SurfnetSvm,
    config: SurfpoolConfig,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
    geyser_events_rx: Receiver<GeyserEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let svm_locker = SurfnetSvmLocker::new(surfnet_svm);
    runloops::start_local_surfnet_runloop(
        svm_locker,
        config,
        subgraph_commands_tx,
        simnet_commands_tx,
        simnet_commands_rx,
        geyser_events_rx,
    )
    .await
}

#[derive(Debug)]
pub enum PluginManagerCommand {
    LoadConfig(Uuid, PluginConfig, Sender<String>),
}

#[cfg(test)]
mod tests;
