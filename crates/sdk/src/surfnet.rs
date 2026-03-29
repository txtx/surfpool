use std::{net::TcpListener, path::PathBuf};

use crossbeam_channel::{Receiver, Sender};
use solana_client::rpc_request::RpcRequest;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_signer::Signer;
use surfpool_core::surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm};
use surfpool_types::{
    BlockProductionMode, RpcConfig, SimnetCommand, SimnetConfig, SimnetEvent, SurfpoolConfig,
};

use crate::{
    Cheatcodes,
    error::{SurfnetError, SurfnetResult},
    report::{SurfnetReportData, TransactionReportEntry},
};

const DEFAULT_REPORT_DIR: &str = "target/surfpool-reports";

/// Builder for configuring a [`Surfnet`] instance before starting it.
///
/// ```rust,no_run
/// use surfpool_sdk::{Surfnet, BlockProductionMode};
///
/// # async fn example() {
/// let surfnet = Surfnet::builder()
///     .offline(true)
///     .block_production_mode(BlockProductionMode::Transaction)
///     .airdrop_sol(10_000_000_000)
///     .start()
///     .await
///     .unwrap();
/// # }
/// ```
pub struct SurfnetBuilder {
    offline_mode: bool,
    remote_rpc_url: Option<String>,
    block_production_mode: BlockProductionMode,
    slot_time_ms: u64,
    airdrop_addresses: Vec<Pubkey>,
    airdrop_lamports: u64,
    payer: Option<Keypair>,
    report: Option<bool>,
    report_dir: Option<PathBuf>,
    test_name: Option<String>,
}

impl Default for SurfnetBuilder {
    fn default() -> Self {
        Self {
            offline_mode: true,
            remote_rpc_url: None,
            block_production_mode: BlockProductionMode::Transaction,
            slot_time_ms: 1,
            airdrop_addresses: vec![],
            airdrop_lamports: 10_000_000_000, // 10 SOL
            payer: None,
            report: None,
            report_dir: None,
            test_name: detect_test_name(),
        }
    }
}

impl SurfnetBuilder {
    /// Run in offline mode (no mainnet RPC fallback). Default: `true`.
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline_mode = offline;
        self
    }

    /// Set a remote RPC URL for account fallback (implies `offline(false)`).
    pub fn remote_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.remote_rpc_url = Some(url.into());
        self.offline_mode = false;
        self
    }

    /// How blocks are produced. Default: `Transaction` (advance on each tx).
    pub fn block_production_mode(mut self, mode: BlockProductionMode) -> Self {
        self.block_production_mode = mode;
        self
    }

    /// Slot time in milliseconds. Default: `1` (fast for tests).
    pub fn slot_time_ms(mut self, ms: u64) -> Self {
        self.slot_time_ms = ms;
        self
    }

    /// Additional addresses to airdrop SOL to at startup.
    pub fn airdrop_addresses(mut self, addresses: Vec<Pubkey>) -> Self {
        self.airdrop_addresses = addresses;
        self
    }

    /// Amount of lamports to airdrop to the payer (and additional addresses) at startup.
    /// Default: 10 SOL.
    pub fn airdrop_sol(mut self, lamports: u64) -> Self {
        self.airdrop_lamports = lamports;
        self
    }

    /// Use a specific keypair as the payer. If not set, a random one is generated.
    pub fn payer(mut self, keypair: Keypair) -> Self {
        self.payer = Some(keypair);
        self
    }

    /// Enable or disable report data export on drop.
    /// Overrides the `SURFPOOL_REPORT` env var.
    pub fn report(mut self, enabled: bool) -> Self {
        self.report = Some(enabled);
        self
    }

    /// Set the directory for report data files. Default: `target/surfpool-reports`.
    pub fn report_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.report_dir = Some(dir.into());
        self
    }

    /// Set a label for this instance in the report (e.g. the test name).
    pub fn test_name(mut self, name: impl Into<String>) -> Self {
        self.test_name = Some(name.into());
        self
    }

    /// Start the surfnet with the configured options.
    pub async fn start(self) -> SurfnetResult<Surfnet> {
        let payer = self.payer.unwrap_or_else(Keypair::new);

        let bind_port = get_free_port()?;
        let ws_port = get_free_port()?;
        let bind_host = "127.0.0.1".to_string();

        let mut airdrop_addresses = vec![payer.pubkey()];
        airdrop_addresses.extend(self.airdrop_addresses);

        let surfpool_config = SurfpoolConfig {
            simnets: vec![SimnetConfig {
                offline_mode: self.offline_mode,
                remote_rpc_url: self.remote_rpc_url,
                slot_time: self.slot_time_ms,
                block_production_mode: self.block_production_mode,
                airdrop_addresses,
                airdrop_token_amount: self.airdrop_lamports,
                ..Default::default()
            }],
            rpc: RpcConfig {
                bind_host: bind_host.clone(),
                bind_port,
                ws_port,
                ..Default::default()
            },
            ..Default::default()
        };

        let rpc_url = format!("http://{bind_host}:{bind_port}");
        let ws_url = format!("ws://{bind_host}:{ws_port}");

        let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::default();
        let (simnet_commands_tx, simnet_commands_rx) = crossbeam_channel::unbounded();

        let svm_locker = SurfnetSvmLocker::new(surfnet_svm);
        let svm_locker_clone = svm_locker.clone();
        let simnet_commands_tx_clone = simnet_commands_tx.clone();

        let _handle = std::thread::Builder::new()
            .name("surfnet-sdk".into())
            .spawn(move || {
                let future = surfpool_core::runloops::start_local_surfnet_runloop(
                    svm_locker_clone,
                    surfpool_config,
                    simnet_commands_tx_clone,
                    simnet_commands_rx,
                    geyser_events_rx,
                );
                if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                    log::error!("Surfnet exited with error: {e}");
                }
            })
            .map_err(|e| SurfnetError::Runtime(e.to_string()))?;

        // Wait for the runtime to signal ready
        wait_for_ready(&simnet_events_rx)?;

        // Resolve report settings: builder override > env var
        let report_enabled = self.report.unwrap_or_else(|| {
            std::env::var("SURFPOOL_REPORT")
                .map(|v| !v.is_empty() && v != "0" && v.to_lowercase() != "false")
                .unwrap_or(false)
        });

        let report_dir = self.report_dir.unwrap_or_else(|| {
            // If SURFPOOL_REPORT is a path (not "1" or "true"), use it as the directory
            std::env::var("SURFPOOL_REPORT")
                .ok()
                .filter(|v| v != "1" && v.to_lowercase() != "true" && !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_REPORT_DIR))
        });

        Ok(Surfnet {
            rpc_url,
            ws_url,
            payer,
            simnet_commands_tx,
            simnet_events_rx,
            svm_locker,
            instance_id: uuid::Uuid::new_v4().to_string(),
            report_enabled,
            report_dir,
            test_name: self.test_name,
        })
    }
}

