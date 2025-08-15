use std::str::FromStr;

use serde_json::json;
use solana_account::Account;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_client::GetConfirmedSignaturesForAddress2Config,
    rpc_config::{
        RpcAccountInfoConfig, RpcBlockConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig,
        RpcSignaturesForAddressConfig, RpcTokenAccountsFilter, RpcTransactionConfig,
    },
    rpc_filter::RpcFilterType,
    rpc_request::{RpcRequest, TokenAccountsFilter},
    rpc_response::{
        RpcAccountBalance, RpcConfirmedTransactionStatusWithSignature, RpcKeyedAccount, RpcResult,
        RpcTokenAccountBalance,
    },
};
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_hash::Hash;
use solana_pubkey::Pubkey;
use solana_sdk::bpf_loader_upgradeable::get_program_data_address;
use solana_signature::Signature;
use solana_transaction_status::UiConfirmedBlock;

use super::GetTransactionResult;
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::is_method_not_supported_error,
    surfnet::{GetAccountResult, locker::is_supported_token_program},
    types::{RemoteRpcResult, TokenAccount},
};

pub struct SurfnetRemoteClient {
    pub client: RpcClient,
}
impl Clone for SurfnetRemoteClient {
    fn clone(&self) -> Self {
        let remote_rpc_url = self.client.url();
        SurfnetRemoteClient {
            client: RpcClient::new(remote_rpc_url),
        }
    }
}

pub trait SomeRemoteCtx {
    fn get_remote_ctx<T>(&self, input: T) -> Option<(SurfnetRemoteClient, T)>;
}

impl SomeRemoteCtx for Option<SurfnetRemoteClient> {
    fn get_remote_ctx<T>(&self, input: T) -> Option<(SurfnetRemoteClient, T)> {
        self.as_ref()
            .map(|remote_rpc_client| (remote_rpc_client.clone(), input))
    }
}

impl SurfnetRemoteClient {
    pub fn new(remote_rpc_url: &str) -> Self {
        SurfnetRemoteClient {
            client: RpcClient::new(remote_rpc_url.to_string()),
        }
    }

    pub async fn get_epoch_info(&self) -> SurfpoolResult<EpochInfo> {
        self.client.get_epoch_info().await.map_err(Into::into)
    }

