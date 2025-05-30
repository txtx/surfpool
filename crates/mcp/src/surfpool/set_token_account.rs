use bs58;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token;
use surfpool_types::verified_tokens::VERIFIED_TOKENS_BY_SYMBOL;

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
struct SetAccountParams {
    pubkey: String,
    update: AccountUpdateParams,
}

#[derive(Serialize)]
struct AccountUpdateParams {
    lamports: Option<u64>,
}

#[derive(Serialize)]
struct SetTokenAccountParams {
    owner: String,
    mint: String,
    update: TokenAccountUpdateParams,
    token_program: Option<String>,
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
    success: Option<AccountUpdated>,
    error: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct AccountUpdated {
    account: SeededAccount,
    token_address: String,         // Mint address or "SOL"
    token_symbol: Option<String>,  // e.g., "SOL", "USDC", or mint symbol
    token_account_address: String, // Wallet address for SOL, ATA for SPL
    amount: u64,
}

#[derive(Serialize, Debug, Clone)]
pub enum SeededAccount {
    Provided(String),      // Existing public key
    Generated(NewAccount), // Details of a newly generated account
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

const DEFAULT_TOKEN_AMOUNT: u64 = 100_000_000_000; // 100 SOL or 100 of a token with 9 decimals
const SOL_SYMBOL: &str = "SOL";
const SOL_MINT_PLACEHOLDER: &str = "So11111111111111111111111111111111111111112"; // Official SOL mint placeholder

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
///
/// # Returns
/// * `SetTokenAccountResponse`: Contains either details of the successful account update (including new wallet details if generated) or an error message.                             
pub fn run(
    surfnet_address: String,
    wallet_address_opt: Option<String>,
    token_identifier_str: String, // Can be "SOL", a mint address, or a known symbol
    token_amount_opt: Option<u64>,
) -> SetTokenAccountResponse {
    let client = Client::new();
    let rpc_url = surfnet_address;

    // Determine the owner's public key. If no wallet address is provided, generate a new keypair.
    // The keypair itself is stored if generated, so the secret key can be returned.
    let owner_keypair: Option<Keypair> = match wallet_address_opt {
        None => Some(Keypair::new()),
        Some(_) => None,
    };
    let owner_pubkey_str = owner_keypair
        .as_ref()
        .map(|kp| kp.pubkey().to_string())
        .unwrap_or_else(|| wallet_address_opt.clone().unwrap_or_default());

    // Validate the owner public key string.
    let owner_pubkey = match Pubkey::try_from(owner_pubkey_str.as_str()) {
        Ok(pk) => pk,
        Err(_) => {
            return SetTokenAccountResponse::error(format!(
                "Invalid owner wallet address: {}",
                owner_pubkey_str
            ))
        }
    };

    let amount_to_set = token_amount_opt.unwrap_or(DEFAULT_TOKEN_AMOUNT);

    // Determine if it's SOL, a direct mint address, or a symbol to look up.
    let is_sol = token_identifier_str.to_uppercase() == SOL_SYMBOL;
    let mut actual_mint_address_str: String;
    let mut actual_token_symbol_opt: Option<String> = None;

    if is_sol {
        actual_mint_address_str = SOL_MINT_PLACEHOLDER.to_string();
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
        let lamports_to_set = match &actual_token_symbol_opt {
            Some(token_symbol) => match VERIFIED_TOKENS_BY_SYMBOL.get(&token_symbol.to_uppercase()) {
                Some(token_info) => amount_to_set
                    .checked_mul(10u64.pow(token_info.decimals as u32))
                    .unwrap_or(amount_to_set),
                None => amount_to_set,
            },
            None => amount_to_set,
        };
        let update_params = AccountUpdateParams {
            lamports: Some(lamports_to_set),
        };
        let params_tuple = (owner_pubkey_str.clone(), update_params);
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
        let update_params = TokenAccountUpdateParams {
            amount: Some(amount_to_set),
        };

        let params_tuple = (
            owner_pubkey_str.clone(),
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
                            // Prepare the `SeededAccount` part of the response.
                            let seeded_account = if let Some(kp) = owner_keypair {
                                // If a new keypair was generated, include its secret and public keys.
                                SeededAccount::Generated(NewAccount {
                                    secret_key: bs58::encode(kp.to_bytes()).into_string(),
                                    public_key: kp.pubkey().to_string(),
                                })
                            } else {
                                // If an existing wallet was used, just return its public key.
                                SeededAccount::Provided(owner_pubkey_str.clone())
                            };

                            let final_token_account_address = if is_sol {
                                owner_pubkey_str.clone()
                            } else {
                                let mint_pubkey =
                                    Pubkey::try_from(actual_mint_address_str.as_str())
                                        .expect("Mint pubkey should be validated by now");
                                get_associated_token_address_with_program_id(
                                    &owner_pubkey,
                                    &mint_pubkey,
                                    &spl_token::id(),
                                )
                                .to_string()
                            };

                            // Construct the successful response details.
                            let account_updated = AccountUpdated {
                                account: seeded_account,
                                token_address: actual_mint_address_str.clone(), // Use resolved mint address
                                token_symbol: actual_token_symbol_opt.clone(), // Use resolved symbol
                                token_account_address: final_token_account_address,
                                amount: amount_to_set,
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
        Err(e) => {
            // Network error or other issue preventing the request from being sent.
            SetTokenAccountResponse::error(format!("Failed to send RPC request: {}", e))
        }
    }
}
