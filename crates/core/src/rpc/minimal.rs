use crate::rpc::{
    utils::{format_account, verify_pubkey},
    State,
};

use super::RunloopContext;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_account_decoder::UiAccount;
use solana_client::{
    rpc_config::{
        RpcAccountInfoConfig, RpcContextConfig, RpcGetVoteAccountsConfig, RpcLeaderScheduleConfig,
        RpcLeaderScheduleConfigWrapper,
    },
    rpc_response::{
        RpcIdentity, RpcLeaderSchedule, RpcSnapshotSlotInfo, RpcVersionInfo, RpcVoteAccountStatus,
    },
};
use solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::{
    clock::{Clock, Slot},
    epoch_info::EpochInfo,
};

#[rpc]
pub trait Minimal {
    type Metadata;

    #[rpc(meta, name = "getAccountInfo")]
    fn get_account_info(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcAccountInfoConfig>,
    ) -> Result<Option<UiAccount>>;

    #[rpc(meta, name = "getBalance")]
    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>>;

    #[rpc(meta, name = "getEpochInfo")]
    fn get_epoch_info(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<EpochInfo>;

    #[rpc(meta, name = "getGenesisHash")]
    fn get_genesis_hash(&self, meta: Self::Metadata) -> Result<String>;

    #[rpc(meta, name = "getHealth")]
    fn get_health(&self, meta: Self::Metadata) -> Result<String>;

    #[rpc(meta, name = "getIdentity")]
    fn get_identity(&self, meta: Self::Metadata) -> Result<RpcIdentity>;

    #[rpc(meta, name = "getSlot")]
    fn get_slot(&self, meta: Self::Metadata, config: Option<RpcContextConfig>) -> Result<Slot>;

    #[rpc(meta, name = "getBlockHeight")]
    fn get_block_height(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<u64>;

    #[rpc(meta, name = "getHighestSnapshotSlot")]
    fn get_highest_snapshot_slot(&self, meta: Self::Metadata) -> Result<RpcSnapshotSlotInfo>;

    #[rpc(meta, name = "getTransactionCount")]
    fn get_transaction_count(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<u64>;

    #[rpc(meta, name = "getVersion")]
    fn get_version(&self, meta: Self::Metadata) -> Result<RpcVersionInfo>;

    // TODO: Refactor `agave-validator wait-for-restart-window` to not require this method, so
    //       it can be removed from rpc_minimal
    #[rpc(meta, name = "getVoteAccounts")]
    fn get_vote_accounts(
        &self,
        meta: Self::Metadata,
        config: Option<RpcGetVoteAccountsConfig>,
    ) -> Result<RpcVoteAccountStatus>;

    // TODO: Refactor `agave-validator wait-for-restart-window` to not require this method, so
    //       it can be removed from rpc_minimal
    #[rpc(meta, name = "getLeaderSchedule")]
    fn get_leader_schedule(
        &self,
        meta: Self::Metadata,
        options: Option<RpcLeaderScheduleConfigWrapper>,
        config: Option<RpcLeaderScheduleConfig>,
    ) -> Result<Option<RpcLeaderSchedule>>;
}

pub struct SurfpoolMinimalRpc;
impl Minimal for SurfpoolMinimalRpc {
    type Metadata = Option<RunloopContext>;

    fn get_account_info(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcAccountInfoConfig>,
    ) -> Result<Option<UiAccount>> {
        println!(
            "get_account_info rpc request received: {:?} {:?}",
            pubkey_str, config
        );
        let pubkey = verify_pubkey(&pubkey_str)?;
        let config = {
            if let Some(config) = config {
                config
            } else {
                RpcAccountInfoConfig::default()
            }
        };

        let state_reader = meta.get_state()?;

        Ok(format_account(
            state_reader.svm.get_account(&pubkey),
            config.encoding,
        ))
    }

    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>> {
        println!("get_balance rpc request received: {:?}", pubkey_str);
        let pubkey = verify_pubkey(&pubkey_str)?;
        // meta.get_balance(&pubkey, config.unwrap_or_default())
        unimplemented!()
    }

    fn get_epoch_info(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<EpochInfo> {
        let state_reader = meta.get_state()?;

        Ok(state_reader.epoch_info.clone())
    }

    fn get_genesis_hash(&self, meta: Self::Metadata) -> Result<String> {
        println!("get_genesis_hash rpc request received");
        // Ok(meta.genesis_hash.to_string())
        unimplemented!()
    }

    fn get_health(&self, meta: Self::Metadata) -> Result<String> {
        let _state_reader = meta.get_state()?;

        // todo: we could check the time from the state clock and compare
        Ok("ok".to_string())
    }

    fn get_identity(&self, meta: Self::Metadata) -> Result<RpcIdentity> {
        println!("get_identity rpc request received");
        // Ok(RpcIdentity {
        //     identity: meta.cluster_info.id().to_string(),
        // })
        unimplemented!()
    }

    fn get_slot(&self, meta: Self::Metadata, _config: Option<RpcContextConfig>) -> Result<Slot> {
        let state_reader = meta.get_state()?;
        let clock: Clock = state_reader.svm.get_sysvar();
        Ok(clock.slot.into())
    }

    fn get_block_height(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<u64> {
        println!("get_block_height rpc request received");
        // meta.get_block_height(config.unwrap_or_default())
        unimplemented!()
    }

    fn get_highest_snapshot_slot(&self, meta: Self::Metadata) -> Result<RpcSnapshotSlotInfo> {
        println!("get_highest_snapshot_slot rpc request received");
        // Ok(RpcSnapshotSlotInfo {
        //     full: 0,
        //     incremental: None,
        // })
        unimplemented!()
    }

    fn get_transaction_count(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<u64> {
        println!("get_transaction_count rpc request received");
        // meta.get_transaction_count(config.unwrap_or_default())
        unimplemented!()
    }

    fn get_version(&self, _: Self::Metadata) -> Result<RpcVersionInfo> {
        println!("get_version rpc request received");

        let version = solana_version::Version::default();
        Ok(RpcVersionInfo {
            solana_core: version.to_string(),
            feature_set: Some(version.feature_set),
        })
    }

    // TODO: Refactor `agave-validator wait-for-restart-window` to not require this method, so
    //       it can be removed from rpc_minimal
    fn get_vote_accounts(
        &self,
        meta: Self::Metadata,
        config: Option<RpcGetVoteAccountsConfig>,
    ) -> Result<RpcVoteAccountStatus> {
        println!("get_vote_accounts rpc request received");
        // meta.get_vote_accounts(config)
        unimplemented!()
    }

    // TODO: Refactor `agave-validator wait-for-restart-window` to not require this method, so
    //       it can be removed from rpc_minimal
    fn get_leader_schedule(
        &self,
        meta: Self::Metadata,
        options: Option<RpcLeaderScheduleConfigWrapper>,
        config: Option<RpcLeaderScheduleConfig>,
    ) -> Result<Option<RpcLeaderSchedule>> {
        // let (slot, maybe_config) = options.map(|options| options.unzip()).unwrap_or_default();
        // let config = maybe_config.or(config).unwrap_or_default();

        // if let Some(ref identity) = config.identity {
        //     let _ = verify_pubkey(identity)?;
        // }

        // let bank = meta.bank(config.commitment);
        // let slot = slot.unwrap_or_else(|| bank.slot());
        // let epoch = bank.epoch_schedule().get_epoch(slot);

        // println!("get_leader_schedule rpc request received: {:?}", slot);

        // Ok(meta
        //     .leader_schedule_cache
        //     .get_epoch_leader_schedule(epoch)
        //     .map(|leader_schedule| {
        //         let mut schedule_by_identity =
        //             solana_ledger::leader_schedule_utils::leader_schedule_by_identity(
        //                 leader_schedule.get_slot_leaders().iter().enumerate(),
        //             );
        //         if let Some(identity) = config.identity {
        //             schedule_by_identity.retain(|k, _| *k == identity);
        //         }
        //         schedule_by_identity
        //     }))
        unimplemented!()
    }
}

pub struct MockForkGraph {}

impl ForkGraph for MockForkGraph {
    fn relationship(&self, a: Slot, b: Slot) -> BlockRelation {
        match a.cmp(&b) {
            std::cmp::Ordering::Less => BlockRelation::Ancestor,
            std::cmp::Ordering::Equal => BlockRelation::Equal,
            std::cmp::Ordering::Greater => BlockRelation::Descendant,
        }
    }
}
