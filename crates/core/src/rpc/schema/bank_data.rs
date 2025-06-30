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
    pub data_len: usize,
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetInflationGovernor {
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSlotLeader {
    pub config: Option<RpcContextConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSlotLeaders {
    pub start_slot: Slot,
    pub limit: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockProduction {
    pub config: Option<RpcBlockProductionConfig>,
}
