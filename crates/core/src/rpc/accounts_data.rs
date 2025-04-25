use std::collections::HashMap;

use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use solana_account::Account;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_account_decoder::{encode_ui_account, UiAccount, UiAccountEncoding};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_client::rpc_response::RpcBlockCommitment;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_runtime::commitment::BlockCommitmentArray;

use super::{not_implemented_err, RunloopContext};

#[rpc]
pub trait AccountsData {
    type Metadata;

    /// Returns detailed information about an account given its public key.
    ///
    /// This method queries the blockchain for the account associated with the provided
    /// public key string. It can be used to inspect balances, ownership, and program-related metadata.
    ///
    /// ## Parameters
    /// - `pubkey_str`: A base-58 encoded string representing the account's public key.
    /// - `config`: Optional configuration that controls encoding, commitment level,
    ///   data slicing, and other response details.
    ///
    /// ## Returns
    /// A [`RpcResponse`] containing an optional [`UiAccount`] object if the account exists.
    /// If the account does not exist, the response will contain `null`.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getAccountInfo",
    ///   "params": [
    ///     "9XQeWMPMPXwW1fzLEQeTTrfF5Eb9dj8Qs3tCPoMw3GiE",
    ///     {
    ///       "encoding": "jsonParsed",
    ///       "commitment": "finalized"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 12345678
    ///     },
    ///     "value": {
    ///       "lamports": 10000000,
    ///       "data": {
    ///         "program": "spl-token",
    ///         "parsed": { ... },
    ///         "space": 165
    ///       },
    ///       "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///       "executable": false,
    ///       "rentEpoch": 203,
    ///       "space": 165
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the public key is malformed or invalid
    /// - Returns an internal error if the ledger cannot be accessed
    ///
    /// ## See also
    /// - [`UiAccount`]: A readable structure representing on-chain accounts
    #[rpc(meta, name = "getAccountInfo")]
    fn get_account_info(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Option<UiAccount>>>>;

    /// Returns commitment levels for a given block (slot).
    ///
    /// This method provides insight into how many validators have voted for a specific block
    /// and with what level of lockout. This can be used to analyze consensus progress and
    /// determine finality confidence.
    ///
    /// ## Parameters
    /// - `block`: The target slot (block) to query.
    ///
    /// ## Returns
    /// A [`RpcBlockCommitment`] containing a [`BlockCommitmentArray`], which is an array of 32
    /// integers representing the number of votes at each lockout level for that block. Each index
    /// corresponds to a lockout level (i.e., confidence in finality).
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlockCommitment",
    ///   "params": [150000000]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "commitment": [0, 4, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ///     "totalStake": 100000000
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - If the slot is not found in the current bank or has been purged, this call may return an error.
    /// - May fail if the RPC node is lagging behind or doesn't have voting history for the slot.
    ///
    /// ## See also
    /// - [`BlockCommitmentArray`]: An array representing votes by lockout level
    /// - [`RpcBlockCommitment`]: Wrapper struct for the full response
    #[rpc(meta, name = "getBlockCommitment")]
    fn get_block_commitment(
        &self,
        meta: Self::Metadata,
        block: Slot,
    ) -> Result<RpcBlockCommitment<BlockCommitmentArray>>;

    /// Returns account information for multiple public keys in a single call.
    ///
    /// This method allows batching of account lookups for improved performance and fewer
    /// network roundtrips. It returns a list of `UiAccount` values in the same order as
    /// the provided public keys.
    ///
    /// ## Parameters
    /// - `pubkey_strs`: A list of base-58 encoded public key strings representing accounts to query.
    /// - `config`: Optional configuration to control encoding, commitment level, data slicing, etc.
    ///
    /// ## Returns
    /// A [`RpcResponse`] wrapping a vector of optional [`UiAccount`] objects.  
    /// Each element in the response corresponds to the public key at the same index in the request.
    /// If an account is not found, the corresponding entry will be `null`.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getMultipleAccounts",
    ///   "params": [
    ///     [
    ///       "9XQeWMPMPXwW1fzLEQeTTrfF5Eb9dj8Qs3tCPoMw3GiE",
    ///       "3nN8SBQ2HqTDNnaCzryrSv4YHd4d6GpVCEyDhKMPxN4o"
    ///     ],
    ///     {
    ///       "encoding": "jsonParsed",
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": { "slot": 12345678 },
    ///     "value": [
    ///       {
    ///         "lamports": 10000000,
    ///         "data": {
    ///           "program": "spl-token",
    ///           "parsed": { ... },
    ///           "space": 165
    ///         },
    ///         "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///         "executable": false,
    ///         "rentEpoch": 203,
    ///         "space": 165
    ///       },
    ///       null
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - If any public key is malformed or invalid, the entire call may fail.
    /// - Returns an internal error if the ledger cannot be accessed or some accounts are purged.
    ///
    /// ## See also
    /// - [`UiAccount`]: Human-readable representation of an account
    /// - [`get_account_info`]: Use when querying a single account
    #[rpc(meta, name = "getMultipleAccounts")]
    fn get_multiple_accounts(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Vec<String>,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<UiAccount>>>>>;

    /// Returns the balance of a token account, given its public key.
    ///
    /// This method fetches the token balance of an account, including its amount and
    /// user-friendly information (like the UI amount in human-readable format). It is useful
    /// for token-related applications, such as checking balances in wallets or exchanges.
    ///
    /// ## Parameters
    /// - `pubkey_str`: The base-58 encoded string of the public key of the token account.
    /// - `commitment`: Optional commitment configuration to specify the desired confirmation level of the query.
    ///
    /// ## Returns
    /// A [`RpcResponse`] containing the token balance in a [`UiTokenAmount`] struct.
    /// If the account doesn't hold any tokens or is invalid, the response will contain `null`.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTokenAccountBalance",
    ///   "params": [
    ///     "3nN8SBQ2HqTDNnaCzryrSv4YHd4d6GpVCEyDhKMPxN4o",
    ///     {
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 12345678
    ///     },
    ///     "value": {
    ///       "uiAmount": 100.0,
    ///       "decimals": 6,
    ///       "amount": "100000000",
    ///       "uiAmountString": "100.000000"
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - If the provided public key is invalid or does not exist.
    /// - If the account is not a valid token account or does not hold any tokens.
    ///
    /// ## See also
    /// - [`UiTokenAmount`]: Represents the token balance in user-friendly format.
    #[rpc(meta, name = "getTokenAccountBalance")]
    fn get_token_account_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>>;

    /// Returns the total supply of a token, given its mint address.
    ///
    /// This method provides the total circulating supply of a specific token, including the raw
    /// amount and human-readable UI-formatted values. It can be useful for tracking token issuance
    /// and verifying the supply of a token on-chain.
    ///
    /// ## Parameters
    /// - `mint_str`: The base-58 encoded string of the mint address for the token.
    /// - `commitment`: Optional commitment configuration to specify the desired confirmation level of the query.
    ///
    /// ## Returns
    /// A [`RpcResponse`] containing the total token supply in a [`UiTokenAmount`] struct.
    /// If the token does not exist or is invalid, the response will return an error.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTokenSupply",
    ///   "params": [
    ///     "So11111111111111111111111111111111111111112",
    ///     {
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 12345678
    ///     },
    ///     "value": {
    ///       "uiAmount": 1000000000.0,
    ///       "decimals": 6,
    ///       "amount": "1000000000000000",
    ///       "uiAmountString": "1000000000.000000"
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - If the mint address is invalid or does not correspond to a token.
    /// - If the token supply cannot be fetched due to network issues or node synchronization problems.
    ///
    /// ## See also
    /// - [`UiTokenAmount`]: Represents the token balance or supply in a user-friendly format.
    #[rpc(meta, name = "getTokenSupply")]
    fn get_token_supply(
        &self,
        meta: Self::Metadata,
        mint_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>>;
}

pub struct SurfpoolAccountsDataRpc;
impl AccountsData for SurfpoolAccountsDataRpc {
    type Metadata = Option<RunloopContext>;

    fn get_account_info(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Option<UiAccount>>>> {
        let config = config.unwrap_or_default();
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };

        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let account = state_reader.svm.get_account(&pubkey);

        // Drop the lock on the state while we fetch accounts
        let absolute_slot = state_reader.epoch_info.absolute_slot;

        let rpc_client = RpcClient::new(state_reader.rpc_url.clone());
        let encoding = config.encoding.clone();
        let data_slice = config.data_slice.clone();
        drop(state_reader);

        Box::pin(async move {
            let account = if let None = account {
                // Fetch and save the missing account
                if let Some(fetched_account) = rpc_client.get_account(&pubkey).await.ok() {
                    // let mut state_reader = meta.get_state_mut()?;
                    // state_reader
                    //     .svm
                    //     .set_account(pubkey, fetched_account.clone())
                    //     .map_err(|err| {
                    //         Error::invalid_params(format!(
                    //             "failed to save fetched account {pubkey:?}: {err:?}"
                    //         ))
                    //     })?;

                    Some(fetched_account)
                } else {
                    None
                }
            } else {
                account
            };

            Ok(RpcResponse {
                context: RpcResponseContext::new(absolute_slot),
                value: account.map(|account| {
                    encode_ui_account(
                        &pubkey,
                        &account,
                        encoding.unwrap_or(UiAccountEncoding::Base64),
                        None,
                        data_slice,
                    )
                }),
            })
        })
    }

    fn get_multiple_accounts(
        &self,
        meta: Self::Metadata,
        pubkeys_str: Vec<String>,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<UiAccount>>>>> {
        let config = config.unwrap_or_default();
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let pubkeys = match pubkeys_str
            .iter()
            .map(|s| verify_pubkey(s))
            .collect::<Result<Vec<_>>>()
        {
            Ok(p) => p,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let accounts = match pubkeys
            .clone()
            .iter()
            .map(|pk| Ok((pk.clone(), state_reader.svm.get_account(&pk))))
            .collect::<Result<Vec<_>>>()
        {
            Ok(accs) => accs,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        // Drop the lock on the state while we fetch accounts
        let absolute_slot = state_reader.epoch_info.absolute_slot;
        let rpc_client = RpcClient::new(state_reader.rpc_url.clone());
        let encoding = config.encoding.clone();
        let data_slice = config.data_slice.clone();
        drop(state_reader);

        Box::pin(async move {
            let missing_accounts_pks = accounts
                .iter()
                .filter_map(|(pk, acc)| if acc.is_none() { Some(*pk) } else { None })
                .collect::<Vec<_>>();
            let fetched_accounts = missing_accounts_pks
                .iter()
                .zip(
                    rpc_client
                        .get_multiple_accounts(&missing_accounts_pks)
                        .await
                        .map_err(|err| {
                            Error::invalid_params(format!("failed to fetch accounts: {err:?}"))
                        })?,
                )
                .filter_map(|(pk, acc)| acc.map(|acc| (*pk, acc)))
                .collect::<HashMap<Pubkey, Account>>();
            let mut state_reader = meta.get_state_mut()?;
            let mut combined_accounts = vec![];
            for (pk, acc) in accounts.iter() {
                if acc.is_some() {
                    combined_accounts.push((pk.clone(), acc.clone()));
                } else if let Some(fetched) = fetched_accounts.get(&pk) {
                    combined_accounts.push((pk.clone(), Some(fetched.clone())));
                    state_reader
                        .svm
                        .set_account(*pk, fetched.clone())
                        .map_err(|err| {
                            Error::invalid_params(format!(
                                "failed to save fetched account {pk:?}: {err:?}"
                            ))
                        })?;
                } else {
                    combined_accounts.push((pk.clone(), None));
                }
            }

            Ok(RpcResponse {
                context: RpcResponseContext::new(absolute_slot),
                value: combined_accounts
                    .into_iter()
                    .map(|(pk, account)| {
                        let encoding = encoding.clone();
                        let data_slice = data_slice.clone();

                        account.map(|account| {
                            encode_ui_account(
                                &pk,
                                &account,
                                encoding.unwrap_or(UiAccountEncoding::Base64),
                                None,
                                data_slice,
                            )
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        })
    }

    fn get_block_commitment(
        &self,
        _meta: Self::Metadata,
        _block: Slot,
    ) -> Result<RpcBlockCommitment<BlockCommitmentArray>> {
        not_implemented_err()
    }

    // SPL Token-specific RPC endpoints
    // See https://github.com/solana-labs/solana-program-library/releases/tag/token-v2.0.0 for
    // program details

    fn get_token_account_balance(
        &self,
        _meta: Self::Metadata,
        _pubkey_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>> {
        not_implemented_err()
    }

    fn get_token_supply(
        &self,
        _meta: Self::Metadata,
        _mint_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>> {
        not_implemented_err()
    }
}
