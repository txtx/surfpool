use schemars::JsonSchema;

use crate::rpc::schema::solana_types::{CommitmentConfig, RpcAccountInfoConfig, Slot};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum AccountsData {
    #[schemars(
        description = "Returns detailed information about an account given its public key."
    )]
    GetAccountInfo(GetAccountInfo),
    #[schemars(description = "Returns commitment levels for a given block (slot).")]
    GetBlockCommitment(GetBlockCommitment),
    #[schemars(
        description = "Returns account information for multiple public keys in a single call."
    )]
    GetMultipleAccounts(GetMultipleAccounts),
    #[schemars(description = "Returns the balance of a token account, given its public key.")]
    GetTokenAccountBalance(GetTokenAccountBalance),
    #[schemars(description = "Returns the total supply of a token, given its mint address.")]
    GetTokenSupply(GetTokenSupply),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetAccountInfo {
    #[schemars(
        description = "The public key of the account to query, as a base-58 encoded string."
    )]
    pub pubkey: String,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockCommitment {
    #[schemars(description = "The slot to query for block commitment.")]
    pub block: Slot,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetMultipleAccounts {
    #[schemars(description = "An array of public keys to query, as base-58 encoded strings.")]
    pub pubkeys: Vec<String>,
    #[schemars(description = "Configuration object for the query.")]
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountBalance {
    #[schemars(
        description = "The public key of the token account to query, as a base-58 encoded string."
    )]
    pub pubkey: String,
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenSupply {
    #[schemars(description = "The public key of the token mint, as a base-58 encoded string.")]
    pub mint: String,
    #[schemars(description = "Commitment level for the query.")]
    pub commitment: Option<CommitmentConfig>,
}