/// A running Surfpool instance with RPC/WS endpoints on dynamic ports.
///
/// Provides:
/// - Pre-funded payer keypair
/// - [`RpcClient`] connected to the local instance
/// - [`Cheatcodes`] for direct state manipulation (fund accounts, set token balances, etc.)
/// - Report data export for test result visualization
///
/// The instance is shut down when dropped. If reporting is enabled
/// (via `SURFPOOL_REPORT=1` or `.report(true)`), transaction profiles
/// are exported to disk before shutdown.
pub struct Surfnet {
    rpc_url: String,
    ws_url: String,
    payer: Keypair,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_events_rx: Receiver<SimnetEvent>,
    #[allow(dead_code)] // retained for future direct profiling access
    svm_locker: SurfnetSvmLocker,
    instance_id: String,
    report_enabled: bool,
    report_dir: PathBuf,
    test_name: Option<String>,
}

impl Surfnet {
    /// Start a surfnet with default settings (offline, transaction-mode blocks, 10 SOL payer).
    pub async fn start() -> SurfnetResult<Self> {
        SurfnetBuilder::default().start().await
    }

    /// Create a builder for custom configuration.
    pub fn builder() -> SurfnetBuilder {
        SurfnetBuilder::default()
    }

    /// The HTTP RPC URL (e.g. `http://127.0.0.1:12345`).
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// The WebSocket URL (e.g. `ws://127.0.0.1:12346`).
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Create a new [`RpcClient`] connected to this surfnet.
    pub fn rpc_client(&self) -> RpcClient {
        RpcClient::new(&self.rpc_url)
    }

    /// The pre-funded payer keypair.
    pub fn payer(&self) -> &Keypair {
        &self.payer
    }

