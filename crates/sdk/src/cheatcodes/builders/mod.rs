//! Builder types for constructing Surfnet cheatcode RPC payloads.
//!
//! These builders are useful when tests need to express optional parameters
//! incrementally and then execute the request through
//! [`crate::Cheatcodes::execute`].
//!
//! ```rust,no_run
//! use surfpool_sdk::{Pubkey, Surfnet};
//! use surfpool_sdk::cheatcodes::builders::set_account::SetAccount;
//!
//! # async fn example() {
//! let surfnet = Surfnet::start().await.unwrap();
//! let cheats = surfnet.cheatcodes();
//! let address = Pubkey::new_unique();
//! let owner = Pubkey::new_unique();
//!
//! cheats
//!     .execute(
//!         SetAccount::new(address)
//!             .lamports(1_000_000)
//!             .owner(owner)
//!             .data(vec![1, 2, 3, 4]),
//!     )
//!     .unwrap();
//! # }
//! ```
pub mod deploy_program;
pub mod set_account;
pub mod set_token_account;
pub mod reset_account;
pub mod stream_account;

/// Trait implemented by typed cheatcode builders.
///
/// `METHOD` is the target Surfnet RPC method, and [`Self::build`] returns
/// the JSON-RPC parameter array for that method.
pub trait CheatcodeBuilder {
    const METHOD: &'static str;
    fn build(self) -> serde_json::Value;
}
