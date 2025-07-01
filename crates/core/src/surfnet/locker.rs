use std::{collections::BTreeMap, str::FromStr, sync::Arc};

use crossbeam_channel::{Receiver, Sender};
use itertools::Itertools;
use jsonrpc_core::futures::future::join_all;
use litesvm::types::{FailedTransactionMetadata, SimulatedTransactionInfo, TransactionResult};
use solana_account::Account;
use solana_account_decoder::{
    encode_ui_account,
    parse_account_data::AccountAdditionalDataV3,
    parse_bpf_loader::{parse_bpf_upgradeable_loader, BpfUpgradeableLoaderAccountType, UiProgram},
    parse_token::UiTokenAmount,
    UiAccount, UiAccountEncoding,
};
use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_client::{
    rpc_config::{
        RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcLargestAccountsFilter,
        RpcSignaturesForAddressConfig,
    },
    rpc_filter::RpcFilterType,
    rpc_request::TokenAccountsFilter,
    rpc_response::{
        RpcAccountBalance, RpcConfirmedTransactionStatusWithSignature, RpcKeyedAccount,
        RpcTokenAccountBalance,
    },
};
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_hash::Hash;
use solana_message::{
    v0::{LoadedAddresses, MessageAddressTableLookup},
    VersionedMessage,
};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_runtime::non_circulating_supply;
use solana_sdk::{
    bpf_loader_upgradeable::{get_program_data_address, UpgradeableLoaderState},
    program_pack::Pack,
    transaction::VersionedTransaction,
};
use solana_signature::Signature;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta,
    TransactionConfirmationStatus as SolanaTransactionConfirmationStatus, UiTransactionEncoding,
};
use spl_token::state::Mint;
use spl_token_2022::extension::StateWithExtensions;
use surfpool_types::{
    ComputeUnitsEstimationResult, ProfileResult, ProfileState, SimnetEvent,
    TransactionConfirmationStatus, TransactionStatusEvent,
};
use tokio::sync::RwLock;

use super::{
    remote::{SomeRemoteCtx, SurfnetRemoteClient},
    AccountFactory, GetAccountResult, GetTransactionResult, GeyserEvent, SignatureSubscriptionType,
    SurfnetSvm,
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::{convert_transaction_metadata_from_canonical, verify_pubkey},
    surfnet::FINALIZATION_SLOT_THRESHOLD,
    types::TransactionWithStatusMeta,
};

pub struct SvmAccessContext<T> {
    pub slot: Slot,
    pub latest_epoch_info: EpochInfo,
    pub latest_blockhash: Hash,
    pub inner: T,
}

impl<T> SvmAccessContext<T> {
    pub fn new(slot: Slot, latest_epoch_info: EpochInfo, latest_blockhash: Hash, inner: T) -> Self {
        Self {
            slot,
            latest_blockhash,
            latest_epoch_info,
            inner,
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn with_new_value<N>(&self, inner: N) -> SvmAccessContext<N> {
        SvmAccessContext {
            slot: self.slot,
            latest_blockhash: self.latest_blockhash,
            latest_epoch_info: self.latest_epoch_info.clone(),
            inner,
        }
    }
}

pub type SurfpoolContextualizedResult<T> = SurfpoolResult<SvmAccessContext<T>>;

pub struct SurfnetSvmLocker(pub Arc<RwLock<SurfnetSvm>>);

impl Clone for SurfnetSvmLocker {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Functions for reading and writing to the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Executes a read-only operation on the underlying `SurfnetSvm` by acquiring a blocking read lock.
    /// Accepts a closure that receives a shared reference to `SurfnetSvm` and returns a value.
    ///
    /// # Returns
    /// The result produced by the closure.
    pub fn with_svm_reader<T, F>(&self, reader: F) -> T
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let read_lock = self.0.clone();
        tokio::task::block_in_place(move || {
            let read_guard = read_lock.blocking_read();
            reader(&read_guard)
        })
    }

    /// Executes a read-only operation and wraps the result in `SvmAccessContext`, capturing
    /// slot, epoch info, and blockhash along with the closure's result.
    fn with_contextualized_svm_reader<T, F>(&self, reader: F) -> SvmAccessContext<T>
    where
        F: Fn(&SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let read_lock = self.0.clone();
        tokio::task::block_in_place(move || {
            let read_guard = read_lock.blocking_read();
            let res = reader(&read_guard);

            SvmAccessContext::new(
                read_guard.get_latest_absolute_slot(),
                read_guard.latest_epoch_info(),
                read_guard.latest_blockhash(),
                res,
            )
        })
    }

    /// Executes a write operation on the underlying `SurfnetSvm` by acquiring a blocking write lock.
    /// Accepts a closure that receives a mutable reference to `SurfnetSvm` and returns a value.
    ///
    /// # Returns
    /// The result produced by the closure.
    pub fn with_svm_writer<T, F>(&self, writer: F) -> T
    where
        F: Fn(&mut SurfnetSvm) -> T + Send + Sync,
        T: Send + 'static,
    {
        let write_lock = self.0.clone();
        tokio::task::block_in_place(move || {
            let mut write_guard = write_lock.blocking_write();
            writer(&mut write_guard)
        })
    }
}

/// Functions for creating and initializing the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Constructs a new `SurfnetSvmLocker` wrapping the given `SurfnetSvm` instance.
    pub fn new(svm: SurfnetSvm) -> Self {
        Self(Arc::new(RwLock::new(svm)))
    }

    /// Initializes the locked `SurfnetSvm` by fetching or defaulting epoch info,
    /// then calling its `initialize` method. Returns the epoch info on success.
    pub async fn initialize(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
    ) -> SurfpoolResult<EpochInfo> {
        let epoch_info = if let Some(remote_client) = remote_ctx {
            remote_client.get_epoch_info().await?
        } else {
            EpochInfo {
                epoch: 0,
                slot_index: 0,
                slots_in_epoch: 0,
                absolute_slot: 0,
                block_height: 0,
                transaction_count: None,
            }
        };

        self.with_svm_writer(|svm_writer| {
            svm_writer.initialize(epoch_info.clone(), remote_ctx);
        });
        Ok(epoch_info)
    }
}

/// Functions for getting accounts from the underlying SurfnetSvm instance or remote client
impl SurfnetSvmLocker {
    /// Retrieves a local account from the SVM cache, returning a contextualized result.
    pub fn get_account_local(&self, pubkey: &Pubkey) -> SvmAccessContext<GetAccountResult> {
        self.with_contextualized_svm_reader(|svm_reader| {
            match svm_reader.inner.get_account(pubkey) {
                Some(account) => GetAccountResult::FoundAccount(
                    *pubkey, account,
                    // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                    false,
                ),
                None => GetAccountResult::None(*pubkey),
            }
        })
    }

