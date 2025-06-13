use schemars::JsonSchema;

use crate::rpc::schema::accounts_scan::AccountsScan;

mod accounts_scan;
mod solana_types;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcGroups {
    #[schemars(
        description = "The accounts scan group, which includes operations related to scanning accounts...."
    )]
    AccountsScan(AccountsScan),
    AccountsData,
    Admin,
    Full,
}

mod test {
    use super::*;

    #[test]
    fn test_rpc_groups_schema() {
        let schema = schemars::schema_for!(RpcGroups);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        // write to file
        std::fs::write("rpc_groups_schema.json", json).unwrap();
    }
}
