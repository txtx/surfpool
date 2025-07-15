use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{
    RpcContextConfig, RpcGetVoteAccountsConfig, RpcLeaderScheduleConfig,
    RpcLeaderScheduleConfigWrapper,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum Minimal {
    #[schemars(description = "Returns the balance for a given address.")]
    GetBalance(GetBalance),
    #[schemars(description = "Returns the epoch info.")]
    GetEpochInfo(GetEpochInfo),
    #[schemars(description = "Returns the genesis hash.")]
    GetGenesisHash,
    #[schemars(description = "Returns the health of the cluster.")]
    GetHealth,
    #[schemars(description = "Returns the identity of the cluster.")]
    GetIdentity(GetIdentity),
    #[schemars(description = "Returns the current slot.")]
    GetSlot(GetSlot),
    #[schemars(description = "Returns the block height.")]
    GetBlockHeight(GetBlockHeight),
    #[schemars(description = "Returns the highest snapshot slot.")]
    GetHighestSnapshotSlot,
    #[schemars(description = "Returns the transaction count.")]
    GetTransactionCount(GetTransactionCount),
    #[schemars(description = "Returns the version of the cluster.")]
    GetVersion,
    #[schemars(description = "Returns the vote accounts.")]
    GetVoteAccounts(GetVoteAccounts),
    #[schemars(description = "Returns the leader schedule.")]
    GetLeaderSchedule(GetLeaderSchedule),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBalance {
    #[schemars(
        description = "The public key of the account to query, as a base-58 encoded string."
    )]
    pub pubkey: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetEpochInfo {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSlot {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockHeight {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTransactionCount {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetVoteAccounts {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcGetVoteAccountsConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetLeaderSchedule {
    #[schemars(description = "Wrapper for slot or configuration.")]
    pub options: Option<RpcLeaderScheduleConfigWrapper>,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcLeaderScheduleConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetIdentity {
    #[schemars(description = "The identity to query, as a base-58 encoded string.")]
    pub identity: String,
}