    /// Attempts local retrieval, then fetches from remote if missing, returning a contextualized result.
    pub async fn get_account_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        pubkey: &Pubkey,
        commitment_config: CommitmentConfig,
    ) -> SurfpoolContextualizedResult<GetAccountResult> {
        let result = self.get_account_local(pubkey);

        if result.inner.is_none() {
            let remote_account = client.get_account(pubkey, commitment_config).await?;
            Ok(result.with_new_value(remote_account))
        } else {
            Ok(result)
        }
    }

    /// Retrieves an account, using local or remote based on context, applying a default factory if provided.
    pub async fn get_account(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        pubkey: &Pubkey,
        factory: Option<AccountFactory>,
    ) -> SurfpoolContextualizedResult<GetAccountResult> {
        let result = if let Some((remote_client, commitment_config)) = remote_ctx {
            self.get_account_local_then_remote(remote_client, pubkey, *commitment_config)
                .await?
        } else {
            self.get_account_local(pubkey)
        };

        match (&result.inner, factory) {
            (&GetAccountResult::None(_), Some(factory)) => {
                let default = factory(self.clone());
                Ok(result.with_new_value(default))
            }
            _ => Ok(result),
        }
    }

    /// Retrieves an account, using local or remote based on context, applying a default factory if provided.
    pub fn get_local_account_associated_data(
        &self,
        account: &GetAccountResult,
    ) -> SvmAccessContext<Option<AccountAdditionalDataV3>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let associated_data = match account {
                GetAccountResult::FoundAccount(_, account, _) => {
                    if !account.owner.eq(&spl_token_2022::id()) {
                        return None;
                    }

                    let Ok(token_data) =
                        StateWithExtensions::<spl_token_2022::state::Account>::unpack(
                            &account.data,
                        )
                    else {
                        return None;
                    };
                    svm_reader
                        .account_associated_data
                        .get(&token_data.base.mint)
                        .map(|e| e.clone())
                }
                _ => None,
            };
            associated_data
        })
    }

    /// Retrieves multiple accounts from local cache, returning a contextualized result.
    pub fn get_multiple_accounts_local(
        &self,
        pubkeys: &[Pubkey],
    ) -> SvmAccessContext<Vec<GetAccountResult>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let mut accounts = vec![];

            for pubkey in pubkeys.iter() {
                let res = match svm_reader.inner.get_account(pubkey) {
                    Some(account) => GetAccountResult::FoundAccount(
                        *pubkey, account,
                        // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                        false,
                    ),
                    None => GetAccountResult::None(*pubkey),
                };
                accounts.push(res);
            }
            accounts
        })
    }

    /// Retrieves multiple accounts, fetching missing ones from remote, returning a contextualized result.
    pub async fn get_multiple_accounts_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        pubkeys: &[Pubkey],
        commitment_config: CommitmentConfig,
    ) -> SurfpoolContextualizedResult<Vec<GetAccountResult>> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: local_results,
        } = self.get_multiple_accounts_local(pubkeys);

        let mut missing_accounts = vec![];
        let mut found_accounts = vec![];
        for result in local_results.into_iter() {
            if let GetAccountResult::None(pubkey) = result {
                missing_accounts.push(pubkey)
            } else {
                found_accounts.push(result.clone());
            }
        }

        if missing_accounts.is_empty() {
            return Ok(SvmAccessContext::new(
                slot,
                latest_epoch_info,
                latest_blockhash,
                found_accounts,
            ));
        }

        let mut remote_results = client
            .get_multiple_accounts(&missing_accounts, commitment_config)
            .await?;
        let mut combined_results = found_accounts.clone();
        combined_results.append(&mut remote_results);

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            combined_results,
        ))
    }

    /// Retrieves multiple accounts, using local or remote context and applying factory defaults if provided.
    pub async fn get_multiple_accounts(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        pubkeys: &[Pubkey],
        factory: Option<AccountFactory>,
    ) -> SurfpoolContextualizedResult<Vec<GetAccountResult>> {
        let results = if let Some((remote_client, commitment_config)) = remote_ctx {
            self.get_multiple_accounts_local_then_remote(remote_client, pubkeys, *commitment_config)
                .await?
        } else {
            self.get_multiple_accounts_local(pubkeys)
        };

        let mut combined = Vec::with_capacity(results.inner.len());
        for result in results.inner.clone() {
            match (&result, &factory) {
                (&GetAccountResult::None(_), Some(factory)) => {
                    let default = factory(self.clone());
                    combined.push(default);
                }
                _ => combined.push(result),
            }
        }
        Ok(results.with_new_value(combined))
    }

    /// Retrieves largest accounts from local cache, returning a contextualized result.
    pub fn get_largest_accounts_local(
        &self,
        config: RpcLargestAccountsConfig,
    ) -> SvmAccessContext<Vec<RpcAccountBalance>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let non_circulating_accounts: Vec<_> = svm_reader
                .non_circulating_accounts
                .iter()
                .flat_map(|acct| verify_pubkey(acct))
                .collect();

            let ordered_accounts = svm_reader
                .accounts_registry
                .iter()
                .sorted_by(|a, b| b.1.lamports.cmp(&a.1.lamports))
                .collect::<Vec<_>>();

            let ordered_filtered_accounts = match config.filter {
                Some(RpcLargestAccountsFilter::NonCirculating) => ordered_accounts
                    .into_iter()
                    .filter(|(pubkey, _)| non_circulating_accounts.contains(pubkey))
                    .collect::<Vec<_>>(),
                Some(RpcLargestAccountsFilter::Circulating) => ordered_accounts
                    .into_iter()
                    .filter(|(pubkey, _)| !non_circulating_accounts.contains(pubkey))
                    .collect::<Vec<_>>(),
                None => ordered_accounts,
            };

            ordered_filtered_accounts
                .iter()
                .take(20)
                .map(|(pubkey, account)| RpcAccountBalance {
                    address: pubkey.to_string(),
                    lamports: account.lamports,
                })
                .collect()
        })
    }

    pub async fn get_largest_accounts_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        config: RpcLargestAccountsConfig,
        commitment_config: CommitmentConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcAccountBalance>> {
        // get all non-circulating and circulating pubkeys from the remote client first,
        // and insert them locally
        {
            let mut remote_non_circulating_pubkeys = client
                .get_largest_accounts(Some(RpcLargestAccountsConfig {
                    filter: Some(RpcLargestAccountsFilter::NonCirculating),
                    ..config.clone()
                }))
                .await?
                .iter()
                .map(|account_balance| verify_pubkey(&account_balance.address))
                .collect::<SurfpoolResult<Vec<_>>>()?;

            let mut remote_circulating_pubkeys = client
                .get_largest_accounts(Some(RpcLargestAccountsConfig {
                    filter: Some(RpcLargestAccountsFilter::Circulating),
                    ..config.clone()
                }))
                .await?
                .iter()
                .map(|account_balance| verify_pubkey(&account_balance.address))
                .collect::<SurfpoolResult<Vec<_>>>()?;

            let mut combined = Vec::with_capacity(
                remote_non_circulating_pubkeys.len() + remote_circulating_pubkeys.len(),
            );
            combined.append(&mut remote_non_circulating_pubkeys);
            combined.append(&mut remote_circulating_pubkeys);

            let get_account_results = self
                .get_multiple_accounts_local_then_remote(client, &combined, commitment_config)
                .await?
                .inner;

            self.write_multiple_account_updates(&get_account_results);
        }

        // now that our local cache is aware of all large remote accounts, we can get the largest accounts locally
        // and filter according to the config
        Ok(self.get_largest_accounts_local(config))
    }

    pub async fn get_largest_accounts(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        config: RpcLargestAccountsConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcAccountBalance>> {
        let results = if let Some((remote_client, commitment_config)) = remote_ctx {
            self.get_largest_accounts_local_then_remote(remote_client, config, *commitment_config)
                .await?
        } else {
            self.get_largest_accounts_local(config)
        };

        Ok(results)
    }
}

