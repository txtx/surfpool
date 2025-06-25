use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{RpcAccountInfoConfig, RpcContextConfig};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum Minimal {
    #[schemars(description = "Returns the balance for a given address.")]
    GetBalance(GetBalance),
    #[schemars(description = "Returns the epoch info.")]
    GetEpochInfo,
    #[schemars(description = "Returns the genesis hash.")]
    GetGenesisHash,
    #[schemars(description = "Returns the health of the cluster.")]
    GetHealth,
    #[schemars(description = "Returns the identity of the cluster.")]
    GetIdentity,
    #[schemars(description = "Returns the current slot.")]
    GetSlot,
    #[schemars(description = "Returns the block height.")]
    GetBlockHeight,
    #[schemars(description = "Returns the highest snapshot slot.")]
    GetHighestSnapshotSlot,
    #[schemars(description = "Returns the transaction count.")]
    GetTransactionCount,
    #[schemars(description = "Returns the version of the cluster.")]
    GetVersion,
    #[schemars(description = "Returns the vote accounts.")]
    GetVoteAccounts,
    #[schemars(description = "Returns the leader schedule.")]
    GetLeaderSchedule,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBalance {
    pub pubkey: String,
    pub config: Option<RpcContextConfig>,
}
