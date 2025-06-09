use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use rmcp::{
    handler::server::wrapper::Json,
    model::{ListResourcesResult, RawResource, ReadResourceResult, *},
    schemars,
    service::RequestContext,
    tool, Error as McpError, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use set_token_account::{SeededAccount, SetAccountSuccess, SetTokenAccountsResponse};
use start_surfnet::StartSurfnetResponse;

use crate::helpers::find_next_available_surfnet_port;

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
        description = "The base58-encoded mint address of the token to fund. If provided, the token symbol will be inferred from the token mint address. If not provided, the token symbol must be provided to infer the token mint address."
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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SurfnetRpcCallSuccess {
    #[schemars(description = "The RPC method that was called")]
    pub method: String,
    #[schemars(description = "The result returned by the RPC method")]
    pub result: Value,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SurfnetRpcCallResponse {
    pub success: Option<SurfnetRpcCallSuccess>,
    pub error: Option<String>,
}

impl SurfnetRpcCallResponse {
    pub fn success(method: String, result: Value) -> Self {
        Self {
            success: Some(SurfnetRpcCallSuccess { method, result }),
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

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![RawResource::new(
                "str:///rpc_endpoint_list",
                "List of RPC endpoints available".to_string(),
            )
            .no_annotation()],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match uri.as_str() {
            "str:///rpc_endpoint_list" => {
                let rpc_endpoints = include_str!("rpc_endpoints.json");
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(rpc_endpoints, uri)],
                })
            }
            _ => Err(McpError::resource_not_found(
                "resource_not_found",
                Some(serde_json::json!({
                    "uri": uri
                })),
            )),
        }
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

    /// Calls any RPC method on a running surfnet instance.
    /// This generic method allows calling any of the available surfnet cheatcode RPC methods.
    /// The LLM will interpret user requests and determine which method to call with appropriate parameters.
    #[tool(
        description = "Calls any RPC method on a running surfnet instance. This is a generic method that can invoke any surfnet cheatcode RPC method. The LLM should interpret user requests and determine the appropriate method and parameters to call."
    )]
    pub async fn call_surfnet_rpc(
        &self,
        #[tool(param)]
        #[schemars(
            description = "The port of a running local surfnet instance (e.g., 8899, 18899, 28899, etc.)."
        )]
        surfnet_port: u16,
        #[tool(param)]
        #[schemars(
            description = "The RPC method name to call (e.g., 'surfnet_setAccount', 'surfnet_setTokenAccount', 'surfnet_cloneProgramAccount', 'surfnet_profileTransaction', 'surfnet_getProfileResults, etc.)"
        )]
        method: String,
        #[tool(param)]
        #[schemars(
            description = "The parameters to pass to the RPC method as a JSON array. The structure depends on the specific method being called."
        )]
        params: Vec<Value>,
    ) -> Json<SurfnetRpcCallResponse> {
        let surfnet_endpoint = format!("http://127.0.0.1:{}", surfnet_port);
        let rpc_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        // Make the HTTP request to the surfnet RPC endpoint
        let client = reqwest::Client::new();
        let response = match client
            .post(&surfnet_endpoint)
            .header("Content-Type", "application/json")
            .json(&rpc_request)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                return Json(SurfnetRpcCallResponse::error(format!(
                    "Failed to send RPC request to {}: {}",
                    surfnet_endpoint, e
                )));
            }
        };

        let response_text = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                return Json(SurfnetRpcCallResponse::error(format!(
                    "Failed to read response text: {}",
                    e
                )));
            }
        };

        let rpc_response: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(json) => json,
            Err(e) => {
                return Json(SurfnetRpcCallResponse::error(format!(
                    "Failed to parse JSON response: {}. Response: {}",
                    e, response_text
                )));
            }
        };

        if let Some(error) = rpc_response.get("error") {
            return Json(SurfnetRpcCallResponse::error(format!(
                "RPC error: {}",
                error
            )));
        }

        // Extract the result
        let result = rpc_response
            .get("result")
            .unwrap_or(&serde_json::Value::Null)
            .clone();

        Json(SurfnetRpcCallResponse::success(method, result))
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
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }
}