/// Get signatures for Addresses
impl SurfnetSvmLocker {
    pub fn get_signatures_for_address_local(
        &self,
        pubkey: &Pubkey,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> SvmAccessContext<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let current_slot = svm_reader.get_latest_absolute_slot();

            let config = config.clone().unwrap_or_default();
            let limit = config.limit.unwrap_or(1000);

            let mut before_slot = None;
            let mut until_slot = None;

            let sigs: Vec<_> = svm_reader
                .transactions
                .iter()
                .filter_map(|(sig, status)| {
                    let TransactionWithStatusMeta(slot, tx, _, err) = status.expect_processed();

                    if *slot < config.clone().min_context_slot.unwrap_or_default() {
                        return None;
                    }

                    if Some(sig.to_string()) == config.clone().before {
                        before_slot = Some(*slot)
                    }

                    if Some(sig.to_string()) == config.clone().until {
                        until_slot = Some(*slot)
                    }

                    // Check if the pubkey is a signer
                    let is_signer = tx
                        .message
                        .static_account_keys()
                        .iter()
                        .position(|pk| pk == pubkey)
                        .map(|i| tx.message.is_signer(i))
                        .unwrap_or(false);

                    if !is_signer {
                        return None;
                    }

                    // Determine confirmation status
                    let confirmation_status = match current_slot {
                        cs if cs == *slot => SolanaTransactionConfirmationStatus::Processed,
                        cs if cs < slot + FINALIZATION_SLOT_THRESHOLD => {
                            SolanaTransactionConfirmationStatus::Confirmed
                        }
                        _ => SolanaTransactionConfirmationStatus::Finalized,
                    };

                    Some(RpcConfirmedTransactionStatusWithSignature {
                        err: err.clone(),
                        slot: *slot,
                        memo: None,
                        block_time: None,
                        confirmation_status: Some(confirmation_status),
                        signature: sig.to_string(),
                    })
                })
                .collect();

            sigs.into_iter()
                .filter(|sig| {
                    if config.before.is_none() && config.until.is_none() {
                        return true;
                    }

                    if config.before.is_some() && before_slot >= Some(sig.slot) {
                        return true;
                    }

                    if config.until.is_some() && until_slot <= Some(sig.slot) {
                        return true;
                    }

                    false
                })
                .take(limit)
                .collect()
        })
    }

    pub async fn get_signatures_for_address_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        pubkey: &Pubkey,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> SurfpoolContextualizedResult<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        let results = self.get_signatures_for_address_local(pubkey, config.clone());
        let limit = config.clone().and_then(|c| c.limit).unwrap_or(1000);

        let mut combined_results = results.inner.clone();
        if combined_results.len() < limit {
            let mut remote_results = client.get_signatures_for_address(pubkey, config).await?;
            combined_results.append(&mut remote_results);
        }

        Ok(results.with_new_value(combined_results))
    }

    pub async fn get_signatures_for_address(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, ())>,
        pubkey: &Pubkey,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> SurfpoolContextualizedResult<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        let results = if let Some((remote_client, _)) = remote_ctx {
            self.get_signatures_for_address_local_then_remote(remote_client, pubkey, config.clone())
                .await?
        } else {
            self.get_signatures_for_address_local(pubkey, config)
        };

        Ok(results)
    }
}

/// Functions for getting transactions from the underlying SurfnetSvm instance or remote client
impl SurfnetSvmLocker {
    /// Retrieves a transaction by signature, using local or remote based on context.
    pub async fn get_transaction(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, Option<UiTransactionEncoding>)>,
        signature: &Signature,
    ) -> SvmAccessContext<GetTransactionResult> {
        if let Some((remote_client, encoding)) = remote_ctx {
            self.get_transaction_local_then_remote(remote_client, signature, *encoding)
                .await
        } else {
            self.get_transaction_local(signature)
        }
    }

    /// Retrieves a transaction from local cache, returning a contextualized result.
    pub fn get_transaction_local(
        &self,
        signature: &Signature,
    ) -> SvmAccessContext<GetTransactionResult> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let latest_absolute_slot = svm_reader.get_latest_absolute_slot();

            match svm_reader.transactions.get(signature).map(|entry| {
                Into::<EncodedConfirmedTransactionWithStatusMeta>::into(
                    entry.expect_processed().clone(),
                )
            }) {
                Some(tx) => {
                    GetTransactionResult::found_transaction(*signature, tx, latest_absolute_slot)
                }
                None => GetTransactionResult::None(*signature),
            }
        })
    }

    /// Retrieves a transaction locally then from remote if missing, returning a contextualized result.
    pub async fn get_transaction_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        signature: &Signature,
        encoding: Option<UiTransactionEncoding>,
    ) -> SvmAccessContext<GetTransactionResult> {
        let local_result = self.get_transaction_local(signature);
        if local_result.inner.is_none() {
            let remote_result = client
                .get_transaction(*signature, encoding, local_result.slot)
                .await;
            local_result.with_new_value(remote_result)
        } else {
            local_result
        }
    }
}

