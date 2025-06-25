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
    pub pubkey: String,
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockCommitment {
    pub block: Slot,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetMultipleAccounts {
    pub pubkeys: Vec<String>,
    pub config: Option<RpcAccountInfoConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenAccountBalance {
    pub pubkey: String,
    pub commitment: Option<CommitmentConfig>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTokenSupply {
    pub mint: String,
    pub commitment: Option<CommitmentConfig>,
}
