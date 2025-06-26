use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{
    CommitmentConfig, RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig,
    RpcSupplyConfig, RpcTokenAccountsFilter,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum AccountsScan {
    #[schemars(description = "Get program accounts owned by a specific program ID.")]
    GetProgramAccounts(GetProgramAccounts),
    #[schemars(description = "Returns the 20 largest accounts by lamport balance.")]
    GetLargestAccounts(GetLargestAccounts),
    #[schemars(description = "Returns information about the current token supply.")]
    GetSupply(GetSupply),
    #[schemars(description = "Returns the largest accounts for a given token mint.")]
    GetTokenLargestAccounts(GetTokenLargestAccounts),
    #[schemars(description = "Returns all SPL Token accounts by owner.")]
    GetTokenAccountsByOwner(GetTokenAccountsByOwner),
    #[schemars(description = "Returns all SPL Token accounts by delegate.")]
    GetTokenAccountsByDelegate(GetTokenAccountsByDelegate),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetProgramAccounts {
    pub program_id: String,
    pub config: Option<RpcProgramAccountsConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetLargestAccounts {
    pub config: Option<RpcLargestAccountsConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSupply {
    pub config: Option<RpcSupplyConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenLargestAccounts {
    pub mint: String,
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountsByOwner {
    pub owner: String,
    #[serde(flatten)]
    pub filter: RpcTokenAccountsFilter,
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountsByDelegate {
    pub delegate: String,
    #[serde(flatten)]
    pub filter: RpcTokenAccountsFilter,
    pub config: Option<RpcAccountInfoConfig>,
}
