use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    fmt,
    path::{Display, PathBuf},
    str::FromStr,
};

use blake3::Hash;
use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender};
// use litesvm::types::TransactionMetadata;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use serde_with::{BytesOrString, serde_as};
use solana_account::Account;
use solana_account_decoder_client_types::{UiAccount, UiAccountEncoding};
use solana_clock::{Clock, Epoch, Slot};
use solana_epoch_info::EpochInfo;
use solana_message::inner_instruction::InnerInstructionsList;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_context::TransactionReturnData;
use solana_transaction_error::TransactionError;
use txtx_addon_kit::indexmap::IndexMap;
use txtx_addon_network_svm_types::subgraph::SubgraphRequest;
use uuid::Uuid;

pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEFAULT_RPC_PORT: u16 = 8899;
pub const DEFAULT_WS_PORT: u16 = 8900;
pub const DEFAULT_STUDIO_PORT: u16 = 8488;
pub const CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED: u16 = 18488;
pub const DEFAULT_NETWORK_HOST: &str = "127.0.0.1";
pub const DEFAULT_SLOT_TIME_MS: u64 = 400;
pub type Idl = anchor_lang_idl::types::Idl;

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
#[derive(Debug, Clone, PartialEq)]
pub struct KeyedProfileResult {
    pub slot: u64,
    pub key: UuidOrSignature,
    pub instruction_profiles: Option<Vec<ProfileResult>>,
    pub transaction_profile: ProfileResult,
    pub readonly_account_states: HashMap<Pubkey, Account>,
}

