use std::{collections::HashMap, net::SocketAddr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use solana_clock::{Epoch, Slot, UnixTimestamp};
use solana_sdk::inflation::Inflation;
use surfpool_types::types::{
    ComputeUnitsEstimationResult, ProfileResult as SurfpoolProfileResult,
    ProfileState as SurfpoolProfileState,
};

pub const MAX_LOCKOUT_HISTORY: usize = 31;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Fee calculator with lamports per signature")]
pub struct FeeCalculatorSchema {
    #[schemars(description = "Cost in lamports to validate a signature")]
    pub lamports_per_signature: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Fee rate governor configuration")]
pub struct FeeRateGovernorSchema {
    #[schemars(description = "Target lamports per signature")]
    pub target_lamports_per_signature: u64,
    #[schemars(description = "Target signatures per slot")]
    pub target_signatures_per_slot: u64,
    #[schemars(description = "Minimum lamports per signature")]
    pub min_lamports_per_signature: u64,
    #[schemars(description = "Maximum lamports per signature")]
    pub max_lamports_per_signature: u64,
    #[schemars(description = "Burn percent")]
    pub burn_percent: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "UI representation of an account")]
pub struct UiAccountSchema {
    #[schemars(description = "Account balance in lamports")]
    pub lamports: u64,
    #[schemars(description = "Account data")]
    pub data: Vec<String>,
    #[schemars(description = "Program that owns this account")]
    pub owner: String,
    #[schemars(description = "Whether this account contains executable code")]
    pub executable: bool,
    #[schemars(description = "Epoch at which this account will next owe rent")]
    pub rent_epoch: u64,
}

impl From<solana_account_decoder::UiAccount> for UiAccountSchema {
    fn from(ui_account: solana_account_decoder::UiAccount) -> Self {
        let data = match ui_account.data {
            solana_account_decoder::UiAccountData::Binary(data, _) => vec![data],
            solana_account_decoder::UiAccountData::Json(json) => {
                vec![serde_json::to_string(&json).unwrap_or_default()]
            }
            _ => vec![],
        };

        Self {
            lamports: ui_account.lamports,
            data,
            owner: ui_account.owner,
            executable: ui_account.executable,
            rent_epoch: ui_account.rent_epoch,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction error information")]
pub struct TransactionErrorSchema {
    #[schemars(description = "Error type")]
    pub error_type: String,
    #[schemars(description = "Error message")]
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction result")]
pub struct TransactionResultSchema {
    #[schemars(description = "Success indicator")]
    pub success: bool,
    #[schemars(description = "Error if transaction failed")]
    pub error: Option<TransactionErrorSchema>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Token amount information")]
pub struct UiTokenAmountSchema {
    #[schemars(description = "Token amount as string")]
    pub amount: Option<String>,
    #[schemars(description = "Number of decimals")]
    pub decimals: u8,
    #[schemars(description = "Human readable amount as float")]
    pub ui_amount: Option<f64>,
    #[schemars(description = "Human readable amount as string")]
    pub ui_amount_string: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction confirmation status")]
pub struct TransactionConfirmationStatusSchema {
    #[schemars(description = "Confirmation level")]
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Confirmed block information")]
pub struct UiConfirmedBlockSchema {
    #[schemars(description = "Previous block hash")]
    pub previous_blockhash: String,
    #[schemars(description = "Block hash")]
    pub blockhash: String,
    #[schemars(description = "Parent slot")]
    pub parent_slot: u64,
    #[schemars(description = "Block time")]
    pub block_time: Option<i64>,
    #[schemars(description = "Block height")]
    pub block_height: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction return data")]
pub struct UiTransactionReturnDataSchema {
    #[schemars(description = "Program ID that returned the data")]
    pub program_id: String,
    #[schemars(description = "Returned data")]
    pub data: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Inner instructions")]
pub struct UiInnerInstructionsSchema {
    #[schemars(description = "Instruction index")]
    pub index: u8,
    #[schemars(description = "List of instructions")]
    pub instructions: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Block commitment information with optional commitment and total stake")]
pub struct RpcBlockCommitment<T> {
    pub commitment: Option<T>,
    pub total_stake: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Blockhash with associated fee calculator")]
pub struct RpcBlockhashFeeCalculator {
    pub blockhash: String,
    pub fee_calculator: FeeCalculatorSchema,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Blockhash with last valid block height")]
pub struct RpcBlockhash {
    pub blockhash: String,
    pub last_valid_block_height: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Fee calculator information")]
pub struct RpcFeeCalculator {
    #[schemars(description = "Fee calculator with lamports per signature")]
    pub fee_calculator: FeeCalculatorSchema,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Fee rate governor information")]
pub struct RpcFeeRateGovernor {
    pub fee_rate_governor: FeeRateGovernorSchema,
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Inflation governor parameters")]
pub struct RpcInflationGovernor {
    pub initial: f64,
    pub terminal: f64,
    pub taper: f64,
    pub foundation: f64,
    pub foundation_term: f64,
}

impl From<Inflation> for RpcInflationGovernor {
    fn from(inflation: Inflation) -> Self {
        Self {
            initial: inflation.initial,
            terminal: inflation.terminal,
            taper: inflation.taper,
            foundation: inflation.foundation,
            foundation_term: inflation.foundation_term,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Inflation rate information for a specific epoch")]
pub struct RpcInflationRate {
    pub total: f64,
    pub validator: f64,
    pub foundation: f64,
    pub epoch: Epoch,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account information with associated public key")]
pub struct RpcKeyedAccount {
    pub pubkey: String,
    pub account: UiAccountSchema,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[schemars(description = "Slot information including current slot, parent slot, and root slot")]
pub struct SlotInfo {
    pub slot: Slot,
    pub parent: Slot,
    pub root: Slot,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction statistics for a slot")]
pub struct SlotTransactionStats {
    pub num_transaction_entries: u64,
    pub num_successful_transactions: u64,
    pub num_failed_transactions: u64,
    pub max_transactions_per_entry: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "type")]
#[schemars(description = "Slot update notifications from the validator")]
pub enum SlotUpdate {
    #[schemars(description = "First shred received for a slot")]
    FirstShredReceived { slot: Slot, timestamp: u64 },
    #[schemars(description = "Slot completed")]
    Completed { slot: Slot, timestamp: u64 },
    #[schemars(description = "Bank created for a slot")]
    CreatedBank {
        slot: Slot,
        parent: Slot,
        timestamp: u64,
    },
    #[schemars(description = "Slot frozen with transaction statistics")]
    Frozen {
        slot: Slot,
        timestamp: u64,
        stats: SlotTransactionStats,
    },
    #[schemars(description = "Slot marked as dead due to an error")]
    Dead {
        slot: Slot,
        timestamp: u64,
        err: String,
    },
    #[schemars(description = "Optimistic confirmation for a slot")]
    OptimisticConfirmation { slot: Slot, timestamp: u64 },
    #[schemars(description = "Slot marked as root")]
    Root { slot: Slot, timestamp: u64 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", untagged)]
#[schemars(description = "Signature confirmation result")]
pub enum RpcSignatureResult {
    ProcessedSignature(ProcessedSignatureResult),
    ReceivedSignature(ReceivedSignatureResult),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Log response containing signature, error, and log messages")]
pub struct RpcLogsResponse {
    pub signature: String,
    pub err: Option<TransactionErrorSchema>,
    pub logs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Processed signature result with optional error")]
pub struct ProcessedSignatureResult {
    pub err: Option<TransactionErrorSchema>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Received signature confirmation")]
pub enum ReceivedSignatureResult {
    ReceivedSignature,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Contact information for a cluster node")]
pub struct RpcContactInfo {
    pub pubkey: String,
    pub gossip: Option<SocketAddrSchema>,
    pub tvu: Option<SocketAddrSchema>,
    pub tpu: Option<SocketAddrSchema>,
    pub tpu_quic: Option<SocketAddrSchema>,
    pub tpu_forwards: Option<SocketAddrSchema>,
    pub tpu_forwards_quic: Option<SocketAddrSchema>,
    pub tpu_vote: Option<SocketAddrSchema>,
    pub serve_repair: Option<SocketAddrSchema>,
    pub rpc: Option<SocketAddrSchema>,
    pub pubsub: Option<SocketAddrSchema>,
    pub version: Option<String>,
    pub feature_set: Option<u32>,
    pub shred_version: Option<u16>,
}

pub type RpcLeaderSchedule = HashMap<String, Vec<usize>>;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Block production range information")]
pub struct RpcBlockProductionRange {
    pub first_slot: Slot,
    pub last_slot: Slot,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Block production statistics by validator identity")]
pub struct RpcBlockProduction {
    pub by_identity: HashMap<String, (usize, usize)>,
    pub range: RpcBlockProductionRange,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(description = "Version information for the Solana software")]
pub struct RpcVersionInfo {
    pub solana_core: String,
    pub feature_set: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(description = "Identity information for the current node")]
pub struct RpcIdentity {
    pub identity: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Vote information including account, slots, hash, and signature")]
pub struct RpcVote {
    pub vote_pubkey: String,
    pub slots: Vec<Slot>,
    pub hash: String,
    pub timestamp: Option<UnixTimestamp>,
    pub signature: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Vote account status including current and delinquent accounts")]
pub struct RpcVoteAccountStatus {
    pub current: Vec<RpcVoteAccountInfo>,
    pub delinquent: Vec<RpcVoteAccountInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Detailed information about a vote account")]
pub struct RpcVoteAccountInfo {
    pub vote_pubkey: String,
    pub node_pubkey: String,
    pub activated_stake: u64,
    pub commission: u8,
    pub epoch_vote_account: bool,
    pub epoch_credits: Vec<(Epoch, u64, u64)>,
    pub last_vote: u64,
    pub root_slot: Slot,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Signature confirmation information")]
pub struct RpcSignatureConfirmation {
    pub confirmations: usize,
    pub status: TransactionResultSchema,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction simulation result")]
pub struct RpcSimulateTransactionResult {
    pub err: Option<TransactionErrorSchema>,
    pub logs: Option<Vec<String>>,
    pub accounts: Option<Vec<Option<UiAccountSchema>>>,
    pub units_consumed: Option<u64>,
    pub return_data: Option<UiTransactionReturnDataSchema>,
    pub inner_instructions: Option<Vec<UiInnerInstructionsSchema>>,
    pub replacement_blockhash: Option<RpcBlockhash>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Storage turn information")]
pub struct RpcStorageTurn {
    pub blockhash: String,
    pub slot: Slot,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account balance information")]
pub struct RpcAccountBalance {
    pub address: String,
    pub lamports: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(
    description = "Supply information including total, circulating, and non-circulating amounts"
)]
pub struct RpcSupply {
    pub total: u64,
    pub circulating: u64,
    pub non_circulating: u64,
    pub non_circulating_accounts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Stake activation state")]
pub enum StakeActivationState {
    Activating,
    Active,
    Deactivating,
    Inactive,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Token account balance information")]
pub struct RpcTokenAccountBalance {
    pub address: String,
    #[serde(flatten)]
    pub amount: UiTokenAmountSchema,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Confirmed transaction status with signature and metadata")]
pub struct RpcConfirmedTransactionStatusWithSignature {
    pub signature: String,
    pub slot: Slot,
    pub err: Option<TransactionErrorSchema>,
    pub memo: Option<String>,
    pub block_time: Option<UnixTimestamp>,
    pub confirmation_status: Option<TransactionConfirmationStatusSchema>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Performance sample data")]
pub struct RpcPerfSample {
    pub slot: Slot,
    pub num_transactions: u64,
    pub num_non_vote_transactions: Option<u64>,
    pub num_slots: u64,
    pub sample_period_secs: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Inflation reward information for an account")]
pub struct RpcInflationReward {
    pub epoch: Epoch,
    pub effective_slot: Slot,
    pub amount: u64,
    pub post_balance: u64,
    pub commission: Option<u8>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Block update notification")]
pub struct RpcBlockUpdate {
    pub slot: Slot,
    pub block: Option<UiConfirmedBlockSchema>,
    pub err: Option<RpcBlockUpdateError>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Block update error")]
pub enum RpcBlockUpdateError {
    UnsupportedTransactionVersion(u8),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[schemars(description = "Snapshot slot information")]
pub struct RpcSnapshotSlotInfo {
    pub full: Slot,
    pub incremental: Option<Slot>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Prioritization fee information for a slot")]
pub struct RpcPrioritizationFee {
    pub slot: Slot,
    pub prioritization_fee: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Node information including identity, addresses, and version")]
pub struct RpcNodeInfo {
    pub identity: String,
    pub gossip: Option<SocketAddrSchema>,
    pub rpc: Option<SocketAddrSchema>,
    pub version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Response context containing slot and API version information")]
pub struct RpcResponseContext {
    #[schemars(description = "The current slot")]
    pub slot: Slot,
    #[schemars(description = "The API version")]
    pub api_version: Option<String>,
}

impl RpcResponseContext {
    pub fn new(slot: Slot) -> Self {
        Self {
            slot,
            api_version: Some("1.0.0".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "RPC response wrapper with context")]
pub struct RpcResponse<T> {
    pub context: RpcResponseContext,
    pub value: T,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", untagged)]
#[schemars(description = "Optional response context wrapper")]
pub enum OptionalContext<T> {
    Context(RpcResponse<T>),
    NoContext(T),
}

pub type BlockCommitmentArray = [u64; MAX_LOCKOUT_HISTORY + 1];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction status information")]
pub struct TransactionStatus {
    pub slot: Slot,
    pub confirmations: Option<usize>,
    pub status: TransactionResultSchema,
    pub err: Option<TransactionErrorSchema>,
    pub confirmation_status: Option<TransactionConfirmationStatusSchema>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Surfpool RPC version information - used by: get_surfpool_version")]
pub struct SurfpoolRpcVersionInfo {
    pub surfnet_version: String,
    pub solana_core: String,
    pub feature_set: Option<u32>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Complete mapping of all Surfpool RPC endpoints to their response types")]
pub struct SurfpoolRpcEndpoints {
    pub minimal: MinimalEndpoints,
    pub full: FullEndpoints,
    pub admin: AdminEndpoints,
    pub accounts_data: AccountsDataEndpoints,
    pub accounts_scan: AccountsScanEndpoints,
    pub bank_data: BankDataEndpoints,
    pub surfnet_cheatcodes: SurfnetCheatcodesEndpoints,
    pub websocket_subscriptions: WebSocketSubscriptions,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Minimal RPC endpoints providing basic cluster information")]
pub struct MinimalEndpoints {
    #[schemars(description = "getBalance - Returns the balance for a given address")]
    pub get_balance: RpcResponse<u64>,

    #[schemars(description = "getEpochInfo - Returns epoch information")]
    pub get_epoch_info: RpcResponse<EpochInfoSchema>,

    #[schemars(description = "getGenesisHash - Returns the genesis hash")]
    pub get_genesis_hash: String,

    #[schemars(description = "getHealth - Returns the health of the cluster")]
    pub get_health: String,

    #[schemars(description = "getIdentity - Returns the identity of the cluster")]
    pub get_identity: RpcIdentity,

    #[schemars(description = "getSlot - Returns the current slot")]
    pub get_slot: RpcResponse<u64>,

    #[schemars(description = "getBlockHeight - Returns the block height")]
    pub get_block_height: RpcResponse<u64>,

    #[schemars(description = "getHighestSnapshotSlot - Returns the highest snapshot slot")]
    pub get_highest_snapshot_slot: RpcSnapshotSlotInfo,

    #[schemars(description = "getTransactionCount - Returns the transaction count")]
    pub get_transaction_count: RpcResponse<u64>,

    #[schemars(description = "getVersion - Returns the version of the cluster")]
    pub get_version: RpcVersionInfo,

    #[schemars(description = "getVoteAccounts - Returns the vote accounts")]
    pub get_vote_accounts: RpcVoteAccountStatus,

    #[schemars(description = "getLeaderSchedule - Returns the leader schedule")]
    pub get_leader_schedule: RpcLeaderSchedule,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(
    description = "Full RPC endpoints providing comprehensive transaction and block information"
)]
pub struct FullEndpoints {
    #[schemars(description = "getInflationReward - Returns inflation rewards for given addresses")]
    pub get_inflation_reward: Vec<RpcInflationReward>,

    #[schemars(description = "getClusterNodes - Returns cluster nodes information")]
    pub get_cluster_nodes: Vec<RpcContactInfo>,

    #[schemars(description = "getRecentPerformanceSamples - Returns recent performance samples")]
    pub get_recent_performance_samples: Vec<RpcPerfSample>,

    #[schemars(description = "getSignatureStatuses - Returns signature statuses")]
    pub get_signature_statuses: RpcResponse<Vec<Option<TransactionStatus>>>,

    #[schemars(description = "getMaxRetransmitSlot - Returns maximum retransmit slot")]
    pub get_max_retransmit_slot: u64,

    #[schemars(description = "getMaxShredInsertSlot - Returns maximum shred insert slot")]
    pub get_max_shred_insert_slot: u64,

    #[schemars(description = "requestAirdrop - Requests an airdrop to a given address")]
    pub request_airdrop: String, // Transaction signature

    #[schemars(description = "sendTransaction - Sends a transaction to the cluster")]
    pub send_transaction: String, // Transaction signature

    #[schemars(description = "simulateTransaction - Simulates a transaction")]
    pub simulate_transaction: RpcResponse<RpcSimulateTransactionResult>,

    #[schemars(description = "minimumLedgerSlot - Returns minimum ledger slot")]
    pub minimum_ledger_slot: u64,

    #[schemars(description = "getBlock - Returns block information for a given slot")]
    pub get_block: Option<UiConfirmedBlockSchema>,

    #[schemars(description = "getBlockTime - Returns block time for a given slot")]
    pub get_block_time: Option<UnixTimestamp>,

    #[schemars(description = "getBlocks - Returns blocks for a range of slots")]
    pub get_blocks: Vec<u64>,

    #[schemars(description = "getBlocksWithLimit - Returns blocks with limit")]
    pub get_blocks_with_limit: Vec<u64>,

    #[schemars(description = "getTransaction - Returns transaction information")]
    pub get_transaction: Option<EncodedConfirmedTransactionSchema>,

    #[schemars(description = "getSignaturesForAddress - Returns signatures for an address")]
    pub get_signatures_for_address: Vec<RpcConfirmedTransactionStatusWithSignature>,

    #[schemars(description = "getFirstAvailableBlock - Returns first available block")]
    pub get_first_available_block: u64,

    #[schemars(description = "getLatestBlockhash - Returns latest blockhash")]
    pub get_latest_blockhash: RpcResponse<RpcBlockhash>,

    #[schemars(description = "isBlockhashValid - Checks if blockhash is valid")]
    pub is_blockhash_valid: RpcResponse<bool>,

    #[schemars(description = "getFeeForMessage - Returns fee for a message")]
    pub get_fee_for_message: RpcResponse<Option<u64>>,

    #[schemars(description = "getStakeMinimumDelegation - Returns stake minimum delegation")]
    pub get_stake_minimum_delegation: RpcResponse<u64>,

    #[schemars(description = "getRecentPrioritizationFees - Returns recent prioritization fees")]
    pub get_recent_prioritization_fees: Vec<RpcPrioritizationFee>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Admin RPC endpoints for cluster administration")]
pub struct AdminEndpoints {
    #[schemars(description = "exit - Immediately shuts down the RPC server")]
    pub exit: (),

    #[schemars(description = "reloadPlugin - Reloads a runtime plugin")]
    pub reload_plugin: ReloadPlugin,

    #[schemars(description = "unloadPlugin - Unloads a runtime plugin")]
    pub unload_plugin: UnloadPlugin,

    #[schemars(description = "loadPlugin - Loads a new plugin")]
    pub load_plugin: LoadPlugin,

    #[schemars(description = "listPlugins - Lists all loaded plugins")]
    pub list_plugins: Vec<String>,

    #[schemars(description = "rpcAddress - Returns RPC server address")]
    pub rpc_address: SocketAddrSchema,

    #[schemars(description = "setLogFilter - Sets log filter")]
    pub set_log_filter: (),

    #[schemars(description = "startTime - Returns system start time")]
    pub start_time: SystemTimeSchema,

    #[schemars(description = "addAuthorizedVoter - Adds authorized voter")]
    pub add_authorized_voter: (),

    #[schemars(description = "addAuthorizedVoterFromBytes - Adds voter from bytes")]
    pub add_authorized_voter_from_bytes: (),

    #[schemars(description = "removeAllAuthorizedVoters - Removes all voters")]
    pub remove_all_authorized_voters: (),

    #[schemars(description = "setIdentity - Sets cluster identity")]
    pub set_identity: (),

    #[schemars(description = "setIdentityFromBytes - Sets identity from bytes")]
    pub set_identity_from_bytes: (),

    #[schemars(description = "setStakedNodesOverrides - Sets staked nodes overrides")]
    pub set_staked_nodes_overrides: (),

    #[schemars(description = "repairShredFromPeer - Repairs shred from peer")]
    pub repair_shred_from_peer: (),

    #[schemars(description = "setRepairWhitelist - Sets repair whitelist")]
    pub set_repair_whitelist: (),

    #[schemars(description = "getSecondaryIndexKeySize - Gets secondary index key size")]
    pub get_secondary_index_key_size: u64,

    #[schemars(description = "setPublicTpuAddress - Sets public TPU address")]
    pub set_public_tpu_address: (),

    #[schemars(description = "setPublicTpuForwardsAddress - Sets public TPU forwards address")]
    pub set_public_tpu_forwards_address: (),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account data RPC endpoints for retrieving account information")]
pub struct AccountsDataEndpoints {
    #[schemars(description = "getAccountInfo - Returns account information")]
    pub get_account_info: RpcResponse<Option<UiAccountSchema>>,

    #[schemars(description = "getBlockCommitment - Returns commitment levels for a given block")]
    pub get_block_commitment: RpcBlockCommitment<BlockCommitmentArray>,

    #[schemars(description = "getMultipleAccounts - Returns multiple accounts information")]
    pub get_multiple_accounts: RpcResponse<Vec<Option<UiAccountSchema>>>,

    #[schemars(description = "getProgramAccounts - Returns program-owned accounts")]
    pub get_program_accounts: Vec<RpcKeyedAccount>,

    #[schemars(description = "getLargestAccounts - Returns largest accounts by balance")]
    pub get_largest_accounts: RpcResponse<Vec<RpcAccountBalance>>,

    #[schemars(description = "getSupply - Returns supply information")]
    pub get_supply: RpcResponse<RpcSupply>,

    #[schemars(description = "getTokenAccountBalance - Returns token account balance")]
    pub get_token_account_balance: RpcResponse<UiTokenAmountSchema>,

    #[schemars(description = "getTokenAccountsByOwner - Returns token accounts by owner")]
    pub get_token_accounts_by_owner: RpcResponse<Vec<RpcKeyedAccount>>,

    #[schemars(description = "getTokenAccountsByDelegate - Returns token accounts by delegate")]
    pub get_token_accounts_by_delegate: RpcResponse<Vec<RpcKeyedAccount>>,

    #[schemars(description = "getTokenLargestAccounts - Returns largest token accounts")]
    pub get_token_largest_accounts: RpcResponse<Vec<RpcTokenAccountBalance>>,

    #[schemars(description = "getTokenSupply - Returns token supply information")]
    pub get_token_supply: RpcResponse<UiTokenAmountSchema>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account scan endpoints for filtered account searches")]
pub struct AccountsScanEndpoints {
    #[schemars(description = "scanAccounts - Scans accounts with filters")]
    pub scan_accounts: Vec<RpcKeyedAccount>,

    #[schemars(description = "getFilteredProgramAccounts - Returns filtered program accounts")]
    pub get_filtered_program_accounts: Vec<RpcKeyedAccount>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Bank data endpoints for blockchain state information")]
pub struct BankDataEndpoints {
    #[schemars(description = "getInflationGovernor - Returns inflation governor")]
    pub get_inflation_governor: RpcInflationGovernor,

    #[schemars(description = "getInflationRate - Returns inflation rate")]
    pub get_inflation_rate: RpcInflationRate,

    #[schemars(description = "getEpochSchedule - Returns epoch schedule")]
    pub get_epoch_schedule: EpochScheduleSchema,

    #[schemars(description = "getFees - Returns fee information (deprecated)")]
    pub get_fees: RpcFeeCalculator,

    #[schemars(description = "getFeeRateGovernor - Returns fee rate governor (deprecated)")]
    pub get_fee_rate_governor: RpcFeeRateGovernor,

    #[schemars(description = "getRent - Returns rent information")]
    pub get_rent: RentSchema,

    #[schemars(
        description = "getMinimumBalanceForRentExemption - Returns minimum balance for rent exemption"
    )]
    pub get_minimum_balance_for_rent_exemption: u64,

    #[schemars(description = "getSlotLeader - Returns the leader of the current slot")]
    pub get_slot_leader: String,

    #[schemars(description = "getSlotLeaders - Returns leaders for a specified range of slots")]
    pub get_slot_leaders: Vec<String>,

    #[schemars(description = "getBlockProduction - Returns block production information")]
    pub get_block_production: RpcResponse<RpcBlockProduction>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Surfnet cheatcodes endpoints for testing and development")]
pub struct SurfnetCheatcodesEndpoints {
    #[schemars(description = "startSurfnet - Starts surfnet instance")]
    pub start_surfnet: (),

    #[schemars(description = "stopSurfnet - Stops surfnet instance")]
    pub stop_surfnet: (),

    #[schemars(description = "resetSurfnet - Resets surfnet state")]
    pub reset_surfnet: (),

    #[schemars(description = "setAccount - Sets account data")]
    pub set_account: RpcResponse<()>,

    #[schemars(description = "setTokenAccount - Sets token account data")]
    pub set_token_account: RpcResponse<()>,

    #[schemars(description = "cloneProgramAccount - Clones a program account")]
    pub clone_program_account: RpcResponse<()>,

    #[schemars(
        description = "profileTransaction - Profiles a transaction for compute unit estimation"
    )]
    pub profile_transaction: RpcResponse<ProfileResultSchema>,

    #[schemars(description = "getProfileResults - Gets profiling results for a tag")]
    pub get_profile_results: RpcResponse<Vec<ProfileResultSchema>>,

    #[schemars(description = "setSupply - Sets supply information for testing")]
    pub set_supply: RpcResponse<()>,

    #[schemars(description = "getAccount - Gets account data")]
    pub get_account: Option<UiAccountSchema>,

    #[schemars(description = "advanceClock - Advances clock")]
    pub advance_clock: (),

    #[schemars(description = "setClock - Sets clock")]
    pub set_clock: (),

    #[schemars(description = "getClock - Gets clock information")]
    pub get_clock: ClockSchema,

    #[schemars(description = "processTransaction - Processes transaction")]
    pub process_transaction: (),

    #[schemars(description = "getSurfpoolVersion - Returns surfpool version")]
    pub get_surfpool_version: SurfpoolRpcVersionInfo,
}

// WebSocket subscription endpoints
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket subscription endpoints for real-time updates")]
pub struct WebSocketSubscriptions {
    #[schemars(description = "accountSubscribe - Subscribe to account changes")]
    pub account_subscribe: UiAccountSchema,

    #[schemars(description = "blockSubscribe - Subscribe to block updates")]
    pub block_subscribe: RpcBlockUpdate,

    #[schemars(description = "logsSubscribe - Subscribe to transaction logs")]
    pub logs_subscribe: RpcLogsResponse,

    #[schemars(description = "programSubscribe - Subscribe to program account changes")]
    pub program_subscribe: RpcKeyedAccount,

    #[schemars(description = "signatureSubscribe - Subscribe to signature confirmations")]
    pub signature_subscribe: RpcSignatureResult,

    #[schemars(description = "slotSubscribe - Subscribe to slot changes")]
    pub slot_subscribe: SlotInfo,

    #[schemars(description = "slotUpdatesSubscribe - Subscribe to slot updates")]
    pub slot_updates_subscribe: SlotUpdate,

    #[schemars(description = "voteSubscribe - Subscribe to vote account updates")]
    pub vote_subscribe: RpcVote,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Standard JSON-RPC error response format")]
pub struct RpcErrorResponse {
    #[schemars(description = "JSON-RPC error code")]
    pub code: i32,

    #[schemars(description = "Human-readable error message")]
    pub message: String,

    #[schemars(description = "Additional error data")]
    pub data: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(
    description = "Epoch information including current epoch, slot index, and slots in epoch"
)]
pub struct EpochInfoSchema {
    #[schemars(description = "Current epoch")]
    pub epoch: u64,
    #[schemars(description = "Current slot index within the epoch")]
    pub slot_index: u64,
    #[schemars(description = "Total number of slots in the epoch")]
    pub slots_in_epoch: u64,
    #[schemars(description = "Absolute root slot")]
    pub absolute_slot: u64,
    #[schemars(description = "Block height")]
    pub block_height: Option<u64>,
    #[schemars(description = "Current transaction count")]
    pub transaction_count: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Epoch schedule configuration")]
pub struct EpochScheduleSchema {
    #[schemars(description = "Number of slots in each epoch")]
    pub slots_per_epoch: u64,
    #[schemars(description = "Duration of leader schedule slot in each epoch")]
    pub leader_schedule_slot_offset: u64,
    #[schemars(description = "Whether epochs start short and grow")]
    pub warmup: bool,
    #[schemars(description = "First normal-length epoch, if any")]
    pub first_normal_epoch: u64,
    #[schemars(description = "First normal-length slot")]
    pub first_normal_slot: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Clock information including current slot and timestamp")]
pub struct ClockSchema {
    #[schemars(description = "Current slot")]
    pub slot: u64,
    #[schemars(description = "Estimated production time of the current slot as Unix timestamp")]
    pub epoch_start_timestamp: i64,
    #[schemars(description = "Current epoch")]
    pub epoch: u64,
    #[schemars(description = "Future leader schedule epoch")]
    pub leader_schedule_epoch: u64,
    #[schemars(description = "Unix timestamp of current slot")]
    pub unix_timestamp: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(
    description = "Rent configuration including lamports per byte-year and exemption threshold"
)]
pub struct RentSchema {
    #[schemars(description = "Rental rate in lamports/byte-year")]
    pub lamports_per_byte_year: u64,
    #[schemars(description = "Rent exemption threshold in years")]
    pub exemption_threshold: u64,
    #[schemars(description = "Percentage of collected rent burned")]
    pub burn_percent: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Encoded confirmed transaction with status metadata")]
pub struct EncodedConfirmedTransactionSchema {
    #[schemars(description = "The transaction slot")]
    pub slot: u64,
    #[schemars(description = "The encoded transaction")]
    pub transaction: EncodedTransactionSchema,
    #[schemars(description = "Transaction metadata")]
    pub meta: Option<TransactionMetaSchema>,
    #[schemars(description = "Block time")]
    pub block_time: Option<i64>,
    #[schemars(description = "Transaction version")]
    pub version: Option<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Encoded transaction information")]
pub struct EncodedTransactionSchema {
    #[schemars(description = "List of signatures")]
    pub signatures: Vec<String>,
    #[schemars(description = "Transaction message")]
    pub message: TransactionMessageSchema,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction message containing accounts and instructions")]
pub struct TransactionMessageSchema {
    #[schemars(description = "Number of required signatures")]
    pub num_required_signatures: u8,
    #[schemars(description = "Number of readonly signed accounts")]
    pub num_readonly_signed_accounts: u8,
    #[schemars(description = "Number of readonly unsigned accounts")]
    pub num_readonly_unsigned_accounts: u8,
    #[schemars(description = "List of account keys")]
    pub account_keys: Vec<String>,
    #[schemars(description = "Recent blockhash")]
    pub recent_blockhash: String,
    #[schemars(description = "List of instructions")]
    pub instructions: Vec<InstructionSchema>,
    #[schemars(description = "List of address table lookups")]
    pub address_table_lookups: Option<Vec<AddressTableLookupSchema>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction instruction")]
pub struct InstructionSchema {
    #[schemars(description = "Program ID index")]
    pub program_id_index: u8,
    #[schemars(description = "Account indices")]
    pub accounts: Vec<u8>,
    #[schemars(description = "Instruction data")]
    pub data: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Address table lookup")]
pub struct AddressTableLookupSchema {
    #[schemars(description = "Address table account key")]
    pub account_key: String,
    #[schemars(description = "Writable indices")]
    pub writable_indexes: Vec<u8>,
    #[schemars(description = "Readonly indices")]
    pub readonly_indexes: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Transaction metadata including fee, balances, and logs")]
pub struct TransactionMetaSchema {
    #[schemars(description = "Transaction error")]
    pub err: Option<TransactionErrorSchema>,
    #[schemars(description = "Transaction fee")]
    pub fee: u64,
    #[schemars(description = "Account balances before transaction")]
    pub pre_balances: Vec<u64>,
    #[schemars(description = "Account balances after transaction")]
    pub post_balances: Vec<u64>,
    #[schemars(description = "Inner instructions")]
    pub inner_instructions: Option<Vec<UiInnerInstructionsSchema>>,
    #[schemars(description = "Log messages")]
    pub log_messages: Option<Vec<String>>,
    #[schemars(description = "Pre token balances")]
    pub pre_token_balances: Option<Vec<UiTokenAmountSchema>>,
    #[schemars(description = "Post token balances")]
    pub post_token_balances: Option<Vec<UiTokenAmountSchema>>,
    #[schemars(description = "Return data")]
    pub return_data: Option<UiTransactionReturnDataSchema>,
    #[schemars(description = "Compute units consumed")]
    pub compute_units_consumed: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "System time representation")]
pub struct SystemTimeSchema {
    #[schemars(description = "Seconds since Unix epoch")]
    pub secs_since_epoch: u64,
    #[schemars(description = "Nanoseconds")]
    pub nanos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Result of compute units estimation")]
pub struct ComputeUnitsEstimationResultSchema {
    #[schemars(description = "Indicates if the estimation was successful")]
    pub success: bool,
    #[schemars(description = "Number of compute units consumed")]
    pub compute_units_consumed: u64,
    #[schemars(description = "Log messages from the transaction")]
    pub log_messages: Option<Vec<String>>,
    #[schemars(description = "Error message if estimation failed")]
    pub error_message: Option<String>,
}

impl From<ComputeUnitsEstimationResult> for ComputeUnitsEstimationResultSchema {
    fn from(result: ComputeUnitsEstimationResult) -> Self {
        Self {
            success: result.success,
            compute_units_consumed: result.compute_units_consumed,
            log_messages: result.log_messages,
            error_message: result.error_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "State of accounts before and after execution")]
pub struct ProfileStateSchema {
    #[schemars(description = "Account states before execution")]
    pub pre_execution: HashMap<String, Option<UiAccountSchema>>,
    #[schemars(description = "Account states after execution")]
    pub post_execution: HashMap<String, Option<UiAccountSchema>>,
}

impl From<SurfpoolProfileState> for ProfileStateSchema {
    fn from(state: SurfpoolProfileState) -> Self {
        let pre_execution = state
            .pre_execution
            .into_iter()
            .map(|(pubkey, account)| {
                (
                    pubkey.to_string(),
                    account.map(|acc| UiAccountSchema::from(acc)),
                )
            })
            .collect();
        let post_execution = state
            .post_execution
            .into_iter()
            .map(|(pubkey, account)| {
                (
                    pubkey.to_string(),
                    account.map(|acc| UiAccountSchema::from(acc)),
                )
            })
            .collect();
        Self {
            pre_execution,
            post_execution,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Profile result")]
pub struct ProfileResultSchema {
    #[schemars(description = "Compute units estimation result")]
    pub compute_units: ComputeUnitsEstimationResultSchema,
    #[schemars(description = "Profile state containing pre and post execution states")]
    pub state: ProfileStateSchema,
}

impl From<SurfpoolProfileResult> for ProfileResultSchema {
    fn from(result: SurfpoolProfileResult) -> Self {
        Self {
            compute_units: result.compute_units.into(),
            state: result.state.into(),
        }
    }
}
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReloadPlugin {
    pub name: String,
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnloadPlugin {
    pub name: String,
}
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadPlugin {
    pub config_file: String,
}
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Socket address")]
pub struct SocketAddrSchema {
    pub ip: String,
    pub port: u16,
}

impl From<SocketAddr> for SocketAddrSchema {
    fn from(addr: SocketAddr) -> Self {
        Self {
            ip: addr.ip().to_string(),
            port: addr.port(),
        }
    }
}
