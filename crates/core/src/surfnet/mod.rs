use crate::{
    rpc::utils::convert_transaction_metadata_from_canonical,
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
};
use chrono::Utc;
use crossbeam_channel::{Receiver, Sender};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_account::Account;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_clock::{Clock, Slot};
use solana_epoch_info::EpochInfo;
use solana_feature_set::{disable_new_loader_v3_deployments, FeatureSet};
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_sdk::{
    bpf_loader_upgradeable::get_program_data_address, system_instruction,
    transaction::VersionedTransaction,
};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, TransactionStatus, UiTransactionEncoding,
};
use std::collections::{HashMap, VecDeque};
use surfpool_types::{
    SimnetEvent, TransactionConfirmationStatus, TransactionMetadata, TransactionStatusEvent,
};

// #[cfg(clippy)]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] = &[0];

// #[cfg(not(clippy))]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] =
//     include_bytes!("../../../../target/release/libsurfpool_subgraph.dylib");

pub type AccountFactory = Box<dyn Fn(&SurfnetSvm) -> Account + Send + Sync>;

pub enum GetAccountStrategy {
    LocalOrDefault(Option<AccountFactory>),
    ConnectionOrDefault(Option<AccountFactory>),
    LocalThenConnectionOrDefault(Option<AccountFactory>),
}

pub enum GeyserEvent {
    NewTransaction(VersionedTransaction, TransactionMetadata, Slot),
}

pub struct SurfnetSvm {
    pub inner: LiteSVM,
    pub transactions: HashMap<Signature, SurfnetTransactionStatus>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub connection: SurfnetDataConnection,
    pub latest_epoch_info: EpochInfo,
    pub simnet_events_tx: Sender<SimnetEvent>,
    pub geyser_events_tx: Sender<GeyserEvent>,
}

pub enum SurfnetDataConnection {
    Local,
    Connected(String, EpochInfo),
}

impl SurfnetSvm {
    pub fn new() -> (Self, Receiver<SimnetEvent>, Receiver<GeyserEvent>) {
        let (simnet_events_tx, simnet_events_rx) = crossbeam_channel::bounded(1024);
        let (geyser_events_tx, geyser_events_rx) = crossbeam_channel::bounded(1024);

        let mut feature_set = FeatureSet::all_enabled();
        // v2.2 of the solana_sdk deprecates the v3 loader, and enables the v4 loader by default.
        // In order to keep the v3 deployments enabled, we need to remove the
        // `disable_new_loader_v3_deployments` feature from the active set, and add it to the inactive set.
        let _ = feature_set
            .active
            .remove(&disable_new_loader_v3_deployments::id());
        feature_set
            .inactive
            .insert(disable_new_loader_v3_deployments::id());

        let inner = LiteSVM::new().with_feature_set(feature_set);

        (
            Self {
                inner,
                transactions: HashMap::new(),
                perf_samples: VecDeque::new(),
                transactions_processed: 0,
                simnet_events_tx,
                geyser_events_tx,
                connection: SurfnetDataConnection::Local,
                latest_epoch_info: EpochInfo {
                    epoch: 0,
                    slot_index: 0,
                    slots_in_epoch: 0,
                    absolute_slot: 0,
                    block_height: 0,
                    transaction_count: None,
                },
            },
            simnet_events_rx,
            geyser_events_rx,
        )
    }

