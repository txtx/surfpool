use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{Arc, RwLock},
};

use rmcp::{
    handler::server::wrapper::Json,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use set_token_account::{SeededAccount, SetAccountSuccess, SetTokenAccountsResponse};
use start_surfnet::StartSurfnetResponse;

mod set_token_account;
mod start_surfnet;

#[derive(Debug, Clone)]
pub struct Surfpool {
    pub surfnets: Arc<RwLock<HashMap<u16, u16>>>,
}

impl Surfpool {
    pub fn new() -> Self {
        Self {
            surfnets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for Surfpool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CreateTokenAccountForOwnerParams {
    #[schemars(
        description = "The owner address for the token accounts. If omitted, a new wallet will be generated and its address returned."
    )]
    pub owner: Option<String>,
    #[schemars(
        description = "A list of parameters for the tokens to be funded, including mint, program ID, and amount."
    )]
    pub params: Vec<CreateTokenAccountParams>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CreateTokenAccountParams {
    #[schemars(
        description = "The base58-encoded mint address of the token to fund. If not provided, the token symbol will be inferred from the token mint address."
    )]
    pub token_mint: Option<String>,
    #[schemars(
        description = "The token program ID (e.g., `Token` or `Token2022`). Defaults to the SPL Token Program if not provided."
    )]
    pub token_program_id: Option<String>,
    #[schemars(
        description = "The token symbol (e.g., USDC, JUP). If not provided, it will be inferred from the mint address."
    )]
    pub token_symbol: Option<String>,
    #[schemars(
        description = "The amount of tokens to fund, in human-readable units. Defaults to 100_000 if not provided."
    )]
    pub token_amount: Option<u64>,
}

fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn find_next_available_surfnet_port() -> Result<(u16, u16), String> {
    let mut surfnet_id: u16 = 0;
    while surfnet_id.checked_add(1).is_some() {
        let port = 8899 + (surfnet_id * 10000);
        let ws_port = port.saturating_sub(9);
        if ws_port > 0 && is_port_available(port) && is_port_available(ws_port) {
            return Ok((surfnet_id, port));
        }
        surfnet_id += 1;
    }
    Err(format!(
        "No available surfnet ports of format 127.0.0.1:x8899 found."
    ))
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StartSurfnetWithTokenAccountsSuccess {
    #[schemars(description = "The RPC URL of the newly started surfnet instance.")]
    pub surfnet_url: String,
    #[schemars(description = "The ID of the newly started surfnet instance.")]
    pub surfnet_id: u16,
    #[schemars(description = "A list of accounts that were created or funded.")]
    pub accounts: Vec<SetAccountSuccess>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StartSurfnetWithTokenAccountsResponse {
    pub success: Option<StartSurfnetWithTokenAccountsSuccess>,
    pub error: Option<String>,
}

impl StartSurfnetWithTokenAccountsResponse {
    pub fn success(data: StartSurfnetWithTokenAccountsSuccess) -> Self {
        Self {
            success: Some(data),
            error: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: None,
            error: Some(message),
        }
    }
}

#[tool(tool_box)]
impl Surfpool {
    /// Starts a new local Solana network (surfnet).
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Starts a new local Solana network (surfnet). Returns the RPC server's binding address and the surfnet ID."
    )]
    pub fn start_surfnet(&self) -> Json<StartSurfnetResponse> {
        let (surfnet_id, port) = match find_next_available_surfnet_port() {
            Ok((id, p)) => (id, p),
            Err(e) => return Json(StartSurfnetResponse::error(e)),
        };

        let res = start_surfnet::run(surfnet_id, port, port.saturating_sub(9));
        if res.success.is_some() {
            self.surfnets.write().unwrap().insert(surfnet_id, port);
        }
        Json(res)
    }

    /// Sets token balances for multiple accounts in your local Solana network (surfnet).
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Sets token balances for one or more accounts on a local Solana network. Supports both SOL and SPL tokens."
    )]
    pub fn set_token_accounts(
        &self,
        #[tool(param)]
        #[schemars(description = "The RPC URL of a running local surfnet instance.")]
        surfnet_address: String,
        #[tool(param)]
        #[schemars(
            description = "A list of accounts to create or fund. For each account, an optional owner can be specified; if omitted, a new wallet is generated. Token parameters include the mint, program ID, and amount."
        )]
        token_params_with_owner: Vec<CreateTokenAccountForOwnerParams>,
    ) -> Json<SetTokenAccountsResponse> {
        let mut results = Vec::new();
        for CreateTokenAccountForOwnerParams { owner, params } in token_params_with_owner {
            let owner_seeded_account = SeededAccount::new(owner);

            for CreateTokenAccountParams {
                token_mint,
                token_amount,
                token_program_id,
                token_symbol,
            } in params
            {
                let set_token_result = set_token_account::run(
                    surfnet_address.clone(),
                    owner_seeded_account.clone(),
                    token_mint,
                    token_amount,
                    token_program_id,
                    token_symbol.clone(),
                );
                results.push(set_token_result);
            }
        }

        Json(SetTokenAccountsResponse::success(
            results
                .into_iter()
                .filter_map(|r| r.success)
                .flatten()
                .collect(),
        ))
    }

    /// Starts a new local Solana network and sets token balances on it.
    /// This is a convenience method that combines `start_surfnet` and `set_token_accounts`.
    #[tool(
        description = "Starts a new local Solana network and sets token balances for one or more accounts. This combines `start_surfnet` and `set_token_accounts`."
    )]
    pub fn start_surfnet_with_token_accounts(
        &self,
        #[tool(param)]
        #[schemars(
            description = "A list of accounts to create or fund. For each account, an optional owner can be specified; if omitted, a new wallet is generated. Token parameters include the mint, program ID, and amount."
        )]
        token_params_with_owner: Vec<CreateTokenAccountForOwnerParams>,
    ) -> Json<StartSurfnetWithTokenAccountsResponse> {
        let (surfnet_id, port) = match find_next_available_surfnet_port() {
            Ok((id, p)) => (id, p),
            Err(e) => return Json(StartSurfnetWithTokenAccountsResponse::error(e)),
        };

        let start_response = start_surfnet::run(surfnet_id, port, port.saturating_sub(9));

        let surfnet_url = match start_response.success {
            Some(ref success_data) => {
                let mut surfnets_writer = self.surfnets.write().unwrap();
                surfnets_writer.insert(surfnet_id, port);
                success_data.surfnet_url.clone()
            }
            None => {
                return Json(StartSurfnetWithTokenAccountsResponse::error(format!(
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
                token_symbol,
            } in params
            {
                let set_token_result = set_token_account::run(
                    surfnet_url.clone(),
                    owner_seeded_account.clone(),
                    token_mint,
                    token_amount,
                    token_program_id,
                    token_symbol.clone(),
                );
                results.push(set_token_result);
            }
        }

        Json(StartSurfnetWithTokenAccountsResponse::success(
            StartSurfnetWithTokenAccountsSuccess {
                surfnet_url,
                surfnet_id,
                accounts: results
                    .into_iter()
                    .filter_map(|r| r.success)
                    .flatten()
                    .collect(),
            },
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
