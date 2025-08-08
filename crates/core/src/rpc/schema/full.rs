use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{
    RpcBlockConfig, RpcBlocksConfigWrapper, RpcContextConfig, RpcEpochConfig,
    RpcRequestAirdropConfig, RpcSendTransactionConfig, RpcSignatureStatusConfig,
    RpcSimulateTransactionConfig, RpcTransactionConfig, Slot,
};

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
    GetLatestBlockhash(GetLatestBlockhash),
    #[schemars(description = "Returns the blockhash validity.")]
    IsBlockhashValid(IsBlockhashValid),
    #[schemars(description = "Returns the fee for a given message.")]
    GetFeeForMessage(GetFeeForMessage),
    #[schemars(description = "Returns the stake minimum delegation.")]
    GetStakeMinimumDelegation(GetStakeMinimumDelegation),
    #[schemars(description = "Returns the recent prioritization fees.")]
    GetRecentPrioritizationFees(GetRecentPrioritizationFees),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetInflationReward {
    #[schemars(description = "An array of public keys to query, as base-58 encoded strings.")]
    pub addresses: Vec<String>,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcEpochConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRecentPerformanceSamples {
    #[schemars(description = "The maximum number of samples to return.")]
    pub limit: Option<usize>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSignatureStatuses {
    #[schemars(
        description = "An array of transaction signatures to query, as base-58 encoded strings."
    )]
    pub signatures: Vec<String>,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcSignatureStatusConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestAirdrop {
    #[schemars(
        description = "The public key of the account to receive the airdrop, as a base-58 encoded string."
    )]
    pub pubkey: String,
    #[schemars(description = "The amount of lamports to airdrop.")]
    pub lamports: u64,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcRequestAirdropConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendTransaction {
    #[schemars(description = "The signed transaction, as a base-64 encoded string.")]
    pub transaction: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcSendTransactionConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SimulateTransaction {
    #[schemars(description = "The transaction to simulate, as a base-64 encoded string.")]
    pub transaction: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcSimulateTransactionConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlock {
    #[schemars(description = "The slot to query for the block.")]
    pub slot: Slot,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcBlockConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockTime {
    #[schemars(description = "The slot to query for the block time.")]
    pub slot: Slot,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlocks {
    #[schemars(description = "The starting slot to query for blocks.")]
    pub start_slot: Slot,
    #[schemars(description = "Wrapper for end slot or context configuration.")]
    pub wrapper: Option<RpcBlocksConfigWrapper>,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlocksWithLimit {
    #[schemars(description = "The starting slot to query for blocks.")]
    pub start_slot: Slot,
    #[schemars(description = "The maximum number of blocks to return.")]
    pub limit: usize,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTransaction {
    #[schemars(description = "The transaction signature to query, as a base-58 encoded string.")]
    pub signature: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcTransactionConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSignaturesForAddress {
    #[schemars(
        description = "The address to query for transaction signatures, as a base-58 encoded string."
    )]
    pub address: String,
    #[schemars(description = "The maximum number of signatures to return.")]
    pub limit: Option<usize>,
    #[schemars(description = "Start searching backwards from this transaction signature.")]
    pub before: Option<String>,
    #[schemars(description = "Search until this transaction signature.")]
    pub until: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetLatestBlockhash {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IsBlockhashValid {
    #[schemars(description = "The blockhash to check, as a base-58 encoded string.")]
    pub blockhash: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetFeeForMessage {
    #[schemars(description = "The message to calculate the fee for, as a base-64 encoded string.")]
    pub message: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetStakeMinimumDelegation {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRecentPrioritizationFees {
    #[schemars(
        description = "An array of account public keys to query for prioritization fees, as base-58 encoded strings."
    )]
    pub pubkeys: Option<Vec<String>>,
}
