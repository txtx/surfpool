use bs58;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token;
use surfpool_types::{types::AccountUpdate, verified_tokens::VERIFIED_TOKENS_BY_SYMBOL};

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse<T> {
    // jsonrpc: String,
    // id: u64,
    #[allow(dead_code)]
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Serialize)]
struct TokenAccountUpdateParams {
    amount: Option<u64>,
    // delegate: Option<String>, // null or pubkey // TODO: add this
    // state: Option<String>, // "uninitialized", "frozen", "initialized" // TODO: add this
    // delegated_amount: Option<u64>, // TODO: add this
    // close_authority: Option<String>, // null or pubkey // TODO: add this
}

#[derive(Serialize, Debug, Clone)]
pub struct SetTokenAccountResponse {
    pub success: Option<AccountUpdated>,
    pub error: Option<String>,
}
#[derive(Serialize, Debug, Clone)]
pub struct SetTokenAccountsResponse {
    pub success: Option<Vec<AccountUpdated>>,
    pub error: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct AccountUpdated {
    account: SeededAccount,
    token_address: String,            // Mint address or "SOL"
    token_symbol: Option<String>,     // e.g., "SOL", "USDC", or mint symbol
    associated_token_address: String, // Wallet address for SOL, ATA for SPL
    amount: u64,
    owner_pubkey: String,
}

#[derive(Serialize, Debug, Clone)]
pub enum SeededAccount {
    Provided(String),      // Existing public key
    Generated(NewAccount), // Details of a newly generated account
}

impl SeededAccount {
    pub fn new(input_account: Option<String>) -> Self {
        match input_account {
            Some(pubkey) => SeededAccount::Provided(pubkey),
            None => {
                let new_keypair = Keypair::new();
                SeededAccount::Generated(NewAccount {
                    secret_key: bs58::encode(new_keypair.to_bytes()).into_string(),
                    public_key: new_keypair.pubkey().to_string(),
                })
            }
        }
    }

