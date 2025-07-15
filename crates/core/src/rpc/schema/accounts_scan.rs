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
    #[schemars(description = "The public key of the program, as a base-58 encoded string.")]
    pub program_id: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcProgramAccountsConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetLargestAccounts {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcLargestAccountsConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSupply {
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcSupplyConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenLargestAccounts {
    #[schemars(description = "The public key of the token mint, as a base-58 encoded string.")]
    pub mint: String,
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountsByOwner {
    #[schemars(description = "The public key of the account owner, as a base-58 encoded string.")]
    pub owner: String,
    #[serde(flatten)]
    #[schemars(description = "Filter to apply to the token accounts.")]
    pub filter: RpcTokenAccountsFilter,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountsByDelegate {
    #[schemars(description = "The public key of the delegate, as a base-58 encoded string.")]
    pub delegate: String,
    #[serde(flatten)]
    #[schemars(description = "Filter to apply to the token accounts.")]
    pub filter: RpcTokenAccountsFilter,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcAccountInfoConfig>,
}