    pub async fn connect(
        &mut self,
        rpc_url: &str,
    ) -> Result<EpochInfo, Box<dyn std::error::Error>> {
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let epoch_info = rpc_client.get_epoch_info().await?;
        self.connection = SurfnetDataConnection::Connected(rpc_url.to_string(), epoch_info.clone());

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::Connected(rpc_url.to_string()));
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));
        Ok(epoch_info)
    }

    pub fn airdrop_pubkeys(&mut self, lamports: u64, addresses: &Vec<Pubkey>) {
        for recipient in addresses.iter() {
            let _ = self.airdrop(&recipient, lamports);
            let _ = self.simnet_events_tx.send(SimnetEvent::info(format!(
                "Genesis airdrop successful {}: {}",
                recipient.to_string(),
                lamports
            )));
        }
    }

    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> TransactionResult {
        let res = self.inner.airdrop(pubkey, lamports);
        if let Ok(ref tx_result) = res {
            let airdrop_keypair = Keypair::new();
            let slot = self.latest_epoch_info.absolute_slot;
            self.transactions.insert(
                tx_result.signature,
                SurfnetTransactionStatus::Processed(TransactionWithStatusMeta(
                    slot,
                    VersionedTransaction::try_new(
                        VersionedMessage::Legacy(Message::new(
                            &[system_instruction::transfer(
                                &airdrop_keypair.pubkey(),
                                &pubkey,
                                lamports,
                            )],
                            Some(&airdrop_keypair.pubkey()),
                        )),
                        &[airdrop_keypair],
                    )
                    .unwrap(),
                    convert_transaction_metadata_from_canonical(&tx_result),
                    None,
                )),
            );
        }
        res
    }

    pub fn expected_rpc_client(&self) -> RpcClient {
        match &self.connection {
            SurfnetDataConnection::Local => unreachable!(),
            SurfnetDataConnection::Connected(rpc_url, _) => {
                let rpc_client = RpcClient::new(rpc_url.to_string());
                rpc_client
            }
        }
    }

    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.latest_epoch_info.absolute_slot
    }

    pub fn latest_blockhash(&self) -> solana_hash::Hash {
        self.inner.latest_blockhash()
    }

    pub async fn set_account(
        &mut self,
        pubkey: &Pubkey,
        account: Account,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.set_account(pubkey.clone(), account);
        Ok(())
    }

    pub async fn get_account(
        &self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => {
                (self.inner.get_account(pubkey), factory)
            }
            GetAccountStrategy::ConnectionOrDefault(factory) => {
                let client = self.expected_rpc_client();
                let entry = client.get_account(pubkey).await?;
                (Some(entry), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.inner.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let entry = client.get_account(pubkey).await?;
                        (Some(entry), factory)
                    }
                }
            }
        };
        let account = match (result, factory) {
            (None, Some(factory)) => Some(factory(self)),
            (None, None) => None,
            (Some(account), _) => Some(account),
        };
        Ok(account)
    }

    pub async fn get_account_mut(
        &mut self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => {
                (self.inner.get_account(pubkey), factory)
            }
            GetAccountStrategy::ConnectionOrDefault(factory) => {
                let client = self.expected_rpc_client();
                let account = client.get_account(&pubkey).await?;
                self.inner.set_account(pubkey.clone(), account.clone());
                (Some(account), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.inner.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let account = client.get_account(&pubkey).await?;
                        self.inner.set_account(pubkey.clone(), account.clone());

                        if account.executable {
                            let program_data_address = get_program_data_address(pubkey);
                            let res = self
                                .get_account(
                                    &program_data_address,
                                    GetAccountStrategy::ConnectionOrDefault(None),
                                )
                                .await?;
                            if let Some(program_data) = res {
                                self.inner.set_account(program_data_address, program_data);
                            }
                        }
                        (Some(account), factory)
                    }
                }
            }
        };
        let account = match (result, factory) {
            (None, Some(factory)) => Some(factory(self)),
            (None, None) => None,
            (Some(account), _) => Some(account),
        };
        Ok(account)
    }

    pub async fn get_multiple_accounts_mut(
        &mut self,
        pubkeys: &Vec<Pubkey>,
        strategy: GetAccountStrategy,
    ) -> Result<Vec<Option<Account>>, Box<dyn std::error::Error>> {
        match strategy {
            GetAccountStrategy::LocalOrDefault(_) => {
                let mut accounts = vec![];
                for pubkey in pubkeys.iter() {
                    let account = self.inner.get_account(pubkey);
                    accounts.push(account);
                }
                Ok(accounts)
            }
            GetAccountStrategy::ConnectionOrDefault(_) => {
                let client = self.expected_rpc_client();
                let entry = client.get_multiple_accounts(pubkeys).await?;
                Ok(entry)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(_) => {
                // Retrieve accounts missing locally
                let mut missing_accounts = Vec::new();
                let mut fetched_accounts = HashMap::new();
                for pubkey in pubkeys.iter() {
                    match self.inner.get_account(pubkey) {
                        Some(entry) => {
                            fetched_accounts.insert(pubkey.clone(), entry.clone());
                        }
                        None => {
                            missing_accounts.push(pubkey.clone());
                        }
                    };
                }

                if missing_accounts.is_empty() {
                    let mut accounts = vec![];
                    for (_, account) in fetched_accounts.into_iter() {
                        accounts.push(Some(account));
                    }
                    return Ok(accounts);
                }

                let client = self.expected_rpc_client();
                let remote_accounts = client.get_multiple_accounts(&missing_accounts).await?;
                for (pubkey, remote_account) in missing_accounts.into_iter().zip(remote_accounts) {
                    if let Some(remote_account) = remote_account {
                        self.inner.set_account(pubkey, remote_account);
                    }
                }

                let mut accounts = vec![];
                for pubkey in pubkeys.iter() {
                    let account = self.inner.get_account(pubkey);
                    accounts.push(account);
                }
                Ok(accounts)
            }
        }
    }

    pub async fn get_transaction(
        &self,
        signature: &Signature,
        encoding: Option<UiTransactionEncoding>,
    ) -> Result<
        Option<(EncodedConfirmedTransactionWithStatusMeta, TransactionStatus)>,
        Box<dyn std::error::Error>,
    > {
        let mut tx = self
            .transactions
            .get(&signature)
            .map(|entry| entry.expect_processed().clone().into());

        if tx.is_none() {
            let client = self.expected_rpc_client();
            let entry = client
                .get_transaction(signature, encoding.unwrap_or(UiTransactionEncoding::Json))
                .await?;
            tx = Some(entry);
        }

        let mut response = None;
        if let Some(tx) = tx {
            let status = TransactionStatus {
                slot: tx.slot,
                confirmations: Some((self.get_latest_absolute_slot() - tx.slot) as usize),
                status: tx.transaction.clone().meta.map_or(Ok(()), |m| m.status),
                err: tx.transaction.clone().meta.map(|m| m.err).flatten(),
                confirmation_status: Some(
                    solana_transaction_status::TransactionConfirmationStatus::Confirmed,
                ),
            };
            response = Some((tx, status));
        }
        Ok(response)
    }

    pub fn send_transaction(&mut self, tx: VersionedTransaction) -> TransactionResult {
        self.transactions_processed += 1;
        match self.inner.send_transaction(tx.clone()) {
            Ok(res) => {
                let transaction_meta = convert_transaction_metadata_from_canonical(&res);

                self.transactions.insert(
                    transaction_meta.signature,
                    SurfnetTransactionStatus::Processed(TransactionWithStatusMeta(
                        self.get_latest_absolute_slot(),
                        tx,
                        transaction_meta.clone(),
                        None,
                    )),
                );
                let _ = self
                    .simnet_events_tx
                    .try_send(SimnetEvent::transaction_processed(transaction_meta, None));
                Ok(res)
            }
            Err(tx_failure) => {
                let transaction_meta =
                    convert_transaction_metadata_from_canonical(&tx_failure.meta);

                let _ = self
                    .simnet_events_tx
                    .try_send(SimnetEvent::transaction_processed(
                        transaction_meta,
                        Some(tx_failure.err.clone()),
                    ));
                Err(tx_failure)
            }
        }
    }

    pub async fn process_transactions(
        &mut self,
        transactions_to_process: Vec<(VersionedTransaction, Sender<TransactionStatusEvent>, bool)>,
        tick: bool,
    ) {
        let unix_timestamp: i64 = Utc::now().timestamp();
        let mut transactions_processed = Vec::new();

        // Handle the accumulated transactions
        for (transaction, status_tx, skip_preflight) in transactions_to_process.into_iter() {
            // verify valid signatures on the transaction
            {
                if transaction
                    .verify_with_results()
                    .iter()
                    .any(|valid| !*valid)
                {
                    let _ = self.simnet_events_tx.try_send(SimnetEvent::error(format!(
                        "Transaction verification failed: {}",
                        transaction.signatures[0]
                    )));
                    continue;
                }
            }

            // find accounts that are needed for this transaction but are missing from the local
            // svm cached, fetch them from the RPC, and insert them locally
            let accounts = match &transaction.message {
                VersionedMessage::Legacy(message) => message.account_keys.clone(),
                VersionedMessage::V0(message) => message.account_keys.clone(),
            };
            let _ = self
                .get_multiple_accounts_mut(
                    &accounts,
                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
                )
                .await;

            // if not skipping preflight, simulate the transaction
            if !skip_preflight {
                let (meta, err) = match self.inner.simulate_transaction(transaction.clone()) {
                    Ok(res) => {
                        let transaction_meta =
                            convert_transaction_metadata_from_canonical(&res.meta);
                        (transaction_meta, None)
                    }
                    Err(res) => {
                        let _ = self.simnet_events_tx.try_send(SimnetEvent::error(format!(
                            "Transaction simulation failed: {}",
                            res.err.to_string()
                        )));
                        (
                            convert_transaction_metadata_from_canonical(&res.meta),
                            Some(res.err),
                        )
                    }
                };

                if let Some(e) = &err {
                    let _ = status_tx
                        .try_send(TransactionStatusEvent::SimulationFailure((e.clone(), meta)));
                    continue;
                }
            }

            // send the transaction to the SVM
            match self.send_transaction(transaction.clone()) {
                Ok(res) => {
                    let transaction_meta = convert_transaction_metadata_from_canonical(&res);
                    let _ = self.geyser_events_tx.send(GeyserEvent::NewTransaction(
                        transaction.clone(),
                        transaction_meta.clone(),
                        self.latest_epoch_info.absolute_slot,
                    ));
                    let _ = status_tx.try_send(TransactionStatusEvent::Success(
                        TransactionConfirmationStatus::Processed,
                    ));
                    transactions_processed.push((transaction, status_tx));
                }
                Err(res) => {
                    let transaction_meta = convert_transaction_metadata_from_canonical(&res.meta);
                    let _ = self.simnet_events_tx.try_send(SimnetEvent::error(format!(
                        "Transaction execution failed: {}",
                        res.err.to_string()
                    )));
                    let _ = status_tx.try_send(TransactionStatusEvent::ExecutionFailure((
                        res.err,
                        transaction_meta,
                    )));
                }
            };
        }

        if !tick {
            return;
        }

        for (_key, status_tx) in transactions_processed.iter() {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
        }

        self.latest_epoch_info.slot_index += 1;
        self.latest_epoch_info.absolute_slot += 1;
        let slot = self.latest_epoch_info.slot_index;

        if self.perf_samples.len() > 30 {
            self.perf_samples.pop_back();
        }
        self.perf_samples.push_front(RpcPerfSample {
            slot,
            num_slots: 1,
            sample_period_secs: 1,
            num_transactions: transactions_processed.len() as u64,
            num_non_vote_transactions: None,
        });

        if self.latest_epoch_info.slot_index > self.latest_epoch_info.slots_in_epoch {
            self.latest_epoch_info.slot_index = 0;
            self.latest_epoch_info.epoch += 1;
        }
        let clock: Clock = Clock {
            slot: self.latest_epoch_info.slot_index,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp,
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::ClockUpdate(clock.clone()));
        self.inner.set_sysvar(&clock);
    }
}
