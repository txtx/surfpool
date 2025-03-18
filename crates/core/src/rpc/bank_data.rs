use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_client::rpc_config::{RpcBlockProductionConfig, RpcContextConfig};
use solana_client::rpc_response::{RpcBlockProduction, RpcInflationGovernor, RpcInflationRate};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::clock::Slot;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::epoch_schedule::EpochSchedule;

use super::{not_implemented_err, RunloopContext, State};

#[rpc]
pub trait BankData {
    type Metadata;

    #[rpc(meta, name = "getMinimumBalanceForRentExemption")]
    fn get_minimum_balance_for_rent_exemption(
        &self,
        meta: Self::Metadata,
        data_len: usize,
        commitment: Option<CommitmentConfig>,
    ) -> Result<u64>;

    #[rpc(meta, name = "getInflationGovernor")]
    fn get_inflation_governor(
        &self,
        meta: Self::Metadata,
        commitment: Option<CommitmentConfig>,
    ) -> Result<RpcInflationGovernor>;

    #[rpc(meta, name = "getInflationRate")]
    fn get_inflation_rate(&self, meta: Self::Metadata) -> Result<RpcInflationRate>;

    #[rpc(meta, name = "getEpochSchedule")]
    fn get_epoch_schedule(&self, meta: Self::Metadata) -> Result<EpochSchedule>;

    #[rpc(meta, name = "getSlotLeader")]
    fn get_slot_leader(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<String>;

    #[rpc(meta, name = "getSlotLeaders")]
    fn get_slot_leaders(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        limit: u64,
    ) -> Result<Vec<String>>;

    #[rpc(meta, name = "getBlockProduction")]
    fn get_block_production(
        &self,
        meta: Self::Metadata,
        config: Option<RpcBlockProductionConfig>,
    ) -> Result<RpcResponse<RpcBlockProduction>>;
}

pub struct SurfpoolBankDataRpc;
impl BankData for SurfpoolBankDataRpc {
    type Metadata = Option<RunloopContext>;

    fn get_minimum_balance_for_rent_exemption(
        &self,
        meta: Self::Metadata,
        data_len: usize,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<u64> {
        let ctx = meta.get_state()?;

        Ok(ctx.svm.minimum_balance_for_rent_exemption(data_len))
    }

    fn get_inflation_governor(
        &self,
        _meta: Self::Metadata,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<RpcInflationGovernor> {
        not_implemented_err()
    }

    fn get_inflation_rate(&self, _meta: Self::Metadata) -> Result<RpcInflationRate> {
        not_implemented_err()
    }

    fn get_epoch_schedule(&self, _meta: Self::Metadata) -> Result<EpochSchedule> {
         not_implemented_err()
    }

    fn get_slot_leader(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<String> {
        not_implemented_err()
    }

    fn get_slot_leaders(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _limit: u64,
    ) -> Result<Vec<String>> {
        not_implemented_err()
    }

    fn get_block_production(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcBlockProductionConfig>,
    ) -> Result<RpcResponse<RpcBlockProduction>> {
        not_implemented_err()
    }
}
