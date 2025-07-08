use std::collections::{HashMap, VecDeque};

use chrono::Utc;
use crossbeam_channel::{unbounded, Receiver, Sender};
use litesvm::{
    types::{
        FailedTransactionMetadata, SimulatedTransactionInfo, TransactionMetadata, TransactionResult,
    },
    LiteSVM,
};
use solana_account::Account;
use solana_account_decoder::{
    encode_ui_account,
    parse_account_data::{AccountAdditionalDataV3, SplTokenAdditionalDataV2},
    UiAccount, UiAccountEncoding,
};
use solana_client::{rpc_client::SerializableTransaction, rpc_response::RpcPerfSample};
use solana_clock::{Clock, Slot, MAX_RECENT_BLOCKHASHES};
use solana_epoch_info::EpochInfo;
use solana_feature_set::{disable_new_loader_v3_deployments, FeatureSet};
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_sdk::{
    genesis_config::GenesisConfig, inflation::Inflation, program_option::COption,
    program_pack::Pack, system_instruction, transaction::VersionedTransaction,
};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedTransaction, EncodedTransactionWithStatusMeta, UiAddressTableLookup,
    UiCompiledInstruction, UiConfirmedBlock, UiMessage, UiRawMessage, UiTransaction,
};
use spl_token::state::Account as TokenAccount;
use spl_token_2022::extension::{
    interest_bearing_mint::InterestBearingConfig, scaled_ui_amount::ScaledUiAmountConfig,
    BaseStateWithExtensions, StateWithExtensions,
};
use surfpool_types::{
    types::{ComputeUnitsEstimationResult, ProfileResult},
    SimnetEvent, TransactionConfirmationStatus, TransactionStatusEvent,
};

use super::{
    remote::SurfnetRemoteClient, AccountSubscriptionData, BlockHeader, BlockIdentifier,
    GetAccountResult, GeyserEvent, SignatureSubscriptionData, SignatureSubscriptionType,
    FINALIZATION_SLOT_THRESHOLD, SLOTS_PER_EPOCH,
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::convert_transaction_metadata_from_canonical,
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
};

pub type AccountOwner = Pubkey;

/// `SurfnetSvm` provides a lightweight Solana Virtual Machine (SVM) for testing and simulation.
///
/// It supports a local in-memory blockchain state,
/// remote RPC connections, transaction processing, and account management.
///
/// It also exposes channels to listen for simulation events (`SimnetEvent`) and Geyser plugin events (`GeyserEvent`).
#[derive(Clone)]
pub struct SurfnetSvm {
    pub inner: LiteSVM,
    pub remote_rpc_url: Option<String>,
    pub chain_tip: BlockIdentifier,
    pub blocks: HashMap<Slot, BlockHeader>,
    pub transactions: HashMap<Signature, SurfnetTransactionStatus>,
    pub transactions_queued_for_confirmation:
        VecDeque<(VersionedTransaction, Sender<TransactionStatusEvent>)>,
    pub transactions_queued_for_finalization:
        VecDeque<(Slot, VersionedTransaction, Sender<TransactionStatusEvent>)>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub latest_epoch_info: EpochInfo,
    pub simnet_events_tx: Sender<SimnetEvent>,
    pub geyser_events_tx: Sender<GeyserEvent>,
    pub signature_subscriptions: HashMap<Signature, Vec<SignatureSubscriptionData>>,
    pub account_subscriptions: AccountSubscriptionData,
    pub slot_subscriptions: Vec<Sender<SlotInfo>>,
    pub tagged_profiling_results: HashMap<String, Vec<ProfileResult>>,
    pub updated_at: u64,
    pub accounts_registry: HashMap<Pubkey, Account>,
    pub accounts_by_owner: HashMap<Pubkey, Vec<Pubkey>>,
    pub account_associated_data: HashMap<Pubkey, AccountAdditionalDataV3>,
    pub token_accounts: HashMap<Pubkey, spl_token::state::Account>,
    pub token_accounts_by_owner: HashMap<Pubkey, Vec<Pubkey>>,
    pub token_accounts_by_delegate: HashMap<Pubkey, Vec<Pubkey>>,
    pub token_accounts_by_mint: HashMap<Pubkey, Vec<Pubkey>>,
    pub total_supply: u64,
    pub circulating_supply: u64,
    pub non_circulating_supply: u64,
    pub non_circulating_accounts: Vec<String>,
    pub genesis_config: GenesisConfig,
    pub inflation: Inflation,
}

impl SurfnetSvm {
    /// Creates a new instance of `SurfnetSvm`.
    ///
    /// Returns a tuple containing the SVM instance, a receiver for simulation events, and a receiver for Geyser plugin events.
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
            .with_blockhash_check(false)
            .with_sigverify(false);

