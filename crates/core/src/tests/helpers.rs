#![allow(dead_code)]
use std::net::TcpListener;

use crossbeam_channel::Sender;
use litesvm::LiteSVM;
use solana_clock::Clock;
use solana_epoch_info::EpochInfo;
use solana_sdk::transaction::VersionedTransaction;
use surfpool_types::SimnetCommand;

use crate::{
    rpc::{utils::convert_transaction_metadata_from_canonical, RunloopContext},
    surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm},
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
};

pub fn get_free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind to port 0: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to parse address: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}

#[derive(Clone)]
pub struct TestSetup<T>
where
    T: Clone,
{
    pub context: RunloopContext,
    pub rpc: T,
}

impl<T> TestSetup<T>
where
    T: Clone,
{
    pub fn new(rpc: T) -> Self {
        let (simnet_commands_tx, _rx) = crossbeam_channel::unbounded();
        let (plugin_manager_commands_tx, _rx) = crossbeam_channel::unbounded();

        let (mut surfnet_svm, _, _) = SurfnetSvm::new();
        let clock = Clock {
            slot: 123,
            epoch_start_timestamp: 123,
            epoch: 1,
            leader_schedule_epoch: 1,
            unix_timestamp: 123,
        };
        surfnet_svm.inner.set_sysvar::<Clock>(&clock);
        surfnet_svm.latest_epoch_info = EpochInfo {
            epoch: clock.epoch,
            slot_index: clock.slot,
            slots_in_epoch: 100,
            absolute_slot: clock.slot,
            block_height: 42,
            transaction_count: Some(2),
        };
        surfnet_svm.transactions_processed = 69;

        TestSetup {
            context: RunloopContext {
                simnet_commands_tx,
                plugin_manager_commands_tx,
                id: None,
                svm_locker: SurfnetSvmLocker::new(surfnet_svm),
                remote_rpc_client: None,
            },
            rpc,
        }
    }

    pub fn new_with_epoch_info(rpc: T, epoch_info: EpochInfo) -> Self {
        let setup = TestSetup::new(rpc);
        setup
            .context
            .svm_locker
            .0
            .blocking_write()
            .latest_epoch_info = epoch_info;
        setup
    }

    pub fn new_with_svm(rpc: T, svm: LiteSVM) -> Self {
        let setup = TestSetup::new(rpc);
        setup.context.svm_locker.0.blocking_write().inner = svm;
        setup
    }

    pub fn new_with_mempool(rpc: T, simnet_commands_tx: Sender<SimnetCommand>) -> Self {
        let mut setup = TestSetup::new(rpc);
        setup.context.simnet_commands_tx = simnet_commands_tx;
        setup
    }

    pub async fn without_blockhash(self) -> Self {
        let mut state_writer = self.context.svm_locker.0.write().await;
        let svm = state_writer.inner.clone();
        let svm = svm.with_blockhash_check(false);
        state_writer.inner = svm;
        drop(state_writer);
        self
    }

    pub async fn process_txs(&mut self, txs: Vec<VersionedTransaction>) {
        for tx in txs {
            let mut state_writer = self.context.svm_locker.0.write().await;
            match state_writer.send_transaction(tx.clone(), false) {
                Ok(res) => state_writer.transactions.insert(
                    tx.signatures[0],
                    SurfnetTransactionStatus::Processed(Box::new(TransactionWithStatusMeta(
                        0,
                        tx,
                        convert_transaction_metadata_from_canonical(&res),
                        None,
                    ))),
                ),
                Err(e) => state_writer.transactions.insert(
                    tx.signatures[0],
                    SurfnetTransactionStatus::Processed(Box::new(TransactionWithStatusMeta(
                        0,
                        tx,
                        convert_transaction_metadata_from_canonical(&e.meta),
                        Some(e.err),
                    ))),
                ),
            };
        }
    }
}
