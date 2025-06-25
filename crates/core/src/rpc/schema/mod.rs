use schemars::JsonSchema;

use crate::rpc::schema::{
    accounts_data::AccountsData, accounts_scan::AccountsScan, admin::Admin, bank_data::BankData,
    full::Full, minimal::Minimal, surfnet_cheatcodes::SurfnetCheatcodes,
};

mod accounts_data;
mod accounts_scan;
mod admin;
mod bank_data;
mod full;
mod minimal;
mod solana_types;
mod surfnet_cheatcodes;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcGroups {
    #[schemars(
        description = "The accounts scan group, which includes operations related to scanning accounts."
    )]
    AccountsScan(AccountsScan),
    #[schemars(
        description = "The accounts data group, which includes operations related to fetching account data."
    )]
    AccountsData(AccountsData),
    #[schemars(description = "The admin group, which includes administrative operations.")]
    Admin(Admin),
    #[schemars(
        description = "The bank data group, which includes operations related to fetching bank data."
    )]
    BankData(BankData),
    #[schemars(description = "The full group, which includes all other operations.")]
    Full(Full),
    #[schemars(description = "The minimal group, which includes a minimal set of operations.")]
    Minimal(Minimal),
    #[schemars(
        description = "The surfnet cheatcodes group, which includes operations for testing and simulation."
    )]
    SurfnetCheatcodes(SurfnetCheatcodes),
}

mod test {
    use super::*;

    #[test]
    fn test_rpc_groups_schema() {
        let schema = schemars::schema_for!(RpcGroups);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("rpc_groups_schema.json", json).unwrap();
    }
}
