#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod rpc;
pub mod runloop;

pub use jsonrpc_core;
pub use jsonrpc_http_server;
pub use litesvm;
pub use solana_rpc_client;
pub use solana_sdk;

use litesvm::LiteSVM;

pub fn start_runloop() {
    let svm = LiteSVM::new();
    hiro_system_kit::nestable_block_on(runloop::start(svm));
}
