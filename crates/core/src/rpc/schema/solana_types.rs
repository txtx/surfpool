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
    pub with_context: Option<bool>,
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
    pub encoding: Option<UiAccountEncoding>,
    pub data_slice: Option<UiDataSliceConfig>,
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
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
