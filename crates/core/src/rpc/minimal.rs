use super::{not_implemented_err, RunloopContext};
use crate::rpc::{utils::verify_pubkey, State};
use jsonrpc_core::{futures::future, BoxFuture, Error, Result};
use jsonrpc_derive::rpc;
use solana_client::{
    rpc_config::{
        RpcContextConfig, RpcGetVoteAccountsConfig, RpcLeaderScheduleConfig,
        RpcLeaderScheduleConfigWrapper,
    },
    rpc_response::{
        RpcIdentity, RpcLeaderSchedule, RpcResponseContext, RpcSnapshotSlotInfo, RpcVersionInfo,
        RpcVoteAccountStatus,
    },
};
use solana_clock::{Clock, Slot};
use solana_epoch_info::EpochInfo;
use solana_rpc_client_api::response::Response as RpcResponse;

#[rpc]
pub trait Minimal {
    type Metadata;

    #[rpc(meta, name = "getBalance")]
    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<RpcResponse<u64>>>;

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

    fn get_balance(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        _config: Option<RpcContextConfig>, // TODO: use config
    ) -> BoxFuture<Result<RpcResponse<u64>>> {
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
        drop(state_reader);

        Box::pin(async move {
            let account = if let None = account {
                // Fetch and save the missing account
                if let Some(fetched_account) = rpc_client.get_account(&pubkey).await.ok() {
                    let mut state_reader = meta.get_state_mut()?;
                    state_reader
                        .svm
                        .set_account(pubkey, fetched_account.clone())
                        .map_err(|err| {
                            Error::invalid_params(format!(
                                "failed to save fetched account {pubkey:?}: {err:?}"
                            ))
                        })?;

                    Some(fetched_account)
                } else {
                    None
                }
            } else {
                account
            };

            Ok(RpcResponse {
                context: RpcResponseContext::new(absolute_slot),
                value: account.map(|account| account.lamports).unwrap_or(0),
            })
        })
    }

    fn get_epoch_info(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<EpochInfo> {
        let state_reader = meta.get_state()?;
        Ok(state_reader.epoch_info.clone())
    }

    fn get_genesis_hash(&self, _meta: Self::Metadata) -> Result<String> {
        not_implemented_err()
    }

    fn get_health(&self, meta: Self::Metadata) -> Result<String> {
        let _state_reader = meta.get_state()?;

        // todo: we could check the time from the state clock and compare
        Ok("ok".to_string())
    }

    fn get_identity(&self, _meta: Self::Metadata) -> Result<RpcIdentity> {
        not_implemented_err()
    }

    fn get_slot(&self, meta: Self::Metadata, _config: Option<RpcContextConfig>) -> Result<Slot> {
        let state_reader = meta.get_state()?;
        let clock: Clock = state_reader.svm.get_sysvar();
        Ok(clock.slot.into())
    }

    fn get_block_height(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<u64> {
        let state_reader = meta.get_state()?;
        Ok(state_reader.epoch_info.block_height)
    }

    fn get_highest_snapshot_slot(&self, _meta: Self::Metadata) -> Result<RpcSnapshotSlotInfo> {
        not_implemented_err()
    }

    fn get_transaction_count(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<u64> {
        let state_reader = meta.get_state()?;
        Ok(state_reader.transactions_processed as u64)
    }

    fn get_version(&self, _: Self::Metadata) -> Result<RpcVersionInfo> {
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
        _meta: Self::Metadata,
        _config: Option<RpcGetVoteAccountsConfig>,
    ) -> Result<RpcVoteAccountStatus> {
        Ok(RpcVoteAccountStatus {
            current: vec![],
            delinquent: vec![],
        })
    }

    // TODO: Refactor `agave-validator wait-for-restart-window` to not require this method, so
    //       it can be removed from rpc_minimal
    fn get_leader_schedule(
        &self,
        _meta: Self::Metadata,
        _options: Option<RpcLeaderScheduleConfigWrapper>,
        _config: Option<RpcLeaderScheduleConfig>,
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
        not_implemented_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::TestSetup;

    #[test]
    fn test_get_health() {
        let setup = TestSetup::new(SurfpoolMinimalRpc);
        let result = setup.rpc.get_health(Some(setup.context));
        assert_eq!(result.unwrap(), "ok");
    }

    #[test]
    fn test_get_transaction_count() {
        let setup = TestSetup::new(SurfpoolMinimalRpc);
        let transactions_processed = setup
            .context
            .state
            .read()
            .map(|s| s.transactions_processed)
            .unwrap();
        let result = setup.rpc.get_transaction_count(Some(setup.context), None);
        assert_eq!(result.unwrap(), transactions_processed);
    }

    #[test]
    fn test_get_epoch_info() {
        let info = EpochInfo {
            epoch: 1,
            slot_index: 1,
            slots_in_epoch: 1,
            absolute_slot: 1,
            block_height: 1,
            transaction_count: Some(1),
        };
        let setup = TestSetup::new_with_epoch_info(SurfpoolMinimalRpc, info.clone());
        let result = setup.rpc.get_epoch_info(Some(setup.context), None).unwrap();
        assert_eq!(result, info);
    }

    #[test]
    fn test_get_slot() {
        let setup = TestSetup::new(SurfpoolMinimalRpc);
        let result = setup.rpc.get_slot(Some(setup.context), None).unwrap();
        assert_eq!(result, 123);
    }

    #[test]
    fn test_get_version() {
        let setup = TestSetup::new(SurfpoolMinimalRpc);
        let result = setup.rpc.get_version(Some(setup.context)).unwrap();
        assert!(!result.solana_core.is_empty());
        assert!(result.feature_set.is_some());
    }

    #[test]
    fn test_get_vote_accounts() {
        let setup = TestSetup::new(SurfpoolMinimalRpc);
        let result = setup
            .rpc
            .get_vote_accounts(Some(setup.context), None)
            .unwrap();
        assert!(result.current.is_empty());
        assert!(result.delinquent.is_empty());
    }
}
