use schemars::JsonSchema;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcProgramAccountsConfig {
    #[schemars(
        description = "Filters to apply to the program accounts. Each filter is a base58-encoded string representing an address or a specific filter type."
    )]
    pub filters: Option<Vec<RpcFilterType>>,
    #[serde(flatten)]
    #[schemars(description = "Configuration for the account info.")]
    pub account_config: RpcAccountInfoConfig,
    #[schemars(description = "Whether to include the context in the response.")]
    pub with_context: Option<bool>,
    #[schemars(description = "Whether to sort the results.")]
    pub sort_results: Option<bool>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcFilterType {
    #[schemars(description = "Filter by program ID")]
    ProgramId(String),
    #[schemars(description = "Filter by data size")]
    DataSize(u64),
    #[schemars(description = "Filter by memory comparison")]
    Memcmp(Memcmp),
    #[schemars(description = "Filter by token account state")]
    TokenAccountState,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Memcmp {
    /// Data offset to begin match
    offset: usize,
    /// Bytes, encoded with specified encoding
    #[serde(flatten)]
    bytes: MemcmpEncodedBytes,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", tag = "encoding", content = "bytes")]
pub enum MemcmpEncodedBytes {
    #[schemars(description = "Base-58 encoded bytes.")]
    Base58(String),
    #[schemars(description = "Base-64 encoded bytes.")]
    Base64(String),
    #[schemars(description = "Raw byte array.")]
    Bytes(Vec<u8>),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcAccountInfoConfig {
    #[schemars(description = "The encoding for the account data.")]
    pub encoding: Option<UiAccountEncoding>,
    #[schemars(description = "The data slice configuration.")]
    pub data_slice: Option<UiDataSliceConfig>,
    #[schemars(description = "The commitment level for the account info.")]
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The minimum context slot for the account info.")]
    pub min_context_slot: Option<Slot>,
}
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiAccountEncoding {
    #[schemars(description = "Binary encoding (legacy).")]
    Binary, // Legacy. Retained for RPC backwards compatibility
    #[schemars(description = "Base-58 encoding.")]
    Base58,
    #[schemars(description = "Base-64 encoding.")]
    Base64,
    #[schemars(description = "JSON parsed encoding.")]
    JsonParsed,
    #[serde(rename = "base64+zstd")]
    #[schemars(description = "Base-64 encoded with Zstandard compression.")]
    Base64Zstd,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiDataSliceConfig {
    #[schemars(description = "The offset of the data slice.")]
    pub offset: usize,
    #[schemars(description = "The length of the data slice.")]
    pub length: usize,
}

#[derive(JsonSchema)]
pub struct CommitmentConfig {
    #[schemars(description = "The commitment level.")]
    pub commitment: CommitmentLevel,
}

#[derive(JsonSchema)]
/// An attribute of a slot. It describes how finalized a block is at some point in time. For example, a slot
/// is said to be at the max level immediately after the cluster recognizes the block at that slot as
/// finalized. When querying the ledger state, use lower levels of commitment to report progress and higher
/// levels to ensure state changes will not be rolled back.
pub enum CommitmentLevel {
    /// The highest slot of the heaviest fork processed by the node. Ledger state at this slot is
    /// not derived from a confirmed or finalized block, but if multiple forks are present, is from
    /// the fork the validator believes is most likely to finalize.
    Processed,

    /// The highest slot that has been voted on by supermajority of the cluster, ie. is confirmed.
    /// Confirmation incorporates votes from gossip and replay. It does not count votes on
    /// descendants of a block, only direct votes on that block, and upholds "optimistic
    /// confirmation" guarantees in release 1.3 and onwards.
    Confirmed,

    /// The highest slot having reached max vote lockout, as recognized by a supermajority of the
    /// cluster.
    Finalized,
}

pub type Slot = u64;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcLargestAccountsConfig {
    #[schemars(description = "The commitment level for the largest accounts.")]
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The filter to apply to the largest accounts.")]
    pub filter: Option<RpcLargestAccountsFilter>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcLargestAccountsFilter {
    #[schemars(description = "Filter for circulating accounts.")]
    Circulating,
    #[schemars(description = "Filter for non-circulating accounts.")]
    NonCirculating,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcSupplyConfig {
    #[schemars(description = "The commitment level for the supply.")]
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "Whether to exclude non-circulating accounts.")]
    pub exclude_non_circulating_accounts_list: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcTokenAccountsFilter {
    #[schemars(description = "Filter by mint address.")]
    Mint(String),
    #[schemars(description = "Filter by program ID.")]
    ProgramId(String),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcContextConfig {
    #[schemars(description = "The commitment level for the context.")]
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The minimum context slot for the context.")]
    pub min_context_slot: Option<Slot>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcEpochConfig {
    #[schemars(description = "The epoch number to query.")]
    pub epoch: Option<u64>,
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The minimum context slot for the epoch.")]
    pub min_context_slot: Option<Slot>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcSendTransactionConfig {
    #[schemars(description = "Whether to skip the preflight check.")]
    pub skip_preflight: bool,
    #[schemars(description = "The commitment level for the preflight check.")]
    pub preflight_commitment: Option<CommitmentLevel>,
    #[schemars(description = "The encoding for the transaction.")]
    pub encoding: Option<UiTransactionEncoding>,
    #[schemars(description = "The maximum number of retries for the transaction.")]
    pub max_retries: Option<usize>,
    #[schemars(description = "The minimum context slot for the transaction.")]
    pub min_context_slot: Option<Slot>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiTransactionEncoding {
    #[schemars(description = "Binary encoding (legacy).")]
    Binary, // Legacy. Retained for RPC backwards compatibility
    #[schemars(description = "Base-58 encoding.")]
    Base58,
    #[schemars(description = "Base-64 encoding.")]
    Base64,
    #[schemars(description = "JSON encoding.")]
    Json,
    #[schemars(description = "JSON parsed encoding.")]
    JsonParsed,
    #[serde(rename = "base64+zstd")]
    #[schemars(description = "Base-64 encoded with Zstandard compression.")]
    Base64Zstd,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlockProductionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Filter by validator identity, as a base-58 encoded string.")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Slot range to query.")]
    pub range: Option<RpcBlockProductionConfigRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlockProductionConfigRange {
    #[schemars(description = "The first slot to include in the range.")]
    pub first_slot: Slot,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The last slot to include in the range.")]
    pub last_slot: Option<Slot>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcSignatureStatusConfig {
    #[schemars(description = "Whether to search the transaction history.")]
    pub search_transaction_history: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequestAirdropConfig {
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcSimulateTransactionConfig {
    #[schemars(description = "Whether to verify transaction signatures.")]
    pub sig_verify: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(description = "Whether to replace the recent blockhash with a new one.")]
    pub replace_recent_blockhash: bool,
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Encoding for the transaction data.")]
    pub encoding: Option<UiTransactionEncoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Accounts to return in the simulation result.")]
    pub accounts: Option<RpcSimulateTransactionAccountsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_context_slot: Option<Slot>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(description = "Whether to include inner instructions in the simulation result.")]
    pub inner_instructions: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcSimulateTransactionAccountsConfig {
    #[schemars(description = "Encoding for the account data.")]
    pub encoding: Option<UiAccountEncoding>,
    #[schemars(
        description = "An array of account addresses to return, as base-58 encoded strings."
    )]
    pub addresses: Vec<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransactionDetails {
    #[schemars(description = "Return full transaction details.")]
    Full,
    #[schemars(description = "Return only account details.")]
    Accounts,
    #[schemars(description = "Return only signature details.")]
    Signatures,
    #[schemars(description = "Return no transaction details.")]
    None,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlockConfig {
    #[schemars(description = "Encoding for transaction data.")]
    pub encoding: Option<UiTransactionEncoding>,
    #[schemars(description = "Level of transaction detail to return.")]
    pub transaction_details: Option<TransactionDetails>,
    #[schemars(description = "Whether to return rewards.")]
    pub rewards: Option<bool>,
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The maximum transaction version to support.")]
    pub max_supported_transaction_version: Option<u8>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransactionConfig {
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
    #[schemars(description = "The maximum transaction version to support.")]
    pub max_supported_transaction_version: Option<u8>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum RpcBlocksConfigWrapper {
    #[schemars(description = "Specify only the end slot.")]
    EndSlotOnly(Option<Slot>),
    #[schemars(description = "Specify only the configuration.")]
    ConfigOnly(RpcContextConfig),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcGetVoteAccountsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Filter by vote account public key.")]
    pub vote_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(description = "Whether to keep unstaked delinquent vote accounts.")]
    pub keep_unstaked_delinquents: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The distance in slots to consider a vote account delinquent.")]
    pub delinquent_slot_distance: Option<u64>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum RpcLeaderScheduleConfigWrapper {
    #[schemars(description = "Specify only the slot.")]
    SlotOnly(Option<Slot>),
    #[schemars(description = "Specify only the configuration.")]
    ConfigOnly(RpcLeaderScheduleConfig),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcLeaderScheduleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Filter by validator identity.")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}
