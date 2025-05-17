use blake3::Hash;
use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender};
// use litesvm::types::TransactionMetadata;
use serde::{Deserialize, Serialize};
use solana_clock::Clock;
use solana_epoch_info::EpochInfo;
use solana_message::inner_instruction::InnerInstructionsList;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_context::TransactionReturnData;
use solana_transaction_error::TransactionError;
use txtx_addon_network_svm_types::subgraph::SubgraphRequest;

use std::{collections::HashMap, path::PathBuf};
use txtx_addon_kit::types::types::Value;
use uuid::Uuid;

pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionMetadata {
    pub signature: Signature,
    pub logs: Vec<String>,
    pub inner_instructions: InnerInstructionsList,
    pub compute_units_consumed: u64,
    pub return_data: TransactionReturnData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionConfirmationStatus {
    Processed,
    Confirmed,
    Finalized,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlockProductionMode {
    #[default]
    Clock,
    Transaction,
    Manual,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub uuid: Uuid,
    pub name: String,
    pub entries: Vec<SubgraphDataEntry>,
}

#[derive(Debug, Clone)]
pub struct SubgraphDataEntry {
    // The UUID of the entry
    pub uuid: Uuid,
    // A map of field names and their values
    pub values: HashMap<String, Value>,
    // The slot that the transaction that created this entry was processed in
    pub block_height: u64,
    // The transaction hash that created this entry
    pub transaction_hash: Hash,
}

impl SubgraphDataEntry {
    pub fn new(values: HashMap<String, Value>, block_height: u64, tx_hash: [u8; 32]) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            values,
            block_height,
            transaction_hash: Hash::from_bytes(tx_hash),
        }
    }
}

#[derive(Debug)]
pub enum SubgraphEvent {
    EndpointReady,
    InfoLog(DateTime<Local>, String),
    ErrorLog(DateTime<Local>, String),
    WarnLog(DateTime<Local>, String),
    DebugLog(DateTime<Local>, String),
    Shutdown,
}

impl SubgraphEvent {
    pub fn info<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::InfoLog(Local::now(), msg.into())
    }

    pub fn warn<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::WarnLog(Local::now(), msg.into())
    }

    pub fn error<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::ErrorLog(Local::now(), msg.into())
    }

    pub fn debug<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::DebugLog(Local::now(), msg.into())
    }
}

/// Result structure for compute units estimation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputeUnitsEstimationResult {
    pub success: bool,
    pub compute_units_consumed: u64,
    pub log_messages: Option<Vec<String>>,
    pub error_message: Option<String>,
}

/// The struct for storing the profiling results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileResult {
    pub compute_units: ComputeUnitsEstimationResult,
    // We can add other variants here in the future, e.g.:
    // pub memory_usage: MemoryUsageResult,
    // pub instruction_trace: InstructionTraceResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SchemaDataSourcingEvent {
    Rountrip(Uuid),
    ApplyEntry(Uuid, Vec<u8>, u64, [u8; 32]),
}

#[derive(Debug, Clone)]
pub enum SubgraphCommand {
    CreateSubgraph(Uuid, SubgraphRequest, Sender<String>),
    ObserveSubgraph(Receiver<SchemaDataSourcingEvent>),
    Shutdown,
}

#[derive(Debug)]
pub enum SimnetEvent {
    Ready,
    Connected(String),
    Aborted(String),
    Shutdown,
    ClockUpdate(Clock),
    EpochInfoUpdate(EpochInfo),
    BlockHashExpired,
    InfoLog(DateTime<Local>, String),
    ErrorLog(DateTime<Local>, String),
    WarnLog(DateTime<Local>, String),
    DebugLog(DateTime<Local>, String),
    PluginLoaded(String),
    TransactionReceived(DateTime<Local>, VersionedTransaction),
    TransactionProcessed(
        DateTime<Local>,
        TransactionMetadata,
        Option<TransactionError>,
    ),
    AccountUpdate(DateTime<Local>, Pubkey),
    TaggedProfile {
        result: ProfileResult,
        tag: String,
        timestamp: DateTime<Local>,
    },
}