    pub fn pubkey(&self) -> String {
        match self {
            SeededAccount::Provided(pubkey) => pubkey.clone(),
            SeededAccount::Generated(new_account) => new_account.public_key.clone(),
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct NewAccount {
    secret_key: String, // Base58 encoded secret key
    public_key: String,
}

impl SetTokenAccountResponse {
    pub fn success(details: AccountUpdated) -> Self {
        Self {
            success: Some(details),
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

impl SetTokenAccountsResponse {
    pub fn success(details: Vec<AccountUpdated>) -> Self {
        Self {
            success: Some(details),
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
const SOL_DECIMALS: u32 = 9; // Define SOL decimals constant
const DEFAULT_TOKEN_AMOUNT: u64 = 100_000_000_000; // 100 SOL or 100 of a token with 9 decimals
const SOL_SYMBOL: &str = "SOL";

/// Handles the MCP tool call to set a token balance for a specified account on a Surfnet instance.
///
/// This function can either fund an existing wallet or generate a new one.
/// It distinguishes between funding SOL (native currency) and SPL tokens (mint addresses).
///
/// # Arguments
/// * `surfnet_address`: The HTTP RPC endpoint of the target Surfnet instance .
/// * `wallet_address_opt`: An optional base58-encoded public key of the wallet to fund.
///                         If `None`, a new keypair is generated, and its details are returned.
/// * `token_identifier_str`: A string indicating the token to fund. Can be "SOL" (case-insensitive),
///                          a base58-encoded SPL token mint address, or a known symbol.
/// * `token_amount_opt`: An optional amount of the token to set (in its smallest unit, e.g., lamports for SOL).
///                       If `None`, a default amount is used.
/// * `program_id_opt`: An optional base58-encoded public key of the token-issuing program.
///                     If `None`, defaults to the standard SPL Token program ID.
///
/// # Returns
/// * `SetTokenAccountResponse`: Contains either details of the successful account update (including new wallet details if generated) or an error message.                             
pub fn run(
    surfnet_address: String,
    owner_seeded_account: SeededAccount,
    token_identifier_str: String, // Can be "SOL", a mint address, or a known symbol
    token_amount_opt: Option<u64>,
    program_id_opt: Option<String>,
) -> SetTokenAccountResponse {
    let client = Client::new();
    let rpc_url = surfnet_address;

    let amount_to_set = token_amount_opt.unwrap_or(DEFAULT_TOKEN_AMOUNT);

    // Determine if it's SOL, a direct mint address, or a symbol to look up.
    let is_sol = token_identifier_str.to_uppercase() == SOL_SYMBOL;
    let actual_mint_address_str: String;
    let mut actual_token_symbol_opt: Option<String> = None;

    if is_sol {
        actual_mint_address_str = SOL_SYMBOL.to_string(); // Set to "SOL" for native SOL
        actual_token_symbol_opt = Some(SOL_SYMBOL.to_string());
    } else {
        // Not SOL, try parsing as a mint address directly.
        match Pubkey::try_from(token_identifier_str.as_str()) {
            Ok(_mint_pubkey) => {
                actual_mint_address_str = token_identifier_str.clone();
                actual_token_symbol_opt = VERIFIED_TOKENS_BY_SYMBOL
                    .iter()
                    .find(|(_, info)| info.address == actual_mint_address_str)
                    .map(|(sym, _)| sym.clone());
            }
            Err(_) => {
                // if not a direct mint address, try looking up as a symbol in VERIFIED_TOKENS_BY_SYMBOL (case-insensitive).
                match VERIFIED_TOKENS_BY_SYMBOL.get(&token_identifier_str.to_uppercase()) {
                    Some(token_info) => {
                        actual_mint_address_str = token_info.address.clone();
                        actual_token_symbol_opt = Some(token_info.symbol.clone());
                    }
                    None => {
                        return SetTokenAccountResponse::error(format!(
                            "The token symbol provided '{}' is not a verified token. Please provide the token address instead.",
                            token_identifier_str
                        ));
                    }
                }
            }
        }
    }

    // Construct the appropriate JSON-RPC request payload based on whether it's SOL or an SPL token.
    let request_payload = if is_sol {
        // For SOL, use the `surfnet_setAccount` RPC method.
        // Parameters: (pubkey: String, update: AccountUpdate)
        // amount_to_set converted to lamports
        let lamports_to_set = amount_to_set
            .checked_mul(10u64.pow(SOL_DECIMALS))
            .unwrap_or(amount_to_set);

        let update_params = AccountUpdate {
            lamports: Some(lamports_to_set),
            ..Default::default()
        };
        let params_tuple = (owner_seeded_account.pubkey(), update_params);
        let params_value = match serde_json::to_value(params_tuple) {
            Ok(v) => v,
            Err(e) => {
                return SetTokenAccountResponse::error(format!(
                    "Failed to serialize params for surfnet_setAccount: {}",
                    e
                ))
            }
        };
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "surfnet_setAccount".to_string(),
            params: params_value,
        }
    } else {
        let token_amount_to_set = match &actual_token_symbol_opt {
            Some(token_symbol) => match VERIFIED_TOKENS_BY_SYMBOL.get(&token_symbol.to_uppercase())
            {
                Some(token_info) => amount_to_set
                    .checked_mul(10u64.pow(token_info.decimals as u32))
                    .unwrap_or(amount_to_set),
                None => amount_to_set,
            },
            None => amount_to_set,
        };
        let update_params = TokenAccountUpdateParams {
            amount: Some(token_amount_to_set),
        };

        let params_tuple = (
            owner_seeded_account.pubkey(), // Use the public key of the provided or generated account
            actual_mint_address_str.clone(), // Use the resolved mint address
            update_params,
            None::<String>,
        );
        let params_value = match serde_json::to_value(params_tuple) {
            Ok(v) => v,
            Err(e) => {
                return SetTokenAccountResponse::error(format!(
                    "Failed to serialize params for surfnet_setTokenAccount: {}",
                    e
                ))
            }
        };
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "surfnet_setTokenAccount".to_string(),
            params: params_value,
        }
    };

    // Send the HTTP POST request to the Surfnet RPC endpoint.
    match client.post(&rpc_url).json(&request_payload).send() {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<JsonRpcResponse<serde_json::Value>>() {
                    Ok(rpc_response) => {
                        if let Some(err) = rpc_response.error {
                            SetTokenAccountResponse::error(format!(
                                "RPC Error (code {}): {}",
                                err.code, err.message
                            ))
                        } else {
                            // Successfully processed by Surfnet.

                            let final_token_account_address = if is_sol {
                                owner_seeded_account.pubkey()
                            } else {
                                let mint_pubkey =
                                    Pubkey::try_from(actual_mint_address_str.as_str())
                                        .expect("Mint pubkey should be validated by now");

                                let token_program_id = match program_id_opt {
                                    Some(id_str) => match Pubkey::try_from(id_str.as_str()) {
                                        Ok(pk) => pk,
                                        Err(_) => {
                                            return SetTokenAccountResponse::error(format!(
                                                "Invalid program_id provided: {}",
                                                id_str
                                            ))
                                        }
                                    },
                                    None => spl_token::id(), // Default to SPL Token program ID
                                };

                                get_associated_token_address_with_program_id(
                                    &Pubkey::from_str_const(&owner_seeded_account.pubkey()),
                                    &mint_pubkey,
                                    &token_program_id, // Use the determined program ID
                                )
                                .to_string()
                            };

                            // Construct the successful response details.
                            let account_updated = AccountUpdated {
                                account: owner_seeded_account.clone(),
                                token_address: actual_mint_address_str.clone(),
                                token_symbol: actual_token_symbol_opt.clone(), // Use resolved symbol
                                associated_token_address: final_token_account_address,
                                amount: amount_to_set,
                                owner_pubkey: owner_seeded_account.pubkey(),
                            };
                            SetTokenAccountResponse::success(account_updated)
                        }
                    }
                    Err(e) => SetTokenAccountResponse::error(format!(
                        "Failed to parse JSON RPC response: {}",
                        e
                    )),
                }
            } else {
                // HTTP request to Surfnet failed.
                SetTokenAccountResponse::error(format!(
                    "HTTP Error: {} - {}",
                    response.status(),
                    response
                        .text()
                        .unwrap_or_else(|_| "<failed to read body>".to_string())
                ))
            }
        }
        Err(_e) => {
            // Network error or other issue preventing the request from being sent.
            SetTokenAccountResponse::error(
                "RPC request failed (timeout or other network error)".to_string(),
            )
        }
    }
}