    /// Access cheatcode helpers for direct state manipulation.
    pub fn cheatcodes(&self) -> Cheatcodes<'_> {
        Cheatcodes::new(&self.rpc_url)
    }

    /// Get a reference to the simnet events receiver for observing runtime events.
    pub fn events(&self) -> &Receiver<SimnetEvent> {
        &self.simnet_events_rx
    }

    /// Send a command to the simnet runtime.
    pub fn send_command(&self, command: SimnetCommand) -> SurfnetResult<()> {
        self.simnet_commands_tx
            .send(command)
            .map_err(|e| SurfnetError::Runtime(format!("failed to send command: {e}")))
    }

    /// The unique instance ID for this surfnet.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Export all transaction profiles from this instance.
    pub fn export_report_data(&self) -> SurfnetResult<SurfnetReportData> {
        let client = self.rpc_client();

        // Fetch all local signatures
        let signatures_response: serde_json::Value = client
            .send(
                RpcRequest::Custom {
                    method: "surfnet_getLocalSignatures",
                },
                serde_json::json!([200]),
            )
            .map_err(|e| SurfnetError::Report(format!("failed to fetch signatures: {e}")))?;

        let entries = signatures_response
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut transactions = Vec::with_capacity(entries.len());

        for entry in &entries {
            let signature = entry
                .get("signature")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();

            let error = entry
                .get("err")
                .filter(|e| !e.is_null())
                .map(|e| e.to_string());

            let logs = entry
                .get("logs")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // Fetch full profile with jsonParsed encoding
            let profile_json_parsed = client
                .send::<serde_json::Value>(
                    RpcRequest::Custom {
                        method: "surfnet_getTransactionProfile",
                    },
                    serde_json::json!([signature, { "encoding": "jsonParsed" }]),
                )
                .ok()
                .and_then(|resp| resp.get("value").cloned())
                .filter(|v| !v.is_null());

            // Fetch full profile with base64 encoding
            let profile_base64 = client
                .send::<serde_json::Value>(
                    RpcRequest::Custom {
                        method: "surfnet_getTransactionProfile",
                    },
                    serde_json::json!([signature, { "encoding": "base64" }]),
                )
                .ok()
                .and_then(|resp| resp.get("value").cloned())
                .filter(|v| !v.is_null());

            let slot = profile_json_parsed
                .as_ref()
                .and_then(|p| p.get("slot"))
                .and_then(|s| s.as_u64())
                .unwrap_or(0);

            transactions.push(TransactionReportEntry {
                signature,
                slot,
                error,
                logs,
                profile_json_parsed,
                profile_base64,
            });
        }

        Ok(SurfnetReportData {
            instance_id: self.instance_id.clone(),
            test_name: self.test_name.clone(),
            rpc_url: self.rpc_url.clone(),
            transactions,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Export report data and write it to the report directory.
    /// Returns the path to the written JSON file.
    pub fn write_report_data(&self) -> SurfnetResult<PathBuf> {
        let data = self.export_report_data()?;
        let dir = &self.report_dir;

        std::fs::create_dir_all(dir)
            .map_err(|e| SurfnetError::Report(format!("failed to create report dir: {e}")))?;

        let path = dir.join(format!("{}.json", self.instance_id));
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| SurfnetError::Report(format!("failed to serialize report data: {e}")))?;

        std::fs::write(&path, json)
            .map_err(|e| SurfnetError::Report(format!("failed to write report file: {e}")))?;

        log::info!("Surfnet report data written to {}", path.display());
        Ok(path)
    }
}

impl Drop for Surfnet {
    fn drop(&mut self) {
        if self.report_enabled {
            if let Err(e) = self.write_report_data() {
                log::warn!("Failed to write surfnet report data: {e}");
            }
        }
        let _ = self.simnet_commands_tx.send(SimnetCommand::Terminate(None));
    }
}

fn get_free_port() -> SurfnetResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| SurfnetError::PortAllocation(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| SurfnetError::PortAllocation(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}

fn wait_for_ready(events_rx: &Receiver<SimnetEvent>) -> SurfnetResult<()> {
    loop {
        match events_rx.recv() {
            Ok(SimnetEvent::Ready(_)) => return Ok(()),
            Ok(SimnetEvent::Aborted(err)) => return Err(SurfnetError::Aborted(err)),
            Ok(SimnetEvent::Shutdown) => {
                return Err(SurfnetError::Aborted(
                    "surfnet shut down during startup".into(),
                ));
            }
            Ok(_) => continue,
            Err(e) => {
                return Err(SurfnetError::Startup(format!(
                    "events channel closed unexpectedly: {e}"
                )));
            }
        }
    }
}

/// Try to extract the test function name from the current thread.
///
/// Rust's test harness names each test thread after the test function
/// (e.g. `my_module::my_test`). This works reliably for `#[test]` and
/// `#[tokio::test]` with `current_thread`. For `multi_thread` tokio tests
/// the builder is often constructed on a worker thread — we detect and
/// skip those names.
fn detect_test_name() -> Option<String> {
    let thread = std::thread::current();
    let name = thread.name()?;

    // Filter out names that aren't test functions
    if name == "main"
        || name.starts_with("tokio-runtime")
        || name.starts_with("surfnet")
        || name.starts_with("Thread-")
    {
        return None;
    }

    // Rust test names look like "module::test_name" or just "test_name".
    // Take the last segment for a clean display name.
    let short = name.rsplit("::").next().unwrap_or(name);
    Some(short.to_string())
}