impl KeyedProfileResult {
    pub fn new(
        slot: u64,
        key: UuidOrSignature,
        instruction_profiles: Option<Vec<ProfileResult>>,
        transaction_profile: ProfileResult,
        readonly_account_states: HashMap<Pubkey, Account>,
    ) -> Self {
        Self {
            slot,
            key,
            instruction_profiles,
            transaction_profile,
            readonly_account_states,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileResult {
    pub pre_execution_capture: ExecutionCapture,
    pub post_execution_capture: ExecutionCapture,
    pub compute_units_consumed: u64,
    pub log_messages: Option<Vec<String>>,
    pub error_message: Option<String>,
}

pub type ExecutionCapture = BTreeMap<Pubkey, Option<Account>>;

impl ProfileResult {
    pub fn new(
        pre_execution_capture: ExecutionCapture,
        post_execution_capture: ExecutionCapture,
        compute_units_consumed: u64,
        log_messages: Option<Vec<String>>,
        error_message: Option<String>,
    ) -> Self {
        Self {
            pre_execution_capture,
            post_execution_capture,
            compute_units_consumed,
            log_messages,
            error_message,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccountProfileState {
    Readonly,
    Writable(AccountChange),
}

impl AccountProfileState {
    pub fn new(
        pubkey: Pubkey,
        pre_account: Option<Account>,
        post_account: Option<Account>,
        readonly_accounts: &[Pubkey],
    ) -> Self {
        if readonly_accounts.contains(&pubkey) {
            return AccountProfileState::Readonly;
        }

        match (pre_account, post_account) {
            (None, Some(post_account)) => {
                AccountProfileState::Writable(AccountChange::Create(post_account))
            }
            (Some(pre_account), None) => {
                AccountProfileState::Writable(AccountChange::Delete(pre_account))
            }
            (Some(pre_account), Some(post_account)) if pre_account == post_account => {
                AccountProfileState::Writable(AccountChange::Unchanged(Some(pre_account)))
            }
            (Some(pre_account), Some(post_account)) => {
                AccountProfileState::Writable(AccountChange::Update(pre_account, post_account))
            }
            (None, None) => AccountProfileState::Writable(AccountChange::Unchanged(None)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccountChange {
    Create(Account),
    Update(Account, Account),
    Delete(Account),
    Unchanged(Option<Account>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcProfileResultConfig {
    pub encoding: Option<UiAccountEncoding>,
    pub depth: Option<RpcProfileDepth>,
}

impl Default for RpcProfileResultConfig {
    fn default() -> Self {
        Self {
            encoding: Some(UiAccountEncoding::JsonParsed),
            depth: Some(RpcProfileDepth::default()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RpcProfileDepth {
    Transaction,
    #[default]
    Instruction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiKeyedProfileResult {
    pub slot: u64,
    pub key: UuidOrSignature,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction_profiles: Option<Vec<UiProfileResult>>,
    pub transaction_profile: UiProfileResult,
    #[serde(with = "profile_state_map")]
    pub readonly_account_states: IndexMap<Pubkey, UiAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiProfileResult {
    #[serde(with = "profile_state_map")]
    pub account_states: IndexMap<Pubkey, UiAccountProfileState>,
    pub compute_units_consumed: u64,
    pub log_messages: Option<Vec<String>>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type", content = "accountChange")]
pub enum UiAccountProfileState {
    Readonly,
    Writable(UiAccountChange),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum UiAccountChange {
    Create(UiAccount),
    Update(UiAccount, UiAccount),
    Delete(UiAccount),
    /// The account didn't change. If [Some], this is the initial state. If [None], the account didn't exist before/after execution.
    Unchanged(Option<UiAccount>),
}

/// P starts with 300 lamports
/// Ix 1 Transfers 100 lamports to P
/// Ix 2 Transfers 100 lamports to P
///
/// Profile result 1 is from executing just Ix 1
/// AccountProfileState::Writable(P, AccountChange::Update( UiAccount { lamports: 300, ...}, UiAccount { lamports: 400, ... }))
///
/// Profile result 2 is from executing Ix 1 and Ix 2
/// AccountProfileState::Writable(P, AccountChange::Update( UiAccount { lamports: 400, ...}, UiAccount { lamports: 500, ... }))

pub mod profile_state_map {
    use super::*;

    pub fn serialize<S, T>(map: &IndexMap<Pubkey, T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let str_map: IndexMap<String, &T> = map.iter().map(|(k, v)| (k.to_string(), v)).collect();
        str_map.serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<IndexMap<Pubkey, T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let str_map: IndexMap<String, T> = IndexMap::deserialize(deserializer)?;
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
    SystemClockUpdated(Clock),
    ClockUpdate(ClockCommand),
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
        result: KeyedProfileResult,
        tag: String,
        timestamp: DateTime<Local>,
    },
    RunbookStarted(String),
    RunbookCompleted(String),
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

    pub fn tagged_profile(result: KeyedProfileResult, tag: String) -> Self {
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
                    "Datasource connection successful. Epoch {} / Slot index {} / Slot {}.",
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
            SimnetEvent::SystemClockUpdated(clock) => {
                format!("Clock ticking (epoch {}, slot {})", clock.epoch, clock.slot)
            }
            _ => {
                unreachable!("This function should only be called for SystemClockUpdated events")
            }
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
    CommandClock(ClockCommand),
    UpdateInternalClock(Clock),
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

#[derive(Clone, Debug, Default, Serialize)]
pub struct SanitizedConfig {
    pub rpc_url: String,
    pub ws_url: String,
    pub rpc_datasource_url: String,
    pub studio_url: String,
    pub graphql_query_route_url: String,
    pub version: String,
    pub workspace: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SurfpoolConfig {
    pub simnets: Vec<SimnetConfig>,
    pub rpc: RpcConfig,
    pub subgraph: SubgraphConfig,
    pub studio: StudioConfig,
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
            slot_time: DEFAULT_SLOT_TIME_MS, // Default to 400ms to match CLI default
            block_production_mode: BlockProductionMode::Clock,
            airdrop_addresses: vec![],
            airdrop_token_amount: 0,
            expiry: None,
        }
    }
}

impl SimnetConfig {
    pub fn get_sanitized_datasource_url(&self) -> String {
        self.remote_rpc_url
            .split("?")
            .map(|e| e.to_string())
            .collect::<Vec<String>>()
            .first()
            .expect("datasource url invalid")
            .to_string()
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
    pub fn get_rpc_base_url(&self) -> String {
        format!("{}:{}", self.bind_host, self.bind_port)
    }
    pub fn get_ws_base_url(&self) -> String {
        format!("{}:{}", self.bind_host, self.ws_port)
    }
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

#[derive(Clone, Debug)]
pub struct StudioConfig {
    pub bind_host: String,
    pub bind_port: u16,
}

impl StudioConfig {
    pub fn get_studio_base_url(&self) -> String {
        format!("{}:{}", self.bind_host, self.bind_port)
    }
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self {
            bind_host: DEFAULT_NETWORK_HOST.to_string(),
            bind_port: CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubgraphPluginConfig {
    pub uuid: Uuid,
    pub ipc_token: String,
    pub subgraph_request: SubgraphRequest,
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

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum UuidOrSignature {
    Uuid(Uuid),
    Signature(Signature),
}

impl std::fmt::Display for UuidOrSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UuidOrSignature::Uuid(uuid) => write!(f, "{}", uuid),
            UuidOrSignature::Signature(signature) => write!(f, "{}", signature),
        }
    }
}

impl<'de> Deserialize<'de> for UuidOrSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        if let Ok(uuid) = Uuid::parse_str(&s) {
            return Ok(UuidOrSignature::Uuid(uuid));
        }

        if let Ok(signature) = s.parse::<Signature>() {
            return Ok(UuidOrSignature::Signature(signature));
        }

        Err(serde::de::Error::custom(
            "expected a Uuid or a valid Solana Signature",
        ))
    }
}

impl<'de> Serialize for UuidOrSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            UuidOrSignature::Uuid(uuid) => serializer.serialize_str(&uuid.to_string()),
            UuidOrSignature::Signature(signature) => {
                serializer.serialize_str(&signature.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum DataIndexingCommand {
    ProcessCollection(Uuid),
    ProcessCollectionEntriesPack(Uuid, Vec<u8>),
}

// Define a wrapper struct
#[derive(Debug, Clone)]
pub struct VersionedIdl(pub Slot, pub Idl);

// Implement ordering based on Slot
impl PartialEq for VersionedIdl {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for VersionedIdl {}

impl PartialOrd for VersionedIdl {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionedIdl {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, value::Index};
    use solana_account_decoder_client_types::{ParsedAccount, UiAccountData};

    use super::*;

    #[test]
    fn print_ui_keyed_profile_result() {
        let pubkey = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let readonly_account_state = UiAccount {
            lamports: 100,
            data: UiAccountData::Binary(
                "ABCDEFG".into(),
                solana_account_decoder_client_types::UiAccountEncoding::Base64,
            ),
            owner: owner.to_string(),
            executable: false,
            rent_epoch: 0,
            space: Some(100),
        };

        let account_1 = UiAccount {
            lamports: 100,
            data: UiAccountData::Json(ParsedAccount {
                program: "custom-program".into(),
                parsed: json!({
                    "field1": "value1",
                    "field2": "value2"
                }),
                space: 50,
            }),
            owner: owner.to_string(),
            executable: false,
            rent_epoch: 0,
            space: Some(100),
        };

        let account_2 = UiAccount {
            lamports: 100,
            data: UiAccountData::Json(ParsedAccount {
                program: "custom-program".into(),
                parsed: json!({
                    "field1": "updated-value1",
                    "field2": "updated-value2"
                }),
                space: 50,
            }),
            owner: owner.to_string(),
            executable: false,
            rent_epoch: 0,
            space: Some(100),
        };
        let profile_result = UiKeyedProfileResult {
            slot: 123,
            key: UuidOrSignature::Uuid(Uuid::new_v4()),
            instruction_profiles: Some(vec![
                UiProfileResult {
                    account_states: IndexMap::from_iter([
                        (
                            pubkey,
                            UiAccountProfileState::Writable(UiAccountChange::Create(
                                account_1.clone(),
                            )),
                        ),
                        (owner, UiAccountProfileState::Readonly),
                    ]),
                    compute_units_consumed: 100,
                    log_messages: Some(vec![
                        "Log message: Creating Account".to_string(),
                        "Log message: Account created".to_string(),
                    ]),
                    error_message: None,
                },
                UiProfileResult {
                    account_states: IndexMap::from_iter([
                        (
                            pubkey,
                            UiAccountProfileState::Writable(UiAccountChange::Update(
                                account_1,
                                account_2.clone(),
                            )),
                        ),
                        (owner, UiAccountProfileState::Readonly),
                    ]),
                    compute_units_consumed: 100,
                    log_messages: Some(vec![
                        "Log message: Updating Account".to_string(),
                        "Log message: Account updated".to_string(),
                    ]),
                    error_message: None,
                },
                UiProfileResult {
                    account_states: IndexMap::from_iter([
                        (
                            pubkey,
                            UiAccountProfileState::Writable(UiAccountChange::Delete(account_2)),
                        ),
                        (owner, UiAccountProfileState::Readonly),
                    ]),
                    compute_units_consumed: 100,
                    log_messages: Some(vec![
                        "Log message: Deleting Account".to_string(),
                        "Log message: Account deleted".to_string(),
                    ]),
                    error_message: None,
                },
            ]),
            transaction_profile: UiProfileResult {
                account_states: IndexMap::from_iter([
                    (
                        pubkey,
                        UiAccountProfileState::Writable(UiAccountChange::Unchanged(None)),
                    ),
                    (owner, UiAccountProfileState::Readonly),
                ]),
                compute_units_consumed: 300,
                log_messages: Some(vec![
                    "Log message: Creating Account".to_string(),
                    "Log message: Account created".to_string(),
                    "Log message: Updating Account".to_string(),
                    "Log message: Account updated".to_string(),
                    "Log message: Deleting Account".to_string(),
                    "Log message: Account deleted".to_string(),
                ]),
                error_message: None,
            },
            readonly_account_states: IndexMap::from_iter([(owner, readonly_account_state)]),
        };
        println!("{}", serde_json::to_string_pretty(&profile_result).unwrap());
    }
}
