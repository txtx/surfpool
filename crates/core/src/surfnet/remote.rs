use solana_client::{
    nonblocking::rpc_client::RpcClient, rpc_request::TokenAccountsFilter,
    rpc_response::RpcKeyedAccount,
};
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_pubkey::Pubkey;
use solana_sdk::bpf_loader_upgradeable::get_program_data_address;
use solana_signature::Signature;
use solana_transaction_status::UiTransactionEncoding;

use super::GetTransactionResult;
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    surfnet::GetAccountResult,
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
        if let Some(remote_rpc_client) = self {
            Some((remote_rpc_client.clone(), input))
        } else {
            None
        }
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
            .get_account_with_commitment(pubkey, commitment_config.clone())
            .await
            .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

        let result = match res.value {
            Some(account) => {
                if !account.executable {
                    GetAccountResult::FoundAccount(
                        *pubkey, account,
                        // Mark this account as needing to be updated in the SVM, since we fetched it
                        true,
                    )
                } else {
                    let program_data_address = get_program_data_address(pubkey);

                    let program_data = self
                        .client
                        .get_account_with_commitment(
                            &program_data_address,
                            commitment_config.clone(),
                        )
                        .await
                        .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

                    GetAccountResult::FoundProgramAccount(
                        (*pubkey, account),
                        (program_data_address, program_data.value),
                    )
                }
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
            .get_multiple_accounts(&pubkeys)
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
                    let program_data_address = get_program_data_address(&pubkey);

                    let program_data = self
                        .client
                        .get_account_with_commitment(
                            &program_data_address,
                            commitment_config.clone(),
                        )
                        .await
                        .map_err(|e| SurfpoolError::get_account(pubkey.clone(), e))?;

                    accounts_result.push(GetAccountResult::FoundProgramAccount(
                        (*pubkey, remote_account),
                        (program_data_address, program_data.value),
                    ));
                }
            }
        }
        Ok(accounts_result)
    }

    pub async fn get_transaction(
        &self,
        signature: Signature,
        encoding: Option<UiTransactionEncoding>,
        latest_absolute_slot: u64,
    ) -> GetTransactionResult {
        match self
            .client
            .get_transaction(
                &signature,
                encoding.unwrap_or(UiTransactionEncoding::Base64),
            )
            .await
        {
            Ok(tx) => GetTransactionResult::found_transaction(signature, tx, latest_absolute_slot),
            Err(_) => GetTransactionResult::None(signature),
        }
    }

    pub async fn get_token_accounts_by_owner(
        &self,

        owner: Pubkey,
        token_program: Pubkey,
    ) -> SurfpoolResult<Vec<RpcKeyedAccount>> {
        self.client
            .get_token_accounts_by_owner(&owner, TokenAccountsFilter::ProgramId(token_program))
            .await
            .map_err(|e| SurfpoolError::get_token_accounts(owner, token_program, e))
    }
}