/// Functions for simulating and processing transactions in the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Simulates a transaction on the SVM, returning detailed info or failure metadata.
    pub fn simulate_transaction(
        &self,
        transaction: VersionedTransaction,
        sigverify: bool,
    ) -> Result<SimulatedTransactionInfo, FailedTransactionMetadata> {
        self.with_svm_reader(|svm_reader| {
            svm_reader.simulate_transaction(transaction.clone(), sigverify)
        })
    }

    /// Processes a transaction: verifies signatures, preflight sim, sends to SVM, and enqueues status events.
    pub async fn process_transaction(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        transaction: VersionedTransaction,
        status_tx: Sender<TransactionStatusEvent>,
        skip_preflight: bool,
    ) -> SurfpoolContextualizedResult<()> {
        let remote_ctx = &remote_ctx.get_remote_ctx(CommitmentConfig::confirmed());
        let (latest_absolute_slot, latest_epoch_info, latest_blockhash) =
            self.with_svm_writer(|svm_writer| {
                let latest_absolute_slot = svm_writer.get_latest_absolute_slot();
                svm_writer.notify_signature_subscribers(
                    SignatureSubscriptionType::received(),
                    &transaction.signatures[0],
                    latest_absolute_slot,
                    None,
                );
                (
                    latest_absolute_slot,
                    svm_writer.latest_epoch_info(),
                    svm_writer.latest_blockhash(),
                )
            });

        let signature = transaction.signatures[0];

        // find accounts that are needed for this transaction but are missing from the local
        // svm cache, fetch them from the RPC, and insert them locally
        let pubkeys_from_message = self
            .get_pubkeys_from_message(remote_ctx, &transaction.message)
            .await?;

        let account_updates = self
            .get_multiple_accounts(remote_ctx, &pubkeys_from_message, None)
            .await?
            .inner;

        self.with_svm_writer(|svm_writer| {
            for update in &account_updates {
                svm_writer.write_account_update(update.clone());
            }

            let accounts_before = pubkeys_from_message
                .iter()
                .map(|p| svm_writer.inner.get_account(p))
                .collect::<Vec<Option<Account>>>();

            // if not skipping preflight, simulate the transaction
            if !skip_preflight {
                match svm_writer.simulate_transaction(transaction.clone(), true) {
                    Ok(_) => {}
                    Err(res) => {
                        let _ = svm_writer
                            .simnet_events_tx
                            .try_send(SimnetEvent::error(format!(
                                "Transaction simulation failed: {}",
                                res.err
                            )));
                        let meta = convert_transaction_metadata_from_canonical(&res.meta);
                        let _ = status_tx.try_send(TransactionStatusEvent::SimulationFailure((
                            res.err.clone(),
                            meta,
                        )));
                        svm_writer.notify_signature_subscribers(
                            SignatureSubscriptionType::processed(),
                            &signature,
                            latest_absolute_slot,
                            Some(res.err),
                        );
                        return;
                    }
                }
            }
            // send the transaction to the SVM
            let err = match svm_writer
                .send_transaction(transaction.clone(), false /* cu_analysis_enabled */)
            {
                Ok(res) => {
                    let accounts_after = pubkeys_from_message
                        .iter()
                        .map(|p| svm_writer.inner.get_account(p))
                        .collect::<Vec<Option<Account>>>();

                    for (pubkey, (before, after)) in pubkeys_from_message
                        .iter()
                        .zip(accounts_before.iter().zip(accounts_after))
                    {
                        if before.ne(&after) {
                            if let Some(after) = &after {
                                svm_writer.update_account_registries(pubkey, after);
                            }
                            svm_writer
                                .notify_account_subscribers(pubkey, &after.unwrap_or_default());
                        }
                    }

                    let transaction_meta = convert_transaction_metadata_from_canonical(&res);
                    let _ = svm_writer
                        .geyser_events_tx
                        .send(GeyserEvent::NewTransaction(
                            transaction.clone(),
                            transaction_meta.clone(),
                            latest_absolute_slot,
                        ));
                    let _ = status_tx.try_send(TransactionStatusEvent::Success(
                        TransactionConfirmationStatus::Processed,
                    ));
                    svm_writer
                        .transactions_queued_for_confirmation
                        .push_back((transaction.clone(), status_tx.clone()));
                    None
                }
                Err(res) => {
                    let transaction_meta = convert_transaction_metadata_from_canonical(&res.meta);
                    let _ = svm_writer
                        .simnet_events_tx
                        .try_send(SimnetEvent::error(format!(
                            "Transaction execution failed: {}",
                            res.err
                        )));
                    let _ = status_tx.try_send(TransactionStatusEvent::ExecutionFailure((
                        res.err.clone(),
                        transaction_meta,
                    )));
                    Some(res.err)
                }
            };

            svm_writer.notify_signature_subscribers(
                SignatureSubscriptionType::processed(),
                &signature,
                latest_absolute_slot,
                err,
            );
        });

        Ok(SvmAccessContext::new(
            latest_absolute_slot,
            latest_epoch_info,
            latest_blockhash,
            (),
        ))
    }
}

/// Functions for writing account updates to the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Writes a single account update into the SVM state if present.
    pub fn write_account_update(&self, account_update: GetAccountResult) {
        if !account_update.requires_update() {
            return;
        }

        self.with_svm_writer(move |svm_writer| {
            svm_writer.write_account_update(account_update.clone())
        })
    }

    /// Writes multiple account updates into the SVM state when any are present.
    pub fn write_multiple_account_updates(&self, account_updates: &[GetAccountResult]) {
        if account_updates
            .iter()
            .all(|update| !update.requires_update())
        {
            return;
        }

        self.with_svm_writer(move |svm_writer| {
            for update in account_updates {
                svm_writer.write_account_update(update.clone());
            }
        });
    }
}

/// Token account related functions
impl SurfnetSvmLocker {
    /// Fetches all token accounts for an owner, returning remote results and missing pubkeys contexts.
    pub async fn get_token_accounts_by_owner(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        owner: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        if let Some(remote_client) = remote_ctx {
            self.get_token_accounts_by_owner_local_then_remote(owner, filter, remote_client, config)
                .await
        } else {
            Ok(self.get_token_accounts_by_owner_local(owner, filter, config))
        }
    }

