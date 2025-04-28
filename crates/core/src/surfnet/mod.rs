use crate::{
    rpc::utils::convert_transaction_metadata_from_canonical,
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
};
use anyhow::anyhow;
use chrono::Utc;
use crossbeam_channel::{Receiver, Sender};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_account::Account;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_clock::{Clock, Slot};
use solana_commitment_config::CommitmentConfig;
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

impl GetAccountStrategy {
    pub fn requires_connection(&self) -> bool {
        match &self {
            Self::LocalOrDefault(_) => false,
            _ => true,
        }
    }
}

pub enum GeyserEvent {
    NewTransaction(VersionedTransaction, TransactionMetadata, Slot),
}

/// `SurfnetSvm` provides a lightweight Solana Virtual Machine (SVM) for testing and simulation.
///
/// It supports a local in-memory blockchain state,
/// remote RPC connections, transaction processing, and account management.
///
/// It also exposes channels to listen for simulation events (`SimnetEvent`) and Geyser plugin events (`GeyserEvent`).
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

#[derive(PartialEq, Eq)]
pub enum SurfnetDataConnection {
    Offline,
    Connected(String, EpochInfo),
}

impl SurfnetSvm {
    /// Creates a new instance of `SurfnetSvm`.
    ///
    /// Returns a tuple containing the instance itself, a receiver for simulation events, and a receiver for Geyser plugin events.
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
                connection: SurfnetDataConnection::Offline,
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

