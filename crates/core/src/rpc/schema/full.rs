use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{RpcAccountInfoConfig, Slot};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum Full {
    #[schemars(description = "Returns the inflation reward for a given address.")]
    GetInflationReward(GetInflationReward),
    #[schemars(description = "Returns the cluster nodes.")]
    GetClusterNodes,
    #[schemars(description = "Returns the recent performance samples.")]
    GetRecentPerformanceSamples(GetRecentPerformanceSamples),
    #[schemars(description = "Returns the signature statuses for a given signature.")]
    GetSignatureStatuses(GetSignatureStatuses),
    #[schemars(description = "Returns the maximum retransmit slot.")]
    GetMaxRetransmitSlot,
    #[schemars(description = "Returns the maximum shred insert slot.")]
    GetMaxShredInsertSlot,
    #[schemars(description = "Requests an airdrop to a given address.")]
    RequestAirdrop(RequestAirdrop),
    #[schemars(description = "Sends a transaction to the cluster.")]
    SendTransaction(SendTransaction),
    #[schemars(description = "Simulates a transaction.")]
    SimulateTransaction(SimulateTransaction),
    #[schemars(description = "Returns the minimum ledger slot.")]
    MinimumLedgerSlot,
    #[schemars(description = "Returns the block for a given slot.")]
    GetBlock(GetBlock),
    #[schemars(description = "Returns the block time for a given slot.")]
    GetBlockTime(GetBlockTime),
    #[schemars(description = "Returns the blocks for a given range of slots.")]
    GetBlocks(GetBlocks),
    #[schemars(description = "Returns the blocks for a given range of slots with a limit.")]
    GetBlocksWithLimit(GetBlocksWithLimit),
    #[schemars(description = "Returns the transaction for a given signature.")]
    GetTransaction(GetTransaction),
    #[schemars(description = "Returns the signatures for a given address.")]
    GetSignaturesForAddress(GetSignaturesForAddress),
    #[schemars(description = "Returns the first available block.")]
    GetFirstAvailableBlock,
    #[schemars(description = "Returns the latest blockhash.")]
    GetLatestBlockhash,
    #[schemars(description = "Returns the blockhash validity.")]
    IsBlockhashValid(IsBlockhashValid),
    #[schemars(description = "Returns the fee for a given message.")]
    GetFeeForMessage(GetFeeForMessage),
    #[schemars(description = "Returns the stake minimum delegation.")]
    GetStakeMinimumDelegation,
    #[schemars(description = "Returns the recent prioritization fees.")]
    GetRecentPrioritizationFees(GetRecentPrioritizationFees),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetInflationReward {
    pub addresses: Vec<String>,
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRecentPerformanceSamples {
    pub limit: Option<usize>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSignatureStatuses {
    pub signatures: Vec<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestAirdrop {
    pub pubkey: String,
    pub lamports: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendTransaction {
    pub transaction: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SimulateTransaction {
    pub transaction: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlock {
    pub slot: Slot,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockTime {
    pub slot: Slot,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlocks {
    pub start_slot: Slot,
    pub end_slot: Option<Slot>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlocksWithLimit {
    pub start_slot: Slot,
    pub limit: usize,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTransaction {
    pub signature: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSignaturesForAddress {
    pub address: String,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub until: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IsBlockhashValid {
    pub blockhash: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetFeeForMessage {
    pub message: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRecentPrioritizationFees {
    pub pubkeys: Option<Vec<String>>,
}
