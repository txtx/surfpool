
#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

pub mod runloop;
pub mod rpc;

pub use litesvm;
pub use solana_sdk;
pub use solana_rpc_client;
pub use jsonrpc_core;
pub use jsonrpc_http_server;

use litesvm::LiteSVM;

pub fn start_runloop() {
    let mut svm = LiteSVM::new();
    hiro_system_kit::nestable_block_on(runloop::start(&mut svm));
}