    /// Connects the `SurfnetSvm` to a live Solana RPC endpoint.
    ///
    /// This updates the internal epoch information and sends connection events over the simulation event channel.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The URL of the Solana RPC endpoint to connect to.
    ///
    /// # Returns
    ///
    /// * `Ok(EpochInfo)` on success, or an error if the RPC request fails.
    pub async fn connect(
        &mut self,
        rpc_url: &str,
    ) -> Result<EpochInfo, Box<dyn std::error::Error>> {
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let epoch_info = rpc_client.get_epoch_info().await?;
        self.connection = SurfnetDataConnection::Connected(rpc_url.to_string(), epoch_info.clone());
        self.latest_epoch_info = epoch_info.clone();

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::Connected(rpc_url.to_string()));
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));
        Ok(epoch_info)
    }

    /// Airdrops a specified amount of lamports to a single public key.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The recipient public key.
    /// * `lamports` - The amount of lamports to airdrop.
    ///
    /// # Returns
    ///
    /// * `TransactionResult` indicating success or failure.
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

    /// Airdrops a specified amount of lamports to a list of public keys.
    ///
    /// # Arguments
    ///
    /// * `lamports` - The amount of lamports to airdrop.
    /// * `addresses` - A vector of `Pubkey` recipients.
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

    /// Returns an `RpcClient` instance pointing to the currently connected RPC URL.
    ///
    /// Panics if the `SurfnetSvm` is not connected to an RPC endpoint.
    pub fn expected_rpc_client(&self) -> RpcClient {
        match &self.connection {
            SurfnetDataConnection::Offline => unreachable!(),
            SurfnetDataConnection::Connected(rpc_url, _) => {
                let rpc_client = RpcClient::new(rpc_url.to_string());
                rpc_client
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        match &self.connection {
            SurfnetDataConnection::Offline => false,
            SurfnetDataConnection::Connected(_, _) => true,
        }
    }

    /// Returns the latest known absolute slot from the local epoch info.
    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.latest_epoch_info.absolute_slot
    }

    /// Returns the latest blockhash known by the `SurfnetSvm`.
    pub fn latest_blockhash(&self) -> solana_hash::Hash {
        self.inner.latest_blockhash()
    }

    /// Sets an account in the local SVM state.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key of the account.
    /// * `account` - The `Account` to insert.
    ///
    /// # Returns
    ///
    /// * `Ok(())` on success, or an error if the operation fails.
    pub async fn set_account(
        &mut self,
        pubkey: &Pubkey,
        account: Account,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.inner.set_account(pubkey.clone(), account);
        Ok(())
    }

    /// Retrieves an account for the specified public key based on the given strategy.
    ///
    /// This function checks the `GetAccountStrategy` to decide whether to fetch the account from the local cache
    /// or from a remote RPC endpoint, falling back to the connection if needed.
    ///
    /// # Parameters
    ///
    /// - `pubkey`: The public key of the account to retrieve.
    /// - `strategy`: The strategy to use for fetching the account (`LocalOrDefault`, `ConnectionOrDefault`, etc.).
    ///
    /// # Returns
    ///
    /// A `Result` containing an optional account, which may be `None` if the account was not found.
    pub async fn get_account(
        &self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(anyhow!("Attempt to retrieve remote data from an offline vm").into());
        }

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

    /// Retrieves a mutable account for the specified public key based on the given strategy.
    ///
    /// This function works similarly to `get_account`, but will mutate the underlying state with the Account.
    /// Note: if the requested account is executable, the data account is also retrieved and stored.
    ///
    /// # Parameters
    ///
    /// - `pubkey`: The public key of the account to retrieve.
    /// - `strategy`: The strategy to use for fetching the account (`LocalOrDefault`, `ConnectionOrDefault`, etc.).
    ///
    /// # Returns
    ///
    /// A `Result` containing an optional mutable account.
    pub async fn get_account_mut(
        &mut self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(anyhow!("Attempt to retrieve remote data from an offline vm").into());
        }

        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => {
                (self.inner.get_account(pubkey), factory)
            }
            GetAccountStrategy::ConnectionOrDefault(factory) => {
                let client = self.expected_rpc_client();
                let res = client
                    .get_account_with_commitment(&pubkey, CommitmentConfig::confirmed())
                    .await?;

                if let Some(account) = &res.value {
                    if account.executable {
                        let program_data_address = get_program_data_address(pubkey);
                        let res = self
                            .get_account(
                                &program_data_address,
                                GetAccountStrategy::ConnectionOrDefault(None),
                            )
                            .await?;
                        if let Some(program_data) = res {
                            let _ = self.inner.set_account(program_data_address, program_data);
                        }
                    }
                    let _ = self.inner.set_account(pubkey.clone(), account.clone());
                }
                (res.value, factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.inner.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let res = client
                            .get_account_with_commitment(&pubkey, CommitmentConfig::confirmed())
                            .await?;

                        if let Some(account) = &res.value {
                            if account.executable {
                                let program_data_address = get_program_data_address(pubkey);
                                let res = self
                                    .get_account(
                                        &program_data_address,
                                        GetAccountStrategy::ConnectionOrDefault(None),
                                    )
                                    .await?;
                                if let Some(program_data) = res {
                                    let _ =
                                        self.inner.set_account(program_data_address, program_data);
                                }
                            }
                            let _ = self.inner.set_account(pubkey.clone(), account.clone());
                        }

                        (res.value, factory)
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

    /// Retrieves multiple accounts in a mutable fashion, based on the specified strategy.
    ///
    /// This function allows fetching multiple accounts at once. It uses different strategies to decide whether to fetch the
    /// account from local storage or the network, depending on the availability of the accounts and the given strategy.
    /// Note: requested accounts that are executable, are also getting their associate data retrieved and stored.
    ///
    /// # Parameters
    ///
    /// - `pubkeys`: A vector of public keys for the accounts to retrieve.
    /// - `strategy`: The strategy for fetching the account information (`LocalOrDefault`, `ConnectionOrDefault`, or `LocalThenConnectionOrDefault`).
    ///
    /// # Returns
    ///
    /// A `Result` containing a vector of `Option<Account>` for each requested public key. If the account is found, it is wrapped
    /// in `Some`; otherwise, `None` will be returned for the missing accounts.
    pub async fn get_multiple_accounts_mut(
        &mut self,
        pubkeys: &Vec<Pubkey>,
        strategy: GetAccountStrategy,
    ) -> Result<Vec<Option<Account>>, Box<dyn std::error::Error>> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(anyhow!("Attempt to retrieve remote data from an offline vm").into());
        }

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
                        if remote_account.executable {
                            let program_data_address = get_program_data_address(&pubkey);
                            let res = self
                                .get_account(
                                    &program_data_address,
                                    GetAccountStrategy::ConnectionOrDefault(None),
                                )
                                .await?;
                            if let Some(program_data) = res {
                                let _ = self.inner.set_account(program_data_address, program_data);
                            }
                        }
                        let _ = self.inner.set_account(pubkey, remote_account);
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

    /// Fetches a transaction's details by its signature.
    ///
    /// This function retrieves the details of a transaction based on its signature. It first checks if the transaction is
    /// cached locally. If not, it fetches the transaction from the RPC client. The transaction details are returned along with
    /// the transaction's status.
    ///
    /// # Parameters
    ///
    /// - `signature`: The signature of the transaction to retrieve.
    /// - `encoding`: An optional parameter specifying the encoding format for the transaction (e.g., `Json` or other formats).
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing either:
    /// - `Some((EncodedConfirmedTransactionWithStatusMeta, TransactionStatus))` if the transaction was successfully found, or
    /// - `None` if the transaction is not found.
    ///
    /// The `EncodedConfirmedTransactionWithStatusMeta` contains the full transaction details, and `TransactionStatus` includes
    /// the status of the transaction (e.g., whether it was confirmed).
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

        if tx.is_none() && self.is_connected() {
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

    /// Sends a transaction to the system for execution.
    ///
    /// This function attempts to send a transaction to the blockchain. It first increments the `transactions_processed` counter.
    /// Then it sends the transaction to the system and updates its status. If the transaction is successfully processed, it is
    /// cached locally, and a "transaction processed" event is sent. If the transaction fails, the error is recorded and an event
    /// is sent indicating the failure.
    ///
    /// # Parameters
    ///
    /// - `tx`: The transaction to send for processing.
    ///
    /// # Returns
    ///
    /// Returns a `Result`:
    /// - `Ok(res)` if the transaction was successfully sent and processed, containing the result of the transaction.
    /// - `Err(tx_failure)` if the transaction failed, containing the error information.
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

    /// Processes a batch of transactions by verifying, simulating, and executing them on the blockchain.
    ///
    /// This function processes a list of transactions, each with an associated sender for status updates. The transactions are
    /// verified for valid signatures, missing accounts are fetched from the network if needed, and the transactions are
    /// simulated or executed depending on the configuration. If the `tick` flag is set to `true`, the final status of each
    /// transaction is updated to "Confirmed". Performance statistics are also updated.
    ///
    /// # Parameters
    ///
    /// - `transactions_to_process`: A vector of tuples where each tuple contains:
    ///     - `VersionedTransaction`: The transaction to process.
    ///     - `Sender<TransactionStatusEvent>`: A sender used to send status updates.
    ///     - `bool`: Whether to skip the preflight simulation step for the transaction.
    /// - `produce_block`: A flag indicating whether to update the transaction statuses to "Confirmed" after execution.
    ///
    /// # Returns
    ///
    /// This function does not return a value. It processes the transactions in-place, sending status updates via the provided
    /// sender and updating internal performance and epoch information.
    pub async fn process_transactions(
        &mut self,
        transactions_to_process: Vec<(VersionedTransaction, Sender<TransactionStatusEvent>, bool)>,
        produce_block: bool,
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
            // svm cache, fetch them from the RPC, and insert them locally
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

        if !produce_block {
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
