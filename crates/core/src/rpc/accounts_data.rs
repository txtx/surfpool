use crate::rpc::utils::{transform_account_to_ui_account, verify_pubkey};
use crate::rpc::State;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_account_decoder::UiAccount;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_client::rpc_custom_error::RpcCustomError;
use solana_client::rpc_response::RpcBlockCommitment;
use solana_client::rpc_response::RpcResponseContext;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_runtime::commitment::BlockCommitmentArray;
use solana_sdk::clock::Slot;
use solana_sdk::commitment_config::CommitmentConfig;

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

    #[rpc(meta, name = "getMultipleAccounts")]
    fn get_multiple_accounts(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Vec<String>,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<UiAccount>>>>>;

    #[rpc(meta, name = "getBlockCommitment")]
    fn get_block_commitment(
        &self,
        meta: Self::Metadata,
        block: Slot,
    ) -> Result<RpcBlockCommitment<BlockCommitmentArray>>;

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
        println!(
            "get_account_info rpc request received: {:?} {:?}",
            pubkey_str, config
        );
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };

        let config = config.unwrap_or_default();
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        println!(
            "account from state: {:?}",
            state_reader.svm.get_account(&pubkey)
        );

        let ui_account = match state_reader.svm.get_account(&pubkey) {
            Some(account) => match transform_account_to_ui_account(&account, &config) {
                Ok(res) => Some(res),
                Err(e) => return Box::pin(future::err(e.into())),
            },
            None => match meta.get_remote_rpc_client() {
                Ok(rpc_client) => match rpc_client.get_account(&pubkey) {
                    Ok(account) => {
                        // let mut mut_state_reader = match meta.get_state_mut() {
                        //     Ok(res) => res,
                        //     Err(e) => return Box::pin(future::err(e.into())),
                        // };
                        // match mut_state_reader.svm.set_account(pubkey, account.clone()) {
                        //     Ok(_) => (),
                        //     Err(e) => {
                        //         println!("error caching setting account: {:?}", e);
                        //     }
                        // };
                        // drop(mut_state_reader);
                        // println!("remote rpc account: {:?}", account);
                        match transform_account_to_ui_account(&account, &config) {
                            Ok(res) => Some(res),
                            Err(e) => return Box::pin(future::err(e.into())),
                        }
                    }
                    Err(_) => None,
                },
                Err(e) => return Box::pin(future::err(e.into())),
            },
        };
        let res = RpcResponse {
            context: RpcResponseContext::new(state_reader.epoch_info.absolute_slot),
            value: ui_account,
        };
        Box::pin(future::ready(Ok(res)))
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

        let res = pubkeys
            .iter()
            .map(|s| {
                let pk = verify_pubkey(s)?;
                match state_reader.svm.get_account(&pk) {
                    Some(account) => Ok(Some(transform_account_to_ui_account(&account, &config)?)),
                    None => Ok(None),
                }
            })
            .collect::<Result<Vec<_>>>()
            .map(|value| RpcResponse {
                context: RpcResponseContext::new(state_reader.epoch_info.absolute_slot),
                value,
            });

        Box::pin(future::ready(res))
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