    pub async fn get_account(
        &self,
        pubkey: &Pubkey,
        commitment_config: CommitmentConfig,
    ) -> SurfpoolResult<GetAccountResult> {
        let res = self
            .client
            .get_account_with_commitment(pubkey, commitment_config)
            .await
            .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

        let result = match res.value {
            Some(account) => {
                let mut result = None;
                if is_supported_token_program(&account.owner) {
                    if let Some(token_account) = TokenAccount::unpack(&account.data).ok() {
                        let mint = self
                            .client
                            .get_account_with_commitment(&token_account.mint(), commitment_config)
                            .await
                            .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

                        result = Some(GetAccountResult::FoundTokenAccount(
                            (*pubkey, account.clone()),
                            (token_account.mint(), mint.value),
                        ));
                    };
                } else if account.executable {
                    let program_data_address = get_program_data_address(pubkey);

                    let program_data = self
                        .client
                        .get_account_with_commitment(&program_data_address, commitment_config)
                        .await
                        .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

                    result = Some(GetAccountResult::FoundProgramAccount(
                        (*pubkey, account.clone()),
                        (program_data_address, program_data.value),
                    ));
                }

                result.unwrap_or(GetAccountResult::FoundAccount(
                    *pubkey, account,
                    // Mark this account as needing to be updated in the SVM, since we fetched it
                    true,
                ))
            }
            None => GetAccountResult::None(*pubkey),
        };
        Ok(result)
    }

    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
        commitment_config: CommitmentConfig,
    ) -> SurfpoolResult<Vec<GetAccountResult>> {
        let remote_accounts = self
            .client
            .get_multiple_accounts(pubkeys)
            .await
            .map_err(SurfpoolError::get_multiple_accounts)?;

        let mut accounts_result = vec![];
        for (pubkey, remote_account) in pubkeys.iter().zip(remote_accounts) {
            if let Some(remote_account) = remote_account {
                if !remote_account.executable {
                    accounts_result.push(GetAccountResult::FoundAccount(
                        *pubkey,
                        remote_account,
                        // Mark this account as needing to be updated in the SVM, since we fetched it
                        true,
                    ));
                } else {
                    let program_data_address = get_program_data_address(pubkey);

                    let program_data = self
                        .client
                        .get_account_with_commitment(&program_data_address, commitment_config)
                        .await
                        .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

                    accounts_result.push(GetAccountResult::FoundProgramAccount(
                        (*pubkey, remote_account),
                        (program_data_address, program_data.value),
                    ));
                }
            } else {
                accounts_result.push(GetAccountResult::None(*pubkey));
            }
        }
        Ok(accounts_result)
    }

    pub async fn get_transaction(
        &self,
        signature: Signature,
        config: RpcTransactionConfig,
        latest_absolute_slot: u64,
    ) -> GetTransactionResult {
        match self
            .client
            .get_transaction_with_config(&signature, config)
            .await
        {
            Ok(tx) => GetTransactionResult::found_transaction(signature, tx, latest_absolute_slot),
            Err(_) => GetTransactionResult::None(signature),
        }
    }

    pub async fn get_token_accounts_by_owner(
        &self,
        owner: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolResult<Vec<RpcKeyedAccount>> {
        let token_account_filter = match filter {
            TokenAccountsFilter::Mint(mint) => RpcTokenAccountsFilter::Mint(mint.to_string()),
            TokenAccountsFilter::ProgramId(program_id) => {
                RpcTokenAccountsFilter::ProgramId(program_id.to_string())
            }
        };

        // the RPC client's default implementation of get_token_accounts_by_owner doesn't allow providing the config,
        // so we need to use the send method directly
        let res: RpcResult<Vec<RpcKeyedAccount>> = self
            .client
            .send(
                RpcRequest::GetTokenAccountsByOwner,
                json!([owner.to_string(), token_account_filter, config]),
            )
            .await;
        res.map_err(|e| SurfpoolError::get_token_accounts(owner, filter, e))
            .map(|res| res.value)
    }

    pub async fn get_token_largest_accounts(
        &self,
        mint: &Pubkey,
        commitment_config: CommitmentConfig,
    ) -> SurfpoolResult<Vec<RpcTokenAccountBalance>> {
        self.client
            .get_token_largest_accounts_with_commitment(mint, commitment_config)
            .await
            .map(|response| response.value)
            .map_err(|e| SurfpoolError::get_token_largest_accounts(*mint, e))
    }

    pub async fn get_token_accounts_by_delegate(
        &self,
        delegate: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolResult<Vec<RpcKeyedAccount>> {
        // validate that the program is supported if using ProgramId filter
        if let TokenAccountsFilter::ProgramId(program_id) = &filter {
            if !is_supported_token_program(program_id) {
                return Err(SurfpoolError::unsupported_token_program(*program_id));
            }
        }

        let token_account_filter = match &filter {
            TokenAccountsFilter::Mint(mint) => RpcTokenAccountsFilter::Mint(mint.to_string()),
            TokenAccountsFilter::ProgramId(program_id) => {
                RpcTokenAccountsFilter::ProgramId(program_id.to_string())
            }
        };

        let res: RpcResult<Vec<RpcKeyedAccount>> = self
            .client
            .send(
                RpcRequest::GetTokenAccountsByDelegate,
                json!([delegate.to_string(), token_account_filter, config]),
            )
            .await;

        res.map_err(|e| SurfpoolError::get_token_accounts_by_delegate_error(delegate, filter, e))
            .map(|res| res.value)
    }

    pub async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
        account_config: RpcAccountInfoConfig,
        filters: Option<Vec<RpcFilterType>>,
    ) -> SurfpoolResult<RemoteRpcResult<Vec<(Pubkey, Account)>>> {
        handle_remote_rpc(|| async {
            self.client
                .get_program_accounts_with_config(
                    program_id,
                    RpcProgramAccountsConfig {
                        filters,
                        with_context: Some(false),
                        account_config,
                        ..Default::default()
                    },
                )
                .await
                .map(|res| res)
                .map_err(|e| SurfpoolError::get_program_accounts(*program_id, e))
        })
        .await
    }

    pub async fn get_largest_accounts(
        &self,
        config: Option<RpcLargestAccountsConfig>,
    ) -> SurfpoolResult<RemoteRpcResult<Vec<RpcAccountBalance>>> {
        handle_remote_rpc(|| async {
            self.client
                .get_largest_accounts_with_config(config.unwrap_or_default())
                .await
                .map(|res| res.value)
                .map_err(SurfpoolError::get_largest_accounts)
        })
        .await
    }

    pub async fn get_genesis_hash(&self) -> SurfpoolResult<Hash> {
        self.client.get_genesis_hash().await.map_err(Into::into)
    }

    pub async fn get_signatures_for_address(
        &self,
        pubkey: &Pubkey,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> SurfpoolResult<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        let c = match config {
            Some(c) => GetConfirmedSignaturesForAddress2Config {
                before: c.before.and_then(|s| Signature::from_str(&s).ok()),
                commitment: c.commitment,
                limit: c.limit,
                until: c.until.and_then(|s| Signature::from_str(&s).ok()),
            },
            _ => GetConfirmedSignaturesForAddress2Config::default(),
        };
        self.client
            .get_signatures_for_address_with_config(pubkey, c)
            .await
            .map_err(SurfpoolError::get_signatures_for_address)
    }

    pub async fn get_block(
        &self,
        slot: &Slot,
        config: RpcBlockConfig,
    ) -> SurfpoolResult<UiConfirmedBlock> {
        self.client
            .get_block_with_config(*slot, config)
            .await
            .map_err(|e| SurfpoolError::get_block(e, *slot))
    }
}

/// Handles remote RPC calls, returning a `RemoteRpcResult` indicating whether the method was supported.
/// If the method is not supported, it returns `RemoteRpcResult::MethodNotSupported`.
/// If the method is supported, it returns `RemoteRpcResult::Ok(T)`.
/// If the method is supported but returns an error, it returns `Err(E)`.
pub async fn handle_remote_rpc<T, E, F, Fut>(fut: F) -> Result<RemoteRpcResult<T>, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    match fut().await {
        Ok(val) => Ok(RemoteRpcResult::Ok(val)),
        Err(e) if is_method_not_supported_error(&e) => Ok(RemoteRpcResult::MethodNotSupported),
        Err(e) => Err(e),
    }
}
