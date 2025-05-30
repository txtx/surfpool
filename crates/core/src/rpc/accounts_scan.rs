use jsonrpc_core::{BoxFuture, Error as JsonRpcCoreError, ErrorCode, Result};
use jsonrpc_derive::rpc;
use solana_account_decoder::{encode_ui_account, UiAccountEncoding};
use solana_client::{
    rpc_config::{
        RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig, RpcSupplyConfig,
        RpcTokenAccountsFilter,
    },
    rpc_filter::RpcFilterType,
    rpc_response::{
        OptionalContext, RpcAccountBalance, RpcKeyedAccount, RpcResponseContext, RpcSupply,
        RpcTokenAccountBalance,
    },
};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::program_pack::Pack;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token::state::Account as TokenAccount;

use super::{
    not_implemented_err_async, utils::verify_pubkey, RunloopContext, State, SurfnetRpcContext,
};
use crate::surfnet::locker::SvmAccessContext;

#[rpc]
pub trait AccountsScan {
    type Metadata;

    /// Returns all accounts owned by the specified program ID, optionally filtered and configured.
    ///
    /// This RPC method retrieves all accounts whose owner is the given program. It is commonly used
    /// to scan on-chain program state, such as finding all token accounts, order books, or PDAs
    /// owned by a given program. The results can be filtered using data size, memory comparisons, and
    /// token-specific criteria.
    ///
    /// ## Parameters
    /// - `program_id_str`: Base-58 encoded program ID to scan for owned accounts.
    /// - `config`: Optional configuration object allowing filters, encoding options, context inclusion,
    ///   and sorting of results.
    ///
    /// ## Returns
    /// A future resolving to a vector of [`RpcKeyedAccount`]s wrapped in an [`OptionalContext`].
    /// Each result includes the account's public key and full account data.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getProgramAccounts",
    ///   "params": [
    ///     "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///     {
    ///       "filters": [
    ///         {
    ///           "dataSize": 165
    ///         },
    ///         {
    ///           "memcmp": {
    ///             "offset": 0,
    ///             "bytes": "3N5kaPhfUGuTQZPQ3mnDZZGkUZ97rS1NVSC94QkgUzKN"
    ///           }
    ///         }
    ///       ],
    ///       "encoding": "jsonParsed",
    ///       "commitment": "finalized",
    ///       "withContext": true
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
    ///     "value": [
    ///       {
    ///         "pubkey": "BvckZ2XDJmJLho7LnFnV7zM19fRZqnvfs8Qy3fLo6EEk",
    ///         "account": {
    ///           "lamports": 2039280,
    ///           "data": {...},
    ///           "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///           "executable": false,
    ///           "rentEpoch": 255,
    ///           "space": 165
    ///         }
    ///       },
    ///       ...
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Filters
    /// - `DataSize(u64)`: Only include accounts with a matching data length.
    /// - `Memcmp`: Match byte patterns at specified offsets in account data.
    /// - `TokenAccountState`: Match on internal token account state (e.g. initialized).
    ///
    /// ## See also
    /// - [`RpcProgramAccountsConfig`]: Main config for filtering and encoding.
    /// - [`UiAccount`]: Returned data representation.
    /// - [`RpcKeyedAccount`]: Wrapper struct with both pubkey and account fields.
    #[rpc(meta, name = "getProgramAccounts")]
    fn get_program_accounts(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>>;

    /// Returns the 20 largest accounts by lamport balance, optionally filtered by account type.
    ///
    /// This RPC endpoint is useful for analytics, network monitoring, or understanding
    /// the distribution of large token holders. It can also be used for sanity checks on
    /// protocol activity or whale tracking.
    ///
    /// ## Parameters
    /// - `config`: Optional configuration allowing for filtering on specific account types
    ///   such as circulating or non-circulating accounts.
    ///
    /// ## Returns
    /// A future resolving to a [`RpcResponse`] containing a list of the 20 largest accounts
    /// by lamports, each represented as an [`RpcAccountBalance`].
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getLargestAccounts",
    ///   "params": [
    ///     {
    ///       "filter": "circulating"
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
    ///       "slot": 15039284
    ///     },
    ///     "value": [
    ///       {
    ///         "lamports": 999999999999,
    ///         "address": "9xQeWvG816bUx9EPaZzdd5eUjuJcN3TBDZcd8DM33zDf"
    ///       },
    ///       ...
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## See also
    /// - [`RpcLargestAccountsConfig`] *(defined elsewhere)*: Config struct that may specify a `filter`.
    /// - [`RpcAccountBalance`]: Struct representing account address and lamport amount.
    ///
    /// # Notes
    /// This method only returns up to 20 accounts and is primarily intended for inspection or diagnostics.
    #[rpc(meta, name = "getLargestAccounts")]
    fn get_largest_accounts(
        &self,
        meta: Self::Metadata,
        config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>>;

    /// Returns information about the current token supply on the network, including
    /// circulating and non-circulating amounts.
    ///
    /// This method provides visibility into the economic state of the chain by exposing
    /// the total amount of tokens issued, how much is in circulation, and what is held in
    /// non-circulating accounts.
    ///
    /// ## Parameters
    /// - `config`: Optional [`RpcSupplyConfig`] that allows specifying commitment level and
    ///   whether to exclude the list of non-circulating accounts from the response.
    ///
    /// ## Returns
    /// A future resolving to a [`RpcResponse`] containing a [`RpcSupply`] struct with
    /// supply metrics in lamports.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getSupply",
    ///   "params": [
    ///     {
    ///       "excludeNonCirculatingAccountsList": true
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
    ///       "slot": 18000345
    ///     },
    ///     "value": {
    ///       "total": 510000000000000000,
    ///       "circulating": 420000000000000000,
    ///       "nonCirculating": 90000000000000000,
    ///       "nonCirculatingAccounts": []
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## See also
    /// - [`RpcSupplyConfig`]: Configuration struct for optional parameters.
    /// - [`RpcSupply`]: Response struct with total, circulating, and non-circulating amounts.
    ///
    /// # Notes
    /// - All values are returned in lamports.
    /// - Use this method to monitor token inflation, distribution, and locked supply dynamics.
    #[rpc(meta, name = "getSupply")]
    fn get_supply(
        &self,
        meta: Self::Metadata,
        config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>>;

    /// Returns the addresses and balances of the largest accounts for a given SPL token mint.
    ///
    /// This method is useful for analyzing token distribution and concentration, especially
    /// to assess decentralization or identify whales.
    ///
    /// ## Parameters
    /// - `mint_str`: The base-58 encoded public key of the mint account of the SPL token.
    /// - `commitment`: Optional commitment level to query the state of the ledger at different levels
    ///   of finality (e.g., `Processed`, `Confirmed`, `Finalized`).
    ///
    /// ## Returns
    /// A [`BoxFuture`] resolving to a [`RpcResponse`] with a vector of [`RpcTokenAccountBalance`]s,
    /// representing the largest accounts holding the token.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTokenLargestAccounts",
    ///   "params": [
    ///     "So11111111111111111111111111111111111111112"
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
    ///       "slot": 18300000
    ///     },
    ///     "value": [
    ///       {
    ///         "address": "5xy34...Abcd1",
    ///         "amount": "100000000000",
    ///         "decimals": 9,
    ///         "uiAmount": 100.0,
    ///         "uiAmountString": "100.0"
    ///       },
    ///       {
    ///         "address": "2aXyZ...Efgh2",
    ///         "amount": "50000000000",
    ///         "decimals": 9,
    ///         "uiAmount": 50.0,
    ///         "uiAmountString": "50.0"
    ///       }
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## See also
    /// - [`UiTokenAmount`]: Describes the token amount in different representations.
    /// - [`RpcTokenAccountBalance`]: Includes token holder address and amount.
    ///
    /// # Notes
    /// - Balances are sorted in descending order.
    /// - Token decimals are used to format the raw amount into a user-friendly float string.
    #[rpc(meta, name = "getTokenLargestAccounts")]
    fn get_token_largest_accounts(
        &self,
        meta: Self::Metadata,
        mint_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>>;

    /// Returns all SPL Token accounts owned by a specific wallet address, optionally filtered by mint or program.
    ///
    /// This endpoint is commonly used by wallets and explorers to retrieve all token balances
    /// associated with a user, and optionally narrow results to a specific token mint or program.
    ///
    /// ## Parameters
    /// - `owner_str`: The base-58 encoded public key of the wallet owner.
    /// - `token_account_filter`: A [`RpcTokenAccountsFilter`] enum that allows filtering results by:
    ///   - Mint address
    ///   - Program ID (usually the SPL Token program)
    /// - `config`: Optional configuration for encoding, data slicing, and commitment.
    ///
    /// ## Returns
    /// A [`BoxFuture`] resolving to a [`RpcResponse`] containing a vector of [`RpcKeyedAccount`]s.
    /// Each entry contains the public key of a token account and its deserialized account data.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTokenAccountsByOwner",
    ///   "params": [
    ///     "4Nd1mKxQmZj...Aa123",
    ///     {
    ///       "mint": "So11111111111111111111111111111111111111112"
    ///     },
    ///     {
    ///       "encoding": "jsonParsed"
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
    ///     "context": { "slot": 19281234 },
    ///     "value": [
    ///       {
    ///         "pubkey": "2sZp...xyz",
    ///         "account": {
    ///           "lamports": 2039280,
    ///           "data": { /* token info */ },
    ///           "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///           "executable": false,
    ///           "rentEpoch": 123
    ///         }
    ///       }
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Filter Enum
    /// [`RpcTokenAccountsFilter`] can be:
    /// - `Mint(String)` — return only token accounts associated with the specified mint.
    /// - `ProgramId(String)` — return only token accounts owned by the specified program (e.g. SPL Token program).
    ///
    /// ## See also
    /// - [`RpcKeyedAccount`]: Contains the pubkey and the associated account data.
    /// - [`RpcAccountInfoConfig`]: Allows tweaking how account data is returned (encoding, commitment, etc.).
    /// - [`UiAccountEncoding`], [`CommitmentConfig`]
    ///
    /// # Notes
    /// - The response may contain `Option::None` for accounts that couldn't be fetched or decoded.
    /// - Encoding `jsonParsed` is recommended when integrating with frontend UIs.
    #[rpc(meta, name = "getTokenAccountsByOwner")]
    fn get_token_accounts_by_owner(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>>;

    /// Returns all SPL Token accounts that have delegated authority to a specific address, with optional filters.
    ///
    /// This RPC method is useful for identifying which token accounts have granted delegate rights
    /// to a particular wallet or program (commonly used in DeFi apps or custodial flows).
    ///
    /// ## Parameters
    /// - `delegate_str`: The base-58 encoded public key of the delegate authority.
    /// - `token_account_filter`: A [`RpcTokenAccountsFilter`] enum to filter results by mint or program.
    /// - `config`: Optional [`RpcAccountInfoConfig`] for controlling account encoding, commitment level, etc.
    ///
    /// ## Returns
    /// A [`BoxFuture`] resolving to a [`RpcResponse`] containing a vector of [`RpcKeyedAccount`]s,
    /// each pairing a token account public key with its associated on-chain data.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTokenAccountsByDelegate",
    ///   "params": [
    ///     "3qTwHcdK1j...XYZ",
    ///     { "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" },
    ///     { "encoding": "jsonParsed" }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": { "slot": 19301523 },
    ///     "value": [
    ///       {
    ///         "pubkey": "8H5k...abc",
    ///         "account": {
    ///           "lamports": 2039280,
    ///           "data": { /* token info */ },
    ///           "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ///           "executable": false,
    ///           "rentEpoch": 131
    ///         }
    ///       }
    ///     ]
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Filters
    /// Use [`RpcTokenAccountsFilter`] to limit the query scope:
    /// - `Mint(String)` – return accounts associated with a given token.
    /// - `ProgramId(String)` – return accounts under a specific program (e.g., SPL Token program).
    ///
    /// # Notes
    /// - Useful for monitoring delegated token activity in governance or trading protocols.
    /// - If a token account doesn't have a delegate, it won't be included in results.
    ///
    /// ## See also
    /// - [`RpcKeyedAccount`], [`RpcAccountInfoConfig`], [`CommitmentConfig`], [`UiAccountEncoding`]
    #[rpc(meta, name = "getTokenAccountsByDelegate")]
    fn get_token_accounts_by_delegate(
        &self,
        meta: Self::Metadata,
        delegate_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>>;
}

pub struct SurfpoolAccountsScanRpc;
impl AccountsScan for SurfpoolAccountsScanRpc {
    type Metadata = Option<RunloopContext>;

    fn get_program_accounts(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>> {
        let config = config.unwrap_or_default();
        let program_id = match verify_pubkey(&program_id_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let current_slot = svm_locker.get_latest_absolute_slot();

            let account_config = config.account_config;

            if let Some(min_context_slot_val) = account_config.min_context_slot.as_ref() {
                if current_slot < *min_context_slot_val {
                    return Err(JsonRpcCoreError {
                        code: ErrorCode::InternalError,
                        message: format!(
                            "Node's current slot {} is less than requested minContextSlot {}",
                            current_slot, min_context_slot_val
                        ),
                        data: None,
                    });
                }
            }

            let mut results: Vec<RpcKeyedAccount> = Vec::new();

            // Get program-owned accounts from the account registry
            let program_accounts = svm_locker
                .get_program_accounts(&remote_ctx.map(|(client, _)| client), &program_id)
                .await?
                .inner;

            for (account_pubkey, account) in program_accounts {
                if let Some(ref active_filters) = config.filters {
                    match apply_rpc_filters(&account.data, active_filters) {
                        Ok(true) => { /* Matches */ }
                        Ok(false) => continue,   // Filtered out
                        Err(e) => return Err(e), // Error applying filter, already JsonRpcError
                    }
                }

                let encoding = account_config.encoding.unwrap_or(UiAccountEncoding::Base64);
                let data_slice = account_config.data_slice;

                let ui_account = encode_ui_account(
                    &account_pubkey,
                    &account,
                    encoding,
                    None, // No additional data for now
                    data_slice,
                );

                results.push(RpcKeyedAccount {
                    pubkey: account_pubkey.to_string(),
                    account: ui_account,
                });
            }

            if config.with_context.unwrap_or(false) {
                Ok(OptionalContext::Context(RpcResponse {
                    context: RpcResponseContext::new(current_slot),
                    value: results,
                }))
            } else {
                Ok(OptionalContext::NoContext(results))
            }
        })
    }

    fn get_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>> {
        not_implemented_err_async("get_largest_accounts")
    }

    fn get_supply(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            svm_locker.with_svm_reader(|svm_reader| {
                let slot = svm_reader.get_latest_absolute_slot();
                Ok(RpcResponse {
                    context: RpcResponseContext::new(slot),
                    value: RpcSupply {
                        total: 1,
                        circulating: 0,
                        non_circulating: 0,
                        non_circulating_accounts: vec![],
                    },
                })
            })
        })
    }

    fn get_token_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _mint_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>> {
        not_implemented_err_async("get_token_largest_accounts")
    }

    fn get_token_accounts_by_owner(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        let config = config.unwrap_or_default();
        let owner = match verify_pubkey(&owner_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(config.commitment.unwrap_or_default()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            match token_account_filter {
                RpcTokenAccountsFilter::Mint(mint) => {
                    let mint = verify_pubkey(&mint)?;

                    let associated_token_address = get_associated_token_address_with_program_id(
                        &owner,
                        &mint,
                        &spl_token::id(),
                    );
                    let SvmAccessContext {
                        slot,
                        inner: account_update,
                        ..
                    } = svm_locker
                        .get_account(&remote_ctx, &associated_token_address, None)
                        .await?;

                    svm_locker.write_account_update(account_update.clone());

                    let token_account = account_update.map_account()?;

                    let _ = TokenAccount::unpack(&token_account.data).map_err(|e| {
                        JsonRpcCoreError::invalid_params(format!(
                            "Failed to unpack token account data: {}",
                            e
                        ))
                    })?;

                    Ok(RpcResponse {
                        context: RpcResponseContext::new(slot),
                        value: vec![RpcKeyedAccount {
                            pubkey: associated_token_address.to_string(),
                            account: encode_ui_account(
                                &associated_token_address,
                                &token_account,
                                config.encoding.unwrap_or(UiAccountEncoding::Base64),
                                None,
                                config.data_slice,
                            ),
                        }],
                    })
                }
                RpcTokenAccountsFilter::ProgramId(program_id) => {
                    let program_id = verify_pubkey(&program_id)?;

                    let remote_ctx = remote_ctx.map(|(r, _)| r);
                    let SvmAccessContext {
                        slot,
                        inner: (keyed_accounts, missing_pubkeys),
                        ..
                    } = svm_locker
                        .get_all_token_accounts(&remote_ctx, owner, program_id)
                        .await?;

                    Ok(RpcResponse {
                        context: RpcResponseContext::new(slot),
                        value: keyed_accounts,
                    })
                }
            }
        })
    }

    fn get_token_accounts_by_delegate(
        &self,
        _meta: Self::Metadata,
        _delegate_str: String,
        _token_account_filter: RpcTokenAccountsFilter,
        _config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        not_implemented_err_async("get_token_accounts_by_delegate")
    }
}

// Helper function to apply filters
fn apply_rpc_filters(
    account_data: &[u8],
    filters: &[RpcFilterType],
) -> std::result::Result<bool, JsonRpcCoreError> {
    for filter in filters {
        match filter {
            RpcFilterType::DataSize(size) => {
                if account_data.len() as u64 != *size {
                    return Ok(false);
                }
            }
            RpcFilterType::Memcmp(memcmp_filter) => {
                // Use the public bytes_match method from solana_client::rpc_filter::Memcmp
                if !memcmp_filter.bytes_match(account_data) {
                    return Ok(false); // Content mismatch or out of bounds handled by bytes_match
                }
            }
            RpcFilterType::TokenAccountState => {
                return Err(JsonRpcCoreError {
                    code: ErrorCode::InternalError,
                    message: "TokenAccountState filter is not yet implemented".to_string(),
                    data: None,
                });
            }
        }
    }
    Ok(true)
}
