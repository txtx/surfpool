use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ErrorData as McpError, *},
    schemars, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use set_token_account::{SeededAccount, SetAccountSuccess, SetTokenAccountsResponse};
use start_surfnet::StartSurfnetResponse;
use surfpool_core::scenarios::TemplateRegistry;
use surfpool_types::{
    CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED, Scenario, VERIFIED_TOKENS_BY_SYMBOL,
};

use crate::helpers::find_next_available_surfnet_port;

mod set_token_account;
mod start_surfnet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartSurfnetParams {
    #[schemars(
        description = "If `false` (default), returns a command for the AI to execute. If `true`, starts surfnet directly as a background process."
    )]
    pub run_as_subprocess: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetTokenAccountsParams {
    #[schemars(description = "The RPC URL of a running local surfnet instance.")]
    pub surfnet_address: String,
    #[schemars(
        description = "A list of accounts to create or fund. For each account, an optional owner can be specified; if omitted, a new wallet is generated. Token parameters include the mint, program ID, and amount."
    )]
    pub token_params_with_owner: Vec<CreateTokenAccountForOwnerParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartSurfnetWithTokenAccountsParams {
    #[schemars(
        description = "A list of accounts to create or fund. For each account, an optional owner can be specified; if omitted, a new wallet is generated. Token parameters include the mint, program ID, and amount."
    )]
    pub token_params_with_owner: Vec<CreateTokenAccountForOwnerParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CallSurfnetRpcParams {
    #[schemars(
        description = "The port of a running local surfnet instance (e.g., 8899, 18899, 28899, etc.)."
    )]
    pub surfnet_port: u16,
    #[schemars(
        description = "The RPC method name to call,for example: 'sendTransaction', 'simulateTransaction', 'getAccountInfo', 'getBalance', 'getTokenAccountBalance', 'getTokenSupply', 'getProgramAccounts', 'getTokenAccountsByOwner', 'getSlot',
        'getEpochInfo', 'requestAirdrop', 'surfnet_setAccount', 'getHealth', 'getTokenAccountsByOwner', 'getTokenAccountsByDelegate',
        'getTokenAccountsByDelegateAndMint', 'getTokenAccountsByDelegateAndMintAndOwner', 'getTokenAccountsByDelegateAndMintAndOwnerAndProgramId', 'getTokenAccountsByDelegateAndMintAndOwnerAndProgramIdAndOwner', surfnet_getProfileResults, etc.
        A list of all the RPC methods available can be found at str:///rpc_endpoints"
    )]
    pub method: String,
    #[schemars(description = "The parameters to pass to the RPC method")]
    pub params: Vec<JsonValue>,
}

#[derive(Debug, Clone)]
pub struct Surfpool {
    pub surfnets: Arc<RwLock<HashMap<u16, u16>>>,
    pub template_registry: Arc<RwLock<TemplateRegistry>>,
    tool_router: ToolRouter<Surfpool>,
}

