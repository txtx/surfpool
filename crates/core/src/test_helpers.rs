#![allow(dead_code)]

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

use crossbeam_channel::Sender;
use litesvm::LiteSVM;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{blake3::Hash, clock::Clock, epoch_info::EpochInfo, transaction::Transaction};

use crate::{
    rpc::RunloopContext,
    simnet::{EntryStatus, GlobalState, TransactionWithStatusMeta},
    types::SimnetCommand,
};

pub struct TestSetup<T> {
    pub context: RunloopContext,
    pub rpc: T,
}

impl<T> TestSetup<T> {
    pub fn new(rpc: T) -> Self {
        let (simnet_commands_tx, _rx) = crossbeam_channel::unbounded();
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
                plugin_manager_commands_tx,
                id: Hash::new_unique(),
                state: Arc::new(RwLock::new(GlobalState {
                    svm,
                    transactions: HashMap::new(),
                    account_insertion_tracker: HashMap::new(),
                    epoch_info: EpochInfo {
                        epoch: 1,
                        slot_index: 0,
                        slots_in_epoch: 100,
                        absolute_slot: 50,
                        block_height: 42,
                        transaction_count: Some(2),
                    },
                    rpc_client: Arc::new(RpcClient::new("http://localhost:8899".to_string())),
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

    pub fn new_without_blockhash(rpc: T) -> Self {
        let setup = TestSetup::new(rpc);
        setup.context.state.write().unwrap().svm = LiteSVM::new().with_blockhash_check(false);
        setup
    }

    pub fn process_txs(&mut self, txs: Vec<Transaction>) {
        for tx in txs {
            let mut state_writer = self.context.state.write().unwrap();
            match state_writer.svm.send_transaction(tx.clone()) {
                Ok(res) => state_writer.transactions.insert(
                    tx.signatures[0],
                    EntryStatus::Processed(TransactionWithStatusMeta(0, tx, res, None)),
                ),
                Err(e) => state_writer.transactions.insert(
                    tx.signatures[0],
                    EntryStatus::Processed(TransactionWithStatusMeta(0, tx, e.meta, Some(e.err))),
                ),
            };
        }
    }
}
