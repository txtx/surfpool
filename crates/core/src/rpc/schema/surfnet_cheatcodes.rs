use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    #[schemars(description = "The new balance in lamports.")]
    pub lamports: Option<u64>,
    #[schemars(description = "The new owner program ID, as a base-58 encoded string.")]
    pub owner: Option<String>,
    #[schemars(description = "Whether the account should be executable.")]
    pub executable: Option<bool>,
    #[schemars(description = "The new rent epoch.")]
    pub rent_epoch: Option<u64>,
    #[schemars(description = "The new account data, as a base-64 encoded string.")]
    pub data: Option<String>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    #[schemars(description = "The new token balance.")]
    pub amount: Option<u64>,
    #[schemars(description = "The new delegate account.")]
    pub delegate: Option<SetSomeAccount>,
    #[schemars(description = "The new account state (e.g., 'initialized').")]
    pub state: Option<String>,
    #[schemars(description = "The new delegated amount.")]
    pub delegated_amount: Option<u64>,
    #[schemars(description = "The new close authority.")]
    pub close_authority: Option<SetSomeAccount>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetSomeAccount {
    #[schemars(description = "Specifies an account, as a base-58 encoded string.")]
    Account(String),
    #[schemars(description = "Specifies no account.")]
    NoAccount,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplyUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The new total supply.")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The new circulating supply.")]
    pub circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The new non-circulating supply.")]
    pub non_circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The new list of non-circulating accounts.")]
    pub non_circulating_accounts: Option<Vec<String>>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum SurfnetCheatcodes {
    #[schemars(description = "Sets the account for a given pubkey.")]
    SetAccount(SetAccount),
    #[schemars(description = "Sets the token account for a given pubkey.")]
    SetTokenAccount(SetTokenAccount),
    #[schemars(description = "Clones a program account for a given pubkey.")]
    CloneProgramAccount(CloneProgramAccount),
    #[schemars(description = "Profiles a transaction for a given tag.")]
    ProfileTransaction(ProfileTransaction),
    #[schemars(description = "Gets the profile results for a given tag.")]
    GetProfileResults(GetProfileResults),
    #[schemars(description = "Sets the supply for the cluster.")]
    SetSupply(SetSupply),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetAccount {
    #[schemars(
        description = "The public key of the account to update, as a base-58 encoded string."
    )]
    pub pubkey: String,
    #[schemars(description = "The account data to update.")]
    pub update: AccountUpdate,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetTokenAccount {
    #[schemars(
        description = "The public key of the token account owner, as a base-58 encoded string."
    )]
    pub owner: String,
    #[schemars(description = "The public key of the token mint, as a base-58 encoded string.")]
    pub mint: String,
    #[schemars(description = "The token account data to update.")]
    pub update: TokenAccountUpdate,
    #[schemars(description = "The token program ID, as a base-58 encoded string.")]
    pub token_program: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneProgramAccount {
    #[schemars(
        description = "The public key of the source program to clone, as a base-58 encoded string."
    )]
    pub source_program_id: String,
    #[schemars(
        description = "The public key of the destination program, as a base-58 encoded string."
    )]
    pub destination_program_id: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTransaction {
    #[schemars(description = "The transaction data to profile, as a base-64 encoded string.")]
    pub transaction_data: String,
    #[schemars(description = "An optional tag to identify the profiling results.")]
    pub tag: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetProfileResults {
    #[schemars(description = "The tag to retrieve profiling results for.")]
    pub tag: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetSupply {
    #[schemars(description = "The supply data to update.")]
    pub update: SupplyUpdate,
}
