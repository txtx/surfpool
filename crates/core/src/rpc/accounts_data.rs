use jsonrpc_core::{BoxFuture, Result};
use jsonrpc_derive::rpc;
use solana_account_decoder::{
    parse_account_data::SplTokenAdditionalDataV2,
    parse_token::{parse_token_v3, TokenAccountType, UiTokenAmount},
    UiAccount,
};
use solana_client::{
    rpc_config::RpcAccountInfoConfig,
    rpc_response::{RpcBlockCommitment, RpcResponseContext},
};
use solana_clock::Slot;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_runtime::commitment::BlockCommitmentArray;
use solana_sdk::program_pack::Pack;
use spl_token::state::{Account as TokenAccount, Mint};

use super::{not_implemented_err, RunloopContext, SurfnetRpcContext};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::{utils::verify_pubkey, State},
    surfnet::locker::SvmAccessContext,
};

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
    ) -> BoxFuture<Result<RpcResponse<UiTokenAmount>>>;
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

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(config.commitment.unwrap_or_default()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: account_update,
                ..
            } = svm_locker.get_account(&remote_ctx, &pubkey, None).await?;

            svm_locker.write_account_update(account_update.clone());

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: account_update.try_into_ui_account(config.encoding, config.data_slice),
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

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(config.commitment.unwrap_or_default()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: account_updates,
                ..
            } = svm_locker
                .get_multiple_accounts(&remote_ctx, &pubkeys, None)
                .await?;

            svm_locker.write_multiple_account_updates(&account_updates);

            let mut ui_accounts = vec![];
            {
                for account_update in account_updates.into_iter() {
                    ui_accounts.push(
                        account_update.try_into_ui_account(config.encoding, config.data_slice),
                    );
                }
            }

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: ui_accounts,
            })
        })
    }

    fn get_block_commitment(
        &self,
        meta: Self::Metadata,
        block: Slot,
    ) -> Result<RpcBlockCommitment<BlockCommitmentArray>> {
        // get the info we need and free up lock before validation
        let (current_slot, block_exists) = meta
            .with_svm_reader(|svm_reader| {
                (
                    svm_reader.get_latest_absolute_slot(),
                    svm_reader.blocks.contains_key(&block),
                )
            })
            .map_err(Into::<jsonrpc_core::Error>::into)?;

        // block is valid if it exists in our block history or it's not too far in the future
        if !block_exists && block > current_slot {
            return Err(jsonrpc_core::Error::invalid_params(format!(
                "Block {} not found",
                block
            )));
        }

        let commitment_array = [0u64; 32];

        Ok(RpcBlockCommitment {
            commitment: Some(commitment_array),
            total_stake: 0,
        })
    }

    // SPL Token-specific RPC endpoints
    // See https://github.com/solana-labs/solana-program-library/releases/tag/token-v2.0.0 for
    // program details

    fn get_token_account_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Option<UiTokenAmount>>>> {
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(commitment.unwrap_or_default()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let token_account_result = svm_locker
                .get_account(&remote_ctx, &pubkey, None)
                .await?
                .inner;

            svm_locker.write_account_update(token_account_result.clone());

            let token_account = token_account_result.map_account()?;

            let unpacked_token_account =
                TokenAccount::unpack(&token_account.data).map_err(|e| {
                    SurfpoolError::invalid_account_data(
                        pubkey,
                        "Invalid token account data",
                        Some(e.to_string()),
                    )
                })?;

            let SvmAccessContext {
                slot,
                inner: mint_account_result,
                ..
            } = svm_locker
                .get_account(&remote_ctx, &unpacked_token_account.mint, None)
                .await?;

            svm_locker.write_account_update(mint_account_result.clone());

            let mint_account = mint_account_result.map_account()?;
            let unpacked_mint_account = Mint::unpack(&mint_account.data).map_err(|e| {
                SurfpoolError::invalid_account_data(
                    unpacked_token_account.mint,
                    "Invalid token mint account data",
                    Some(e.to_string()),
                )
            })?;

            let token_decimals = unpacked_mint_account.decimals;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: {
                    parse_token_v3(
                        &token_account.data,
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
        meta: Self::Metadata,
        mint_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<UiTokenAmount>>> {
        let mint_pubkey = match verify_pubkey(&mint_str) {
            Ok(pubkey) => pubkey,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(commitment.unwrap_or_default()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: mint_account_result,
                ..
            } = svm_locker
                .get_account(&remote_ctx, &mint_pubkey, None)
                .await?;

            svm_locker.write_account_update(mint_account_result.clone());

            let mint_account = mint_account_result.map_account()?;

            if !matches!(mint_account.owner, owner if owner == spl_token::id() || owner == spl_token_2022::id())
            {
                return Err(SurfpoolError::invalid_account_data(
                    mint_pubkey,
                    "Account is not owned by the SPL Token program",
                    None::<String>,
                )
                .into());
            }

            let mint_data = Mint::unpack(&mint_account.data).map_err(|e| {
                SurfpoolError::invalid_account_data(
                    mint_pubkey,
                    "Invalid token mint account data",
                    Some(e.to_string()),
                )
            })?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: {
                    parse_token_v3(
                        &mint_account.data,
                        Some(&SplTokenAdditionalDataV2 {
                            decimals: mint_data.decimals,
                            ..Default::default()
                        }),
                    )
                    .ok()
                    .and_then(|t| match t {
                        TokenAccountType::Mint(mint) => {
                            let supply_u64 = mint.supply.parse::<u64>().unwrap_or(0);
                            let ui_amount = if supply_u64 == 0 {
                                Some(0.0)
                            } else {
                                let divisor = 10_u64.pow(mint.decimals as u32);
                                Some(supply_u64 as f64 / divisor as f64)
                            };

                            Some(UiTokenAmount {
                                amount: mint.supply.clone(),
                                decimals: mint.decimals,
                                ui_amount,
                                ui_amount_string: mint.supply,
                            })
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        SurfpoolError::invalid_account_data(
                            mint_pubkey,
                            "Failed to parse token mint account",
                            None::<String>,
                        )
                    })?
                },
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use solana_account::Account;
    use solana_pubkey::Pubkey;
    use solana_sdk::{program_option::COption, program_pack::Pack};
    use spl_token::state::{Account as TokenAccount, AccountState, Mint};

    use super::*;
    use crate::{
        surfnet::{remote::SurfnetRemoteClient, GetAccountResult},
        tests::helpers::TestSetup,
    };

    #[ignore = "connection-required"]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_account_balance() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        let mint_pk = Pubkey::new_unique();

        let minimum_rent = setup.context.svm_locker.with_svm_reader(|svm_reader| {
            svm_reader
                .inner
                .minimum_balance_for_rent_exemption(Mint::LEN)
        });

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
            .svm_locker
            .write_account_update(GetAccountResult::FoundAccount(mint_pk, mint_account, true));

        let token_account_pk = Pubkey::new_unique();

        let minimum_rent = setup.context.svm_locker.with_svm_reader(|svm_reader| {
            svm_reader
                .inner
                .minimum_balance_for_rent_exemption(TokenAccount::LEN)
        });

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
            .svm_locker
            .write_account_update(GetAccountResult::FoundAccount(
                token_account_pk,
                token_account,
                true,
            ));

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

    #[test]
    fn test_get_block_commitment_past_slot() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);
        let current_slot = setup.context.svm_locker.get_latest_absolute_slot();
        let past_slot = if current_slot > 10 {
            current_slot - 10
        } else {
            0
        };

        let result = setup
            .rpc
            .get_block_commitment(Some(setup.context), past_slot)
            .unwrap();

        // Should return commitment data for past slot
        assert!(result.commitment.is_some());
        assert_eq!(result.total_stake, 0);
    }

    #[test]
    fn test_get_block_commitment_with_actual_block() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        // create a block in the SVM's block history
        let test_slot = 12345;
        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            use crate::surfnet::BlockHeader;

            svm_writer.blocks.insert(
                test_slot,
                BlockHeader {
                    hash: "test_hash".to_string(),
                    previous_blockhash: "prev_hash".to_string(),
                    parent_slot: test_slot - 1,
                    block_time: chrono::Utc::now().timestamp_millis(),
                    block_height: test_slot,
                    signatures: vec![],
                },
            );
        });

        let result = setup
            .rpc
            .get_block_commitment(Some(setup.context), test_slot)
            .unwrap();

        // should return commitment data for the existing block
        assert!(result.commitment.is_some());
        assert_eq!(result.total_stake, 0);
    }

    #[test]
    fn test_get_block_commitment_no_metadata() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        let result = setup.rpc.get_block_commitment(None, 123);

        assert!(result.is_err());
        // This should fail because meta is None, triggering the SurfpoolError::no_locker() path
    }

    #[test]
    fn test_get_block_commitment_future_slot_error() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);
        let current_slot = setup.context.svm_locker.get_latest_absolute_slot();
        let future_slot = current_slot + 1000;

        let result = setup
            .rpc
            .get_block_commitment(Some(setup.context), future_slot);

        // Should return an error for future slots
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.code, jsonrpc_core::ErrorCode::InvalidParams);
        assert!(error.message.contains("Block") && error.message.contains("not found"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_supply_with_real_mint() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        let mint_pubkey = Pubkey::new_unique();

        // Create mint account data
        let mut mint_data = [0u8; Mint::LEN];
        let mint = Mint {
            mint_authority: COption::Some(Pubkey::new_unique()),
            supply: 1_000_000_000_000,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        Mint::pack(mint, &mut mint_data).unwrap();

        let mint_account = Account {
            lamports: setup.context.svm_locker.with_svm_reader(|svm_reader| {
                svm_reader
                    .inner
                    .minimum_balance_for_rent_exemption(Mint::LEN)
            }),
            data: mint_data.to_vec(),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            svm_writer
                .set_account(&mint_pubkey, mint_account.clone())
                .unwrap();
        });

        let res = setup
            .rpc
            .get_token_supply(
                Some(setup.context),
                mint_pubkey.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await
            .unwrap();

        assert_eq!(res.value.amount, "1000000000000");
        assert_eq!(res.value.decimals, 6);
        assert_eq!(res.value.ui_amount_string, "1000000000000");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_token_supply_caches_local_account() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        // USDC mint pubkey
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let mint_pubkey = Pubkey::from_str_const(usdc_mint);

        // verify account is not cached initially
        let account_before = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.inner.get_account(&mint_pubkey));
        assert!(
            account_before.is_none(),
            "Account should not be cached initially"
        );
        println!("Confirmed: USDC mint not in local cache initially");

        // create dummy mint account data
        let minimum_rent = setup.context.svm_locker.with_svm_reader(|svm_reader| {
            svm_reader
                .inner
                .minimum_balance_for_rent_exemption(Mint::LEN)
        });

        let mut mint_data = [0; Mint::LEN];
        let dummy_mint = Mint {
            mint_authority: COption::None,
            supply: 41_000_000_000_000_000,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        dummy_mint.pack_into_slice(&mut mint_data);

        let mint_account = Account {
            lamports: minimum_rent,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
            data: mint_data.to_vec(),
        };

        // write the dummy account to local storage
        setup
            .context
            .svm_locker
            .write_account_update(GetAccountResult::FoundAccount(
                mint_pubkey,
                mint_account.clone(),
                true,
            ));

        println!("created dummy USDC mint account locally");

        let start_time = std::time::Instant::now();
        let res = setup
            .rpc
            .get_token_supply(
                Some(setup.context.clone()),
                usdc_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await
            .unwrap();
        let first_call_duration = start_time.elapsed();

        println!("First call completed in {:?}", first_call_duration);
        println!(
            "Supply: {}, Decimals: {}",
            res.value.amount, res.value.decimals
        );

        // verify account is cached
        let account_after = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.inner.get_account(&mint_pubkey));
        assert!(
            account_after.is_some(),
            "Account should be available after first call"
        );
        println!("Confirmed: USDC mint available locally");

        let cached_account = account_after.unwrap();
        assert_eq!(
            cached_account.owner,
            spl_token::id(),
            "Account should be owned by SPL Token program"
        );

        // verify we can unpack the account data
        let cached_mint = Mint::unpack(&cached_account.data).unwrap();
        assert_eq!(cached_mint.decimals, 6, "Mint should have 6 decimals");
        assert_eq!(
            cached_mint.supply, 41_000_000_000_000_000,
            "Mint should have expected supply"
        );
        assert!(cached_mint.is_initialized, "Mint should be initialized");
        println!(
            "Account data is valid: supply={}, decimals={}",
            cached_mint.supply, cached_mint.decimals
        );

        // second call - should be fast since it's already in memory
        let start_time = std::time::Instant::now();
        let res2 = setup
            .rpc
            .get_token_supply(
                Some(setup.context.clone()),
                usdc_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await
            .unwrap();
        let second_call_duration = start_time.elapsed();

        println!("Second call completed in {:?}", second_call_duration);

        // verify both calls return same data
        assert_eq!(
            res.value.amount, res2.value.amount,
            "Both calls should return same supply"
        );
        assert_eq!(
            res.value.decimals, res2.value.decimals,
            "Both calls should return same decimals"
        );

        // both calls should be fast since no network is involved
        assert!(
            first_call_duration < std::time::Duration::from_millis(100),
            "First call should be fast (local), took {:?}",
            first_call_duration
        );
        assert!(
            second_call_duration < std::time::Duration::from_millis(100),
            "Second call should be fast (local), took {:?}",
            second_call_duration
        );

        println!("Local account test passed!");
        println!("Both calls used local data");
        println!(
            "First call: {:?}, Second call: {:?}",
            first_call_duration, second_call_duration
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalid_pubkey_format() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        // test various invalid pubkey formats
        let invalid_pubkeys = vec![
            "",
            "invalid",
            "123",
            "not-a-valid-base58-string!@#$",
            "11111111111111111111111111111111111111111111111111111111111111111",
            "invalid-base58-characters-ö",
        ];

        for invalid_pubkey in invalid_pubkeys {
            let res = setup
                .rpc
                .get_token_supply(
                    Some(setup.context.clone()),
                    invalid_pubkey.to_string(),
                    Some(CommitmentConfig::confirmed()),
                )
                .await;

            assert!(
                res.is_err(),
                "Should fail for invalid pubkey: '{}'",
                invalid_pubkey
            );

            let error_msg = res.unwrap_err().to_string();
            assert!(
                error_msg.contains("Invalid") || error_msg.contains("invalid"),
                "Error should mention invalidity for '{}': {}",
                invalid_pubkey,
                error_msg
            );
        }

        println!("✅ All invalid pubkey formats correctly rejected");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nonexistent_account() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        // valid pubkey format but nonexistent account
        let nonexistent_mint = Pubkey::new_unique();

        let res = setup
            .rpc
            .get_token_supply(
                Some(setup.context),
                nonexistent_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await;

        assert!(res.is_err(), "Should fail for nonexistent account");

        let error_msg = res.unwrap_err().to_string();
        assert!(
            error_msg.contains("not found") || error_msg.contains("account"),
            "Error should mention account not found: {}",
            error_msg
        );

        println!("✅ Nonexistent account correctly rejected: {}", error_msg);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalid_mint_data() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        let fake_mint = Pubkey::new_unique();

        setup.context.svm_locker.with_svm_writer(|svm_writer| {
            // create an account owned by SPL Token but with invalid data
            let invalid_mint_account = Account {
                lamports: 1000000,
                data: vec![0xFF; 50], // invalid mint data (random bytes)
                owner: spl_token::id(),
                executable: false,
                rent_epoch: 0,
            };

            svm_writer
                .set_account(&fake_mint, invalid_mint_account)
                .unwrap();
        });

        let res = setup
            .rpc
            .get_token_supply(
                Some(setup.context),
                fake_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await;

        assert!(
            res.is_err(),
            "Should fail for account with invalid mint data"
        );

        let error_msg = res.unwrap_err().to_string();
        assert!(
            error_msg.contains("deserialize")
                || error_msg.contains("Invalid")
                || error_msg.contains("parse"),
            "Error should mention deserialization failure: {}",
            error_msg
        );

        println!("✅ Invalid mint data correctly rejected: {}", error_msg);
    }

    #[ignore = "requires-network"]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_remote_rpc_failure() {
        // test with invalid remote RPC URL
        let bad_remote_client =
            SurfnetRemoteClient::new("https://invalid-url-that-doesnt-exist.com");
        let mut setup = TestSetup::new(SurfpoolAccountsDataRpc);
        setup.context.remote_rpc_client = Some(bad_remote_client);

        let nonexistent_mint = Pubkey::new_unique();

        let res = setup
            .rpc
            .get_token_supply(
                Some(setup.context),
                nonexistent_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await;

        assert!(res.is_err(), "Should fail when remote RPC is unreachable");

        let error_msg = res.unwrap_err().to_string();
        println!("✅ Remote RPC failure handled: {}", error_msg);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_context_metadata() {
        let setup = TestSetup::new(SurfpoolAccountsDataRpc);

        // test with None metadata (no context)
        let valid_mint = Pubkey::new_unique();

        let res = setup
            .rpc
            .get_token_supply(
                None, // no context
                valid_mint.to_string(),
                Some(CommitmentConfig::confirmed()),
            )
            .await;

        assert!(res.is_err(), "Should fail when no context provided");
    }
}
