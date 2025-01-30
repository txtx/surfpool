#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod rpc;
pub mod simnet;

pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
pub use solana_rpc_client;
pub use solana_sdk;


pub fn start_simnet() {
    hiro_system_kit::nestable_block_on(simnet::start());
}
