use jsonrpc_core::BoxFuture;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig, RpcSupplyConfig,
    RpcTokenAccountsFilter,
};
use solana_client::rpc_response::RpcResponseContext;
use solana_client::rpc_response::{
    OptionalContext, RpcAccountBalance, RpcKeyedAccount, RpcSupply, RpcTokenAccountBalance,
};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::commitment_config::CommitmentConfig;

use super::RunloopContext;

#[rpc]
pub trait AccountsScan {
    type Metadata;

    #[rpc(meta, name = "getProgramAccounts")]
    fn get_program_accounts(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>>;

    #[rpc(meta, name = "getLargestAccounts")]
    fn get_largest_accounts(
        &self,
        meta: Self::Metadata,
        config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>>;

    #[rpc(meta, name = "getSupply")]
    fn get_supply(
        &self,
        meta: Self::Metadata,
        config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>>;

    // SPL Token-specific RPC endpoints
    // See https://github.com/solana-labs/solana-program-library/releases/tag/token-v2.0.0 for
    // program details

    #[rpc(meta, name = "getTokenLargestAccounts")]
    fn get_token_largest_accounts(
        &self,
        meta: Self::Metadata,
        mint_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>>;

    #[rpc(meta, name = "getTokenAccountsByOwner")]
    fn get_token_accounts_by_owner(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>>;

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
        _meta: Self::Metadata,
        _program_id_str: String,
        _config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>> {
        unimplemented!()
    }

    fn get_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>> {
        unimplemented!()
    }

    fn get_supply(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>> {
        Box::pin(async {
            Ok(RpcResponse {
                context: RpcResponseContext::new(0),
                value: RpcSupply {
                    total: 1,
                    circulating: 0,
                    non_circulating: 0,
                    non_circulating_accounts: vec![],
                },
            })
        })
    }

    fn get_token_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _mint_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>> {
        unimplemented!()
    }

    fn get_token_accounts_by_owner(
        &self,
        _meta: Self::Metadata,
        _owner_str: String,
        _token_account_filter: RpcTokenAccountsFilter,
        _config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        unimplemented!()
    }

    fn get_token_accounts_by_delegate(
        &self,
        _meta: Self::Metadata,
        _delegate_str: String,
        _token_account_filter: RpcTokenAccountsFilter,
        _config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        unimplemented!()
    }
}
