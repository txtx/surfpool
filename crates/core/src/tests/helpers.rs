#![allow(dead_code)]
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

use crossbeam_channel::Sender;
use litesvm::LiteSVM;
use solana_blake3_hasher::Hash;
use solana_clock::Clock;
use solana_epoch_info::EpochInfo;
use solana_sdk::transaction::VersionedTransaction;
use surfpool_types::SimnetCommand;

use crate::{
    rpc::{utils::convert_transaction_metadata_from_canonical, RunloopContext},
    types::{EntryStatus, GlobalState, TransactionWithStatusMeta},
};

use std::net::TcpListener;

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
        let (simnet_events_tx, _rx) = crossbeam_channel::unbounded();
        let (plugin_manager_commands_tx, _rx) = crossbeam_channel::unbounded();

        let mut svm = LiteSVM::new();
        let clock = Clock {
            slot: 123,
            epoch_start_timestamp: 123,
            epoch: 1,
            leader_schedule_epoch: 1,
            unix_timestamp: 123,
        };
        svm.set_sysvar::<Clock>(&clock);

        TestSetup {
            context: RunloopContext {
                simnet_commands_tx,
                simnet_events_tx: simnet_events_tx.clone(),
                plugin_manager_commands_tx,
                id: Hash::new_unique(),
                state: Arc::new(RwLock::new(GlobalState {
                    svm,
                    transactions: HashMap::new(),
                    epoch_info: EpochInfo {
                        epoch: 1,
                        slot_index: 0,
                        slots_in_epoch: 100,
                        absolute_slot: 50,
                        block_height: 42,
                        transaction_count: Some(2),
                    },
                    rpc_url: "http://localhost:8899".to_string(),
                    perf_samples: VecDeque::new(),
                    transactions_processed: 69,
                })),
            },
            rpc,
        }
    }

    pub fn new_with_epoch_info(rpc: T, epoch_info: EpochInfo) -> Self {
        let setup = TestSetup::new(rpc);
        setup.context.state.write().unwrap().epoch_info = epoch_info;
        setup
    }

    pub fn new_with_svm(rpc: T, svm: LiteSVM) -> Self {
        let setup = TestSetup::new(rpc);
        setup.context.state.write().unwrap().svm = svm;
        setup
    }

    pub fn new_with_mempool(rpc: T, simnet_commands_tx: Sender<SimnetCommand>) -> Self {
        let mut setup = TestSetup::new(rpc);
        setup.context.simnet_commands_tx = simnet_commands_tx;
        setup
    }

    pub fn without_blockhash(self) -> Self {
        let mut state_writer = self.context.state.write().unwrap();
        let svm = state_writer.svm.clone();
        let svm = svm.with_blockhash_check(false);
        state_writer.svm = svm;
        drop(state_writer);
        self
    }

    pub fn process_txs(&mut self, txs: Vec<VersionedTransaction>) {
        for tx in txs {
            let mut state_writer = self.context.state.write().unwrap();
            match state_writer.svm.send_transaction(tx.clone()) {
                Ok(res) => state_writer.transactions.insert(
                    tx.signatures[0],
                    EntryStatus::Processed(TransactionWithStatusMeta(
                        0,
                        tx,
                        convert_transaction_metadata_from_canonical(&res),
                        None,
                    )),
                ),
                Err(e) => state_writer.transactions.insert(
                    tx.signatures[0],
                    EntryStatus::Processed(TransactionWithStatusMeta(
                        0,
                        tx,
                        convert_transaction_metadata_from_canonical(&e.meta),
                        Some(e.err),
                    )),
                ),
            };
        }
    }
}