    pub fn get_token_accounts_by_owner_local(
        &self,
        owner: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SvmAccessContext<Vec<RpcKeyedAccount>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            svm_reader
                .get_parsed_token_accounts_by_owner(&owner)
                .iter()
                .filter_map(|(pubkey, token_account)| {
                    let account = svm_reader.accounts_registry.get(pubkey)?;
                    if match filter {
                        TokenAccountsFilter::Mint(mint) => token_account.mint.eq(mint),
                        TokenAccountsFilter::ProgramId(program_id) => account.owner.eq(program_id),
                    } {
                        let account_data = encode_ui_account(
                            pubkey,
                            account,
                            config.encoding.unwrap_or(UiAccountEncoding::Base64),
                            None,
                            config.data_slice,
                        );
                        Some(RpcKeyedAccount {
                            pubkey: pubkey.to_string(),
                            account: account_data,
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
    }

    pub async fn get_token_accounts_by_owner_local_then_remote(
        &self,
        owner: Pubkey,
        filter: &TokenAccountsFilter,
        remote_client: &SurfnetRemoteClient,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: local_accounts,
        } = self.get_token_accounts_by_owner_local(owner, filter, &config);

        let remote_accounts = remote_client
            .get_token_accounts_by_owner(owner, filter, config)
            .await?;

        let mut combined_accounts = remote_accounts;

        for local_account in local_accounts {
            if let Some((pos, _)) = combined_accounts
                .iter()
                .find_position(|RpcKeyedAccount { pubkey, .. }| pubkey.eq(&local_account.pubkey))
            {
                combined_accounts[pos] = local_account;
            } else {
                combined_accounts.push(local_account);
            }
        }

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            combined_accounts,
        ))
    }

    pub async fn get_token_accounts_by_delegate(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        delegate: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        // Validate that the program is supported if using ProgramId filter
        if let TokenAccountsFilter::ProgramId(program_id) = filter {
            if !is_supported_token_program(program_id) {
                return Err(SurfpoolError::unsupported_token_program(*program_id));
            }
        }

        if let Some(remote_client) = remote_ctx {
            self.get_token_accounts_by_delegate_local_then_remote(
                delegate,
                filter,
                remote_client,
                config,
            )
            .await
        } else {
            Ok(self.get_token_accounts_by_delegate_local(delegate, filter, config))
        }
    }
}

/// Token account by delegate related functions
impl SurfnetSvmLocker {
    pub fn get_token_accounts_by_delegate_local(
        &self,
        delegate: Pubkey,
        filter: &TokenAccountsFilter,
        config: &RpcAccountInfoConfig,
    ) -> SvmAccessContext<Vec<RpcKeyedAccount>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            svm_reader
                .get_token_accounts_by_delegate(&delegate)
                .iter()
                .filter_map(|(pubkey, token_account)| {
                    let include = match filter {
                        TokenAccountsFilter::Mint(mint) => token_account.mint == *mint,
                        TokenAccountsFilter::ProgramId(program_id) => {
                            if let Some(account) = svm_reader.accounts_registry.get(pubkey) {
                                account.owner == *program_id
                                    && is_supported_token_program(program_id)
                            } else {
                                false
                            }
                        }
                    };

                    if include {
                        if let Some(account) = svm_reader.accounts_registry.get(pubkey) {
                            Some(RpcKeyedAccount {
                                pubkey: pubkey.to_string(),
                                account: encode_ui_account(
                                    pubkey,
                                    account,
                                    config.encoding.unwrap_or(UiAccountEncoding::Base64),
                                    None,
                                    config.data_slice,
                                ),
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
    }

    pub async fn get_token_accounts_by_delegate_local_then_remote(
        &self,
        delegate: Pubkey,
        filter: &TokenAccountsFilter,
        remote_client: &SurfnetRemoteClient,
        config: &RpcAccountInfoConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: local_accounts,
        } = self.get_token_accounts_by_delegate_local(delegate, filter, config);

        let remote_accounts = remote_client
            .get_token_accounts_by_delegate(delegate, filter, config)
            .await?;

        let mut combined_accounts = remote_accounts;

        for local_account in local_accounts {
            if let Some((pos, _)) = combined_accounts
                .iter()
                .find_position(|RpcKeyedAccount { pubkey, .. }| pubkey.eq(&local_account.pubkey))
            {
                // Replace remote account with local one (local takes precedence)
                combined_accounts[pos] = local_account;
            } else {
                // Add local account that wasn't found in remote results
                combined_accounts.push(local_account);
            }
        }

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            combined_accounts,
        ))
    }
}

/// Get largest account related account
impl SurfnetSvmLocker {
    pub fn get_token_largest_accounts_local(
        &self,
        mint: &Pubkey,
    ) -> SvmAccessContext<Vec<RpcTokenAccountBalance>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let token_accounts = svm_reader.get_token_accounts_by_mint(mint);

            // get mint information to determine decimals
            let mint_decimals = if let Some(mint_account) = svm_reader.accounts_registry.get(mint) {
                if let Ok(mint_data) = Mint::unpack(&mint_account.data) {
                    mint_data.decimals
                } else {
                    0
                }
            } else {
                0
            };

            // convert to RpcTokenAccountBalance and sort by balance
            let mut balances: Vec<RpcTokenAccountBalance> = token_accounts
                .into_iter()
                .map(|(pubkey, token_account)| {
                    let ui_amount = if mint_decimals > 0 {
                        Some(
                            token_account.amount as f64 / (10_u64.pow(mint_decimals as u32) as f64),
                        )
                    } else {
                        Some(token_account.amount as f64)
                    };

                    let ui_amount_string = if mint_decimals > 0 {
                        format!(
                            "{:.precision$}",
                            token_account.amount as f64 / (10_u64.pow(mint_decimals as u32) as f64),
                            precision = mint_decimals as usize
                        )
                    } else {
                        token_account.amount.to_string()
                    };

                    RpcTokenAccountBalance {
                        address: pubkey.to_string(),
                        amount: UiTokenAmount {
                            amount: token_account.amount.to_string(),
                            decimals: mint_decimals,
                            ui_amount,
                            ui_amount_string,
                        },
                    }
                })
                .collect();

            // sort by amount in descending order
            balances.sort_by(|a, b| {
                let amount_a: u64 = a.amount.amount.parse().unwrap_or(0);
                let amount_b: u64 = b.amount.amount.parse().unwrap_or(0);
                amount_b.cmp(&amount_a)
            });

            // limit to top 20 accounts
            balances.truncate(20);

            balances
        })
    }

    pub async fn get_token_largest_accounts_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        mint: &Pubkey,
        commitment_config: CommitmentConfig,
    ) -> SurfpoolContextualizedResult<Vec<RpcTokenAccountBalance>> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: local_accounts,
        } = self.get_token_largest_accounts_local(mint);

        let remote_accounts = client
            .get_token_largest_accounts(mint, commitment_config)
            .await?;

        let mut combined_accounts = remote_accounts;

        // if the account is in both the local and remote list, add the local one and not the remote
        for local_account in local_accounts {
            if let Some((pos, _)) = combined_accounts
                .iter()
                .find_position(|remote_account| remote_account.address == local_account.address)
            {
                combined_accounts[pos] = local_account;
            } else {
                combined_accounts.push(local_account);
            }
        }

        // re-sort and limit after combining
        combined_accounts.sort_by(|a, b| {
            let amount_a: u64 = a.amount.amount.parse().unwrap_or(0);
            let amount_b: u64 = b.amount.amount.parse().unwrap_or(0);
            amount_b.cmp(&amount_a)
        });
        combined_accounts.truncate(20);

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            combined_accounts,
        ))
    }

    /// Fetches the largest token accounts for a specific mint, returning contextualized results.
    pub async fn get_token_largest_accounts(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        mint: &Pubkey,
    ) -> SurfpoolContextualizedResult<Vec<RpcTokenAccountBalance>> {
        if let Some((remote_client, commitment_config)) = remote_ctx {
            self.get_token_largest_accounts_local_then_remote(
                remote_client,
                mint,
                *commitment_config,
            )
            .await
        } else {
            Ok(self.get_token_largest_accounts_local(mint))
        }
    }
}

