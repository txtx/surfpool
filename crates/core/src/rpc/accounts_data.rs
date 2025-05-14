use crate::error::SurfpoolError;
use crate::error::SurfpoolResult;
use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;
use crate::surfnet::GetAccountStrategy;

use jsonrpc_core::BoxFuture;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_account_decoder::parse_account_data::SplTokenAdditionalDataV2;
use solana_account_decoder::parse_token::parse_token_v3;
use solana_account_decoder::parse_token::TokenAccountType;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_account_decoder::{encode_ui_account, UiAccount, UiAccountEncoding};
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_client::rpc_response::RpcBlockCommitment;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_runtime::commitment::BlockCommitmentArray;
use solana_sdk::program_pack::Pack;
use spl_token::state::Account as TokenAccount;
use spl_token::state::Mint;

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
    ) -> BoxFuture<Result<RpcResponse<Option<UiTokenAmount>>>>;

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

#[derive(Clone)]
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
            Err(e) => return e.into(),
        };

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            let some_account = svm_writer
                .get_account_mut(
                    &pubkey,
                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
                )
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: some_account.map(|account| {
                    encode_ui_account(
                        &pubkey,
                        &account,
                        config.encoding.unwrap_or(UiAccountEncoding::Base64),
                        None,
                        config.data_slice,
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
        let pubkeys = match pubkeys_str
            .iter()
            .map(|s| verify_pubkey(s))
            .collect::<SurfpoolResult<Vec<_>>>()
        {
            Ok(p) => p,
            Err(e) => return e.into(),
        };

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            let accounts = svm_writer
                .get_multiple_accounts_mut(
                    &pubkeys,
                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
                )
                .await?;
            let mut ui_accounts = vec![];
            for (account, pubkey) in accounts.into_iter().zip(pubkeys) {
                let ui_account = match account {
                    None => None,
                    Some(account) => Some(encode_ui_account(
                        &pubkey,
                        &account,
                        config.encoding.unwrap_or(UiAccountEncoding::Base64),
                        None,
                        config.data_slice,
                    )),
                };
                ui_accounts.push(ui_account);
            }

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: ui_accounts,
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
        meta: Self::Metadata,
        pubkey_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Option<UiTokenAmount>>>> {
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            let some_account = svm_writer
                .get_account_mut(
                    &pubkey,
                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
                )
                .await?;

            let (token_account, token_account_slice) = some_account
                .and_then(|account| {
                    let t = TokenAccount::unpack(&account.data).ok()?;
                    Some((t, account.data))
                })
                .ok_or(SurfpoolError::get_account(
                    pubkey,
                    "Error fetching token account",
                ))?;

            let mint_account = svm_writer
                .get_account(
                    &token_account.mint,
                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
                )
                .await?;

            let token_decimals = mint_account
                .and_then(|account| Mint::unpack(&account.data).ok())
                .ok_or(SurfpoolError::get_account(
                    pubkey,
                    "Error fetching token mint account",
                ))?
                .decimals;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: {
                    parse_token_v3(
                        &token_account_slice,
                        Some(&SplTokenAdditionalDataV2 {
                            decimals: token_decimals,
                            ..Default::default()
                        }),
                    )
                    .ok()
                    .and_then(|t| match t {
                        TokenAccountType::Account(account) => Some(account.token_amount),
                        _ => None,
                    })
                },
            })
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::TestSetup;
    use solana_account::Account;
    use solana_pubkey::Pubkey;
    use solana_sdk::program_pack::Pack;
    use spl_token::state::{Account as TokenAccount, AccountState};

    #[ignore = "connection-required"]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_account_balance() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        let mint_pk = Pubkey::new_unique();

        let minimum_rent = setup
            .context
            .surfnet_svm
            .write()
            .await
            .inner
            .minimum_balance_for_rent_exemption(Mint::LEN);

        let mut data = [0; Mint::LEN];

        let default = Mint {
            decimals: 6,
            supply: 1000000000000000,
            is_initialized: true,
            ..Default::default()
        };
        default.pack_into_slice(&mut data);

        let mint_account = Account {
            lamports: minimum_rent,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
            data: data.to_vec(),
        };

        setup
            .context
            .surfnet_svm
            .write()
            .await
            .set_account(&mint_pk, mint_account)
            .unwrap();

        let token_account_pk = Pubkey::new_unique();

        let minimum_rent = setup
            .context
            .surfnet_svm
            .write()
            .await
            .inner
            .minimum_balance_for_rent_exemption(TokenAccount::LEN);

        let mut data = [0; TokenAccount::LEN];

        let default = TokenAccount {
            mint: mint_pk,
            owner: spl_token::ID,
            state: AccountState::Initialized,
            amount: 100 * 1000000,
            ..Default::default()
        };
        default.pack_into_slice(&mut data);

        let token_account = Account {
            lamports: minimum_rent,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
            data: data.to_vec(),
        };

        setup
            .context
            .surfnet_svm
            .write()
            .await
            .set_account(&token_account_pk, token_account)
            .unwrap();

        let res = setup
            .rpc
            .get_token_account_balance(Some(setup.context), token_account_pk.to_string(), None)
            .await
            .unwrap();

        assert_eq!(
            res.value.unwrap(),
            UiTokenAmount {
                amount: String::from("100000000"),
                decimals: 6,
                ui_amount: Some(100.0),
                ui_amount_string: String::from("100")
            }
        );
    }
}
