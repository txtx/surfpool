use std::sync::{Arc, RwLock};

use rmcp::{
    handler::server::wrapper::Json,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use set_token_account::{SeededAccount, SetTokenAccountResponse, SetTokenAccountsResponse};
use start_surfnet::StartSurfnetResponse;

mod set_token_account;
mod start_surfnet;

#[derive(Debug, Clone, Default)]
pub struct Surfpool {
    pub surfnets: Arc<RwLock<Vec<u16>>>,
}

impl Surfpool {
    pub fn new() -> Self {
        Self {
            surfnets: Arc::new(RwLock::new(vec![])),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CreateTokenAccountForOwnerParams {
    #[schemars(
        description = "An optional owner address for the accounts. If provided, all token accounts will be created under this owner. If omitted, a new wallet will be generated and returned."
    )]
    pub owner: Option<String>, // Optional owner address for the token account
    #[schemars(
        description = "A list of token parameters to dictate what the owner should be funded with. Each parameter includes the token mint, program ID, and amount."
    )]
    pub params: Vec<CreateTokenAccountParams>, // Parameters for creating the token account
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CreateTokenAccountParams {
    #[schemars(
        description = "The mint address or symbol of the token to set the balance for. Can be a full base58-encoded mint address or a symbol like 'SOL', 'USDC', etc."
    )]
    pub token_mint: String, // Mint address or symbol of the token
    #[schemars(
        description = "The program ID of the token. Defaults to the SPL token program ID if not provided."
    )]
    pub token_program_id: Option<String>, // Program ID of the token, defaults to SPL Token program
    #[schemars(
        description = "The amount of tokens to assign to the wallet. Defaults to 100_000 if not provided."
    )]
    pub token_amount: Option<u64>, // Amount of tokens to assign, defaults to 100_000
}

#[tool(tool_box)]
impl Surfpool {
    /// Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server and the ID of the surfnet.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Start a new local Solana network, also called surfnet or localnet. Returns the binding address of the RPC server, or an error that must be displayed"
    )]
    pub fn start_surfnet(&self) -> Json<StartSurfnetResponse> {
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
            description = "The RPC url of the local surfnet instance where the operation will take place. Must reference a currently running localnet."
        )]
        surfnet_address: String,
        #[tool(param)]
        #[schemars(
            description = "The public key of the wallet to fund. If omitted, a new wallet will be generated and returned"
        )]
        wallet_address: Option<String>,
        #[tool(param)]
        #[schemars(
            description = "The token to set the balance for. Can be a symbol (e.g., SOL, USDC) or a full base58-encoded mint address."
        )]
        token: String,
        #[tool(param)]
        #[schemars(
            description = "The token amount to assign to the wallet. Defaults to 100_000 if not provided"
        )]
        token_amount: Option<u64>,
        #[tool(param)]
        #[schemars(
            description = "The program ID of the token. Defaults to the SPL token program ID if not provided. Use 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpLAs' for Token-2022."
        )]
        program_id: Option<String>,
    ) -> Json<SetTokenAccountResponse> {
        let owner_seeded_account = SeededAccount::new(wallet_address);
        let res = set_token_account::run(
            surfnet_address,
            owner_seeded_account,
            token,
            token_amount,
            program_id,
        );
        Json(res)
    }

    /// Starts a new local Solana network AND sets a token balance for an account on it.
    /// Combines starting a surfnet and setting an initial token account.
    /// Returns details of the token setup or an error message if any step fails.
    #[tool(
        description = "Starts a new local Solana network, then sets a token balance for a specified or new account on it. Displays token setup details or errors from either start-up or token funding."
    )]
    pub fn start_surfnet_with_token_accounts(
        &self,
        #[tool(param)]
        #[schemars(
            description = "A list of accounts to either create or fund with tokens. Each entry in the vector contains an optional owner address and a list of token parameters. If the owner address is omitted, a new wallet will be generated and returned. Each token parameter includes the token mint, program ID, and amount."
        )]
        token_params_with_owner: Vec<CreateTokenAccountForOwnerParams>,
    ) -> Json<SetTokenAccountsResponse> {
        let surfnet_id = {
            let surfnets_guard = self.surfnets.read().unwrap();
            surfnets_guard.len() as u16
        };

        let start_response = start_surfnet::run(surfnet_id);

        let surfnet_url = match start_response.success {
            Some(ref success_data) => {
                let mut surfnets_writer = self.surfnets.write().unwrap();
                if !surfnets_writer.contains(&surfnet_id) {
                    surfnets_writer.push(surfnet_id);
                }
                success_data.surfnet_url.clone()
            }
            None => {
                return Json(SetTokenAccountsResponse::error(format!(
                    "Failed to start Surfnet (ID {}): {}. Token account not set.",
                    surfnet_id,
                    start_response
                        .error
                        .unwrap_or_else(|| "Unknown error starting surfnet".to_string())
                )));
            }
        };

        let mut results = Vec::new();
        for CreateTokenAccountForOwnerParams { owner, params } in token_params_with_owner {
            let owner_seeded_account = SeededAccount::new(owner);

            for CreateTokenAccountParams {
                token_mint,
                token_amount,
                token_program_id,
            } in params
            {
                let set_token_result = set_token_account::run(
                    surfnet_url.clone(),
                    owner_seeded_account.clone(),
                    token_mint,
                    token_amount,
                    token_program_id,
                );
                results.push(set_token_result);
            }
        }

        Json(SetTokenAccountsResponse::success(
            results.into_iter().filter_map(|r| r.success).collect(),
        ))
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
