use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    pub lamports: Option<u64>,
    pub owner: Option<String>,
    pub executable: Option<bool>,
    pub rent_epoch: Option<u64>,
    pub data: Option<String>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    pub amount: Option<u64>,
    pub delegate: Option<SetSomeAccount>,
    pub state: Option<String>,
    pub delegated_amount: Option<u64>,
    pub close_authority: Option<SetSomeAccount>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetSomeAccount {
    Account(String),
    NoAccount,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplyUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    pub pubkey: String,
    pub update: AccountUpdate,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetTokenAccount {
    pub owner: String,
    pub mint: String,
    pub update: TokenAccountUpdate,
    pub token_program: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneProgramAccount {
    pub source_program_id: String,
    pub destination_program_id: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTransaction {
    pub transaction_data: String,
    pub tag: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetProfileResults {
    pub tag: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetSupply {
    pub update: SupplyUpdate,
}