impl Surfpool {
    pub fn new() -> Self {
        Self {
            surfnets: Arc::new(RwLock::new(HashMap::new())),
            template_registry: Arc::new(RwLock::new(TemplateRegistry::new())),
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for Surfpool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum JsonValue {
    #[schemars(description = "A JSON representation of null")]
    Null,
    #[schemars(description = "A JSON representation of a boolean")]
    Bool(bool),
    #[schemars(description = "A JSON representation of a number")]
    Number(i64),
    #[schemars(description = "A JSON representation of a string")]
    String(String),
    #[schemars(description = "A JSON representation of an array")]
    Array(Vec<JsonValue>),
    #[schemars(description = "A JSON representation of an object with key-value pairs")]
    Object(HashMap<String, JsonValue>),
    #[schemars(description = "A JSON representation of a base58-encoded public key")]
    PublicKey(String),
    #[schemars(description = "A JSON representation of a base58-encoded mint address")]
    MintAddress(String),
    #[schemars(description = "A JSON representation of a program ID")]
    ProgramId(String),
}

impl From<JsonValue> for Value {
    fn from(val: JsonValue) -> Self {
        match val {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(b),
            JsonValue::Number(n) => Value::Number(serde_json::Number::from(n)),
            JsonValue::String(s) => Value::String(s),
            JsonValue::PublicKey(s) => Value::String(s),
            JsonValue::MintAddress(s) => Value::String(s),
            JsonValue::ProgramId(s) => Value::String(s),
            JsonValue::Array(arr) => Value::Array(arr.into_iter().map(Value::from).collect()),
            JsonValue::Object(obj) => {
                Value::Object(obj.into_iter().map(|(k, v)| (k, Value::from(v))).collect())
            }
        }
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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RegisterScenarioResponse {
    pub error: Option<String>,
    pub url: Option<String>,
}
impl RegisterScenarioResponse {
    pub fn success(url: String) -> Self {
        Self {
            error: None,
            url: Some(url),
        }
    }
    pub fn error(message: String) -> Self {
        Self {
            error: Some(message),
            url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetTokenAddressParams {
    #[schemars(description = "The token symbol to look up (e.g., 'USDC', 'SOL', 'JUP')")]
    pub symbol: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TokenAddressResponse {
    pub symbol: String,
    pub address: Option<String>,
    pub name: Option<String>,
    pub decimals: Option<u8>,
    pub error: Option<String>,
}

impl TokenAddressResponse {
    pub fn success(symbol: String, address: String, name: String, decimals: u8) -> Self {
        Self {
            symbol,
            address: Some(address),
            name: Some(name),
            decimals: Some(decimals),
            error: None,
        }
    }
    pub fn not_found(symbol: String) -> Self {
        Self {
            symbol: symbol.clone(),
            address: None,
            name: None,
            decimals: None,
            error: Some(format!(
                "Token '{}' not found in verified tokens list",
                symbol
            )),
        }
    }
}

#[tool_router]
impl Surfpool {
    /// Returns a command to start a new local Solana network (surfnet).
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(description = "Starts a new local Solana network (surfnet).
        This tool returns a shell command that the AI must execute in a new terminal.
        This is the default behavior (`run_as_subprocess: false`).
        If the user explicitly asks to run surfnet as a background process,
        set `run_as_subprocess: true` to start it directly. When in doubt, return the command for execution.")]
    async fn start_surfnet(
        &self,
        Parameters(params): Parameters<StartSurfnetParams>,
    ) -> Result<CallToolResult, McpError> {
        let (surfnet_id, port) = match find_next_available_surfnet_port() {
            Ok((id, p)) => (id, p),
            Err(e) => {
                let response = StartSurfnetResponse::error(e);
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let res = match params.run_as_subprocess {
            true => start_surfnet::run_headless(surfnet_id, port, port.saturating_sub(9)),
            false => start_surfnet::run_command(surfnet_id, port, port.saturating_sub(9)),
        };

        // Keep track of the surfnet instance in the registry
        if res.success.is_some() {
            self.surfnets.write().unwrap().insert(surfnet_id, port);
        }

        let json_str = serde_json::to_string(&res).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    /// Sets token balances for multiple accounts in your local Solana network (surfnet).
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(
        description = "Sets token balances for one or more accounts on a local Solana network. Supports both SOL and SPL tokens."
    )]
    async fn set_token_accounts(
        &self,
        Parameters(params): Parameters<SetTokenAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut results = Vec::new();
        for CreateTokenAccountForOwnerParams {
            owner,
            params: token_params,
        } in params.token_params_with_owner
        {
            let owner_seeded_account = SeededAccount::new(owner);

            for CreateTokenAccountParams {
                token_mint,
                token_amount,
                token_program_id,
                token_symbol,
            } in token_params
            {
                let set_token_result = set_token_account::run(
                    params.surfnet_address.clone(),
                    owner_seeded_account.clone(),
                    token_mint,
                    token_amount,
                    token_program_id,
                    token_symbol.clone(),
                );
                results.push(set_token_result);
            }
        }

        let response = SetTokenAccountsResponse::success(
            results
                .into_iter()
                .filter_map(|r| r.success)
                .flatten()
                .collect(),
        );
        let json_str = serde_json::to_string(&response).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    /// Starts a new local Solana network and sets token balances on it.
    /// This is a convenience method that combines `start_surfnet` and `set_token_accounts`.
    #[tool(
        description = "Starts a new local Solana network and sets token balances for one or more accounts. This combines `start_surfnet` and `set_token_accounts`."
    )]
    async fn start_surfnet_with_token_accounts(
        &self,
        Parameters(params): Parameters<StartSurfnetWithTokenAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let (surfnet_id, port) = match find_next_available_surfnet_port() {
            Ok((id, p)) => (id, p),
            Err(e) => {
                let response = StartSurfnetWithTokenAccountsResponse::error(e);
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let start_response = start_surfnet::run_headless(surfnet_id, port, port.saturating_sub(9));

        let surfnet_url = match start_response.success {
            Some(ref success_data) => {
                let mut surfnets_writer = self.surfnets.write().unwrap();
                surfnets_writer.insert(surfnet_id, port);
                success_data.surfnet_url.clone()
            }
            None => {
                let response = StartSurfnetWithTokenAccountsResponse::error(format!(
                    "Failed to start Surfnet (ID {}): {}. Token account not set.",
                    surfnet_id,
                    start_response
                        .error
                        .unwrap_or_else(|| "Unknown error starting surfnet".to_string())
                ));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let mut results = Vec::new();
        for CreateTokenAccountForOwnerParams {
            owner,
            params: token_params,
        } in params.token_params_with_owner
        {
            let owner_seeded_account = SeededAccount::new(owner);

            for CreateTokenAccountParams {
                token_mint,
                token_amount,
                token_program_id,
                token_symbol,
            } in token_params
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

        let response =
            StartSurfnetWithTokenAccountsResponse::success(StartSurfnetWithTokenAccountsSuccess {
                surfnet_url,
                surfnet_id,
                accounts: results
                    .into_iter()
                    .filter_map(|r| r.success)
                    .flatten()
                    .collect(),
            });
        let json_str = serde_json::to_string(&response).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    /// Calls any RPC method on a running surfnet instance.
    /// This generic method allows calling any of the available surfnet cheatcode RPC methods.
    /// The LLM will interpret user requests and determine which method to call with appropriate parameters.
    #[tool(description = r#"
        Calls any RPC method on a running surfnet instance.
        This is a generic method that can invoke any surfnet RPC method.
        The LLM should interpret user requests and determine the appropriate method and parameters to call. To retrieve the list of RPC endpoints available check the resource str:///rpc_endpoints

        IMPORTANT:
        - If a user asks to create a scenario, DO NOT USE this method. Instead, use the dedicated `create_scenario` tool.
        - There is NO RPC method called `surfnet_getOverrideTemplates`. Override templates are ONLY available via the MCP resource str:///override_templates (use read_resource, not this RPC tool).
        "#)]
    async fn call_surfnet_rpc(
        &self,
        Parameters(call_params): Parameters<CallSurfnetRpcParams>,
    ) -> Result<CallToolResult, McpError> {
        let surfnet_endpoint = format!("http://127.0.0.1:{}", call_params.surfnet_port);
        let rpc_params: Vec<Value> = call_params.params.into_iter().map(Into::into).collect();
        let rpc_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": call_params.method,
            "params": rpc_params
        });

        // Make the RPC request to the surfnet RPC endpoint
        let client = reqwest::blocking::Client::new();
        let http_response = match client
            .post(&surfnet_endpoint)
            .header("Content-Type", "application/json")
            .json(&rpc_request)
            .send()
        {
            Ok(resp) => resp,
            Err(e) => {
                let response = SurfnetRpcCallResponse::error(format!(
                    "Failed to send RPC request to {}: {}",
                    surfnet_endpoint, e
                ));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let response_text = match http_response.text() {
            Ok(text) => text,
            Err(e) => {
                let response =
                    SurfnetRpcCallResponse::error(format!("Failed to read response text: {}", e));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let rpc_response: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(json) => json,
            Err(e) => {
                let response = SurfnetRpcCallResponse::error(format!(
                    "Failed to parse JSON response: {}. Response: {}",
                    e, response_text
                ));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        if let Some(error) = rpc_response.get("error") {
            let response = SurfnetRpcCallResponse::error(format!("RPC error: {}", error));
            let json_str = serde_json::to_string(&response).unwrap_or_default();
            return Ok(CallToolResult::success(vec![Content::text(json_str)]));
        }

        // Extract the result
        let result = rpc_response
            .get("result")
            .unwrap_or(&serde_json::Value::Null)
            .clone();

        let response = SurfnetRpcCallResponse::success(call_params.method, result);
        let json_str = serde_json::to_string(&response).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    #[tool(description = r#"
        Creates a Scenario - a list of account state overrides applied at specific slots.
        Returns a URL to Surfpool Studio for testing. 1 slot = 400ms.

        ⚠️ CRITICAL JSON FORMAT RULES:
        - The `overrides` field MUST be a JSON array [], NOT a JSON string
        - The `values` field MUST be a JSON object {}, NOT a JSON string
        - The `tags` field MUST be a JSON array [], NOT a JSON string
        - DO NOT stringify nested objects - pass them as native JSON

        ⚠️ CRITICAL: You MUST call `get_override_templates` FIRST to get valid template data.
        DO NOT invent templateId values, property names, or account addresses - they MUST come from the templates.

        STRICT RULES - VIOLATIONS WILL CAUSE ERRORS:
        1. `templateId` MUST exactly match a template's `id` field (e.g., "raydium-clmm-custom", NOT "raydium_clmm_v1")
        2. `values` keys MUST be from the template's `properties` array
        3. For PDA addresses, DO NOT provide `account` - it will be generated from template + values
        4. For constant_ref properties (like feed_id), the value MUST be from the template's constants options

        CORRECT JSON STRUCTURE FOR PYTH PRICE FEED:
        {
          "id": "my-scenario-id",
          "name": "Price Crash Scenario",
          "description": "SOL crashes to $85",
          "overrides": [
            {
              "id": "override-1",
              "templateId": "pyth-price-feed-v2",
              "values": {
                "feed_id": "0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
                "price_message.price": 8500000000
              },
              "scenarioRelativeSlot": 0,
              "enabled": true,
              "fetchBeforeUse": true,
              "label": "SOL price crash"
            }
          ],
          "tags": ["pyth", "price"]
        }

        NOTE: The `account` field will be auto-generated from the template's PDA configuration.
        For Pyth feeds, use feed_id values from get_override_templates constants.
        "#)]
    async fn create_scenario(
        &self,
        Parameters(mut scenario): Parameters<Scenario>,
    ) -> Result<CallToolResult, McpError> {
        // Validate all templateIds exist in the registry
        let registry = self.template_registry.read().map_err(|_| {
            use std::borrow::Cow;
            McpError {
                code: ErrorCode(-32603),
                message: Cow::from("Failed to read template registry"),
                data: None,
            }
        })?;

        let mut invalid_templates: Vec<String> = Vec::new();
        let mut validation_errors: Vec<String> = Vec::new();

        for override_instance in &mut scenario.overrides {
            // Check if template exists
            if !registry.contains(&override_instance.template_id) {
                invalid_templates.push(override_instance.template_id.clone());
                continue;
            }

            // Get the template for validation and normalization
            if let Some(template) = registry.get(&override_instance.template_id) {
                // Normalize: Extract values from malformed PDA seeds
                // LLMs sometimes put values directly in seeds instead of in values map
                if let surfpool_types::AccountAddress::Pda { seeds, .. } =
                    &override_instance.account
                {
                    for seed in seeds {
                        match seed {
                            // If bytes32Ref contains a hex value (starts with 0x), extract it
                            surfpool_types::PdaSeed::Bytes32Ref(value)
                                if value.starts_with("0x") =>
                            {
                                // Find the corresponding property reference from template
                                if let surfpool_types::AccountAddress::Pda {
                                    seeds: template_seeds,
                                    ..
                                } = &template.address
                                {
                                    for template_seed in template_seeds.iter() {
                                        if let surfpool_types::PdaSeed::Bytes32Ref(prop_name) =
                                            template_seed
                                        {
                                            // The template has a property reference, but LLM put the value directly
                                            // Add the value to the values map using the property name
                                            if !override_instance.values.contains_key(prop_name) {
                                                override_instance.values.insert(
                                                    prop_name.clone(),
                                                    serde_json::Value::String(value.clone()),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // After extracting values from seeds, rebuild the account from template
                // This ensures the PDA is correctly constructed with property references
                override_instance.account = template.address.clone();

                // Validate constant_ref values against template constants
                for prop in &template.properties {
                    if prop.is_constant_ref() {
                        if let Some(constant_name) = prop.constant_name() {
                            if let Some(constant_def) = template.constants.get(constant_name) {
                                // Check if the value exists in values map
                                if let Some(value) = override_instance.values.get(&prop.path) {
                                    if let Some(value_str) = value.as_str() {
                                        // Validate the value is one of the valid options
                                        let is_valid = constant_def.options.iter().any(|opt| {
                                            opt.value.to_lowercase() == value_str.to_lowercase()
                                        });
                                        if !is_valid {
                                            // Show only first 10 options to avoid overwhelming error messages
                                            let sample_options: Vec<String> = constant_def
                                                .options
                                                .iter()
                                                .take(10)
                                                .map(|opt| format!("{}: {}", opt.label, opt.value))
                                                .collect();
                                            let total = constant_def.options.len();
                                            let more_msg = if total > 10 {
                                                format!(" (showing 10 of {} options)", total)
                                            } else {
                                                String::new()
                                            };
                                            validation_errors.push(format!(
                                                "Override '{}' (template '{}'): Invalid value '{}' for '{}' (constant: '{}'). Valid options{}:\n  {}",
                                                override_instance.id,
                                                override_instance.template_id,
                                                value_str,
                                                prop.path,
                                                constant_name,
                                                more_msg,
                                                sample_options.join("\n  ")
                                            ));
                                        }
                                    }
                                } else {
                                    // Value is missing - required for PDA derivation
                                    let valid_options: Vec<String> = constant_def
                                        .options
                                        .iter()
                                        .take(5) // Show first 5 options
                                        .map(|opt| format!("{}: {}", opt.label, opt.value))
                                        .collect();
                                    validation_errors.push(format!(
                                        "Override '{}' (template '{}'): Missing required value for '{}' (constant: '{}', used in PDA derivation). Add it to values. Example options:\n  {}",
                                        override_instance.id,
                                        override_instance.template_id,
                                        prop.path,
                                        constant_name,
                                        valid_options.join("\n  ")
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        if !invalid_templates.is_empty() {
            let valid_ids: Vec<String> = registry.list_ids();
            let response = RegisterScenarioResponse::error(format!(
                "Invalid templateId(s): {:?}. You MUST use templateIds from get_override_templates. Valid IDs are: {:?}",
                invalid_templates, valid_ids
            ));
            let json_str = serde_json::to_string(&response).unwrap_or_default();
            return Ok(CallToolResult::success(vec![Content::text(json_str)]));
        }

        if !validation_errors.is_empty() {
            let response = RegisterScenarioResponse::error(format!(
                "Validation errors:\n{}",
                validation_errors.join("\n")
            ));
            let json_str = serde_json::to_string(&response).unwrap_or_default();
            return Ok(CallToolResult::success(vec![Content::text(json_str)]));
        }

        drop(registry);

        let load_scenarios_endpoint = format!(
            "http://127.0.0.1:{}/v1/scenarios",
            CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED
        );
        let payload = serde_json::json!(scenario);

        // Make the RPC request to the surfnet RPC endpoint
        let client = reqwest::blocking::Client::new();
        let http_response = match client
            .post(&load_scenarios_endpoint)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
        {
            Ok(resp) => resp,
            Err(e) => {
                let response = RegisterScenarioResponse::error(format!(
                    "Failed to load scenarios at {}: {}",
                    load_scenarios_endpoint, e
                ));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let response_text = match http_response.text() {
            Ok(text) => text,
            Err(e) => {
                let response =
                    RegisterScenarioResponse::error(format!("Failed to read response text: {}", e));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        let rpc_response: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(json) => json,
            Err(e) => {
                let response = RegisterScenarioResponse::error(format!(
                    "Failed to parse JSON response: {}. Response: {}",
                    e, response_text
                ));
                let json_str = serde_json::to_string(&response).unwrap_or_default();
                return Ok(CallToolResult::success(vec![Content::text(json_str)]));
            }
        };

        if let Some(error) = rpc_response.get("error") {
            let response = RegisterScenarioResponse::error(format!("RPC error: {}", error));
            let json_str = serde_json::to_string(&response).unwrap_or_default();
            return Ok(CallToolResult::success(vec![Content::text(json_str)]));
        }

        // Extract the scenario id from the response
        let scenario_id = rpc_response
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&scenario.id);

        let url = format!(
            "http://127.0.0.1:{}/scenarios?id={}&tab=editor",
            CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED, scenario_id
        );
        let response = RegisterScenarioResponse::success(url);
        let json_str = serde_json::to_string(&response).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    #[tool(
        description = "Fetches ALL available override templates. MUST be called before create_scenario to get valid templateId values, property names, and account addresses."
    )]
    async fn get_override_templates(&self) -> Result<CallToolResult, McpError> {
        let registry = self.template_registry.read().map_err(|_| {
            use std::borrow::Cow;
            McpError {
                code: ErrorCode(-32603),
                message: Cow::from("Failed to read template registry"),
                data: None,
            }
        })?;

        // Return compact version without full IDL to avoid token limits
        let templates: Vec<serde_json::Value> = registry
            .all()
            .iter()
            .map(|t| {
                let mut obj = serde_json::json!({
                    "id": t.id,
                    "name": t.name,
                    "description": t.description,
                    "protocol": t.protocol,
                    "accountType": t.account_type,
                    "properties": t.properties,
                    "address": t.address,
                    "tags": t.tags
                });
                // Include llm_context if present
                if let Some(ref ctx) = t.llm_context {
                    obj["llmContext"] = serde_json::Value::String(ctx.clone());
                }
                obj
            })
            .collect();

        let json_str = serde_json::to_string(&templates).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    #[tool(
        description = "Translates a token symbol (e.g., 'USDC', 'SOL', 'JUP') into its mint address. Uses the verified tokens list."
    )]
    async fn get_token_address(
        &self,
        Parameters(params): Parameters<GetTokenAddressParams>,
    ) -> Result<CallToolResult, McpError> {
        let symbol_upper = params.symbol.to_uppercase();

        let response = match VERIFIED_TOKENS_BY_SYMBOL.get(&symbol_upper) {
            Some(token_info) => TokenAddressResponse::success(
                token_info.symbol.clone(),
                token_info.address.clone(),
                token_info.name.clone(),
                token_info.decimals,
            ),
            None => TokenAddressResponse::not_found(params.symbol),
        };

        let json_str = serde_json::to_string(&response).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }
}

#[tool_handler]
impl ServerHandler for Surfpool {
    /// Return information about the server, including its capabilities and description.
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Surfpool MCP server, your personal surfnet manager to start surfing on Solana!"
                    .to_string(),
            ),
        }
    }
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource {
                    uri: "str:///rpc_endpoints".to_string(),
                    name: "List of available RPC endpoints".to_string(),
                    title: Some("RPC Endpoints".to_string()),
                    description: Some("A json file containing all the RPC methods and the parameters available for being able to handle any RPC call with the tool call_surfnet_rpc".to_string()),
                    mime_type: Some("application/json".to_string()),
                    size: None,
                    icons: None,
                }.no_annotation(),
                RawResource {
                    uri: "str:///override_templates".to_string(),
                    name: "List of override templates".to_string(),
                    title: Some("Override Templates".to_string()),
                    description: Some("A json file containing all the override templates available for scenario registration with account overrides".to_string()),
                    mime_type: Some("application/json".to_string()),
                    size: None,
                    icons: None,
                }.no_annotation()
            ],
            next_cursor: None,
        })
    }
    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match uri.as_str() {
            "str:///rpc_endpoints" => {
                let rpc_endpoints = include_str!("../../../types/src/rpc_endpoints.json");
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri,
                        mime_type: Some("application/json".to_string()),
                        text: rpc_endpoints.to_string(),
                        meta: None,
                    }],
                })
            }
            "str:///override_templates" => {
                let registry = self.template_registry.read().map_err(|_| {
                    use std::borrow::Cow;
                    McpError {
                        code: ErrorCode(-32603),
                        message: Cow::from("Failed to read template registry"),
                        data: None,
                    }
                })?;

                // Return compact version without full IDL to avoid token limits
                let templates: Vec<serde_json::Value> = registry
                    .all()
                    .iter()
                    .map(|t| {
                        let mut obj = serde_json::json!({
                            "id": t.id,
                            "name": t.name,
                            "description": t.description,
                            "protocol": t.protocol,
                            "accountType": t.account_type,
                            "properties": t.properties,
                            "address": t.address,
                            "tags": t.tags
                        });
                        // Include llm_context if present
                        if let Some(ref ctx) = t.llm_context {
                            obj["llmContext"] = serde_json::Value::String(ctx.clone());
                        }
                        obj
                    })
                    .collect();

                let templates_json = serde_json::to_string(&templates).map_err(|_| {
                    use std::borrow::Cow;
                    McpError {
                        code: ErrorCode(-32603),
                        message: Cow::from("Failed to serialize templates"),
                        data: None,
                    }
                })?;

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri,
                        mime_type: Some("application/json".to_string()),
                        text: templates_json,
                        meta: None,
                    }],
                })
            }
            _ => {
                use std::borrow::Cow;
                Err(McpError {
                    code: ErrorCode(-32002),
                    message: Cow::from("Resource not found"),
                    data: Some(serde_json::json!({
                        "uri": uri
                    })),
                })
            }
        }
    }
    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }
}
