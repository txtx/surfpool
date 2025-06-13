use schemars::JsonSchema;

use crate::rpc::schema::solana_types::RpcProgramAccountsConfig;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]

pub enum AccountsScan {
    #[schemars(description = "Get program accounts owned by a specific program ID.")]
    GetProgramAccounts(GetProgramAccounts),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetProgramAccounts {
    pub program_id: String,
    pub config: Option<RpcProgramAccountsConfig>,
}
