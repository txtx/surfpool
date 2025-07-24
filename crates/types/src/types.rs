use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr};

use blake3::Hash;
use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender};
// use litesvm::types::TransactionMetadata;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use serde_with::{BytesOrString, serde_as};
use solana_account_decoder_client_types::UiAccount;
use solana_clock::{Clock, Epoch};
use solana_epoch_info::EpochInfo;
use solana_message::inner_instruction::InnerInstructionsList;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_context::TransactionReturnData;
use solana_transaction_error::TransactionError;
use txtx_addon_network_svm_types::subgraph::SubgraphRequest;
use uuid::Uuid;

pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEFAULT_RPC_PORT: u16 = 8899;
pub const DEFAULT_WS_PORT: u16 = 8900;
pub const DEFAULT_NETWORK_HOST: &str = "127.0.0.1";

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
    pub state: ProfileState,
    pub slot: u64,
    pub uuid: Option<Uuid>,
}

impl ProfileResult {
    pub fn success(
        compute_units_consumed: u64,
        logs: Vec<String>,
        pre_execution: BTreeMap<Pubkey, Option<UiAccount>>,
        post_execution: BTreeMap<Pubkey, Option<UiAccount>>,
        slot: u64,
    ) -> Self {
        Self {
            compute_units: ComputeUnitsEstimationResult {
                success: true,
                compute_units_consumed,
                log_messages: Some(logs),
                error_message: None,
            },
            state: ProfileState::new(pre_execution, post_execution),
            slot,
            uuid: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]

pub struct ProfileState {
    #[serde(with = "profile_state_map")]
    pub pre_execution: BTreeMap<Pubkey, Option<UiAccount>>,
    #[serde(with = "profile_state_map")]
    pub post_execution: BTreeMap<Pubkey, Option<UiAccount>>,
}

impl ProfileState {
    pub fn new(
        pre_execution: BTreeMap<Pubkey, Option<UiAccount>>,
        post_execution: BTreeMap<Pubkey, Option<UiAccount>>,
    ) -> Self {
        Self {
            pre_execution,
            post_execution,
        }
    }
}

pub mod profile_state_map {
    use super::*;

    pub fn serialize<S>(
        map: &BTreeMap<Pubkey, Option<UiAccount>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let str_map: BTreeMap<String, &Option<UiAccount>> =
            map.iter().map(|(k, v)| (k.to_string(), v)).collect();
        str_map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<Pubkey, Option<UiAccount>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let str_map: BTreeMap<String, Option<UiAccount>> = BTreeMap::deserialize(deserializer)?;
        str_map
            .into_iter()
            .map(|(k, v)| {
                Pubkey::from_str(&k)
                    .map(|pk| (pk, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub enum SubgraphCommand {
    CreateCollection(Uuid, SubgraphRequest, Sender<String>),
    ObserveCollection(Receiver<DataIndexingCommand>),
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
    VerificationFailure(String),
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
    pub expiry: Option<u64>,
}

impl Default for SimnetConfig {
    fn default() -> Self {
        Self {
            remote_rpc_url: DEFAULT_RPC_URL.to_string(),
            slot_time: 0,
            block_production_mode: BlockProductionMode::Clock,
            airdrop_addresses: vec![],
            airdrop_token_amount: 0,
            expiry: None,
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
            bind_host: DEFAULT_NETWORK_HOST.to_string(),
            bind_port: DEFAULT_RPC_PORT,
            ws_port: DEFAULT_WS_PORT,
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

#[derive(Serialize, Deserialize)]
pub struct DeleteNetworkRequest {
    pub workspace_id: Uuid,
    pub network_id: Uuid,
}

impl DeleteNetworkRequest {
    pub fn new(workspace_id: Uuid, network_id: Uuid) -> Self {
        Self {
            workspace_id,
            network_id,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct DeleteNetworkResponse;

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    /// providing this value sets the lamports in the account
    pub lamports: Option<u64>,
    /// providing this value sets the data held in this account
    #[serde_as(as = "Option<BytesOrString>")]
    pub data: Option<Vec<u8>>,
    ///  providing this value sets the program that owns this account. If executable, the program that loads this account.
    pub owner: Option<String>,
    /// providing this value sets whether this account's data contains a loaded program (and is now read-only)
    pub executable: Option<bool>,
    /// providing this value sets the epoch at which this account will next owe rent
    pub rent_epoch: Option<Epoch>,
}

#[derive(Debug, Clone)]
pub enum SetSomeAccount {
    Account(String),
    NoAccount,
}

impl<'de> Deserialize<'de> for SetSomeAccount {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SetSomeAccountVisitor;

        impl<'de> Visitor<'de> for SetSomeAccountVisitor {
            type Value = SetSomeAccount;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Pubkey String or the String 'null'")
            }

            fn visit_some<D_>(self, deserializer: D_) -> std::result::Result<Self::Value, D_::Error>
            where
                D_: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer).map(|v: String| match v.as_str() {
                    "null" => SetSomeAccount::NoAccount,
                    _ => SetSomeAccount::Account(v.to_string()),
                })
            }
        }

        deserializer.deserialize_option(SetSomeAccountVisitor)
    }
}

impl Serialize for SetSomeAccount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            SetSomeAccount::Account(val) => serializer.serialize_str(val),
            SetSomeAccount::NoAccount => serializer.serialize_str("null"),
        }
    }
}

#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    /// providing this value sets the amount of the token in the account data
    pub amount: Option<u64>,
    /// providing this value sets the delegate of the token account
    pub delegate: Option<SetSomeAccount>,
    /// providing this value sets the state of the token account
    pub state: Option<String>,
    /// providing this value sets the amount authorized to the delegate
    pub delegated_amount: Option<u64>,
    /// providing this value sets the close authority of the token account
    pub close_authority: Option<SetSomeAccount>,
}

// token supply update for set supply method in SVM tricks
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SupplyUpdate {
    pub total: Option<u64>,
    pub circulating: Option<u64>,
    pub non_circulating: Option<u64>,
    pub non_circulating_accounts: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub enum UuidOrSignature {
    Uuid(Uuid),
    Signature(Signature),
}

/// Intermediate struct used for deserialization of [UuidOrSignature].
#[derive(Deserialize)]
#[serde(untagged)]
enum UuidOrSignatureHelper {
    Uuid { uuid: Uuid },
    Signature { signature: String },
}

impl Serialize for UuidOrSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            UuidOrSignature::Uuid(uuid) => {
                let mut map = std::collections::HashMap::new();
                map.insert("uuid", uuid.to_string());
                map.serialize(serializer)
            }
            UuidOrSignature::Signature(sig) => {
                println!("Serializing signature: {}", sig);
                let mut map = std::collections::HashMap::new();
                map.insert("signature", sig.to_string());
                map.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for UuidOrSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = UuidOrSignatureHelper::deserialize(deserializer)?;

        match helper {
            UuidOrSignatureHelper::Uuid { uuid } => Ok(UuidOrSignature::Uuid(uuid)),
            UuidOrSignatureHelper::Signature { signature } => {
                println!("Deserializing signature: {}", signature);
                let sig = Signature::from_str(&signature).map_err(serde::de::Error::custom);
                println!("Deserialized signature: {:?}", sig);
                Ok(UuidOrSignature::Signature(sig?))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum DataIndexingCommand {
    ProcessCollection(Uuid),
    ProcessCollectionEntriesPack(Uuid, Vec<u8>),
}