/// Address lookup table related functions
impl SurfnetSvmLocker {
    /// Extracts pubkeys from a VersionedMessage, resolving address lookup tables as needed.
    pub async fn get_pubkeys_from_message(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        message: &VersionedMessage,
    ) -> SurfpoolResult<Vec<Pubkey>> {
        match message {
            VersionedMessage::Legacy(message) => Ok(message.account_keys.clone()),
            VersionedMessage::V0(message) => {
                let alts = message.address_table_lookups.clone();
                let mut acc_keys = message.account_keys.clone();
                let mut alt_pubkeys = alts.iter().map(|msg| msg.account_key).collect::<Vec<_>>();

                let mut table_entries = join_all(alts.iter().map(|msg| async {
                    let loaded_addresses = self
                        .get_lookup_table_addresses(remote_ctx, msg)
                        .await?
                        .inner;
                    let mut combined = loaded_addresses.writable;
                    combined.extend(loaded_addresses.readonly);
                    Ok::<_, SurfpoolError>(combined)
                }))
                .await
                .into_iter()
                .collect::<Result<Vec<Vec<Pubkey>>, SurfpoolError>>()?
                .into_iter()
                .flatten()
                .collect();

                acc_keys.append(&mut alt_pubkeys);
                acc_keys.append(&mut table_entries);
                Ok(acc_keys)
            }
        }
    }

    /// Retrieves loaded addresses from a lookup table account, validating owner and indices.
    pub async fn get_lookup_table_addresses(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        address_table_lookup: &MessageAddressTableLookup,
    ) -> SurfpoolContextualizedResult<LoadedAddresses> {
        let result = self
            .get_account(remote_ctx, &address_table_lookup.account_key, None)
            .await?;
        let table_account = result.inner.clone().map_account()?;

        if table_account.owner == solana_sdk_ids::address_lookup_table::id() {
            let SvmAccessContext {
                slot: current_slot,
                inner: slot_hashes,
                ..
            } = self.with_contextualized_svm_reader(|svm_reader| {
                svm_reader
                    .inner
                    .get_sysvar::<solana_sdk::sysvar::slot_hashes::SlotHashes>()
            });

            //let current_slot = self.get_latest_absolute_slot(); // or should i use this?
            let data = &table_account.data.clone();
            let lookup_table = AddressLookupTable::deserialize(data).map_err(|_ix_err| {
                SurfpoolError::invalid_account_data(
                    address_table_lookup.account_key,
                    table_account.data,
                    Some("Attempted to lookup addresses from an invalid account"),
                )
            })?;

            let loaded_addresses = LoadedAddresses {
                writable: lookup_table
                    .lookup(
                        current_slot,
                        &address_table_lookup.writable_indexes,
                        &slot_hashes,
                    )
                    .map_err(|_ix_err| {
                        SurfpoolError::invalid_lookup_index(address_table_lookup.account_key)
                    })?,
                readonly: lookup_table
                    .lookup(
                        current_slot,
                        &address_table_lookup.readonly_indexes,
                        &slot_hashes,
                    )
                    .map_err(|_ix_err| {
                        SurfpoolError::invalid_lookup_index(address_table_lookup.account_key)
                    })?,
            };
            Ok(result.with_new_value(loaded_addresses))
        } else {
            Err(SurfpoolError::invalid_account_owner(
                table_account.owner,
                Some("Attempted to lookup addresses from an account owned by the wrong program"),
            ))
        }
    }
}

