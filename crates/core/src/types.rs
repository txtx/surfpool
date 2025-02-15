use agave_geyser_plugin_interface::geyser_plugin_interface::ReplicaTransactionInfoVersions;
use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender};
use litesvm::types::TransactionMetadata;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_sdk::{
    blake3::Hash,
    clock::{Clock, Slot},
    epoch_info::EpochInfo,
    pubkey::Pubkey,
    transaction::{SanitizedTransaction, TransactionError, VersionedTransaction},
};
use solana_transaction_status::TransactionConfirmationStatus;
use std::{collections::HashMap, path::PathBuf};
use txtx_addon_network_svm::codec::subgraph::{PluginConfig, SubgraphRequest};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunloopTriggerMode {
    Clock,
    Manual,
    Transaction,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub uuid: Uuid,
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub uuid: Uuid,
    pub values: HashMap<String, String>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SchemaDatasourceingEvent {
    Rountrip(Uuid),
    ApplyEntry(Uuid, String), //, SubgraphRequest, u64),
}

#[derive(Debug, Clone)]
pub enum SubgraphCommand {
    CreateSubgraph(Uuid, SubgraphRequest, Sender<String>),
    ObserveSubgraph(Receiver<SchemaDatasourceingEvent>),
    Shutdown,
}

#[derive(Debug)]
pub enum SimnetEvent {
    Ready,
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
    TransactionSimulated(DateTime<Local>, VersionedTransaction),
    AccountUpdate(DateTime<Local>, Pubkey),
}

pub enum TransactionStatusEvent {
    Success(TransactionConfirmationStatus),
    SimulationFailure((TransactionError, TransactionMetadata)),
    ExecutionFailure((TransactionError, TransactionMetadata)),
}

pub enum SimnetCommand {
    SlotForward,
    SlotBackward,
    UpdateClock(ClockCommand),
    UpdateRunloopMode(RunloopTriggerMode),
    TransactionReceived(
        Hash,
        VersionedTransaction,
        Sender<TransactionStatusEvent>,
        RpcSendTransactionConfig,
    ),
}

pub enum PluginManagerCommand {
    LoadConfig(Uuid, PluginConfig, Sender<String>),
}

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

#[derive(Debug, Clone)]
pub struct SurfpoolConfig {
    pub simnet: SimnetConfig,
    pub rpc: RpcConfig,
    pub subgraph: SubgraphConfig,
    pub plugin_config_path: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SimnetConfig {
    pub remote_rpc_url: String,
    pub slot_time: u64,
    pub runloop_trigger_mode: RunloopTriggerMode,
    pub airdrop_addresses: Vec<Pubkey>,
    pub airdrop_token_amount: u64,
}

#[derive(Clone, Debug)]
pub struct SubgraphConfig {}

#[derive(Clone, Debug)]
pub struct RpcConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub remote_rpc_url: String,
}

impl RpcConfig {
    pub fn get_socket_address(&self) -> String {
        format!("{}:{}", self.bind_host, self.bind_port)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubgraphPluginConfig {
    pub uuid: Uuid,
    pub ipc_token: String,
    pub subgraph_request: SubgraphRequest,
}
