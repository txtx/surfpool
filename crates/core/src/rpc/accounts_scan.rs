use jsonrpc_core::{BoxFuture, Error as JsonRpcCoreError, ErrorCode, Result};
use jsonrpc_derive::rpc;
use solana_client::{
    rpc_config::{
        RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig, RpcSupplyConfig,
        RpcTokenAccountsFilter,
    },
    rpc_request::TokenAccountsFilter,
    rpc_response::{
        OptionalContext, RpcAccountBalance, RpcKeyedAccount, RpcResponseContext, RpcSupply,
        RpcTokenAccountBalance,
    },
};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::response::Response as RpcResponse;

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

#[derive(Clone)]
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

            // Get program-owned accounts from the account registry
            let program_accounts = svm_locker
                .get_program_accounts(
                    &remote_ctx.map(|(client, _)| client),
                    &program_id,
                    account_config,
                    config.filters,
                )
                .await?
                .inner;

            if config.with_context.unwrap_or(false) {
                Ok(OptionalContext::Context(RpcResponse {
                    context: RpcResponseContext::new(current_slot),
                    value: program_accounts,
                }))
            } else {
                Ok(OptionalContext::NoContext(program_accounts))
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
        config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            svm_locker.with_svm_reader(|svm_reader| {
                let slot = svm_reader.get_latest_absolute_slot();

                // Check if we should exclude non-circulating accounts list
                let exclude_accounts = config
                    .as_ref()
                    .map(|c| c.exclude_non_circulating_accounts_list)
                    .unwrap_or(false);

                Ok(RpcResponse {
                    context: RpcResponseContext::new(slot),
                    value: RpcSupply {
                        total: svm_reader.total_supply,
                        circulating: svm_reader.circulating_supply,
                        non_circulating: svm_reader.non_circulating_supply,
                        non_circulating_accounts: if exclude_accounts {
                            vec![]
                        } else {
                            svm_reader.non_circulating_accounts.clone()
                        },
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

        let filter = match token_account_filter {
            RpcTokenAccountsFilter::Mint(mint_str) => {
                let mint = match verify_pubkey(&mint_str) {
                    Ok(res) => res,
                    Err(e) => return e.into(),
                };
                TokenAccountsFilter::Mint(mint)
            }
            RpcTokenAccountsFilter::ProgramId(program_id_str) => {
                let program_id = match verify_pubkey(&program_id_str) {
                    Ok(res) => res,
                    Err(e) => return e.into(),
                };
                TokenAccountsFilter::ProgramId(program_id)
            }
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: token_accounts,
                ..
            } = svm_locker
                .get_token_accounts_by_owner(
                    &remote_ctx.map(|(client, _)| client),
                    owner,
                    &filter,
                    &config,
                )
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: token_accounts,
            })
        })
    }

    fn get_token_accounts_by_delegate(
        &self,
        meta: Self::Metadata,
        delegate_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        let config = config.unwrap_or_default();
        let delegate = match verify_pubkey(&delegate_str) {
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
                RpcTokenAccountsFilter::Mint(mint_str) => {
                    let mint = verify_pubkey(&mint_str)?;

                    let remote_ctx = remote_ctx.map(|(r, _)| r);
                    let SvmAccessContext {
                        slot,
                        inner: keyed_accounts,
                        ..
                    } = svm_locker
                        .get_token_accounts_by_delegate(
                            &remote_ctx,
                            delegate,
                            &TokenAccountsFilter::Mint(mint),
                            &config,
                        )
                        .await?;

                    Ok(RpcResponse {
                        context: RpcResponseContext::new(slot),
                        value: keyed_accounts,
                    })
                }
                RpcTokenAccountsFilter::ProgramId(program_id_str) => {
                    let program_id = verify_pubkey(&program_id_str)?;

                    let remote_ctx = remote_ctx.map(|(r, _)| r);
                    let SvmAccessContext {
                        slot,
                        inner: keyed_accounts,
                        ..
                    } = svm_locker
                        .get_token_accounts_by_delegate(
                            &remote_ctx,
                            delegate,
                            &TokenAccountsFilter::ProgramId(program_id),
                            &config,
                        )
                        .await?;

                    Ok(RpcResponse {
                        context: RpcResponseContext::new(slot),
                        value: keyed_accounts,
                    })
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {

    use core::panic;

    use solana_account::Account;
    use solana_client::{
        rpc_config::{RpcProgramAccountsConfig, RpcSupplyConfig, RpcTokenAccountsFilter},
        rpc_filter::{Memcmp, RpcFilterType},
        rpc_response::OptionalContext,
    };
    use solana_pubkey::Pubkey;
    use solana_sdk::program_pack::Pack;
    use surfpool_types::SupplyUpdate;

    use super::{AccountsScan, SurfpoolAccountsScanRpc};
    use crate::{
        rpc::surfnet_cheatcodes::{SurfnetCheatcodesRpc, SvmTricksRpc},
        tests::helpers::TestSetup,
    };

    const VALID_PUBKEY_1: &str = "11111111111111111111111111111112";

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_program_accounts() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);

        // The owner program id that owns the accounts we will query
        let owner_pubkey = Pubkey::new_unique();
        // The owned accounts with different data sizes to filter by
        let owned_pubkey_short_data = Pubkey::new_unique();
        let owner_pubkey_long_data = Pubkey::new_unique();
        // Another account that is not owned by the owner program
        let other_pubkey = Pubkey::new_unique();

        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            svm_writer
                .set_account(
                    &owned_pubkey_short_data,
                    Account {
                        lamports: 1000,
                        data: vec![4, 5, 6],
                        owner: owner_pubkey,
                        executable: false,
                        rent_epoch: 0,
                    },
                )
                .unwrap();

            svm_writer
                .set_account(
                    &owner_pubkey_long_data,
                    Account {
                        lamports: 2000,
                        data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                        owner: owner_pubkey,
                        executable: false,
                        rent_epoch: 0,
                    },
                )
                .unwrap();

            svm_writer
                .set_account(
                    &other_pubkey,
                    Account {
                        lamports: 500,
                        data: vec![4, 5, 6],
                        owner: Pubkey::new_unique(),
                        executable: false,
                        rent_epoch: 0,
                    },
                )
                .unwrap();
        });

        // Test with no filters
        {
            let res = setup
                .rpc
                .get_program_accounts(Some(setup.context.clone()), owner_pubkey.to_string(), None)
                .await
                .expect("Failed to get program accounts");
            match res {
                OptionalContext::Context(_) => {
                    panic!("Expected no context");
                }
                OptionalContext::NoContext(value) => {
                    assert_eq!(value.len(), 2);

                    let short_data_account = value
                        .iter()
                        .find(|acc| acc.pubkey == owned_pubkey_short_data.to_string())
                        .expect("Short data account not found");
                    assert_eq!(short_data_account.account.lamports, 1000);

                    let long_data_account = value
                        .iter()
                        .find(|acc| acc.pubkey == owner_pubkey_long_data.to_string())
                        .expect("Long data account not found");
                    assert_eq!(long_data_account.account.lamports, 2000);
                }
            }
        }

        // Test with data size filter
        {
            let res = setup
                .rpc
                .get_program_accounts(
                    Some(setup.context.clone()),
                    owner_pubkey.to_string(),
                    Some(RpcProgramAccountsConfig {
                        filters: Some(vec![RpcFilterType::DataSize(3)]),
                        with_context: Some(true),
                        ..Default::default()
                    }),
                )
                .await
                .expect("Failed to get program accounts with data size filter");

            match res {
                OptionalContext::Context(response) => {
                    assert_eq!(response.value.len(), 1);

                    let short_data_account = response
                        .value
                        .iter()
                        .find(|acc| acc.pubkey == owned_pubkey_short_data.to_string())
                        .expect("Short data account not found");
                    assert_eq!(short_data_account.account.lamports, 1000);
                }
                OptionalContext::NoContext(_) => {
                    panic!("Expected context");
                }
            }
        }

        // Test with memcmp filter
        {
            let res = setup
                .rpc
                .get_program_accounts(
                    Some(setup.context.clone()),
                    owner_pubkey.to_string(),
                    Some(RpcProgramAccountsConfig {
                        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                            1,
                            vec![5, 6],
                        ))]),
                        with_context: Some(false),
                        ..Default::default()
                    }),
                )
                .await
                .expect("Failed to get program accounts with memcmp filter");

            match res {
                OptionalContext::Context(_) => {
                    panic!("Expected no context");
                }
                OptionalContext::NoContext(value) => {
                    assert_eq!(value.len(), 1);

                    let short_data_account = value
                        .iter()
                        .find(|acc| acc.pubkey == owned_pubkey_short_data.to_string())
                        .expect("Short data account not found");
                    assert_eq!(short_data_account.account.lamports, 1000);
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_and_get_supply() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        // test initial default values
        let initial_supply = setup
            .rpc
            .get_supply(Some(setup.context.clone()), None)
            .await
            .unwrap();

        assert_eq!(initial_supply.value.total, 0);
        assert_eq!(initial_supply.value.circulating, 0);
        assert_eq!(initial_supply.value.non_circulating, 0);
        assert_eq!(initial_supply.value.non_circulating_accounts.len(), 0);

        // set supply values using cheatcode
        let supply_update = SupplyUpdate {
            total: Some(1_000_000_000_000_000),
            circulating: Some(800_000_000_000_000),
            non_circulating: Some(200_000_000_000_000),
            non_circulating_accounts: Some(vec![
                VALID_PUBKEY_1.to_string(),
                VALID_PUBKEY_1.to_string(),
            ]),
        };

        let set_result = cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), supply_update)
            .await
            .unwrap();

        assert_eq!(set_result.value, ());

        // verify the values are returned by getSupply
        let supply = setup
            .rpc
            .get_supply(Some(setup.context.clone()), None)
            .await
            .unwrap();

        assert_eq!(supply.value.total, 1_000_000_000_000_000);
        assert_eq!(supply.value.circulating, 800_000_000_000_000);
        assert_eq!(supply.value.non_circulating, 200_000_000_000_000);
        assert_eq!(supply.value.non_circulating_accounts.len(), 2);
        assert_eq!(supply.value.non_circulating_accounts[0], VALID_PUBKEY_1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_supply_exclude_accounts() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        // set supply with non-circulating accounts
        let supply_update = SupplyUpdate {
            total: Some(1_000_000_000_000_000),
            circulating: Some(800_000_000_000_000),
            non_circulating: Some(200_000_000_000_000),
            non_circulating_accounts: Some(vec![
                VALID_PUBKEY_1.to_string(),
                VALID_PUBKEY_1.to_string(),
            ]),
        };

        cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), supply_update)
            .await
            .unwrap();

        // test with exclude_non_circulating_accounts_list = true
        let config_exclude = RpcSupplyConfig {
            commitment: None,
            exclude_non_circulating_accounts_list: true,
        };

        let supply_excluded = setup
            .rpc
            .get_supply(Some(setup.context.clone()), Some(config_exclude))
            .await
            .unwrap();

        assert_eq!(supply_excluded.value.total, 1_000_000_000_000_000);
        assert_eq!(supply_excluded.value.circulating, 800_000_000_000_000);
        assert_eq!(supply_excluded.value.non_circulating, 200_000_000_000_000);
        assert_eq!(supply_excluded.value.non_circulating_accounts.len(), 0); // should be empty

        // test with exclude_non_circulating_accounts_list = false (default)
        let supply_included = setup
            .rpc
            .get_supply(Some(setup.context.clone()), None)
            .await
            .unwrap();

        assert_eq!(supply_included.value.non_circulating_accounts.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_partial_supply_update() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        // set initial values
        let initial_update = SupplyUpdate {
            total: Some(1_000_000_000_000_000),
            circulating: Some(800_000_000_000_000),
            non_circulating: Some(200_000_000_000_000),
            non_circulating_accounts: Some(vec![VALID_PUBKEY_1.to_string()]),
        };

        cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), initial_update)
            .await
            .unwrap();

        // update only the total supply
        let partial_update = SupplyUpdate {
            total: Some(2_000_000_000_000_000),
            circulating: None,
            non_circulating: None,
            non_circulating_accounts: None,
        };

        cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), partial_update)
            .await
            .unwrap();

        // verify only total was updated, others remain the same
        let supply = setup
            .rpc
            .get_supply(Some(setup.context), None)
            .await
            .unwrap();

        assert_eq!(supply.value.total, 2_000_000_000_000_000); // updated
        assert_eq!(supply.value.circulating, 800_000_000_000_000);
        assert_eq!(supply.value.non_circulating, 200_000_000_000_000);
        assert_eq!(supply.value.non_circulating_accounts.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_supply_with_multiple_invalid_pubkeys() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        // test with multiple invalid pubkeys - should fail on the first one
        let supply_update = SupplyUpdate {
            total: Some(1_000_000_000_000_000),
            circulating: Some(800_000_000_000_000),
            non_circulating: Some(200_000_000_000_000),
            non_circulating_accounts: Some(vec![
                VALID_PUBKEY_1.to_string(),   // Valid
                "invalid_pubkey".to_string(), // Invalid - should fail here
                "also_invalid".to_string(),   // Also invalid but won't reach here
            ]),
        };

        let result = cheatcodes_rpc
            .set_supply(Some(setup.context), supply_update)
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, jsonrpc_core::ErrorCode::InvalidParams);
        assert!(error.message.contains("Invalid pubkey at index 1"));
        assert!(error.message.contains("invalid_pubkey"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_supply_with_max_values() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        let supply_update = SupplyUpdate {
            total: Some(u64::MAX),
            circulating: Some(u64::MAX - 1),
            non_circulating: Some(1),
            non_circulating_accounts: Some(vec![VALID_PUBKEY_1.to_string()]),
        };

        let result = cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), supply_update)
            .await;

        assert!(result.is_ok());

        let supply = setup
            .rpc
            .get_supply(Some(setup.context), None)
            .await
            .unwrap();

        assert_eq!(supply.value.total, u64::MAX);
        assert_eq!(supply.value.circulating, u64::MAX - 1);
        assert_eq!(supply.value.non_circulating, 1);
        assert_eq!(supply.value.non_circulating_accounts[0], VALID_PUBKEY_1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_supply_large_valid_account_list() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let cheatcodes_rpc = SurfnetCheatcodesRpc;

        let large_account_list: Vec<String> = (0..100)
            .map(|i| match i % 10 {
                0 => "3rSZJHysEk2ueFVovRLtZ8LGnQBMZGg96H2Q4jErspAF".to_string(),
                1 => "11111111111111111111111111111111".to_string(),
                2 => "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                3 => "So11111111111111111111111111111111111111112".to_string(),
                4 => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                5 => "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
                6 => "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".to_string(),
                7 => "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E".to_string(),
                8 => "2FPyTwcZLUg1MDrwsyoP4D6s1tM7hAkHYRjkNb5w6Pxk".to_string(),
                _ => "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(),
            })
            .collect();

        let supply_update = SupplyUpdate {
            total: Some(1_000_000_000_000_000),
            circulating: Some(800_000_000_000_000),
            non_circulating: Some(200_000_000_000_000),
            non_circulating_accounts: Some(large_account_list.clone()),
        };

        let result = cheatcodes_rpc
            .set_supply(Some(setup.context.clone()), supply_update)
            .await;

        assert!(result.is_ok());

        let supply = setup
            .rpc
            .get_supply(Some(setup.context), None)
            .await
            .unwrap();

        assert_eq!(supply.value.non_circulating_accounts.len(), 100);
        assert_eq!(
            supply.value.non_circulating_accounts[0],
            "3rSZJHysEk2ueFVovRLtZ8LGnQBMZGg96H2Q4jErspAF"
        );
        assert_eq!(
            supply.value.non_circulating_accounts[1],
            "11111111111111111111111111111111"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_supply_with_invalid_config() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);

        let config = RpcSupplyConfig {
            commitment: Some(solana_commitment_config::CommitmentConfig {
                commitment: solana_commitment_config::CommitmentLevel::Processed,
            }),
            exclude_non_circulating_accounts_list: false,
        };

        let result = setup
            .rpc
            .get_supply(Some(setup.context), Some(config))
            .await;

        assert!(result.is_ok());
        let supply = result.unwrap();
        assert_eq!(supply.value.total, 0);
        assert_eq!(supply.value.circulating, 0);
        assert_eq!(supply.value.non_circulating, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_accounts_by_delegate() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);

        let delegate = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let token_account_pubkey = Pubkey::new_unique();
        let token_program = spl_token::id();

        // create a token account with delegate
        let mut token_account_data = [0u8; spl_token::state::Account::LEN];
        let token_account = spl_token::state::Account {
            mint,
            owner,
            amount: 1000,
            delegate: solana_sdk::program_option::COption::Some(delegate),
            state: spl_token::state::AccountState::Initialized,
            is_native: solana_sdk::program_option::COption::None,
            delegated_amount: 500,
            close_authority: solana_sdk::program_option::COption::None,
        };
        solana_sdk::program_pack::Pack::pack_into_slice(&token_account, &mut token_account_data);

        let account = Account {
            lamports: 1000000,
            data: token_account_data.to_vec(),
            owner: token_program,
            executable: false,
            rent_epoch: 0,
        };

        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            svm_writer
                .set_account(&token_account_pubkey, account.clone())
                .unwrap();
        });

        // programId filter - should find the account
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                delegate.to_string(),
                RpcTokenAccountsFilter::ProgramId(token_program.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "ProgramId filter should succeed");
        let response = result.unwrap();
        assert_eq!(response.value.len(), 1, "Should find 1 token account");
        assert_eq!(response.value[0].pubkey, token_account_pubkey.to_string());

        // mint filter - should find the account
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                delegate.to_string(),
                RpcTokenAccountsFilter::Mint(mint.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "Mint filter should succeed");
        let response = result.unwrap();
        assert_eq!(response.value.len(), 1, "Should find 1 token account");
        assert_eq!(response.value[0].pubkey, token_account_pubkey.to_string());

        // non-existent delegate - should return empty
        let non_existent_delegate = Pubkey::new_unique();
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                non_existent_delegate.to_string(),
                RpcTokenAccountsFilter::ProgramId(token_program.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "Non-existent delegate should succeed");
        let response = result.unwrap();
        assert_eq!(response.value.len(), 0, "Should find 0 token accounts");

        // wrong mint - should return empty
        let wrong_mint = Pubkey::new_unique();
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                delegate.to_string(),
                RpcTokenAccountsFilter::Mint(wrong_mint.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "Wrong mint should succeed");
        let response = result.unwrap();
        assert_eq!(response.value.len(), 0, "Should find 0 token accounts");

        // invalid delegate pubkey - should fail
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                "invalid_pubkey".to_string(),
                RpcTokenAccountsFilter::ProgramId(token_program.to_string()),
                None,
            )
            .await;

        assert!(result.is_err(), "Invalid pubkey should fail");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_accounts_by_delegate_multiple_accounts() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);

        let delegate = Pubkey::new_unique();
        let owner1 = Pubkey::new_unique();
        let owner2 = Pubkey::new_unique();
        let mint1 = Pubkey::new_unique();
        let mint2 = Pubkey::new_unique();
        let token_account1 = Pubkey::new_unique();
        let token_account2 = Pubkey::new_unique();
        let token_program = spl_token::id();

        // create first token account with delegate
        let mut token_account_data1 = [0u8; spl_token::state::Account::LEN];
        let token_account_struct1 = spl_token::state::Account {
            mint: mint1,
            owner: owner1,
            amount: 1000,
            delegate: solana_sdk::program_option::COption::Some(delegate),
            state: spl_token::state::AccountState::Initialized,
            is_native: solana_sdk::program_option::COption::None,
            delegated_amount: 500,
            close_authority: solana_sdk::program_option::COption::None,
        };
        solana_sdk::program_pack::Pack::pack_into_slice(
            &token_account_struct1,
            &mut token_account_data1,
        );

        // create second token account with same delegate
        let mut token_account_data2 = [0u8; spl_token::state::Account::LEN];
        let token_account_struct2 = spl_token::state::Account {
            mint: mint2,
            owner: owner2,
            amount: 2000,
            delegate: solana_sdk::program_option::COption::Some(delegate),
            state: spl_token::state::AccountState::Initialized,
            is_native: solana_sdk::program_option::COption::None,
            delegated_amount: 1000,
            close_authority: solana_sdk::program_option::COption::None,
        };
        solana_sdk::program_pack::Pack::pack_into_slice(
            &token_account_struct2,
            &mut token_account_data2,
        );

        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            svm_writer
                .set_account(
                    &token_account1,
                    Account {
                        lamports: 1000000,
                        data: token_account_data1.to_vec(),
                        owner: token_program,
                        executable: false,
                        rent_epoch: 0,
                    },
                )
                .unwrap();

            svm_writer
                .set_account(
                    &token_account2,
                    Account {
                        lamports: 1000000,
                        data: token_account_data2.to_vec(),
                        owner: token_program,
                        executable: false,
                        rent_epoch: 0,
                    },
                )
                .unwrap();
        });

        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                delegate.to_string(),
                RpcTokenAccountsFilter::ProgramId(token_program.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "ProgramId filter should succeed");
        let response = result.unwrap();
        assert_eq!(response.value.len(), 2, "Should find 2 token accounts");

        let returned_pubkeys: std::collections::HashSet<String> = response
            .value
            .iter()
            .map(|acc| acc.pubkey.clone())
            .collect();
        assert!(returned_pubkeys.contains(&token_account1.to_string()));
        assert!(returned_pubkeys.contains(&token_account2.to_string()));

        // Test: Mint filter for mint1 - should find only first account
        let result = setup
            .rpc
            .get_token_accounts_by_delegate(
                Some(setup.context.clone()),
                delegate.to_string(),
                RpcTokenAccountsFilter::Mint(mint1.to_string()),
                None,
            )
            .await;

        assert!(result.is_ok(), "Mint filter should succeed");
        let response = result.unwrap();
        assert_eq!(
            response.value.len(),
            1,
            "Should find 1 token account for mint1"
        );
        assert_eq!(response.value[0].pubkey, token_account1.to_string());
    }
}