/// Profiling helper functions
impl SurfnetSvmLocker {
    /// Estimates compute units for a transaction via contextualized simulation.
    pub fn estimate_compute_units(
        &self,
        transaction: &VersionedTransaction,
    ) -> SvmAccessContext<ComputeUnitsEstimationResult> {
        self.with_contextualized_svm_reader(|svm_reader| {
            svm_reader.estimate_compute_units(transaction)
        })
    }
    pub async fn profile_transaction(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        transaction: &VersionedTransaction,
        encoding: Option<UiAccountEncoding>,
    ) -> SurfpoolContextualizedResult<ProfileResult> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: mut svm_clone,
        } = self.with_contextualized_svm_reader(|svm_reader| svm_reader.clone());

        let (dummy_simnet_tx, _) = crossbeam_channel::bounded(1);
        let (dummy_geyser_tx, _) = crossbeam_channel::bounded(1);
        svm_clone.simnet_events_tx = dummy_simnet_tx;
        svm_clone.geyser_events_tx = dummy_geyser_tx;

        let svm_locker = SurfnetSvmLocker::new(svm_clone);

        let remote_ctx_with_config = remote_ctx
            .clone()
            .map(|client| (client, CommitmentConfig::confirmed()));

        let account_keys = svm_locker
            .get_pubkeys_from_message(&remote_ctx_with_config, &transaction.message)
            .await?;

        let pre_execution_capture = {
            let mut capture = BTreeMap::new();
            for pubkey in &account_keys {
                let account = svm_locker
                    .get_account(&remote_ctx_with_config, pubkey, None)
                    .await?
                    .inner;

                snapshot_get_account_result(
                    &mut capture,
                    account,
                    encoding.unwrap_or(UiAccountEncoding::Base64),
                );
            }
            capture
        };

        let compute_units_estimation_result = svm_locker.estimate_compute_units(transaction).inner;

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let _ = svm_locker
            .process_transaction(remote_ctx, transaction.clone(), status_tx, true)
            .await?;

        let simnet_events_tx = self.simnet_events_tx();
        loop {
            if let Ok(status) = status_rx.try_recv() {
                match status {
                    TransactionStatusEvent::Success(_) => break,
                    TransactionStatusEvent::ExecutionFailure((err, _)) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::WarnLog(
                            chrono::Local::now(),
                            format!(
                                "Transaction {} failed during snapshot simulation: {}",
                                transaction.signatures[0], err
                            ),
                        ));
                        return Err(SurfpoolError::internal(format!(
                            "Transaction {} failed during snapshot simulation: {}",
                            transaction.signatures[0], err
                        )));
                    }
                    TransactionStatusEvent::SimulationFailure(_) => unreachable!(),
                    TransactionStatusEvent::VerificationFailure(_) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::WarnLog(
                            chrono::Local::now(),
                            format!(
                                "Transaction {} verification failed during snapshot simulation",
                                transaction.signatures[0]
                            ),
                        ));
                        return Err(SurfpoolError::internal(format!(
                            "Transaction {} verification failed during snapshot simulation",
                            transaction.signatures[0]
                        )));
                    }
                }
            }
        }

        let post_execution_capture = {
            let mut capture = BTreeMap::new();
            for pubkey in &account_keys {
                // get the account locally after execution, because execution should have inserted from
                // the remote if not found locally
                let account = svm_locker.get_account_local(pubkey).inner;

                snapshot_get_account_result(
                    &mut capture,
                    account,
                    encoding.unwrap_or(UiAccountEncoding::Base64),
                );
            }
            capture
        };

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            ProfileResult {
                compute_units: compute_units_estimation_result,
                state: ProfileState::new(pre_execution_capture, post_execution_capture),
            },
        ))
    }

    /// Records profiling results under a tag and emits a tagged profile event.
    pub fn write_profiling_results(&self, tag: String, profile_result: ProfileResult) {
        self.with_svm_writer(|svm_writer| {
            svm_writer
                .tagged_profiling_results
                .entry(tag.clone())
                .or_default()
                .push(profile_result.clone());
            let _ = svm_writer
                .simnet_events_tx
                .try_send(SimnetEvent::tagged_profile(
                    profile_result.clone(),
                    tag.clone(),
                ));
        });
    }
}

/// Program account related functions
impl SurfnetSvmLocker {
    /// Clones a program account from source to destination, handling upgradeable loader state.
    pub async fn clone_program_account(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        source_program_id: &Pubkey,
        destination_program_id: &Pubkey,
    ) -> SurfpoolContextualizedResult<()> {
        let expected_source_program_data_address = get_program_data_address(source_program_id);

        let result = self
            .get_multiple_accounts(
                remote_ctx,
                &[*source_program_id, expected_source_program_data_address],
                None,
            )
            .await?;

        let mut accounts = result
            .inner
            .clone()
            .into_iter()
            .map(|a| a.map_account())
            .collect::<SurfpoolResult<Vec<Account>>>()?;

        let source_program_data_account = accounts.remove(1);
        let source_program_account = accounts.remove(0);

        let BpfUpgradeableLoaderAccountType::Program(UiProgram {
            program_data: source_program_data_address,
        }) = parse_bpf_upgradeable_loader(&source_program_account.data).map_err(|e| {
            SurfpoolError::invalid_program_account(source_program_id, e.to_string())
        })?
        else {
            return Err(SurfpoolError::expected_program_account(source_program_id));
        };

        if source_program_data_address.ne(&expected_source_program_data_address.to_string()) {
            return Err(SurfpoolError::invalid_program_account(
                source_program_id,
                format!(
                    "Program data address mismatch: expected {}, found {}",
                    expected_source_program_data_address, source_program_data_address
                ),
            ));
        }

        let destination_program_data_address = get_program_data_address(destination_program_id);

        // create a new program account that has the `program_data` field set to the
        // destination program data address
        let mut new_program_account = source_program_account;
        new_program_account.data = bincode::serialize(&UpgradeableLoaderState::Program {
            programdata_address: destination_program_data_address,
        })
        .map_err(|e| SurfpoolError::internal(format!("Failed to serialize program data: {}", e)))?;

        self.with_svm_writer(|svm_writer| {
            svm_writer.set_account(
                &destination_program_data_address,
                source_program_data_account.clone(),
            )?;

            svm_writer.set_account(destination_program_id, new_program_account.clone())?;
            Ok::<(), SurfpoolError>(())
        })?;

        Ok(result.with_new_value(()))
    }

    pub async fn get_program_accounts(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        program_id: &Pubkey,
        account_config: RpcAccountInfoConfig,
        filters: Option<Vec<RpcFilterType>>,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        if let Some(remote_client) = remote_ctx {
            self.get_program_accounts_local_then_remote(
                remote_client,
                program_id,
                account_config,
                filters,
            )
            .await
        } else {
            self.get_program_accounts_local(program_id, account_config, filters)
        }
    }

    /// Retrieves program accounts from the local SVM cache, returning a contextualized result.
    pub fn get_program_accounts_local(
        &self,
        program_id: &Pubkey,
        account_config: RpcAccountInfoConfig,
        filters: Option<Vec<RpcFilterType>>,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        let res = self.with_contextualized_svm_reader(|svm_reader| {
            svm_reader.get_account_owned_by(*program_id)
        });

        let mut filtered = vec![];
        for (pubkey, account) in res.inner.iter() {
            if let Some(ref active_filters) = filters {
                match apply_rpc_filters(&account.data, active_filters) {
                    Ok(true) => {}           // Account matches all filters
                    Ok(false) => continue,   // Filtered out
                    Err(e) => return Err(e), // Error applying filter, already JsonRpcError
                }
            }
            let data_slice = account_config.data_slice;

            filtered.push(RpcKeyedAccount {
                pubkey: pubkey.to_string(),
                account: encode_ui_account(
                    &pubkey,
                    account,
                    account_config.encoding.unwrap_or(UiAccountEncoding::Base64),
                    None, // No additional data for now
                    data_slice,
                ),
            });
        }
        Ok(res.with_new_value(filtered))
    }

