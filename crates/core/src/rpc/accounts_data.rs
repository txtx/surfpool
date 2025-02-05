use std::sync::Arc;

use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;

use jsonrpc_core::futures::future::{self, join_all};
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_account_decoder::{encode_ui_account, UiAccount, UiAccountEncoding};
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcContextConfig};
use solana_client::rpc_response::RpcBlockCommitment;
use solana_client::rpc_response::RpcResponseContext;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_runtime::commitment::BlockCommitmentArray;
use solana_sdk::{clock::Slot, commitment_config::CommitmentConfig};

use super::RunloopContext;

#[rpc]
pub trait AccountsData {
    type Metadata;

    #[rpc(meta, name = "getAccountInfo")]
    fn get_account_info(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Option<UiAccount>>>>;

    #[rpc(meta, name = "getBalance")]
    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<RpcResponse<u64>>>;

    #[rpc(meta, name = "getBlockCommitment")]
    fn get_block_commitment(
        &self,
        meta: Self::Metadata,
        block: Slot,
    ) -> Result<RpcBlockCommitment<BlockCommitmentArray>>;

    #[rpc(meta, name = "getMultipleAccounts")]
    fn get_multiple_accounts(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Vec<String>,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<UiAccount>>>>>;

    // SPL Token-specific RPC endpoints
    // See https://github.com/solana-labs/solana-program-library/releases/tag/token-v2.0.0 for
    // program details

    #[rpc(meta, name = "getTokenAccountBalance")]
    fn get_token_account_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>>;

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
        let rpc_client = state_reader.rpc_client.clone();
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

    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<RpcResponse<u64>>> {
        let absolute_slot = match meta.get_state() {
            Ok(s) => s.epoch_info.absolute_slot,
            Err(e) => return Box::pin(future::err(e.into())),
        };
        let account =
            self.get_account_info(meta, pubkey_str, Some(RpcAccountInfoConfig::default()));

        Box::pin(async move {
            let lamports = account.await?.value.map_or(0, |account| account.lamports);

            Ok(RpcResponse {
                context: RpcResponseContext::new(absolute_slot),
                value: lamports,
            })
        })
    }

    fn get_multiple_accounts(
        &self,
        meta: Self::Metadata,
        pubkeys: Vec<String>,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<UiAccount>>>>> {
        let config = config.unwrap_or_default();
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let accounts = match pubkeys
            .iter()
            .map(|s| {
                let pk = verify_pubkey(s)?;
                Ok((pk, state_reader.svm.get_account(&pk)))
            })
            .collect::<Result<Vec<_>>>()
        {
            Ok(accs) => accs,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        // Drop the lock on the state while we fetch accounts
        let absolute_slot = state_reader.epoch_info.absolute_slot;
        let rpc_client = state_reader.rpc_client.clone();
        let encoding = config.encoding.clone();
        let data_slice = config.data_slice.clone();
        drop(state_reader);

        Box::pin(async move {
            // Fetch all accounts at once and then only use those that are missing
            let fetched_accounts = join_all(accounts.iter().map(|(pk, _)| {
                let rpc_client = Arc::clone(&rpc_client);

                async move { (pk, rpc_client.get_account(&pk).await.ok()) }
            }))
            .await;

            // Save missing fetched accounts
            let mut state_reader = meta.get_state_mut()?;
            let combined_accounts = accounts
                .iter()
                .zip(fetched_accounts)
                .map(|((pk, local), (_, fetched_account))| {
                    if local.is_some() {
                        Ok((pk, local.clone()))
                    } else if let Some(account) = fetched_account {
                        state_reader
                            .svm
                            .set_account(*pk, account.clone())
                            .map_err(|err| {
                                Error::invalid_params(format!(
                                    "failed to save fetched account {pk:?}: {err:?}"
                                ))
                            })?;
                        Ok((pk, Some(account)))
                    } else {
                        Ok((pk, None))
                    }
                })
                .collect::<Result<Vec<_>>>()?;

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
        unimplemented!()
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
        unimplemented!()
    }

    fn get_token_supply(
        &self,
        _meta: Self::Metadata,
        _mint_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<RpcResponse<UiTokenAmount>> {
        unimplemented!()
    }
}