        (
            Self {
                inner,
                remote_rpc_url: None,
                chain_tip: BlockIdentifier::zero(),
                blocks: HashMap::new(),
                transactions: HashMap::new(),
                perf_samples: VecDeque::new(),
                transactions_processed: 0,
                simnet_events_tx,
                geyser_events_tx,
                latest_epoch_info: EpochInfo {
                    epoch: 0,
                    slot_index: 0,
                    slots_in_epoch: SLOTS_PER_EPOCH,
                    absolute_slot: 0,
                    block_height: 0,
                    transaction_count: None,
                },
                transactions_queued_for_confirmation: VecDeque::new(),
                transactions_queued_for_finalization: VecDeque::new(),
                signature_subscriptions: HashMap::new(),
                account_subscriptions: HashMap::new(),
                slot_subscriptions: Vec::new(),
                tagged_profiling_results: HashMap::new(),
                updated_at: Utc::now().timestamp_millis() as u64,
                accounts_registry: HashMap::new(),
                accounts_by_owner: HashMap::new(),
                account_associated_data: HashMap::new(),
                token_accounts: HashMap::new(),
                token_accounts_by_owner: HashMap::new(),
                token_accounts_by_delegate: HashMap::new(),
                token_accounts_by_mint: HashMap::new(),
                total_supply: 0,
                circulating_supply: 0,
                non_circulating_supply: 0,
                non_circulating_accounts: Vec::new(),
                genesis_config: GenesisConfig::default(),
                inflation: Inflation::default(),
            },
            simnet_events_rx,
            geyser_events_rx,
        )
    }

    /// Initializes the SVM with the provided epoch info and optionally notifies about remote connection.
    ///
    /// Updates the internal epoch info, sends connection and epoch update events, and sets the clock sysvar.
    ///
    /// # Arguments
    /// * `epoch_info` - The epoch information to initialize with.
    /// * `remote_ctx` - Optional remote client context for event notification.
    ///
    pub fn initialize(&mut self, epoch_info: EpochInfo, remote_ctx: &Option<SurfnetRemoteClient>) {
        self.latest_epoch_info = epoch_info.clone();
        self.updated_at = Utc::now().timestamp_millis() as u64;

        if let Some(remote_client) = remote_ctx {
            let _ = self
                .simnet_events_tx
                .send(SimnetEvent::Connected(remote_client.client.url()));
        }
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));

        let clock: Clock = Clock {
            slot: self.latest_epoch_info.absolute_slot,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp: Utc::now().timestamp(),
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };
        self.inner.set_sysvar(&clock);
    }

    /// Airdrops a specified amount of lamports to a single public key.
    ///
    /// # Arguments
    /// * `pubkey` - The recipient public key.
    /// * `lamports` - The amount of lamports to airdrop.
    ///
    /// # Returns
    /// A `TransactionResult` indicating success or failure.
    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> TransactionResult {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let res = self.inner.airdrop(pubkey, lamports);
        if let Ok(ref tx_result) = res {
            let airdrop_keypair = Keypair::new();
            let slot = self.latest_epoch_info.absolute_slot;
            self.transactions.insert(
                tx_result.signature,
                SurfnetTransactionStatus::Processed(Box::new(TransactionWithStatusMeta(
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
                    convert_transaction_metadata_from_canonical(tx_result),
                    None,
                ))),
            );
            let account = self.inner.get_account(pubkey).unwrap();
            let _ = self.set_account(pubkey, account);
        }
        res
    }

    /// Airdrops a specified amount of lamports to a list of public keys.
    ///
    /// # Arguments
    /// * `lamports` - The amount of lamports to airdrop.
    /// * `addresses` - Slice of recipient public keys.
    pub fn airdrop_pubkeys(&mut self, lamports: u64, addresses: &[Pubkey]) {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        for recipient in addresses.iter() {
            let _ = self.airdrop(recipient, lamports);
            let _ = self.simnet_events_tx.send(SimnetEvent::info(format!(
                "Genesis airdrop successful {}: {}",
                recipient, lamports
            )));
        }
    }

    /// Returns the latest known absolute slot from the local epoch info.
    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.latest_epoch_info.absolute_slot
    }

    /// Returns the latest blockhash known by the SVM.
    pub fn latest_blockhash(&self) -> solana_hash::Hash {
        self.inner.latest_blockhash()
    }

    /// Returns the latest epoch info known by the `SurfnetSvm`.
    pub fn latest_epoch_info(&self) -> EpochInfo {
        self.latest_epoch_info.clone()
    }

    /// Generates and sets a new blockhash, updating the RecentBlockhashes sysvar.
    ///
    /// # Returns
    /// A new `BlockIdentifier` for the updated blockhash.
    #[allow(deprecated)]
    fn new_blockhash(&mut self) -> BlockIdentifier {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        // cache the current blockhashes
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

        let mut slot_hashes = self
            .inner
            .get_sysvar::<solana_sdk::sysvar::slot_hashes::SlotHashes>();
        slot_hashes.add(self.get_latest_absolute_slot() + 1, latest_entry.blockhash);

        self.inner
            .set_sysvar(&solana_sdk::sysvar::slot_hashes::SlotHashes::new(
                &slot_hashes,
            ));

        BlockIdentifier::new(
            self.chain_tip.index + 1,
            latest_entry.blockhash.to_string().as_str(),
        )
    }

    /// Checks if the provided blockhash is recent (present in the RecentBlockhashes sysvar).
    ///
    /// # Arguments
    /// * `recent_blockhash` - The blockhash to check.
    ///
    /// # Returns
    /// `true` if the blockhash is recent, `false` otherwise.
    pub fn check_blockhash_is_recent(&self, recent_blockhash: &Hash) -> bool {
        #[allow(deprecated)]
        self.inner
            .get_sysvar::<solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes>()
            .iter()
            .any(|entry| entry.blockhash == *recent_blockhash)
    }

    /// Sets an account in the local SVM state and notifies listeners.
    ///
    /// # Arguments
    /// * `pubkey` - The public key of the account.
    /// * `account` - The [Account] to insert.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if the operation fails.
    pub fn set_account(&mut self, pubkey: &Pubkey, account: Account) -> SurfpoolResult<()> {
        self.updated_at = Utc::now().timestamp_millis() as u64;

        self.inner
            .set_account(*pubkey, account.clone())
            .map_err(|e| SurfpoolError::set_account(*pubkey, e))?;

        // Update the account registries and indexes
        self.update_account_registries(pubkey, &account);

        // Notify account subscribers
        self.notify_account_subscribers(pubkey, &account);

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::account_update(*pubkey));
        Ok(())
    }

    pub fn update_account_registries(&mut self, pubkey: &Pubkey, account: &Account) {
        // only if successful, update our indexes
        if let Some(old_account) = self.accounts_registry.get(pubkey).cloned() {
            self.remove_from_indexes(pubkey, &old_account);
        }

        // update the main registry
        self.accounts_registry.insert(*pubkey, account.clone());

        // add to owner index (check for duplicates)
        let owner_accounts = self.accounts_by_owner.entry(account.owner).or_default();
        if !owner_accounts.contains(pubkey) {
            owner_accounts.push(*pubkey);
        }

        // if it's a token account, update token-specific indexes
        if account.owner == spl_token::id() || account.owner == spl_token_2022::id() {
            if let Ok(token_account) = TokenAccount::unpack(&account.data) {
                self.token_accounts.insert(*pubkey, token_account);

                // index by owner -> check for duplicates
                let token_owner_accounts = self
                    .token_accounts_by_owner
                    .entry(token_account.owner)
                    .or_default();

                if !token_owner_accounts.contains(pubkey) {
                    token_owner_accounts.push(*pubkey);
                }

                // index by mint -> check for duplicates
                let mint_accounts = self
                    .token_accounts_by_mint
                    .entry(token_account.mint)
                    .or_default();

                if !mint_accounts.contains(pubkey) {
                    mint_accounts.push(*pubkey);
                }

                if let COption::Some(delegate) = token_account.delegate {
                    let delegate_accounts =
                        self.token_accounts_by_delegate.entry(delegate).or_default();
                    if !delegate_accounts.contains(pubkey) {
                        delegate_accounts.push(*pubkey);
                    }
                }
            }
        }

        if account.owner == spl_token_2022::id() {
            if let Ok(mint) =
                StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&account.data)
            {
                let unix_timestamp = self.inner.get_sysvar::<Clock>().unix_timestamp;
                let interest_bearing_config = mint
                    .get_extension::<InterestBearingConfig>()
                    .map(|x| (*x, unix_timestamp))
                    .ok();
                let scaled_ui_amount_config = mint
                    .get_extension::<ScaledUiAmountConfig>()
                    .map(|x| (*x, unix_timestamp))
                    .ok();
                self.account_associated_data.insert(
                    *pubkey,
                    AccountAdditionalDataV3 {
                        spl_token_additional_data: Some(SplTokenAdditionalDataV2 {
                            decimals: mint.base.decimals,
                            interest_bearing_config,
                            scaled_ui_amount_config,
                        }),
                    },
                );
            };
        }
    }

    fn remove_from_indexes(&mut self, pubkey: &Pubkey, old_account: &Account) {
        if let Some(accounts) = self.accounts_by_owner.get_mut(&old_account.owner) {
            accounts.retain(|pk| pk != pubkey);
            if accounts.is_empty() {
                self.accounts_by_owner.remove(&old_account.owner);
            }
        }

        // if it was a token account, remove from token indexes
        if old_account.owner == spl_token::id() {
            if let Some(old_token_account) = self.token_accounts.remove(pubkey) {
                if let Some(accounts) = self
                    .token_accounts_by_owner
                    .get_mut(&old_token_account.owner)
                {
                    accounts.retain(|pk| pk != pubkey);
                    if accounts.is_empty() {
                        self.token_accounts_by_owner
                            .remove(&old_token_account.owner);
                    }
                }

                if let Some(accounts) = self.token_accounts_by_mint.get_mut(&old_token_account.mint)
                {
                    accounts.retain(|pk| pk != pubkey);
                    if accounts.is_empty() {
                        self.token_accounts_by_mint.remove(&old_token_account.mint);
                    }
                }

                if let COption::Some(delegate) = old_token_account.delegate {
                    if let Some(accounts) = self.token_accounts_by_delegate.get_mut(&delegate) {
                        accounts.retain(|pk| pk != pubkey);
                        if accounts.is_empty() {
                            self.token_accounts_by_delegate.remove(&delegate);
                        }
                    }
                }
            }
        }
    }

    /// Sends a transaction to the system for execution.
    ///
    /// This function attempts to send a transaction to the blockchain. It first increments the `transactions_processed` counter.
    /// Then it sends the transaction to the system and updates its status. If the transaction is successfully processed, it is
    /// cached locally, and a "transaction processed" event is sent. If the transaction fails, the error is recorded and an event
    /// is sent indicating the failure.
    ///
    /// # Arguments
    /// * `tx` - The transaction to send.
    /// * `cu_analysis_enabled` - Whether compute unit analysis is enabled.
    ///
    /// # Returns
    /// `Ok(res)` if processed successfully, or `Err(tx_failure)` if failed.
    pub fn send_transaction(
        &mut self,
        tx: VersionedTransaction,
        cu_analysis_enabled: bool,
    ) -> TransactionResult {
        if tx.verify_with_results().iter().any(|valid| !*valid) {
            return Err(FailedTransactionMetadata {
                err: TransactionError::SignatureFailure,
                meta: TransactionMetadata::default(),
            });
        }

        if cu_analysis_enabled {
            let estimation_result = self.estimate_compute_units(&tx);
            let _ =
                self.simnet_events_tx.try_send(SimnetEvent::info(format!(
                "CU Estimation for tx: {} | Consumed: {} | Success: {} | Logs: {:?} | Error: {:?}",
                tx.signatures.first().map_or_else(|| "N/A".to_string(), |s| s.to_string()),
                estimation_result.compute_units_consumed,
                estimation_result.success,
                estimation_result.log_messages,
                estimation_result.error_message
            )));
        }
        self.updated_at = Utc::now().timestamp_millis() as u64;
        self.transactions_processed += 1;

        if !self.check_blockhash_is_recent(tx.message.recent_blockhash()) {
            let meta = TransactionMetadata::default();
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
        self.inner.set_blockhash_check(false);

        match self.inner.send_transaction(tx.clone()) {
            Ok(res) => {
                let transaction_meta = convert_transaction_metadata_from_canonical(&res);

                self.transactions.insert(
                    transaction_meta.signature,
                    SurfnetTransactionStatus::Processed(Box::new(TransactionWithStatusMeta(
                        self.get_latest_absolute_slot(),
                        tx,
                        transaction_meta.clone(),
                        None,
                    ))),
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

    /// Estimates the compute units that a transaction will consume by simulating it.
    ///
    /// Does not commit any state changes to the SVM.
    ///
    /// # Arguments
    /// * `transaction` - The transaction to simulate.
    ///
    /// # Returns
    /// A `ComputeUnitsEstimationResult` with simulation details.
    pub fn estimate_compute_units(
        &self,
        transaction: &VersionedTransaction,
    ) -> ComputeUnitsEstimationResult {
        if !self.check_blockhash_is_recent(transaction.message.recent_blockhash()) {
            return ComputeUnitsEstimationResult {
                success: false,
                compute_units_consumed: 0,
                log_messages: None,
                error_message: Some(
                    solana_transaction_error::TransactionError::BlockhashNotFound.to_string(),
                ),
            };
        }

        match self.inner.simulate_transaction(transaction.clone()) {
            Ok(sim_info) => ComputeUnitsEstimationResult {
                success: true,
                compute_units_consumed: sim_info.meta.compute_units_consumed,
                log_messages: Some(sim_info.meta.logs),
                error_message: None,
            },
            Err(failed_meta) => ComputeUnitsEstimationResult {
                success: false,
                compute_units_consumed: failed_meta.meta.compute_units_consumed,
                log_messages: Some(failed_meta.meta.logs),
                error_message: Some(failed_meta.err.to_string()),
            },
        }
    }

    /// Simulates a transaction and returns detailed simulation info or failure metadata.
    ///
    /// # Arguments
    /// * `tx` - The transaction to simulate.
    ///
    /// # Returns
    /// `Ok(SimulatedTransactionInfo)` if successful, or `Err(FailedTransactionMetadata)` if failed.
    pub fn simulate_transaction(
        &self,
        tx: VersionedTransaction,
        sigverify: bool,
    ) -> Result<SimulatedTransactionInfo, FailedTransactionMetadata> {
        if sigverify && tx.verify_with_results().iter().any(|valid| !*valid) {
            return Err(FailedTransactionMetadata {
                err: TransactionError::SignatureFailure,
                meta: TransactionMetadata::default(),
            });
        }

        if !self.check_blockhash_is_recent(tx.message.recent_blockhash()) {
            let meta = TransactionMetadata::default();
            let err = TransactionError::BlockhashNotFound;

            return Err(FailedTransactionMetadata { err, meta });
        }
        self.inner.simulate_transaction(tx)
    }

    /// Confirms transactions queued for confirmation, updates epoch/slot, and sends events.
    ///
    /// # Returns
    /// `Ok(Vec<Signature>)` with confirmed signatures, or `Err(SurfpoolError)` on error.
    fn confirm_transactions(&mut self) -> Result<Vec<Signature>, SurfpoolError> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let mut confirmed_transactions = vec![];
        let slot = self.latest_epoch_info.slot_index;

        while let Some((tx, status_tx)) = self.transactions_queued_for_confirmation.pop_front() {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
            let signature = tx.signatures[0];
            let finalized_at = self.latest_epoch_info.absolute_slot + FINALIZATION_SLOT_THRESHOLD;
            self.transactions_queued_for_finalization
                .push_back((finalized_at, tx, status_tx));

            self.notify_signature_subscribers(
                SignatureSubscriptionType::confirmed(),
                &signature,
                slot,
                None,
            );
            confirmed_transactions.push(signature);
        }

        Ok(confirmed_transactions)
    }

    /// Finalizes transactions queued for finalization, sending finalized events as needed.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(SurfpoolError)` on error.
    fn finalize_transactions(&mut self) -> Result<(), SurfpoolError> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let current_slot = self.latest_epoch_info.absolute_slot;
        let mut requeue = VecDeque::new();
        while let Some((finalized_at, tx, status_tx)) =
            self.transactions_queued_for_finalization.pop_front()
        {
            if current_slot >= finalized_at {
                let _ = status_tx.try_send(TransactionStatusEvent::Success(
                    TransactionConfirmationStatus::Finalized,
                ));
                self.notify_signature_subscribers(
                    SignatureSubscriptionType::finalized(),
                    &tx.signatures[0],
                    self.latest_epoch_info.absolute_slot,
                    None,
                );
            } else {
                requeue.push_back((finalized_at, tx, status_tx));
            }
        }
        // Requeue any transactions that are not yet finalized
        self.transactions_queued_for_finalization
            .append(&mut requeue);

        Ok(())
    }

    /// Notifies listeners of an invalid transaction and sends a verification failure event.
    ///
    /// # Arguments
    /// * `signature` - The transaction signature.
    /// * `status_tx` - The status event sender.
    pub fn notify_invalid_transaction(
        &self,
        signature: Signature,
        status_tx: Sender<TransactionStatusEvent>,
    ) {
        let _ = self.simnet_events_tx.try_send(SimnetEvent::error(format!(
            "Transaction verification failed: {}",
            signature
        )));
        let _ = status_tx.try_send(TransactionStatusEvent::VerificationFailure(
            signature.to_string(),
        ));
    }

    /// Writes account updates to the SVM state based on the provided account update result.
    ///
    /// # Arguments
    /// * `account_update` - The account update result to process.
    pub fn write_account_update(&mut self, account_update: GetAccountResult) {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        match account_update {
            GetAccountResult::FoundAccount(pubkey, account, do_update_account) => {
                if do_update_account {
                    if let Err(e) = self.set_account(&pubkey, account.clone()) {
                        let _ = self
                            .simnet_events_tx
                            .send(SimnetEvent::error(e.to_string()));
                    }
                }
            }
            GetAccountResult::FoundProgramAccount((pubkey, account), (_, None)) => {
                if let Err(e) = self.set_account(&pubkey, account.clone()) {
                    let _ = self
                        .simnet_events_tx
                        .send(SimnetEvent::error(e.to_string()));
                }
            }
            GetAccountResult::FoundProgramAccount(
                (pubkey, account),
                (data_pubkey, Some(data_account)),
            ) => {
                // The data account _must_ be set first, as the program account depends on it.
                if let Err(e) = self.set_account(&data_pubkey, data_account.clone()) {
                    let _ = self
                        .simnet_events_tx
                        .send(SimnetEvent::error(e.to_string()));
                }
                if let Err(e) = self.set_account(&pubkey, account.clone()) {
                    let _ = self
                        .simnet_events_tx
                        .send(SimnetEvent::error(e.to_string()));
                }
            }
            GetAccountResult::None(_) => {}
        }
    }

    pub fn confirm_current_block(&mut self) -> Result<(), SurfpoolError> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        // Confirm processed transactions
        let confirmed_signatures = self.confirm_transactions()?;
        let num_transactions = confirmed_signatures.len() as u64;

        let previous_chain_tip = self.chain_tip.clone();
        self.chain_tip = self.new_blockhash();

        self.blocks.insert(
            self.get_latest_absolute_slot(),
            BlockHeader {
                hash: self.chain_tip.hash.clone(),
                previous_blockhash: previous_chain_tip.hash.clone(),
                block_time: chrono::Utc::now().timestamp_millis(),
                block_height: self.chain_tip.index,
                parent_slot: self.get_latest_absolute_slot(),
                signatures: confirmed_signatures,
            },
        );

        if self.perf_samples.len() > 30 {
            self.perf_samples.pop_back();
        }
        self.perf_samples.push_front(RpcPerfSample {
            slot: self.latest_epoch_info.slot_index,
            num_slots: 1,
            sample_period_secs: 1,
            num_transactions,
            num_non_vote_transactions: None,
        });

        self.latest_epoch_info.slot_index += 1;
        self.latest_epoch_info.block_height = self.chain_tip.index;
        self.latest_epoch_info.absolute_slot += 1;
        if self.latest_epoch_info.slot_index > self.latest_epoch_info.slots_in_epoch {
            self.latest_epoch_info.slot_index = 0;
            self.latest_epoch_info.epoch += 1;
        }

        let parent_slot = self.latest_epoch_info.absolute_slot.saturating_sub(1);
        let new_slot = self.latest_epoch_info.absolute_slot;
        let root = new_slot.saturating_sub(FINALIZATION_SLOT_THRESHOLD);
        self.notify_slot_subscribers(new_slot, parent_slot, root);

        let clock: Clock = Clock {
            slot: self.latest_epoch_info.absolute_slot,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp: Utc::now().timestamp(),
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::ClockUpdate(clock.clone()));
        self.inner.set_sysvar(&clock);

        self.finalize_transactions()?;

        Ok(())
    }

    /// Subscribes for updates on a transaction signature for a given subscription type.
    ///
    /// # Arguments
    /// * `signature` - The transaction signature to subscribe to.
    /// * `subscription_type` - The type of subscription (confirmed/finalized).
    ///
    /// # Returns
    /// A receiver for slot and transaction error updates.
    pub fn subscribe_for_signature_updates(
        &mut self,
        signature: &Signature,
        subscription_type: SignatureSubscriptionType,
    ) -> Receiver<(Slot, Option<TransactionError>)> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let (tx, rx) = unbounded();
        self.signature_subscriptions
            .entry(*signature)
            .or_default()
            .push((subscription_type, tx));
        rx
    }

    pub fn subscribe_for_account_updates(
        &mut self,
        account_pubkey: &Pubkey,
        encoding: Option<UiAccountEncoding>,
    ) -> Receiver<UiAccount> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let (tx, rx) = unbounded();
        self.account_subscriptions
            .entry(*account_pubkey)
            .or_default()
            .push((encoding, tx));
        rx
    }

    /// Notifies signature subscribers of a status update, sending slot and error info.
    ///
    /// # Arguments
    /// * `status` - The subscription type (confirmed/finalized).
    /// * `signature` - The transaction signature.
    /// * `slot` - The slot number.
    /// * `err` - Optional transaction error.
    pub fn notify_signature_subscribers(
        &mut self,
        status: SignatureSubscriptionType,
        signature: &Signature,
        slot: Slot,
        err: Option<TransactionError>,
    ) {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let mut remaining = vec![];
        if let Some(subscriptions) = self.signature_subscriptions.remove(signature) {
            for (subscription_type, tx) in subscriptions {
                if status.eq(&subscription_type) {
                    if tx.send((slot, err.clone())).is_err() {
                        // The receiver has been dropped, so we can skip notifying
                        continue;
                    }
                } else {
                    remaining.push((subscription_type, tx));
                }
            }
            if !remaining.is_empty() {
                self.signature_subscriptions.insert(*signature, remaining);
            }
        }
    }

    pub fn notify_account_subscribers(
        &mut self,
        account_updated_pubkey: &Pubkey,
        account: &Account,
    ) {
        let mut remaining = vec![];
        if let Some(subscriptions) = self.account_subscriptions.remove(account_updated_pubkey) {
            for (encoding, tx) in subscriptions {
                let account = encode_ui_account(
                    account_updated_pubkey,
                    account,
                    encoding.unwrap_or(UiAccountEncoding::Base64),
                    None,
                    None,
                );
                if tx.send(account).is_err() {
                    // The receiver has been dropped, so we can skip notifying
                    continue;
                } else {
                    remaining.push((encoding, tx));
                }
            }
            if !remaining.is_empty() {
                self.account_subscriptions
                    .insert(*account_updated_pubkey, remaining);
            }
        }
    }

    /// Retrieves a confirmed block at the given slot, including transactions and metadata.
    ///
    /// # Arguments
    /// * `slot` - The slot number to retrieve the block for.
    ///
    /// # Returns
    /// `Some(UiConfirmedBlock)` if found, or `None` if not present.
    pub fn get_block_at_slot(&self, slot: Slot) -> Option<UiConfirmedBlock> {
        // Retrieve block
        let block = self.blocks.get(&slot)?;

        // Retrieve parent block

        // Retrieve transactions
        let mut transactions = vec![];
        for signature in block.signatures.iter() {
            let Some(TransactionWithStatusMeta(_slot, tx, _meta, _err)) = self
                .transactions
                .get(signature)
                .map(|t| t.expect_processed().clone())
            else {
                continue;
            };

            let (header, account_keys, instructions) = match &tx.message {
                VersionedMessage::Legacy(message) => (
                    message.header,
                    message.account_keys.iter().map(|k| k.to_string()).collect(),
                    message
                        .instructions
                        .iter()
                        // TODO: use stack height
                        .map(|ix| UiCompiledInstruction::from(ix, None))
                        .collect(),
                ),
                VersionedMessage::V0(message) => (
                    message.header,
                    message.account_keys.iter().map(|k| k.to_string()).collect(),
                    message
                        .instructions
                        .iter()
                        // TODO: use stack height
                        .map(|ix| UiCompiledInstruction::from(ix, None))
                        .collect(),
                ),
            };

            let transaction = EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: tx.signatures.iter().map(|s| s.to_string()).collect(),
                    message: UiMessage::Raw(UiRawMessage {
                        header,
                        account_keys,
                        recent_blockhash: tx.get_recent_blockhash().to_string(),
                        instructions,
                        address_table_lookups: match tx.message {
                            VersionedMessage::Legacy(_) => None,
                            VersionedMessage::V0(ref msg) => Some(
                                msg.address_table_lookups
                                    .iter()
                                    .map(UiAddressTableLookup::from)
                                    .collect::<Vec<UiAddressTableLookup>>(),
                            ),
                        },
                    }),
                }),
                meta: None,
                version: None,
            };
            // let transaction = EncodedTransaction::Json(UiTransaction::from(res));
            transactions.push(transaction);
        }

        // Construct block
        let block = UiConfirmedBlock {
            blockhash: block.hash.clone(),
            previous_blockhash: block.previous_blockhash.clone(),
            rewards: None,
            num_reward_partitions: None,
            block_time: Some(block.block_time),
            block_height: Some(block.block_height),
            parent_slot: block.parent_slot,
            transactions: Some(transactions),
            signatures: Some(block.signatures.iter().map(|t| t.to_string()).collect()),
        };

        Some(block)
    }

    /// Gets all accounts owned by a specific program ID from the account registry.
    ///
    /// # Arguments
    ///
    /// * `program_id` - The program ID to search for owned accounts.
    ///
    /// # Returns
    ///
    /// * A vector of (account_pubkey, account) tuples for all accounts owned by the program.
    pub fn get_account_owned_by(&self, program_id: Pubkey) -> Vec<(Pubkey, Account)> {
        if let Some(account_pubkeys) = self.accounts_by_owner.get(&program_id) {
            account_pubkeys
                .iter()
                .filter_map(|pubkey| {
                    self.accounts_registry
                        .get(pubkey)
                        .map(|account| (*pubkey, account.clone()))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Gets all token accounts that have delegated authority to a specific delegate.
    ///
    /// # Arguments
    ///
    /// * `delegate` - The delegate pubkey to search for token accounts that have granted authority.
    ///
    /// # Returns
    ///
    /// * A vector of (account_pubkey, token_account) tuples for all token accounts delegated to the specified delegate.
    pub fn get_token_accounts_by_delegate(&self, delegate: &Pubkey) -> Vec<(Pubkey, TokenAccount)> {
        if let Some(account_pubkeys) = self.token_accounts_by_delegate.get(delegate) {
            account_pubkeys
                .iter()
                .filter_map(|pk| self.token_accounts.get(pk).map(|ta| (*pk, *ta)))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Gets all token accounts owned by a specific owner.
    ///
    /// # Arguments
    ///
    /// * `owner` - The owner pubkey to search for token accounts.
    ///
    /// # Returns
    ///
    /// * A vector of (account_pubkey, token_account) tuples for all token accounts owned by the specified owner.
    pub fn get_parsed_token_accounts_by_owner(
        &self,
        owner: &Pubkey,
    ) -> Vec<(Pubkey, TokenAccount)> {
        if let Some(account_pubkeys) = self.token_accounts_by_owner.get(owner) {
            account_pubkeys
                .iter()
                .filter_map(|pk| self.token_accounts.get(pk).map(|ta| (*pk, *ta)))
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_token_accounts_by_owner(&self, owner: &Pubkey) -> Vec<(Pubkey, Account)> {
        self.token_accounts_by_owner
            .get(owner)
            .map(|account_pubkeys| {
                account_pubkeys
                    .iter()
                    .filter_map(|pk| {
                        self.accounts_registry
                            .get(pk)
                            .map(|account| (*pk, account.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets all token accounts for a specific mint (token type).
    ///
    /// # Arguments
    ///
    /// * `mint` - The mint pubkey to search for token accounts.
    ///
    /// # Returns
    ///
    /// * A vector of (account_pubkey, token_account) tuples for all token accounts of the specified mint.
    pub fn get_token_accounts_by_mint(&self, mint: &Pubkey) -> Vec<(Pubkey, TokenAccount)> {
        if let Some(account_pubkeys) = self.token_accounts_by_mint.get(mint) {
            account_pubkeys
                .iter()
                .filter_map(|pk| self.token_accounts.get(pk).map(|ta| (*pk, *ta)))
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn subscribe_for_slot_updates(&mut self) -> Receiver<SlotInfo> {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        let (tx, rx) = unbounded();
        self.slot_subscriptions.push(tx);
        rx
    }

    pub fn notify_slot_subscribers(&mut self, slot: Slot, parent: Slot, root: Slot) {
        self.updated_at = Utc::now().timestamp_millis() as u64;
        self.slot_subscriptions
            .retain(|tx| tx.send(SlotInfo { slot, parent, root }).is_ok());
    }
}

#[cfg(test)]
mod tests {
    // use test_log::test; // uncomment to get logs from litesvm
    use solana_account::Account;
    use solana_sdk::{
        bpf_loader_upgradeable::{self, get_program_data_address},
        program_pack::Pack,
    };
    use spl_token::state::{Account as TokenAccount, AccountState};

    use super::*;

    #[test]
    fn test_token_account_indexing() {
        let (mut svm, _events_rx, _geyser_rx) = SurfnetSvm::new();

        let owner = Pubkey::new_unique();
        let delegate = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let token_account_pubkey = Pubkey::new_unique();

        // create a token account with delegate
        let mut token_account_data = [0u8; TokenAccount::LEN];
        let token_account = TokenAccount {
            mint,
            owner,
            amount: 1000,
            delegate: COption::Some(delegate),
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 500,
            close_authority: COption::None,
        };
        token_account.pack_into_slice(&mut token_account_data);

        let account = Account {
            lamports: 1000000,
            data: token_account_data.to_vec(),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&token_account_pubkey, account).unwrap();

        // test all indexes were created correctly
        assert_eq!(svm.accounts_registry.len(), 1);
        assert_eq!(svm.token_accounts.len(), 1);

        // test owner index
        let owner_accounts = svm.get_parsed_token_accounts_by_owner(&owner);
        assert_eq!(owner_accounts.len(), 1);
        assert_eq!(owner_accounts[0].0, token_account_pubkey);

        // test delegate index
        let delegate_accounts = svm.get_token_accounts_by_delegate(&delegate);
        assert_eq!(delegate_accounts.len(), 1);
        assert_eq!(delegate_accounts[0].0, token_account_pubkey);

        // test mint index
        let mint_accounts = svm.get_token_accounts_by_mint(&mint);
        assert_eq!(mint_accounts.len(), 1);
        assert_eq!(mint_accounts[0].0, token_account_pubkey);
    }

    #[test]
    fn test_account_update_removes_old_indexes() {
        let (mut svm, _events_rx, _geyser_rx) = SurfnetSvm::new();

        let owner = Pubkey::new_unique();
        let old_delegate = Pubkey::new_unique();
        let new_delegate = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let token_account_pubkey = Pubkey::new_unique();

        //  reate initial token account with old delegate
        let mut token_account_data = [0u8; TokenAccount::LEN];
        let token_account = TokenAccount {
            mint,
            owner,
            amount: 1000,
            delegate: COption::Some(old_delegate),
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 500,
            close_authority: COption::None,
        };
        token_account.pack_into_slice(&mut token_account_data);

        let account = Account {
            lamports: 1000000,
            data: token_account_data.to_vec(),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        // insert initial account
        svm.set_account(&token_account_pubkey, account).unwrap();

        // verify old delegate has the account
        assert_eq!(svm.get_token_accounts_by_delegate(&old_delegate).len(), 1);
        assert_eq!(svm.get_token_accounts_by_delegate(&new_delegate).len(), 0);

        // update with new delegate
        let updated_token_account = TokenAccount {
            mint,
            owner,
            amount: 1000,
            delegate: COption::Some(new_delegate),
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 500,
            close_authority: COption::None,
        };
        updated_token_account.pack_into_slice(&mut token_account_data);

        let updated_account = Account {
            lamports: 1000000,
            data: token_account_data.to_vec(),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        // update the account
        svm.set_account(&token_account_pubkey, updated_account)
            .unwrap();

        // verify indexes were updated correctly
        assert_eq!(svm.get_token_accounts_by_delegate(&old_delegate).len(), 0);
        assert_eq!(svm.get_token_accounts_by_delegate(&new_delegate).len(), 1);
        assert_eq!(svm.get_parsed_token_accounts_by_owner(&owner).len(), 1);
    }

    #[test]
    fn test_non_token_accounts_not_indexed() {
        let (mut svm, _events_rx, _geyser_rx) = SurfnetSvm::new();

        let system_account_pubkey = Pubkey::new_unique();
        let account = Account {
            lamports: 1000000,
            data: vec![],
            owner: solana_system_interface::program::id(), // system program, not token program
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&system_account_pubkey, account).unwrap();

        // should be in general registry but not token indexes
        assert_eq!(svm.accounts_registry.len(), 1);
        assert_eq!(svm.token_accounts.len(), 0);
        assert_eq!(svm.token_accounts_by_owner.len(), 0);
        assert_eq!(svm.token_accounts_by_delegate.len(), 0);
        assert_eq!(svm.token_accounts_by_mint.len(), 0);
    }

    fn expect_account_update_event(
        events_rx: &Receiver<SimnetEvent>,
        svm: &SurfnetSvm,
        pubkey: &Pubkey,
        expected_account: &Account,
    ) -> bool {
        match events_rx.recv() {
            Ok(event) => match event {
                SimnetEvent::AccountUpdate(_, account_pubkey) => {
                    assert_eq!(pubkey, &account_pubkey);
                    assert_eq!(svm.accounts_registry.get(&pubkey), Some(expected_account));
                    true
                }
                event => {
                    println!("unexpected simnet event: {:?}", event);
                    false
                }
            },
            Err(_) => false,
        }
    }

    fn expect_error_event(events_rx: &Receiver<SimnetEvent>, expected_error: &str) -> bool {
        match events_rx.recv() {
            Ok(event) => match event {
                SimnetEvent::ErrorLog(_, err) => {
                    assert_eq!(err, expected_error);

                    true
                }
                event => {
                    println!("unexpected simnet event: {:?}", event);
                    false
                }
            },
            Err(_) => false,
        }
    }

    fn create_program_accounts() -> (Pubkey, Account, Pubkey, Account) {
        let program_pubkey = Pubkey::new_unique();
        let program_data_address = get_program_data_address(&program_pubkey);
        let program_account = Account {
            lamports: 1000000000000,
            data: bincode::serialize(&bpf_loader_upgradeable::UpgradeableLoaderState::Program {
                programdata_address: program_data_address,
            })
            .unwrap(),
            owner: bpf_loader_upgradeable::ID,
            executable: true,
            rent_epoch: 10000000000000,
        };

        let mut bin = include_bytes!("../tests/assets/metaplex_program.bin").to_vec();
        let mut data = bincode::serialize(
            &bpf_loader_upgradeable::UpgradeableLoaderState::ProgramData {
                slot: 0,
                upgrade_authority_address: Some(Pubkey::new_unique()),
            },
        )
        .unwrap();
        data.append(&mut bin); // push our binary after the state data
        let program_data_account = Account {
            lamports: 10000000000000,
            data,
            owner: bpf_loader_upgradeable::ID,
            executable: false,
            rent_epoch: 10000000000000,
        };
        (
            program_pubkey,
            program_account,
            program_data_address,
            program_data_account,
        )
    }

    #[test]
    fn test_inserting_account_updates() {
        let (mut svm, events_rx, _geyser_rx) = SurfnetSvm::new();

        let pubkey = Pubkey::new_unique();
        let account = Account {
            lamports: 1000,
            data: vec![1, 2, 3],
            owner: Pubkey::new_unique(),
            executable: false,
            rent_epoch: 0,
        };

        // GetAccountResult::None should be a noop when writing account updates
        {
            let index_before = svm.accounts_registry.clone();
            let empty_update = GetAccountResult::None(pubkey);
            svm.write_account_update(empty_update);
            assert_eq!(svm.accounts_registry, index_before);
        }

        // GetAccountResult::FoundAccount with `DoUpdateSvm` flag to false should be a noop
        {
            let index_before = svm.accounts_registry.clone();
            let found_update = GetAccountResult::FoundAccount(pubkey, account.clone(), false);
            svm.write_account_update(found_update);
            assert_eq!(svm.accounts_registry, index_before);
        }

        // GetAccountResult::FoundAccount with `DoUpdateSvm` flag to true should update the account
        {
            let index_before = svm.accounts_registry.clone();
            let found_update = GetAccountResult::FoundAccount(pubkey, account.clone(), true);
            svm.write_account_update(found_update);
            assert_eq!(svm.accounts_registry.len(), index_before.len() + 1);
            if !expect_account_update_event(&events_rx, &svm, &pubkey, &account) {
                panic!("Expected account update event not received after GetAccountResult::FoundAccount update");
            }
        }

        // GetAccountResult::FoundProgramAccount with no program account fails
        {
            let (program_address, program_account, program_data_address, _) =
                create_program_accounts();

            let index_before = svm.accounts_registry.clone();
            let found_program_account_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, None),
            );
            svm.write_account_update(found_program_account_update);

            if !expect_error_event(&events_rx, &format!("Internal error: \"Failed to set account {}: An account required by the instruction is missing\"", program_address)) {
                panic!("Expected error event not received after inserting program account with no program data account");
            }
            assert_eq!(svm.accounts_registry, index_before);
        }

        // GetAccountResult::FoundProgramAccount with program account + program data account inserts two accounts
        {
            let (program_address, program_account, program_data_address, program_data_account) =
                create_program_accounts();

            let index_before = svm.accounts_registry.clone();
            let found_program_account_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, Some(program_data_account.clone())),
            );
            svm.write_account_update(found_program_account_update);
            assert_eq!(svm.accounts_registry.len(), index_before.len() + 2);
            if !expect_account_update_event(
                &events_rx,
                &svm,
                &program_data_address,
                &program_data_account,
            ) {
                panic!("Expected account update event not received after GetAccountResult::FoundProgramAccount update for program data pubkey");
            }

            if !expect_account_update_event(&events_rx, &svm, &program_address, &program_account) {
                panic!("Expected account update event not received after GetAccountResult::FoundProgramAccount update for program pubkey");
            }
        }

        // If we insert the program data account ahead of time, then have a GetAccountResult::FoundProgramAccount with just the program data account,
        // we should get one insert
        {
            let (program_address, program_account, program_data_address, program_data_account) =
                create_program_accounts();

            let index_before = svm.accounts_registry.clone();
            let found_update = GetAccountResult::FoundAccount(
                program_data_address,
                program_data_account.clone(),
                true,
            );
            svm.write_account_update(found_update);
            assert_eq!(svm.accounts_registry.len(), index_before.len() + 1);
            if !expect_account_update_event(
                &events_rx,
                &svm,
                &program_data_address,
                &program_data_account,
            ) {
                panic!("Expected account update event not received after GetAccountResult::FoundAccount update");
            }

            let index_before = svm.accounts_registry.clone();
            let program_account_found_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, None),
            );
            svm.write_account_update(program_account_found_update);
            assert_eq!(svm.accounts_registry.len(), index_before.len() + 1);
            if !expect_account_update_event(&events_rx, &svm, &program_address, &program_account) {
                panic!("Expected account update event not received after GetAccountResult::FoundAccount update");
            }
        }
    }
}
