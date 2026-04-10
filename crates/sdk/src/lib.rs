//! # Surfpool SDK
//!
//! Embed a full Surfpool instance directly in your Rust integration tests.
//! No external process needed — just spin up a `Surfnet`, point your RPC client at it,
//! and test against a real Solana-compatible runtime.
//!
//! ```rust,no_run
//! use surfpool_sdk::Surfnet;
//!
//! #[tokio::test]
//! async fn my_test() {
//!     let surfnet = Surfnet::start().await.unwrap();
//!
//!     let rpc = surfnet.rpc_client();
//!     let balance = rpc.get_balance(&surfnet.payer().pubkey()).unwrap();
//!     assert!(balance > 0);
//! }
//! ```
//!
//! ## Reporting
//!
//! Set `SURFPOOL_REPORT=1` to automatically export transaction data on drop,
//! then generate a consolidated HTML report:
//!
//! ```rust,no_run
//! use surfpool_sdk::report::{generate, generate_default, SurfpoolReport, SurfpoolReportOptions};
//!
//! // After tests complete:
//! let report = SurfpoolReport::from_directory("target/surfpool-reports").unwrap();
//! report.write_html("target/surfpool-report.html").unwrap();
//!
//! // Or use the explicit one-shot API:
//! generate_default().unwrap();
//! generate(SurfpoolReportOptions::default()).unwrap();
//! ```

pub mod cheatcodes;
mod error;
pub mod report;
mod surfnet;

pub use cheatcodes::Cheatcodes;
pub use error::{SurfnetError, SurfnetResult};
// Re-export key Solana types for convenience
pub use solana_keypair::Keypair;
pub use solana_pubkey::Pubkey;
pub use solana_rpc_client::rpc_client::RpcClient;
pub use solana_signer::Signer;
pub use surfnet::{Surfnet, SurfnetBuilder};
pub use surfpool_types::BlockProductionMode;
pub use surfpool_types::SimnetCommand;
pub use surfpool_types::SimnetEvent;