impl SimnetEvent {
    pub fn info<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::InfoLog(Local::now(), msg.into())
    }

    pub fn warn<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::WarnLog(Local::now(), msg.into())
    }

    pub fn error<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::ErrorLog(Local::now(), msg.into())
    }

    pub fn debug<S>(msg: S) -> Self
    where
        S: Into<String>,
    {
        Self::DebugLog(Local::now(), msg.into())
    }

    pub fn transaction_processed(meta: TransactionMetadata, err: Option<TransactionError>) -> Self {
        Self::TransactionProcessed(Local::now(), meta, err)
    }

    pub fn transaction_received(tx: VersionedTransaction) -> Self {
        Self::TransactionReceived(Local::now(), tx)
    }

    pub fn account_update(pubkey: Pubkey) -> Self {
        Self::AccountUpdate(Local::now(), pubkey)
    }

    pub fn tagged_profile(result: ProfileResult, tag: String) -> Self {
        Self::TaggedProfile {
            result,
            tag,
            timestamp: Local::now(),
        }
    }

    pub fn account_update_msg(&self) -> String {
        match self {
            SimnetEvent::AccountUpdate(_, pubkey) => {
                format!("Account {} updated.", pubkey)
            }
            _ => unreachable!("This function should only be called for AccountUpdate events"),
        }
    }

    pub fn epoch_info_update_msg(&self) -> String {
        match self {
            SimnetEvent::EpochInfoUpdate(epoch_info) => {
                format!(
                    "Connection established. Epoch {} / Slot index {} / Slot {}.",
                    epoch_info.epoch, epoch_info.slot_index, epoch_info.absolute_slot
                )
            }
            _ => unreachable!("This function should only be called for EpochInfoUpdate events"),
        }
    }

    pub fn plugin_loaded_msg(&self) -> String {
        match self {
            SimnetEvent::PluginLoaded(plugin_name) => {
                format!("Plugin {} successfully loaded.", plugin_name)
            }
            _ => unreachable!("This function should only be called for PluginLoaded events"),
        }
    }

    pub fn clock_update_msg(&self) -> String {
        match self {
            SimnetEvent::ClockUpdate(clock) => {
                format!("Clock ticking (epoch {}, slot {})", clock.epoch, clock.slot)
            }
            _ => unreachable!("This function should only be called for ClockUpdate events"),
        }
    }
}

#[derive(Debug)]
pub enum TransactionStatusEvent {
    Success(TransactionConfirmationStatus),
    SimulationFailure((TransactionError, TransactionMetadata)),
    ExecutionFailure((TransactionError, TransactionMetadata)),
    VerificationFailure((String)),
}

#[derive(Debug)]
pub enum SimnetCommand {
    SlotForward(Option<Hash>),
    SlotBackward(Option<Hash>),
    UpdateClock(ClockCommand),
    UpdateBlockProductionMode(BlockProductionMode),
    TransactionReceived(
        Option<Hash>,
        VersionedTransaction,
        Sender<TransactionStatusEvent>,
        bool,
    ),
    Terminate(Option<Hash>),
}

#[derive(Debug)]
pub enum ClockCommand {
    Pause,
    Resume,
    Toggle,
    UpdateSlotInterval(u64),
}

pub enum ClockEvent {
    Tick,
    ExpireBlockHash,
}

#[derive(Clone, Debug, Default)]
pub struct SurfpoolConfig {
    pub simnets: Vec<SimnetConfig>,
    pub rpc: RpcConfig,
    pub subgraph: SubgraphConfig,
    pub plugin_config_path: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SimnetConfig {
    pub remote_rpc_url: String,
    pub slot_time: u64,
    pub block_production_mode: BlockProductionMode,
    pub airdrop_addresses: Vec<Pubkey>,
    pub airdrop_token_amount: u64,
}

impl Default for SimnetConfig {
    fn default() -> Self {
        Self {
            remote_rpc_url: DEFAULT_RPC_URL.to_string(),
            slot_time: 0,
            block_production_mode: BlockProductionMode::Clock,
            airdrop_addresses: vec![],
            airdrop_token_amount: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubgraphConfig {}

#[derive(Clone, Debug)]
pub struct RpcConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub ws_port: u16,
}

impl RpcConfig {
    pub fn get_socket_address(&self) -> String {
        format!("{}:{}", self.bind_host, self.bind_port)
    }
    pub fn get_ws_address(&self) -> String {
        format!("{}:{}", self.bind_host, self.ws_port)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubgraphPluginConfig {
    pub uuid: Uuid,
    pub ipc_token: String,
    pub subgraph_request: SubgraphRequest,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 8899,
            ws_port: 8900,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SvmSimnetInitializationRequest {
    pub domain: String,
    pub block_production_mode: BlockProductionMode,
    pub datasource_rpc_url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SvmSimnetCommand {
    Init(SvmSimnetInitializationRequest),
}

#[derive(Serialize, Deserialize)]
pub struct CreateNetworkRequest {
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub datasource_rpc_url: String,
    pub block_production_mode: BlockProductionMode,
}

impl CreateNetworkRequest {
    pub fn new(
        workspace_id: Uuid,
        name: String,
        description: Option<String>,
        datasource_rpc_url: String,
        block_production_mode: BlockProductionMode,
    ) -> Self {
        Self {
            workspace_id,
            name,
            description,
            datasource_rpc_url,
            block_production_mode,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct CreateNetworkResponse {
    pub rpc_url: String,
}
