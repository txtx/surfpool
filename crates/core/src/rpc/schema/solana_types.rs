use schemars::JsonSchema;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcProgramAccountsConfig {
    #[schemars(
        description = "Filters to apply to the program accounts. Each filter is a base58-encoded string representing an address or a specific filter type."
    )]
    pub filters: Option<Vec<RpcFilterType>>,
    #[serde(flatten)]
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
    Base58(String),
    Base64(String),
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
    Binary, // Legacy. Retained for RPC backwards compatibility
    Base58,
    Base64,
    JsonParsed,
    #[serde(rename = "base64+zstd")]
    Base64Zstd,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiDataSliceConfig {
    pub offset: usize,
    pub length: usize,
}

#[derive(JsonSchema)]
pub struct CommitmentConfig {
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
    Circulating,
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
    Binary, // Legacy. Retained for RPC backwards compatibility
    Base58,
    Base64,
    Json,
    JsonParsed,
    #[serde(rename = "base64+zstd")]
    Base64Zstd,
}
