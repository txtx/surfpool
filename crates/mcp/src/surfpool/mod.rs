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
pub struct Surfpool;

#[tool(tool_box)]
impl Surfpool {
    /// Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server, or an error that must be displayed"
    )]
    pub fn start_surfnet(
        &self,
        #[tool(param)]
        #[schemars(
            description = "the port to bind the RPC server to. Defaults to 8899 if not provided. If you want to use the default port, you can omit this parameter."
        )]
        rpc_port: Option<u16>,
        #[tool(param)]
        #[schemars(
            description = "the port to bind the WS server to. Defaults to 8900 if not provided. If you want to use the default port, you can omit this parameter."
        )]
        ws_port: Option<u16>,
    ) -> Json<StartSurfnetResponse> {
        let res = start_surfnet::run(rpc_port, ws_port);
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