    /// Retrieves program accounts from the local cache and remote client, combining results.
    pub async fn get_program_accounts_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        program_id: &Pubkey,
        account_config: RpcAccountInfoConfig,
        filters: Option<Vec<RpcFilterType>>,
    ) -> SurfpoolContextualizedResult<Vec<RpcKeyedAccount>> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: local_accounts,
        } = self.get_program_accounts_local(program_id, account_config.clone(), filters.clone())?;
        let remote_accounts = client
            .get_program_accounts(program_id, account_config, filters)
            .await?;

        let mut combined_accounts = remote_accounts;

        for local_account in local_accounts {
            // if the local account is in the remote set, replace it with the local one
            if let Some((pos, _)) = combined_accounts.iter().find_position(
                |RpcKeyedAccount {
                     pubkey: remote_pubkey,
                     ..
                 }| remote_pubkey.eq(&local_account.pubkey),
            ) {
                combined_accounts[pos] = local_account;
            } else {
                // otherwise, add the local account to the combined list
                combined_accounts.push(local_account);
            };
        }

        Ok(SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: combined_accounts,
        })
    }
}

impl SurfnetSvmLocker {
    pub fn get_genesis_hash_local(&self) -> SvmAccessContext<Hash> {
        self.with_contextualized_svm_reader(|svm_reader| svm_reader.genesis_config.hash())
    }

    pub async fn get_genesis_hash_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
    ) -> SurfpoolContextualizedResult<Hash> {
        let local_hash = self.get_genesis_hash_local();
        let remote_hash = client.get_genesis_hash().await?;

        Ok(local_hash.with_new_value(remote_hash))
    }

    pub async fn get_genesis_hash(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
    ) -> SurfpoolContextualizedResult<Hash> {
        if let Some(client) = remote_ctx {
            self.get_genesis_hash_local_then_remote(client).await
        } else {
            Ok(self.get_genesis_hash_local())
        }
    }
}

/// Pass through functions for accessing the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Returns a sender for simulation events from the underlying SVM.
    pub fn simnet_events_tx(&self) -> Sender<SimnetEvent> {
        self.with_svm_reader(|svm_reader| svm_reader.simnet_events_tx.clone())
    }

    /// Retrieves the latest epoch info from the underlying SVM.
    pub fn get_epoch_info(&self) -> EpochInfo {
        self.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())
    }

    /// Retrieves the latest absolute slot from the underlying SVM.
    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.with_svm_reader(|svm_reader| svm_reader.get_latest_absolute_slot())
    }

    /// Executes an airdrop via the underlying SVM.
    pub fn airdrop(&self, pubkey: &Pubkey, lamports: u64) -> TransactionResult {
        self.with_svm_writer(|svm_writer| svm_writer.airdrop(pubkey, lamports))
    }

    /// Executes a batch airdrop via the underlying SVM.
    pub fn airdrop_pubkeys(&self, lamports: u64, addresses: &[Pubkey]) {
        self.with_svm_writer(|svm_writer| svm_writer.airdrop_pubkeys(lamports, addresses))
    }

    /// Confirms the current block on the underlying SVM, returning `Ok(())` or an error.
    pub fn confirm_current_block(&self) -> SurfpoolResult<()> {
        self.with_svm_writer(|svm_writer| svm_writer.confirm_current_block())
    }

    /// Subscribes for signature updates (confirmed/finalized) and returns a receiver of events.
    pub fn subscribe_for_signature_updates(
        &self,
        signature: &Signature,
        subscription_type: SignatureSubscriptionType,
    ) -> Receiver<(Slot, Option<TransactionError>)> {
        self.with_svm_writer(|svm_writer| {
            svm_writer.subscribe_for_signature_updates(signature, subscription_type.clone())
        })
    }

    /// Subscribes for account updates and returns a receiver of account updates.
    pub fn subscribe_for_account_updates(
        &self,
        account_pubkey: &Pubkey,
        encoding: Option<UiAccountEncoding>,
    ) -> Receiver<UiAccount> {
        // Handles the locking/unlocking safely
        self.with_svm_writer(|svm_writer| {
            svm_writer.subscribe_for_account_updates(account_pubkey, encoding)
        })
    }

    /// Subscribes for slot updates and returns a receiver of slot updates.
    pub fn subscribe_for_slot_updates(&self) -> Receiver<SlotInfo> {
        self.with_svm_writer(|svm_writer| svm_writer.subscribe_for_slot_updates())
    }
}

fn snapshot_get_account_result(
    capture: &mut BTreeMap<Pubkey, Option<UiAccount>>,
    result: GetAccountResult,
    encoding: UiAccountEncoding,
) {
    match result {
        GetAccountResult::None(pubkey) => {
            capture.insert(pubkey, None);
        }
        GetAccountResult::FoundAccount(pubkey, account, _) => {
            capture.insert(
                pubkey,
                Some(encode_ui_account(&pubkey, &account, encoding, None, None)),
            );
        }
        GetAccountResult::FoundProgramAccount((pubkey, account), (data_pubkey, data_account)) => {
            capture.insert(
                pubkey,
                Some(encode_ui_account(&pubkey, &account, encoding, None, None)),
            );
            if let Some(data_account) = data_account {
                capture.insert(
                    data_pubkey,
                    Some(encode_ui_account(
                        &data_pubkey,
                        &data_account,
                        encoding,
                        None,
                        None,
                    )),
                );
            }
        }
    }
}

// Helper function to apply filters
fn apply_rpc_filters(account_data: &[u8], filters: &[RpcFilterType]) -> SurfpoolResult<bool> {
    for filter in filters {
        match filter {
            RpcFilterType::DataSize(size) => {
                if account_data.len() as u64 != *size {
                    return Ok(false);
                }
            }
            RpcFilterType::Memcmp(memcmp_filter) => {
                // Use the public bytes_match method from solana_client::rpc_filter::Memcmp
                if !memcmp_filter.bytes_match(account_data) {
                    return Ok(false); // Content mismatch or out of bounds handled by bytes_match
                }
            }
            RpcFilterType::TokenAccountState => {
                return Err(SurfpoolError::internal(
                    "TokenAccountState filter is not supported",
                ));
            }
        }
    }
    Ok(true)
}

// used in the remote.rs
pub fn is_supported_token_program(program_id: &Pubkey) -> bool {
    *program_id == spl_token::ID || *program_id == spl_token_2022::ID
}
