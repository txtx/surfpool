use std::sync::{Arc, RwLock};

use rmcp::{
    handler::server::wrapper::Json,
    model::{ServerCapabilities, ServerInfo},
    tool, ServerHandler,
};
use set_token_account::SetTokenAccountResponse;
use start_surfnet::StartSurfnetResponse;

mod set_token_account;
mod start_surfnet;

#[derive(Debug, Clone, Default)]
pub struct Surfpool {
    pub surfnets: Arc<RwLock<Vec<u16>>>
}

impl Surfpool {
    pub fn new() -> Self {
        Self {
            surfnets: Arc::new(RwLock::new(vec![])),
        }
    }
}

#[tool(tool_box)]
impl Surfpool {
    /// Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server and the ID of the surfnet.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server, or an error that must be displayed"
    )]
    pub fn start_surfnet(
        &self,
    ) -> Json<StartSurfnetResponse> {
        let mut surfnets_writer = self.surfnets.as_ref().write().unwrap();

        let surfnet_id = surfnets_writer.len() as u16;

        let res = start_surfnet::run(surfnet_id);
        if res.success.is_some() {
            surfnets_writer.push(surfnet_id);
        }
        Json(res)
    }

    /// Sets the token balance for any account in your local Solana network. Supports SOL and any SPL token.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Sets the token balance for any account in your local Solana network. Supports SOL and any SPL token. If succesful, all the data should be displayed in the chat, if not, share the error message"
    )]
    pub fn set_token_account(
        &self,
        #[tool(param)]
        #[schemars(
            description = "the RPC url of the local surfnet instance where the operation will take place. Must reference a currently running localnet."
        )]
        surfnet_address: String,
        #[tool(param)]
        #[schemars(
            description = "the public key of the wallet to fund. If omitted, a new wallet will be generated and returned"
        )]
        wallet_address: Option<String>,
        #[tool(param)]
        #[schemars(
            description = "the token to set the balance for. Can be a symbol (e.g., SOL, USDC) or a full base58-encoded mint address."
        )]
        token: String,
        #[tool(param)]
        #[schemars(
            description = "the token amount to assign to the wallet. Defaults to 100_000 if not provided"
        )]
        token_amount: Option<u64>,
    ) -> Json<SetTokenAccountResponse> {
        let res = set_token_account::run(surfnet_address, wallet_address, token, token_amount);
        Json(res)
    }

    /// Starts a new local Solana network, also called surfnet or localnet, and sets the token balance for any account in your local Solana network. Supports SOL and any SPL token.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Starts a new local Solana network, also called surfnet or localnet, and sets the token balance for any account in your local Solana network. Supports SOL and any SPL token. If succesful, all the data should be displayed in the chat, if not, share the error message"
    )]
    pub fn start_surfnet_with_token_accounts(
        &self,
        #[tool(param)]
        #[schemars(
            description = "the RPC url of the local surfnet instance where the operation will take place. Must reference a currently running localnet."
        )]
        surfnet_address: String,
        #[tool(param)]
        #[schemars(
            description = "the public key of the wallet to fund. If omitted, a new wallet will be generated and returned"
        )]
        wallet_address: Option<String>,
        #[tool(param)]
        #[schemars(
            description = "the token to set the balance for. Can be a symbol (e.g., SOL, USDC) or a full base58-encoded mint address."
        )]
        token: String,
        #[tool(param)]
        #[schemars(
            description = "the token amount to assign to the wallet. Defaults to 100_000 if not provided"
        )]
        token_amount: Option<u64>,
    ) -> Json<SetTokenAccountResponse> {
        let res = set_token_account::run(surfnet_address, wallet_address, token, token_amount);
        Json(res)
    }
}

#[tool(tool_box)]
impl ServerHandler for Surfpool {
    /// Return information about the server, including its capabilities and description.
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Surfpool MCP server, your personal surfnet manager to start surfing on Solana!"
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
