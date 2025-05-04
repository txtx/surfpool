use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::convert_transaction_metadata_from_canonical,
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
};
use chrono::Utc;
use crossbeam_channel::{Receiver, Sender};
use litesvm::{
    types::{FailedTransactionMetadata, SimulatedTransactionInfo, TransactionResult},
    LiteSVM,
};
use solana_account::Account;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_clock::{Clock, Slot, MAX_RECENT_BLOCKHASHES};
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_feature_set::{disable_new_loader_v3_deployments, FeatureSet};
use solana_hash::Hash;
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

pub const FINALIZATION_SLOT_THRESHOLD: u64 = 2;

// #[cfg(clippy)]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] = &[0];

// #[cfg(not(clippy))]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] =
//     include_bytes!("../../../../target/release/libsurfpool_subgraph.dylib");

pub type AccountFactory = Box<dyn Fn(&SurfnetSvm) -> Account + Send + Sync>;

pub enum GetAccountStrategy {
    LocalOrDefault(Option<AccountFactory>),
    LocalThenConnectionOrDefault(Option<AccountFactory>),
}

impl GetAccountStrategy {
    pub fn requires_connection(&self) -> bool {
        !matches!(self, Self::LocalOrDefault(_))
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
    pub transactions_queued_for_confirmation:
        VecDeque<(VersionedTransaction, Sender<TransactionStatusEvent>)>,
    pub transactions_queued_for_finalization:
        VecDeque<(Slot, VersionedTransaction, Sender<TransactionStatusEvent>)>,
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

        let inner = LiteSVM::new()
            .with_feature_set(feature_set)
            .with_blockhash_check(false);

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
                transactions_queued_for_confirmation: VecDeque::new(),
                transactions_queued_for_finalization: VecDeque::new(),
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
                                pubkey,
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
    pub fn airdrop_pubkeys(&mut self, lamports: u64, addresses: &[Pubkey]) {
        for recipient in addresses.iter() {
            let _ = self.airdrop(&recipient, lamports);
            let _ = self.simnet_events_tx.send(SimnetEvent::info(format!(
                "Genesis airdrop successful {}: {}",
                recipient, lamports
            )));
        }
    }

    /// Returns an `RpcClient` instance pointing to the currently connected RPC URL.
    ///
    /// Panics if the `SurfnetSvm` is not connected to an RPC endpoint.
    pub fn expected_rpc_client(&self) -> RpcClient {
        match &self.connection {
            SurfnetDataConnection::Offline => unreachable!(),
            SurfnetDataConnection::Connected(rpc_url, _) => RpcClient::new(rpc_url.to_string()),
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

    #[allow(deprecated)]
    pub fn new_blockhash(&mut self) {
        // cache th current blockhashes
        let blockhashes = self
            .inner
            .get_sysvar::<solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes>();
        let max_entries_len = blockhashes.len().min(MAX_RECENT_BLOCKHASHES);
        let mut entries = Vec::with_capacity(max_entries_len);
        // note: expire blockhash has a bug with liteSVM.
        // they only keep one blockhash in their RecentBlockhashes sysvar, so this function
        // clears out the other valid hashes.
        // so we manually rehydrate the sysvar with new latest blockhash + cached blockhashes.
        self.inner.expire_blockhash();
        let latest_entries = self
            .inner
            .get_sysvar::<solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes>();
        let latest_entry = latest_entries.first().unwrap();
        entries.push(solana_sdk::sysvar::recent_blockhashes::IterItem(
            0,
            &latest_entry.blockhash,
            latest_entry.fee_calculator.lamports_per_signature,
        ));
        for (i, entry) in blockhashes.iter().enumerate() {
            if i == MAX_RECENT_BLOCKHASHES - 1 {
                break;
            }

            entries.push(solana_sdk::sysvar::recent_blockhashes::IterItem(
                i as u64 + 1,
                &entry.blockhash,
                entry.fee_calculator.lamports_per_signature,
            ));
        }

        self.inner.set_sysvar(
            &solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes::from_iter(entries),
        );
    }

    pub fn check_blockhash_is_recent(&self, recent_blockhash: &Hash) -> bool {
        #[allow(deprecated)]
        self.inner
            .get_sysvar::<solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes>()
            .iter()
            .any(|entry| entry.blockhash == *recent_blockhash)
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
    pub fn set_account(&mut self, pubkey: &Pubkey, account: Account) -> SurfpoolResult<()> {
        self.inner
            .set_account(*pubkey, account)
            .map_err(|e| SurfpoolError::set_account(*pubkey, e))
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
    ) -> Result<Option<Account>, SurfpoolError> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(SurfpoolError::get_account(
                *pubkey,
                "Attempt to retrieve remote data from an offline vm",
            ));
        }

        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => {
                (self.inner.get_account(pubkey), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.inner.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let res = client
                            .get_account_with_commitment(pubkey, CommitmentConfig::confirmed())
                            .await
                            .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;
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
    ) -> Result<Option<Account>, SurfpoolError> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(SurfpoolError::get_account(
                *pubkey,
                "Attempt to retrieve remote data from an offline vm",
            ));
        }

        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => {
                (self.inner.get_account(pubkey), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.inner.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let res = client
                            .get_account_with_commitment(pubkey, CommitmentConfig::confirmed())
                            .await
                            .map_err(|e| SurfpoolError::get_account(*pubkey, e))?;

                        if let Some(account) = &res.value {
                            if account.executable {
                                let program_data_address = get_program_data_address(pubkey);
                                let res = self
                                    .get_account(
                                        &program_data_address,
                                        GetAccountStrategy::LocalThenConnectionOrDefault(None),
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
    ) -> Result<Vec<Option<Account>>, SurfpoolError> {
        // Ensure consistency between connection and strategy
        if !self.is_connected() && strategy.requires_connection() {
            return Err(SurfpoolError::get_multiple_accounts(
                "Attempt to retrieve remote data from an offline vm",
            ));
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
            GetAccountStrategy::LocalThenConnectionOrDefault(_) => {
                // Retrieve accounts missing locally
                let mut missing_accounts = Vec::new();
                let mut fetched_accounts = HashMap::new();
                for pubkey in pubkeys.iter() {
                    match self.inner.get_account(pubkey) {
                        Some(entry) => {
                            fetched_accounts.insert(pubkey, entry.clone());
                        }
                        None => {
                            missing_accounts.push(*pubkey);
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
                let remote_accounts = client
                    .get_multiple_accounts(&missing_accounts)
                    .await
                    .map_err(SurfpoolError::get_multiple_accounts)?;
                for (pubkey, remote_account) in missing_accounts.into_iter().zip(remote_accounts) {
                    if let Some(remote_account) = remote_account {
                        if remote_account.executable {
                            let program_data_address = get_program_data_address(&pubkey);
                            let res = self
                                .get_account(
                                    &program_data_address,
                                    GetAccountStrategy::LocalThenConnectionOrDefault(None),
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

        if !self.check_blockhash_is_recent(tx.message.recent_blockhash()) {
            let meta = litesvm::types::TransactionMetadata::default();
            let err = solana_transaction_error::TransactionError::BlockhashNotFound;

            let transaction_meta = convert_transaction_metadata_from_canonical(&meta);

            let _ = self
                .simnet_events_tx
                .try_send(SimnetEvent::transaction_processed(
                    transaction_meta,
                    Some(err.clone()),
                ));
            return Err(FailedTransactionMetadata { err, meta });
        }
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

    pub fn simulate_transaction(
        &mut self,
        tx: VersionedTransaction,
    ) -> Result<SimulatedTransactionInfo, FailedTransactionMetadata> {
        if !self.check_blockhash_is_recent(tx.message.recent_blockhash()) {
            let meta = litesvm::types::TransactionMetadata::default();
            let err = solana_transaction_error::TransactionError::BlockhashNotFound;

            return Err(FailedTransactionMetadata { err, meta });
        }
        self.inner.simulate_transaction(tx)
    }

    /// Processes a transaction by verifying, simulating, and executing it on the blockchain.
    ///
    /// This function processes a transaction with an associated sender for status updates. The transaction is
    /// verified for valid signatures, missing accounts are fetched from the network if needed, and the transaction is
    /// simulated or executed depending on the configuration.
    ///
    /// # Parameters
    ///
    /// - `transaction`: The `VersionedTransaction` to process.
    /// - `status_tx`: A `Sender<TransactionStatusEvent>` used to send status updates.
    /// - `skip_preflight`: A `bool` indicating whether to skip the preflight simulation step for the transaction.
    ///
    /// # Returns
    ///
    /// Returns a `Result`:
    /// - `Ok(())` if the transaction was successfully processed.
    /// - `Err(SurfpoolError)` if an error occurred during processing.
    pub async fn process_transaction(
        &mut self,
        transaction: VersionedTransaction,
        status_tx: Sender<TransactionStatusEvent>,
        skip_preflight: bool,
    ) -> Result<(), SurfpoolError> {
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
                return Ok(());
            }
        }

        // find accounts that are needed for this transaction but are missing from the local
        // svm cache, fetch them from the RPC, and insert them locally
        let accounts = match &transaction.message {
            VersionedMessage::Legacy(message) => message.account_keys.clone(),
            VersionedMessage::V0(message) => { 
                let alts = message.address_table_lookups.clone();
                let mut acc_keys = message.account_keys.clone();
                  if !alts.is_empty() {
                    let mut alt_pubkeys = 
                    alts
                   .iter()
                   .map(|msg| msg.account_key)
                   .collect::<Vec<_>>();
                   acc_keys.append(&mut alt_pubkeys);
                   acc_keys
                  } else {
                    acc_keys
                  }
            },
        };
        
        let _ = self
            .get_multiple_accounts_mut(
                &accounts,
                GetAccountStrategy::LocalThenConnectionOrDefault(None),
            )
            .await?;

        // if not skipping preflight, simulate the transaction
        if !skip_preflight {
            let (meta, err) = match self.inner.simulate_transaction(transaction.clone()) {
                Ok(res) => {
                    let transaction_meta = convert_transaction_metadata_from_canonical(&res.meta);
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
                return Ok(());
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
                self.transactions_queued_for_confirmation
                    .push_back((transaction, status_tx));
            }
            Err(res) => {
                let transaction_meta = convert_transaction_metadata_from_canonical(&res.meta);
                let _ = self.simnet_events_tx.try_send(SimnetEvent::error(format!(
                    "Transaction execution failed: {}",
                    res.err
                )));
                let _ = status_tx.try_send(TransactionStatusEvent::ExecutionFailure((
                    res.err,
                    transaction_meta,
                )));
            }
        };
        Ok(())
    }

    /// Confirms transactions that are queued for confirmation.
    ///
    /// This function processes all transactions in the confirmation queue, sending a confirmation
    /// event for each transaction. It updates the internal epoch and slot information, ensuring
    /// that the latest epoch and slot are reflected in the system. Additionally, it maintains
    /// performance samples for the last 30 slots, tracking the number of transactions processed
    /// in each slot.
    ///
    /// The function performs the following steps:
    /// 1. Iterates through the `transactions_queued_for_confirmation` queue and sends a
    ///    `TransactionStatusEvent::Success` event for each transaction, indicating that the
    ///    transaction has been confirmed.
    /// 2. Updates the `latest_epoch_info` to increment the slot index and absolute slot.
    /// 3. If the slot index exceeds the number of slots in the current epoch, it resets the slot
    ///    index to 0 and increments the epoch.
    /// 4. Maintains a rolling window of performance samples, ensuring that only the last 30 slots
    ///    are retained.
    /// 5. Sends a `SimnetEvent::ClockUpdate` event with the updated clock information.
    /// 6. Updates the system's clock sysvar with the new clock information.
    ///
    /// This function is typically called periodically to ensure that transactions are confirmed
    /// and the system's state remains consistent with the passage of time.
    pub fn confirm_transactions(&mut self) -> Result<(), SurfpoolError> {
        let num_transactions = self.transactions_queued_for_confirmation.len();
        while let Some((tx, status_tx)) = self.transactions_queued_for_confirmation.pop_front() {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
            // .map_err(Into::into)?;

            let finalized_at = self.latest_epoch_info.absolute_slot + FINALIZATION_SLOT_THRESHOLD;
            self.transactions_queued_for_finalization
                .push_back((finalized_at, tx, status_tx));
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
            num_transactions: num_transactions as u64,
            num_non_vote_transactions: None,
        });

        if self.latest_epoch_info.slot_index > self.latest_epoch_info.slots_in_epoch {
            self.latest_epoch_info.slot_index = 0;
            self.latest_epoch_info.epoch += 1;
        }
        let clock: Clock = Clock {
            slot: self.latest_epoch_info.slot_index,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp: Utc::now().timestamp(),
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::ClockUpdate(clock.clone()));
        self.inner.set_sysvar(&clock);
        Ok(())
    }

    /// Finalizes transactions that are queued for finalization.
    ///
    /// This function processes transactions in the finalization queue, sending a
    /// `TransactionStatusEvent::Success` event with the `Finalized` status for each transaction
    /// that has reached the required finalization slot threshold. Transactions that have not yet
    /// reached the finalization threshold are requeued for future processing.
    ///
    /// The function performs the following steps:
    /// 1. Iterates through the `transactions_queued_for_finalization` queue.
    /// 2. For each transaction, checks if the current slot has reached or exceeded the
    ///    transaction's `finalized_at` slot.
    /// 3. If the transaction is ready for finalization, sends a `TransactionStatusEvent::Success`
    ///    event with the `Finalized` status.
    /// 4. If the transaction is not yet ready for finalization, requeues it for future processing.
    ///
    /// This function ensures that transactions are finalized only after the required number of
    /// slots have passed, maintaining consistency with the blockchain's finalization rules.
    ///
    /// # Returns
    ///
    /// * `Ok(())` on successful processing of the finalization queue.
    /// * `Err(SurfpoolError)` if an error occurs during processing.
    pub fn finalize_transactions(&mut self) -> Result<(), SurfpoolError> {
        let current_slot = self.latest_epoch_info.absolute_slot;
        let mut requeue = VecDeque::new();
        while let Some((finalized_at, _tx, status_tx)) =
            self.transactions_queued_for_finalization.pop_front()
        {
            if current_slot >= finalized_at {
                let _ = status_tx.try_send(TransactionStatusEvent::Success(
                    TransactionConfirmationStatus::Finalized,
                ));
                // .map_err(Into::into)?;
            } else {
                requeue.push_back((finalized_at, _tx, status_tx));
            }
        }
        // Requeue any transactions that are not yet finalized
        self.transactions_queued_for_finalization
            .append(&mut requeue);

        Ok(())
    }
}
