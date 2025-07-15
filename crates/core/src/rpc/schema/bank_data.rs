use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{
    CommitmentConfig, RpcBlockProductionConfig, RpcContextConfig, Slot,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum BankData {
    #[schemars(description = "Returns the minimum balance required for rent exemption.")]
    GetMinimumBalanceForRentExemption(GetMinimumBalanceForRentExemption),
    #[schemars(description = "Retrieves the inflation governor settings.")]
    GetInflationGovernor(GetInflationGovernor),
    #[schemars(description = "Retrieves the current inflation rate.")]
    GetInflationRate,
    #[schemars(description = "Retrieves the epoch schedule.")]
    GetEpochSchedule,
    #[schemars(description = "Retrieves the leader of the current slot.")]
    GetSlotLeader(GetSlotLeader),
    #[schemars(description = "Retrieves the leaders for a specified range of slots.")]
    GetSlotLeaders(GetSlotLeaders),
    #[schemars(description = "Retrieves block production information.")]
    GetBlockProduction(GetBlockProduction),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetMinimumBalanceForRentExemption {
    #[schemars(description = "The account data length in bytes.")]
    pub data_len: usize,
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetInflationGovernor {
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSlotLeader {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSlotLeaders {
    #[schemars(description = "The starting slot to query for leaders.")]
    pub start_slot: Slot,
    #[schemars(description = "The maximum number of leaders to return.")]
    pub limit: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockProduction {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcBlockProductionConfig>,
}
