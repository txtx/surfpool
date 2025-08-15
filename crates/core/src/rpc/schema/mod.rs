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
pub mod response;
mod solana_types;
mod surfnet_cheatcodes;
mod ws;

pub use response::{RpcErrorResponse, RpcResponseContext, SurfpoolRpcEndpoints};
pub use ws::{SurfpoolWebSocketApiDocumentation, SurfpoolWebSocketEndpoints};

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

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RpcResponse {
    #[schemars(description = "Response types for the rpc methods")]
    #[serde(untagged)]
    Response(SurfpoolRpcEndpoints),
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rpc_groups_schema() {
        let schema = schemars::schema_for!(RpcGroups);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("rpc_groups_schema.json", json).unwrap();
    }

    #[test]
    fn test_response_types_schema() {
        let schema = schemars::schema_for!(SurfpoolRpcEndpoints);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("response_schema.json", json).unwrap();
    }

    #[test]
    fn test_websocket_endpoints_schema() {
        let schema = schemars::schema_for!(SurfpoolWebSocketEndpoints);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("websocket_schema.json", json).unwrap();
    }

    #[test]
    fn test_websocket_api_documentation_schema() {
        let schema = schemars::schema_for!(SurfpoolWebSocketApiDocumentation);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("websocket_api_documentation_schema.json", json).unwrap();
    }
}
