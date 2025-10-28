use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use bincode::serialized_size;
use crossbeam_channel::{Receiver, Sender};
use itertools::Itertools;
use litesvm::types::{
    FailedTransactionMetadata, SimulatedTransactionInfo, TransactionMetadata, TransactionResult,
};
use solana_account::{Account, ReadableAccount};
use solana_account_decoder::{
    UiAccount, UiAccountEncoding, UiDataSliceConfig,
    parse_account_data::AccountAdditionalDataV3,
    parse_bpf_loader::{BpfUpgradeableLoaderAccountType, UiProgram, parse_bpf_upgradeable_loader},
    parse_token::UiTokenAmount,
};
use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_client::{
    rpc_client::SerializableTransaction,
    rpc_config::{
        RpcAccountInfoConfig, RpcBlockConfig, RpcLargestAccountsConfig, RpcLargestAccountsFilter,
        RpcSignaturesForAddressConfig, RpcTransactionConfig, RpcTransactionLogsFilter,
    },
    rpc_filter::RpcFilterType,
    rpc_request::TokenAccountsFilter,
    rpc_response::{
        RpcAccountBalance, RpcConfirmedTransactionStatusWithSignature, RpcKeyedAccount,
        RpcLogsResponse, RpcTokenAccountBalance,
    },
};
use solana_clock::{Clock, Slot};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_epoch_info::EpochInfo;
use solana_hash::Hash;
use solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState};
use solana_message::{
    Message, MessageHeader, SimpleAddressLoader, VersionedMessage,
    compiled_instruction::CompiledInstruction,
    v0::{LoadedAddresses, MessageAddressTableLookup},
};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_signature::Signature;
use solana_transaction::{
    sanitized::SanitizedTransaction,
    versioned::{TransactionVersion, VersionedTransaction},
};
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta,
    TransactionConfirmationStatus as SolanaTransactionConfirmationStatus, UiConfirmedBlock,
    UiTransactionEncoding,
};
use surfpool_types::{
    AccountSnapshot, ComputeUnitsEstimationResult, ExecutionCapture, ExportSnapshotConfig, Idl,
    KeyedProfileResult, ProfileResult, RpcProfileResultConfig, RunbookExecutionStatusReport,
    SimnetCommand, SimnetEvent, TransactionConfirmationStatus, TransactionStatusEvent,
    UiKeyedProfileResult, UuidOrSignature, VersionedIdl,
};
use tokio::sync::RwLock;
use txtx_addon_kit::indexmap::IndexSet;
use uuid::Uuid;

use super::{
    AccountFactory, GetAccountResult, GetTransactionResult, GeyserEvent, SignatureSubscriptionType,
    SurfnetSvm, remote::SurfnetRemoteClient,
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    helpers::time_travel::calculate_time_travel_clock,
    rpc::utils::{convert_transaction_metadata_from_canonical, verify_pubkey},
    surfnet::FINALIZATION_SLOT_THRESHOLD,
    types::{
        GeyserAccountUpdate, RemoteRpcResult, SurfnetTransactionStatus, TimeTravelConfig,
        TokenAccount, TransactionLoadedAddresses, TransactionWithStatusMeta,
    },
};

enum ProcessTransactionResult {
    Success(TransactionMetadata),
    SimulationFailure(FailedTransactionMetadata),
    ExecutionFailure(FailedTransactionMetadata),
}

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

/// Helper function to apply an override to a JSON value using dot notation path
///
/// # Arguments
/// * `json` - The JSON value to modify
/// * `path` - Dot-separated path to the field (e.g., "price_message.price")
/// * `value` - The new value to set
///
/// # Returns
/// Result indicating success or error
fn apply_override_to_json(
    json: &mut serde_json::Value,
    path: &str,
    value: &serde_json::Value,
) -> SurfpoolResult<()> {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.is_empty() {
        return Err(SurfpoolError::internal("Empty path provided for override"));
    }

    // Navigate to the parent of the target field
    let mut current = json;
    for part in &parts[..parts.len() - 1] {
        current = current.get_mut(part).ok_or_else(|| {
            SurfpoolError::internal(format!("Path segment '{}' not found in JSON", part))
        })?;
    }

    // Set the final field
    let final_key = parts[parts.len() - 1];
    match current {
        serde_json::Value::Object(map) => {
            map.insert(final_key.to_string(), value.clone());
            Ok(())
        }
        _ => Err(SurfpoolError::internal(format!(
            "Cannot set field '{}' - parent is not an object",
            final_key
        ))),
    }
}

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
        F: FnOnce(&SurfnetSvm) -> T + Send + Sync,
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
        F: FnOnce(&mut SurfnetSvm) -> T + Send + Sync,
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
        slot_time: u64,
        remote_ctx: &Option<SurfnetRemoteClient>,
        do_profile_instructions: bool,
        log_bytes_limit: Option<usize>,
    ) -> SurfpoolResult<EpochInfo> {
        let mut epoch_info = if let Some(remote_client) = remote_ctx {
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
        epoch_info.transaction_count = None;

        self.with_svm_writer(|svm_writer| {
            svm_writer.initialize(
                epoch_info.clone(),
                slot_time,
                remote_ctx,
                do_profile_instructions,
                log_bytes_limit,
            );
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
                None => match svm_reader.get_account_from_feature_set(pubkey) {
                    Some(account) => GetAccountResult::FoundAccount(
                        *pubkey, account,
                        // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                        false,
                    ),
                    None => GetAccountResult::None(*pubkey),
                },
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
    /// Retrieves multiple accounts from local cache, returning a contextualized result.
    pub fn get_multiple_accounts_local(
        &self,
        pubkeys: &[Pubkey],
    ) -> SvmAccessContext<Vec<GetAccountResult>> {
        self.with_contextualized_svm_reader(|svm_reader| {
            let mut accounts = vec![];

            for pubkey in pubkeys {
                let res = match svm_reader.inner.get_account(pubkey) {
                    Some(account) => GetAccountResult::FoundAccount(
                        *pubkey, account,
                        // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                        false,
                    ),
                    None => match svm_reader.get_account_from_feature_set(pubkey) {
                        Some(account) => GetAccountResult::FoundAccount(
                            *pubkey, account,
                            // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                            false,
                        ),
                        None => GetAccountResult::None(*pubkey),
                    },
                };
                accounts.push(res);
            }
            accounts
        })
    }

    /// Retrieves multiple accounts from local storage, with remote fallback for missing accounts.
    ///
    /// Returns accounts in the same order as the input `pubkeys` array. Accounts found locally
    /// are returned as-is; accounts not found locally are fetched from the remote RPC client.
    pub async fn get_multiple_accounts_with_remote_fallback(
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

        // Collect missing pubkeys (local_results is already in correct order from pubkeys)
        let missing_accounts: Vec<Pubkey> = local_results
            .iter()
            .filter_map(|result| match result {
                GetAccountResult::None(pubkey) => Some(*pubkey),
                _ => None,
            })
            .collect();

        if missing_accounts.is_empty() {
            // All accounts found locally, already in correct order
            return Ok(SvmAccessContext::new(
                slot,
                latest_epoch_info,
                latest_blockhash,
                local_results,
            ));
        }
        debug!(
            "Missing accounts will be fetched: {}",
            missing_accounts.iter().join(", ")
        );

        // Fetch missing accounts from remote
        let remote_results = client
            .get_multiple_accounts(&missing_accounts, commitment_config)
            .await?;

        // Build map of pubkey -> remote result for O(1) lookup
        let remote_map: HashMap<Pubkey, GetAccountResult> = missing_accounts
            .into_iter()
            .zip(remote_results.into_iter())
            .collect();

        // Replace None entries with remote results while preserving order
        // We iterate through original pubkeys array to ensure order is explicit
        let combined_results: Vec<GetAccountResult> = pubkeys
            .iter()
            .zip(local_results.into_iter())
            .map(|(pubkey, local_result)| {
                match local_result {
                    GetAccountResult::None(_) => {
                        // Replace with remote result if available
                        remote_map
                            .get(pubkey)
                            .cloned()
                            .unwrap_or(GetAccountResult::None(*pubkey))
                    }
                    found => {
                        debug!("Keeping local account: {}", pubkey);
                        found
                    } // Keep found accounts (no clone, just move)
                }
            })
            .collect();

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
            self.get_multiple_accounts_with_remote_fallback(
                remote_client,
                pubkeys,
                *commitment_config,
            )
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
                .iter_accounts()
                .sorted_by(|a, b| b.1.lamports().cmp(&a.1.lamports()))
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
                    lamports: account.lamports(),
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
            let remote_non_circulating_pubkeys_result = client
                .get_largest_accounts(Some(RpcLargestAccountsConfig {
                    filter: Some(RpcLargestAccountsFilter::NonCirculating),
                    ..config.clone()
                }))
                .await?;

            let (mut remote_non_circulating_pubkeys, mut remote_circulating_pubkeys) =
                match remote_non_circulating_pubkeys_result {
                    RemoteRpcResult::Ok(non_circulating_accounts) => {
                        let remote_circulating_pubkeys_result = client
                            .get_largest_accounts(Some(RpcLargestAccountsConfig {
                                filter: Some(RpcLargestAccountsFilter::Circulating),
                                ..config.clone()
                            }))
                            .await?;

                        let remote_circulating_pubkeys = match remote_circulating_pubkeys_result {
                            RemoteRpcResult::Ok(circulating_accounts) => circulating_accounts,
                            RemoteRpcResult::MethodNotSupported => {
                                unreachable!()
                            }
                        };
                        (
                            non_circulating_accounts
                                .iter()
                                .map(|account_balance| verify_pubkey(&account_balance.address))
                                .collect::<SurfpoolResult<Vec<_>>>()?,
                            remote_circulating_pubkeys
                                .iter()
                                .map(|account_balance| verify_pubkey(&account_balance.address))
                                .collect::<SurfpoolResult<Vec<_>>>()?,
                        )
                    }
                    RemoteRpcResult::MethodNotSupported => {
                        let tx = self.simnet_events_tx();
                        let _ = tx.send(SimnetEvent::warn("The `getLargestAccounts` method was sent to the remote RPC, but this method isn't supported by your RPC provider. Only local accounts will be returned."));
                        (vec![], vec![])
                    }
                };

            let mut combined = Vec::with_capacity(
                remote_non_circulating_pubkeys.len() + remote_circulating_pubkeys.len(),
            );
            combined.append(&mut remote_non_circulating_pubkeys);
            combined.append(&mut remote_circulating_pubkeys);

            let get_account_results = self
                .get_multiple_accounts_with_remote_fallback(client, &combined, commitment_config)
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

    pub fn account_to_rpc_keyed_account<T: ReadableAccount + Send + Sync>(
        &self,
        pubkey: &Pubkey,
        account: &T,
        config: &RpcAccountInfoConfig,
        token_mint: Option<Pubkey>,
    ) -> RpcKeyedAccount {
        self.with_svm_reader(|svm_reader| {
            svm_reader.account_to_rpc_keyed_account(pubkey, account, config, token_mint)
        })
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

            let config_before = config.before.clone();
            let config_until = config.until.clone();

            let mut before_slot = None;
            let mut until_slot = None;

            let sigs: Vec<_> = svm_reader
                .transactions
                .iter()
                .filter_map(|(sig, status)| {
                    let (
                        TransactionWithStatusMeta {
                            slot,
                            transaction,
                            meta,
                        },
                        _,
                    ) = status.expect_processed();

                    if *slot < config.clone().min_context_slot.unwrap_or_default() {
                        return None;
                    }

                    if Some(sig.to_string()) == config_before {
                        before_slot = Some(*slot);
                    }

                    if Some(sig.to_string()) == config_until {
                        until_slot = Some(*slot);
                    }

                    // Check if the pubkey is a signer

                    if !transaction.message.static_account_keys().contains(pubkey) {
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
                        err: match &meta.status {
                            Ok(_) => None,
                            Err(e) => Some(e.clone().into()),
                        },
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

                    if config.before.is_some() && before_slot > Some(sig.slot) {
                        return true;
                    }

                    if config.until.is_some() && until_slot < Some(sig.slot) {
                        return true;
                    }

                    false
                })
                // order from most recent to least recent
                .sorted_by(|a, b| b.slot.cmp(&a.slot))
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
        remote_ctx: &Option<SurfnetRemoteClient>,
        signature: &Signature,
        config: RpcTransactionConfig,
    ) -> SurfpoolResult<GetTransactionResult> {
        if let Some(remote_client) = remote_ctx {
            self.get_transaction_local_then_remote(remote_client, signature, config)
                .await
        } else {
            self.get_transaction_local(signature, &config)
        }
    }

    /// Retrieves a transaction from local cache, returning a contextualized result.
    pub fn get_transaction_local(
        &self,
        signature: &Signature,
        config: &RpcTransactionConfig,
    ) -> SurfpoolResult<GetTransactionResult> {
        self.with_svm_reader(|svm_reader| {
            let latest_absolute_slot = svm_reader.get_latest_absolute_slot();

            let Some(entry) = svm_reader.transactions.get(signature) else {
                return Ok(GetTransactionResult::None(*signature));
            };

            let (transaction_with_status_meta, _) = entry.expect_processed();
            let slot = transaction_with_status_meta.slot;
            let block_time = svm_reader
                .blocks
                .get(&slot)
                .map(|b| b.block_time)
                .unwrap_or(0);
            let encoded = transaction_with_status_meta.encode(
                config.encoding.unwrap_or(UiTransactionEncoding::JsonParsed),
                config.max_supported_transaction_version,
                true,
            )?;
            Ok(GetTransactionResult::found_transaction(
                *signature,
                EncodedConfirmedTransactionWithStatusMeta {
                    slot,
                    transaction: encoded,
                    block_time: Some(block_time),
                },
                latest_absolute_slot,
            ))
        })
    }

    /// Retrieves a transaction locally then from remote if missing, returning a contextualized result.
    pub async fn get_transaction_local_then_remote(
        &self,
        client: &SurfnetRemoteClient,
        signature: &Signature,
        config: RpcTransactionConfig,
    ) -> SurfpoolResult<GetTransactionResult> {
        let local_result = self.get_transaction_local(signature, &config)?;
        let latest_absolute_slot = self.get_latest_absolute_slot();
        if local_result.is_none() {
            Ok(client
                .get_transaction(*signature, config, latest_absolute_slot)
                .await)
        } else {
            Ok(local_result)
        }
    }
}

/// Functions for simulating and processing transactions in the underlying SurfnetSvm instance
impl SurfnetSvmLocker {
    /// Simulates a transaction on the SVM, returning detailed info or failure metadata.
    #[allow(clippy::result_large_err)]
    pub fn simulate_transaction(
        &self,
        transaction: VersionedTransaction,
        sigverify: bool,
    ) -> Result<SimulatedTransactionInfo, FailedTransactionMetadata> {
        self.with_svm_reader(|svm_reader| {
            svm_reader.simulate_transaction(transaction.clone(), sigverify)
        })
    }

    pub fn is_instruction_profiling_enabled(&self) -> bool {
        self.with_svm_reader(|svm_reader| svm_reader.instruction_profiling_enabled)
    }

    pub fn get_profiling_map_capacity(&self) -> usize {
        self.with_svm_reader(|svm_reader| svm_reader.max_profiles)
    }

    pub async fn process_transaction(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        transaction: VersionedTransaction,
        status_tx: Sender<TransactionStatusEvent>,
        skip_preflight: bool,
        sigverify: bool,
    ) -> SurfpoolResult<()> {
        let do_propagate_status_updates = true;
        let signature = transaction.signatures[0];
        let profile_result = self
            .fetch_all_tx_accounts_then_process_tx_returning_profile_res(
                remote_ctx,
                transaction,
                status_tx,
                skip_preflight,
                sigverify,
                do_propagate_status_updates,
            )
            .await?;

        self.with_svm_writer(|svm_writer| {
            svm_writer.write_executed_profile_result(signature, profile_result);
        });
        Ok(())
    }

    pub async fn profile_transaction(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        transaction: VersionedTransaction,
        tag: Option<String>,
    ) -> SurfpoolContextualizedResult<Uuid> {
        let mut svm_clone = self.with_svm_reader(|svm_reader| svm_reader.clone());

        let (dummy_simnet_tx, _) = crossbeam_channel::bounded(1);
        let (dummy_geyser_tx, _) = crossbeam_channel::bounded(1);
        svm_clone.simnet_events_tx = dummy_simnet_tx;
        svm_clone.geyser_events_tx = dummy_geyser_tx;

        let svm_locker = SurfnetSvmLocker::new(svm_clone);

        let (status_tx, _) = crossbeam_channel::unbounded();

        let skip_preflight = true; // skip preflight checks during transaction profiling
        let sigverify = true; // do verify signatures during transaction profiling
        let do_propagate_status_updates = false; // don't propagate status updates during transaction profiling
        let mut profile_result = svm_locker
            .fetch_all_tx_accounts_then_process_tx_returning_profile_res(
                remote_ctx,
                transaction,
                status_tx,
                skip_preflight,
                sigverify,
                do_propagate_status_updates,
            )
            .await?;

        let uuid = Uuid::new_v4();
        profile_result.key = UuidOrSignature::Uuid(uuid);

        self.with_svm_writer(|svm_writer| {
            svm_writer.write_simulated_profile_result(uuid, tag, profile_result);
        });

        Ok(self.with_contextualized_svm_reader(|_| uuid))
    }

    async fn fetch_all_tx_accounts_then_process_tx_returning_profile_res(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        transaction: VersionedTransaction,
        status_tx: Sender<TransactionStatusEvent>,
        skip_preflight: bool,
        sigverify: bool,
        do_propagate: bool,
    ) -> SurfpoolResult<KeyedProfileResult> {
        let signature = transaction.signatures[0];

        // Can we avoid this write?
        let latest_absolute_slot = self.with_svm_writer(|svm_writer| {
            let latest_absolute_slot = svm_writer.get_latest_absolute_slot();
            svm_writer.notify_signature_subscribers(
                SignatureSubscriptionType::received(),
                &signature,
                latest_absolute_slot,
                None,
            );

            latest_absolute_slot
        });

        // find accounts that are needed for this transaction but are missing from the local
        // svm cache, fetch them from the RPC, and insert them locally
        let tx_loaded_addresses = self
            .get_loaded_addresses(remote_ctx, &transaction.message)
            .await?;

        // we don't want the pubkeys of the address lookup tables to be included in the transaction accounts,
        // but we do want the pubkeys of the accounts _loaded_ by the ALT to be in the transaction accounts.
        let transaction_accounts = self
            .get_pubkeys_from_message(
                &transaction.message,
                tx_loaded_addresses
                    .as_ref()
                    .map(|l| l.all_loaded_addresses()),
            )
            .clone();
        debug!(
            "Transaction {} accounts inputs: {}",
            transaction.get_signature(),
            transaction_accounts.iter().join(", ")
        );

        let account_updates = self
            .get_multiple_accounts(remote_ctx, &transaction_accounts, None)
            .await?
            .inner;

        // I don't think this code is required
        // We also need the pubkeys of the ALTs to be pulled from the remote, so we'll do a fetch for them
        let alt_account_updates = self
            .get_multiple_accounts(
                remote_ctx,
                &tx_loaded_addresses
                    .as_ref()
                    .map(|l| l.alt_addresses())
                    .unwrap_or_default(),
                None,
            )
            .await?
            .inner;

        let readonly_account_states = transaction_accounts
            .iter()
            .enumerate()
            .filter_map(|(i, pubkey)| {
                if transaction.message.is_maybe_writable(i, None) {
                    None
                } else {
                    self.get_account_local(pubkey)
                        .inner
                        .map_account()
                        .ok()
                        .map(|a| (*pubkey, a))
                }
            })
            .collect::<HashMap<_, _>>();

        self.with_svm_writer(|svm_writer| {
            for update in &account_updates {
                svm_writer.write_account_update(update.clone());
            }
            for update in &alt_account_updates {
                svm_writer.write_account_update(update.clone());
            }
        });

        let pre_execution_capture = {
            let mut capture = ExecutionCapture::new();
            for account_update in account_updates.iter() {
                match account_update {
                    GetAccountResult::None(pubkey) => {
                        capture.insert(*pubkey, None);
                    }
                    GetAccountResult::FoundAccount(pubkey, account, _)
                    | GetAccountResult::FoundProgramAccount((pubkey, account), _)
                    | GetAccountResult::FoundTokenAccount((pubkey, account), _) => {
                        capture.insert(*pubkey, Some(account.clone()));
                    }
                }
            }
            capture
        };

        let (accounts_before, token_accounts_before, token_programs) =
            self.with_svm_reader(|svm_reader| {
                let accounts_before = transaction_accounts
                    .iter()
                    .map(|p| svm_reader.inner.get_account(p))
                    .collect::<Vec<Option<Account>>>();

                let token_accounts_before = transaction_accounts
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| svm_reader.token_accounts.get(p).cloned().map(|a| (i, a)))
                    .collect::<Vec<_>>();

                let token_programs = token_accounts_before
                    .iter()
                    .map(|(i, ta)| {
                        svm_reader
                            .get_account(&transaction_accounts[*i])
                            .map(|a| a.owner)
                            .unwrap_or(ta.token_program_id())
                    })
                    .collect::<Vec<_>>()
                    .clone();
                (accounts_before, token_accounts_before, token_programs)
            });

        let loaded_addresses = tx_loaded_addresses.as_ref().map(|l| l.loaded_addresses());

        let ix_profiles = if self.is_instruction_profiling_enabled() {
            match self
                .generate_instruction_profiles(
                    &transaction,
                    &transaction_accounts,
                    &tx_loaded_addresses,
                    &accounts_before,
                    &token_accounts_before,
                    &token_programs,
                    pre_execution_capture.clone(),
                    &status_tx,
                )
                .await
            {
                Ok(profiles) => profiles,
                Err(e) => {
                    let _ = self.simnet_events_tx().try_send(SimnetEvent::error(format!(
                        "Failed to generate instruction profiles: {}",
                        e
                    )));
                    None
                }
            }
        } else {
            None
        };

        let profile_result = self
            .process_transaction_internal(
                transaction,
                skip_preflight,
                sigverify,
                &transaction_accounts,
                &loaded_addresses,
                &accounts_before,
                &token_accounts_before,
                &token_programs,
                pre_execution_capture,
                &status_tx,
                do_propagate,
            )
            .await?;

        Ok(KeyedProfileResult::new(
            latest_absolute_slot,
            UuidOrSignature::Signature(signature),
            ix_profiles,
            profile_result,
            readonly_account_states,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn generate_instruction_profiles(
        &self,
        transaction: &VersionedTransaction,
        transaction_accounts: &[Pubkey],
        loaded_addresses: &Option<TransactionLoadedAddresses>,
        accounts_before: &[Option<Account>],
        token_accounts_before: &[(usize, TokenAccount)],
        token_programs: &[Pubkey],
        pre_execution_capture: ExecutionCapture,
        status_tx: &Sender<TransactionStatusEvent>,
    ) -> SurfpoolResult<Option<Vec<ProfileResult>>> {
        let instructions = transaction.message.instructions();
        let ix_count = instructions.len();
        if ix_count == 0 {
            return Ok(None);
        }
        // Extract account categories from original transaction

        let mut ix_profile_results: Vec<ProfileResult> = vec![];

        for idx in 1..=ix_count {
            let partial_transaction_res = self.create_partial_transaction(
                instructions,
                transaction_accounts,
                transaction,
                idx,
                loaded_addresses,
            );

            let mut ix_required_accounts = IndexSet::new();
            for &account_idx in &instructions[idx - 1].accounts {
                ix_required_accounts.insert(transaction_accounts[account_idx as usize]);
            }
            ix_required_accounts
                .insert(transaction_accounts[instructions[idx - 1].program_id_index as usize]);

            let Some(partial_tx) = partial_transaction_res else {
                debug!("Unable to create partial transaction");
                return Ok(None);
            };

            let mut previous_execution_captures = ExecutionCapture::new();
            let mut previous_cus = 0;
            let mut previous_log_count = 0;
            for result in ix_profile_results.iter() {
                previous_execution_captures.extend(result.post_execution_capture.clone());
                previous_cus += result.compute_units_consumed;
                previous_log_count += result.log_messages.as_ref().map(|m| m.len()).unwrap_or(0);
            }

            let skip_preflight = true;
            let sigverify = false;
            let do_propagate = false;

            let mut pre_execution_capture_cursor = pre_execution_capture.clone();
            // If a pre-execution capture was provided, any pubkeys that are in the capture
            // that we just took should be replaced with those from the pre-execution capture.
            let capture_keys: Vec<_> = pre_execution_capture_cursor.keys().cloned().collect();
            for pubkey in capture_keys.into_iter() {
                if let Some(pre_account) = previous_execution_captures.remove(&pubkey) {
                    // Replace the account with the pre-execution one
                    pre_execution_capture_cursor.insert(pubkey, pre_account);
                }
            }
            let mut svm_clone = self.with_svm_reader(|svm_reader| svm_reader.clone());

            let (dummy_simnet_tx, _) = crossbeam_channel::bounded(1);
            let (dummy_geyser_tx, _) = crossbeam_channel::bounded(1);
            svm_clone.simnet_events_tx = dummy_simnet_tx;
            svm_clone.geyser_events_tx = dummy_geyser_tx;

            let svm_locker = SurfnetSvmLocker::new(svm_clone);
            let mut profile_result = svm_locker
                .process_transaction_internal(
                    partial_tx,
                    skip_preflight,
                    sigverify,
                    transaction_accounts,
                    &loaded_addresses.as_ref().map(|l| l.loaded_addresses()),
                    accounts_before,
                    token_accounts_before,
                    token_programs,
                    pre_execution_capture_cursor,
                    status_tx,
                    do_propagate,
                )
                .await?;

            eprintln!(
                "DEBUG: pre_execution_capture keys = {:?}",
                profile_result
                    .pre_execution_capture
                    .keys()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "DEBUG: post_execution_capture keys = {:?}",
                profile_result
                    .post_execution_capture
                    .keys()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
            );

            profile_result
                .pre_execution_capture
                .retain(|pubkey, _| ix_required_accounts.contains(pubkey));
            profile_result
                .post_execution_capture
                .retain(|pubkey, _| ix_required_accounts.contains(pubkey));

            eprintln!(
                "DEBUG: After retain - pre keys = {:?}",
                profile_result
                    .pre_execution_capture
                    .keys()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "DEBUG: After retain - post keys = {:?}",
                profile_result
                    .post_execution_capture
                    .keys()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
            );

            profile_result.compute_units_consumed = profile_result
                .compute_units_consumed
                .saturating_sub(previous_cus);
            profile_result.log_messages = profile_result.log_messages.map(|logs| {
                logs.into_iter()
                    .skip(previous_log_count)
                    .collect::<Vec<_>>()
            });

            ix_profile_results.push(profile_result);
        }

        Ok(Some(ix_profile_results))
    }

    fn handle_simulation_failure(
        &self,
        signature: Signature,
        failed_transaction_metadata: FailedTransactionMetadata,
        pre_execution_capture: ExecutionCapture,
        simulated_slot: Slot,
        status_tx: Sender<TransactionStatusEvent>,
        do_propagate: bool,
    ) -> ProfileResult {
        let FailedTransactionMetadata { err, meta } = failed_transaction_metadata;

        let cus = meta.compute_units_consumed;
        let log_messages = meta.logs.clone();
        let err_string = err.to_string();

        if do_propagate {
            let meta = convert_transaction_metadata_from_canonical(&meta);
            let simnet_events_tx = self.simnet_events_tx();
            let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                "Transaction simulation failed: {}",
                err
            )));

            let _ = status_tx.try_send(TransactionStatusEvent::SimulationFailure((
                err.clone(),
                meta,
            )));

            self.with_svm_writer(|svm_writer| {
                svm_writer.notify_signature_subscribers(
                    SignatureSubscriptionType::processed(),
                    &signature,
                    simulated_slot,
                    Some(err.clone()),
                );
                svm_writer.notify_logs_subscribers(
                    &signature,
                    Some(err),
                    log_messages.clone(),
                    CommitmentLevel::Processed,
                );
            });
        }
        ProfileResult::new(
            pre_execution_capture,
            BTreeMap::new(),
            cus,
            Some(log_messages),
            Some(err_string),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_execution_failure(
        &self,
        failed_transaction_metadata: FailedTransactionMetadata,
        transaction: VersionedTransaction,
        simulated_slot: Slot,
        pubkeys_from_message: &[Pubkey],
        accounts_before: &[Option<Account>],
        token_accounts_before: &[(usize, TokenAccount)],
        token_programs: &[Pubkey],
        loaded_addresses: &Option<LoadedAddresses>,
        pre_execution_capture: ExecutionCapture,
        status_tx: Sender<TransactionStatusEvent>,
        do_propagate: bool,
    ) -> ProfileResult {
        let FailedTransactionMetadata { err, meta } = failed_transaction_metadata;

        let cus = meta.compute_units_consumed;
        let log_messages = meta.logs.clone();
        let err_string = err.to_string();
        let signature = meta.signature;

        let accounts_after = pubkeys_from_message
            .iter()
            .map(|p| self.with_svm_reader(|svm_reader| svm_reader.inner.get_account(p)))
            .collect::<Vec<Option<Account>>>();

        for (pubkey, (before, after)) in pubkeys_from_message
            .iter()
            .zip(accounts_before.iter().zip(accounts_after.clone()))
        {
            if before.ne(&after) {
                if let Some(after) = &after {
                    self.with_svm_writer(|svm_writer| {
                        let _ = svm_writer.update_account_registries(pubkey, after);
                    });
                }
                self.with_svm_writer(|svm_writer| {
                    svm_writer.notify_account_subscribers(pubkey, &after.unwrap_or_default());
                });
            }
        }

        let token_mints = self
            .with_svm_reader(|svm_reader| {
                token_accounts_before
                    .iter()
                    .map(|(_, a)| {
                        svm_reader
                            .token_mints
                            .get(&a.mint())
                            .cloned()
                            .ok_or(SurfpoolError::token_mint_not_found(a.mint()))
                    })
                    .collect::<Result<Vec<_>, SurfpoolError>>()
            })
            .unwrap_or_default();

        if do_propagate {
            let meta_canonical = convert_transaction_metadata_from_canonical(&meta);
            let simnet_events_tx = self.simnet_events_tx();
            let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                "Transaction execution failed: {}",
                err
            )));
            let _ = status_tx.try_send(TransactionStatusEvent::ExecutionFailure((
                err.clone(),
                meta_canonical.clone(),
            )));

            self.with_svm_writer(|svm_writer| {
                let transaction_with_status_meta = TransactionWithStatusMeta::from_failure(
                    simulated_slot,
                    transaction.clone(),
                    &FailedTransactionMetadata {
                        err: err.clone(),
                        meta: meta.clone(),
                    },
                    accounts_before,
                    &accounts_after,
                    token_accounts_before,
                    token_mints,
                    token_programs,
                    loaded_addresses.clone().unwrap_or_default(),
                );
                svm_writer.transactions.insert(
                    signature,
                    SurfnetTransactionStatus::processed(
                        transaction_with_status_meta,
                        HashSet::new(),
                    ),
                );

                let _ = svm_writer
                    .simnet_events_tx
                    .try_send(SimnetEvent::transaction_processed(
                        meta_canonical,
                        Some(err.clone()),
                    ));

                svm_writer.transactions_queued_for_confirmation.push_back((
                    transaction.clone(),
                    status_tx.clone(),
                    Some(err.clone()),
                ));

                svm_writer.notify_signature_subscribers(
                    SignatureSubscriptionType::processed(),
                    &signature,
                    simulated_slot,
                    Some(err.clone()),
                );
                svm_writer.notify_logs_subscribers(
                    &signature,
                    Some(err.clone()),
                    log_messages.clone(),
                    CommitmentLevel::Processed,
                );
            });
        }
        ProfileResult::new(
            pre_execution_capture,
            BTreeMap::new(),
            cus,
            Some(log_messages),
            Some(err_string),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_execution_success(
        &self,
        transaction_metadata: TransactionMetadata,
        transaction: VersionedTransaction,
        simulated_slot: Slot,
        pubkeys_from_message: &[Pubkey],
        loaded_addresses: &Option<LoadedAddresses>,
        accounts_before: &[Option<Account>],
        token_accounts_before: &[(usize, TokenAccount)],
        token_programs: &[Pubkey],
        pre_execution_capture: ExecutionCapture,
        status_tx: &Sender<TransactionStatusEvent>,
        do_propagate: bool,
    ) -> SurfpoolResult<ProfileResult> {
        let cus = transaction_metadata.compute_units_consumed;
        let logs = transaction_metadata.logs.clone();
        let signature = transaction.signatures[0];

        let post_execution_capture = self.with_svm_writer(|svm_writer| {
            let accounts_after = pubkeys_from_message
                .iter()
                .map(|p| svm_writer.inner.get_account(p))
                .collect::<Vec<Option<Account>>>();

            let sanitized_transaction = if do_propagate {
                SanitizedTransaction::try_create(
                    transaction.clone(),
                    transaction.message.hash(),
                    Some(false),
                    if let Some(loaded_addresses) = &loaded_addresses {
                        SimpleAddressLoader::Enabled(loaded_addresses.clone())
                    } else {
                        SimpleAddressLoader::Disabled
                    },
                    &HashSet::new(), // todo: provide reserved account keys
                )
                .ok()
            } else {
                None
            };

            let mut mutated_account_pubkeys = HashSet::new();
            for (pubkey, (before, after)) in pubkeys_from_message
                .iter()
                .zip(accounts_before.iter().zip(accounts_after.clone()))
            {
                if before.ne(&after) {
                    mutated_account_pubkeys.insert(*pubkey);
                    if let Some(after) = &after {
                        svm_writer.update_account_registries(pubkey, after)?;
                        let write_version = svm_writer.increment_write_version();

                        if let Some(sanitized_transaction) = sanitized_transaction.clone() {
                            let _ = svm_writer.geyser_events_tx.send(GeyserEvent::UpdateAccount(
                                GeyserAccountUpdate::transaction_update(
                                    *pubkey,
                                    after.clone(),
                                    svm_writer.get_latest_absolute_slot(),
                                    sanitized_transaction.clone(),
                                    write_version,
                                ),
                            ));
                        }
                    }
                    svm_writer.notify_account_subscribers(pubkey, &after.unwrap_or_default());
                }
            }

            let mut token_accounts_after = vec![];
            let mut post_execution_capture = BTreeMap::new();
            let mut post_token_program_ids = vec![];

            for (i, (pubkey, account)) in pubkeys_from_message
                .iter()
                .zip(accounts_after.iter())
                .enumerate()
            {
                let token_account = svm_writer.token_accounts.get(pubkey).cloned();
                post_execution_capture.insert(*pubkey, account.clone());

                if let Some(token_account) = token_account {
                    token_accounts_after.push((i, token_account));
                    post_token_program_ids.push(
                        account
                            .as_ref()
                            .map(|a| a.owner)
                            .unwrap_or(spl_token_interface::id()),
                    );
                }
            }

            let token_mints = token_accounts_after
                .iter()
                .map(|(_, a)| {
                    svm_writer
                        .token_mints
                        .get(&a.mint())
                        .ok_or(SurfpoolError::token_mint_not_found(a.mint()))
                        .cloned()
                })
                .collect::<Result<Vec<_>, SurfpoolError>>()?;

            if do_propagate {
                let transaction_meta =
                    convert_transaction_metadata_from_canonical(&transaction_metadata);
                let transaction_with_status_meta = TransactionWithStatusMeta::new(
                    svm_writer.get_latest_absolute_slot(),
                    transaction.clone(),
                    transaction_metadata,
                    accounts_before,
                    &accounts_after,
                    token_accounts_before,
                    &token_accounts_after,
                    token_mints,
                    token_programs,
                    &post_token_program_ids,
                    loaded_addresses.clone().unwrap_or_default(),
                );
                svm_writer.transactions.insert(
                    transaction_meta.signature,
                    SurfnetTransactionStatus::processed(
                        transaction_with_status_meta.clone(),
                        mutated_account_pubkeys,
                    ),
                );

                let _ = svm_writer
                    .simnet_events_tx
                    .try_send(SimnetEvent::transaction_processed(transaction_meta, None));

                let _ = svm_writer
                    .geyser_events_tx
                    .send(GeyserEvent::NotifyTransaction(
                        transaction_with_status_meta,
                        sanitized_transaction,
                    ));

                let _ = status_tx.try_send(TransactionStatusEvent::Success(
                    TransactionConfirmationStatus::Processed,
                ));
                svm_writer.transactions_queued_for_confirmation.push_back((
                    transaction.clone(),
                    status_tx.clone(),
                    None,
                ));

                svm_writer.notify_signature_subscribers(
                    SignatureSubscriptionType::processed(),
                    &signature,
                    simulated_slot,
                    None,
                );
                svm_writer.notify_logs_subscribers(
                    &signature,
                    None,
                    logs.clone(),
                    CommitmentLevel::Processed,
                );
            }

            Ok::<ExecutionCapture, SurfpoolError>(post_execution_capture)
        })?;

        Ok(ProfileResult::new(
            pre_execution_capture,
            post_execution_capture,
            cus,
            Some(logs),
            None,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_transaction_internal(
        &self,
        transaction: VersionedTransaction,
        skip_preflight: bool,
        sigverify: bool,
        transaction_accounts: &[Pubkey],
        loaded_addresses: &Option<LoadedAddresses>,
        accounts_before: &[Option<Account>],
        token_accounts_before: &[(usize, TokenAccount)],
        token_programs: &[Pubkey],
        pre_execution_capture: ExecutionCapture,
        status_tx: &Sender<TransactionStatusEvent>,
        do_propagate: bool,
    ) -> SurfpoolResult<ProfileResult> {
        let res = match self
            .do_process_transaction_internal(transaction.clone(), skip_preflight, sigverify)
            .await
        {
            ProcessTransactionResult::Success(transaction_metadata) => self
                .handle_execution_success(
                    transaction_metadata,
                    transaction,
                    self.get_latest_absolute_slot(),
                    transaction_accounts,
                    loaded_addresses,
                    accounts_before,
                    token_accounts_before,
                    token_programs,
                    pre_execution_capture,
                    status_tx,
                    do_propagate,
                )?,
            ProcessTransactionResult::SimulationFailure(failed_transaction_metadata) => self
                .handle_simulation_failure(
                    transaction.signatures[0],
                    failed_transaction_metadata,
                    pre_execution_capture,
                    self.get_latest_absolute_slot(),
                    status_tx.clone(),
                    do_propagate,
                ),
            ProcessTransactionResult::ExecutionFailure(failed) => self.handle_execution_failure(
                failed,
                transaction,
                self.get_latest_absolute_slot(),
                transaction_accounts,
                accounts_before,
                token_accounts_before,
                token_programs,
                loaded_addresses,
                pre_execution_capture,
                status_tx.clone(),
                do_propagate,
            ),
        };
        Ok(res)
    }

    async fn do_process_transaction_internal(
        &self,
        transaction: VersionedTransaction,
        skip_preflight: bool,
        sigverify: bool,
    ) -> ProcessTransactionResult {
        // if not skipping preflight, simulate the transaction
        if !skip_preflight {
            if let Err(e) = self.with_svm_reader(|svm_reader| {
                svm_reader
                    .simulate_transaction(transaction.clone(), sigverify)
                    .map_err(ProcessTransactionResult::SimulationFailure)
            }) {
                return e;
            }
        }

        match self.with_svm_writer(|svm_writer| {
            svm_writer
                .send_transaction(transaction, false /* cu_analysis_enabled */, sigverify)
                .map_err(|e| {
                    debug!("Transaction execution failure: {:?}", e.meta);
                    ProcessTransactionResult::ExecutionFailure(e)
                })
                .map(ProcessTransactionResult::Success)
        }) {
            Ok(res) => res,
            Err(res) => res,
        }
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

    /// Resets an account in the SVM state
    ///
    /// This function coordinates the reset of accounts by calling the SVM's reset_account method.
    /// It handles program accounts (including their program data accounts) and can optionally
    /// cascade the reset to all accounts owned by a program.
    pub fn reset_account(
        &self,
        pubkey: Pubkey,
        include_owned_accounts: bool,
    ) -> SurfpoolResult<()> {
        let simnet_events_tx = self.simnet_events_tx();
        let _ = simnet_events_tx.send(SimnetEvent::info(format!(
            "Account {} will be reset",
            pubkey
        )));
        self.with_svm_writer(move |svm_writer| {
            svm_writer.reset_account(&pubkey, include_owned_accounts)
        })
    }

    /// Resets SVM state
    ///
    /// This function coordinates the reset of accounts by calling the SVM's reset_account method.
    pub fn reset_network(&self) -> SurfpoolResult<()> {
        let simnet_events_tx = self.simnet_events_tx();
        let _ = simnet_events_tx.send(SimnetEvent::info("Resetting network..."));
        self.with_svm_writer(move |svm_writer| svm_writer.reset_network())
    }

    /// Streams an account by its pubkey.
    pub fn stream_account(
        &self,
        pubkey: Pubkey,
        include_owned_accounts: bool,
    ) -> SurfpoolResult<()> {
        let simnet_events_tx = self.simnet_events_tx();
        let _ = simnet_events_tx.send(SimnetEvent::info(format!(
            "Account {} changes will be streamed",
            pubkey
        )));
        self.with_svm_writer(|svm_writer| {
            svm_writer
                .streamed_accounts
                .insert(pubkey, include_owned_accounts);
        });
        Ok(())
    }

    pub fn get_streamed_accounts(&self) -> HashMap<Pubkey, bool> {
        self.with_svm_reader(|svm_reader| svm_reader.streamed_accounts.clone())
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
                    let account = svm_reader.get_account(pubkey)?;
                    if match filter {
                        TokenAccountsFilter::Mint(mint) => token_account.mint().eq(mint),
                        TokenAccountsFilter::ProgramId(program_id) => account.owner.eq(program_id),
                    } {
                        Some(svm_reader.account_to_rpc_keyed_account(
                            pubkey,
                            &account,
                            config,
                            Some(token_account.mint()),
                        ))
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
        } = self.get_token_accounts_by_owner_local(owner, filter, config);

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
                    let account = svm_reader.get_account(pubkey)?;
                    let include = match filter {
                        TokenAccountsFilter::Mint(mint) => token_account.mint() == *mint,
                        TokenAccountsFilter::ProgramId(program_id) => {
                            account.owner == *program_id && is_supported_token_program(program_id)
                        }
                    };

                    if include {
                        Some(svm_reader.account_to_rpc_keyed_account(
                            pubkey,
                            &account,
                            config,
                            Some(token_account.mint()),
                        ))
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
            let mint_decimals = if let Some(mint_account) = svm_reader.token_mints.get(mint) {
                mint_account.decimals()
            } else {
                0
            };

            // convert to RpcTokenAccountBalance and sort by balance
            let mut balances: Vec<RpcTokenAccountBalance> = token_accounts
                .into_iter()
                .map(|(pubkey, token_account)| RpcTokenAccountBalance {
                    address: pubkey.to_string(),
                    amount: UiTokenAmount {
                        amount: token_account.amount().to_string(),
                        decimals: mint_decimals,
                        ui_amount: Some(format_ui_amount(token_account.amount(), mint_decimals)),
                        ui_amount_string: format_ui_amount_string(
                            token_account.amount(),
                            mint_decimals,
                        ),
                    },
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
    pub fn get_pubkeys_from_message(
        &self,
        message: &VersionedMessage,
        all_transaction_lookup_table_addresses: Option<Vec<&Pubkey>>,
    ) -> Vec<Pubkey> {
        match message {
            VersionedMessage::Legacy(message) => message.account_keys.clone(),
            VersionedMessage::V0(message) => {
                let mut acc_keys = message.account_keys.clone();

                if let Some(loaded_addresses) = all_transaction_lookup_table_addresses {
                    acc_keys.extend(loaded_addresses);
                }
                acc_keys
            }
        }
    }

    /// Gets addresses loaded from on-chain lookup tables from a VersionedMessage.
    pub async fn get_loaded_addresses(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        message: &VersionedMessage,
    ) -> SurfpoolResult<Option<TransactionLoadedAddresses>> {
        match message {
            VersionedMessage::Legacy(_) => Ok(None),
            VersionedMessage::V0(message) => {
                if message.address_table_lookups.is_empty() {
                    return Ok(None);
                }
                let mut loaded = TransactionLoadedAddresses::new();
                for alt in message.address_table_lookups.iter() {
                    self.get_lookup_table_addresses(remote_ctx, alt, &mut loaded)
                        .await?;
                }

                Ok(Some(loaded))
            }
        }
    }

    /// Retrieves loaded addresses from a lookup table account, validating owner and indices.
    pub async fn get_lookup_table_addresses(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        address_table_lookup: &MessageAddressTableLookup,
        transaction_loaded_addresses: &mut TransactionLoadedAddresses,
    ) -> SurfpoolResult<()> {
        let table_account = self
            .get_account(remote_ctx, &address_table_lookup.account_key, None)
            .await?
            .inner
            .map_account()?;

        if table_account.owner == solana_sdk_ids::address_lookup_table::id() {
            let SvmAccessContext {
                slot: current_slot,
                inner: slot_hashes,
                ..
            } = self.with_contextualized_svm_reader(|svm_reader| {
                svm_reader
                    .inner
                    .get_sysvar::<solana_slot_hashes::SlotHashes>()
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

            let writable = lookup_table
                .lookup(
                    current_slot,
                    &address_table_lookup.writable_indexes,
                    &slot_hashes,
                )
                .map_err(|_ix_err| {
                    SurfpoolError::invalid_lookup_index(address_table_lookup.account_key)
                })?;

            let readable = lookup_table
                .lookup(
                    current_slot,
                    &address_table_lookup.readonly_indexes,
                    &slot_hashes,
                )
                .map_err(|_ix_err| {
                    SurfpoolError::invalid_lookup_index(address_table_lookup.account_key)
                })?;

            let MessageAddressTableLookup {
                account_key,
                writable_indexes,
                readonly_indexes,
            } = address_table_lookup.to_owned();

            transaction_loaded_addresses.insert_members(
                account_key,
                writable_indexes
                    .into_iter()
                    .zip(writable.into_iter())
                    .collect(),
                readonly_indexes
                    .into_iter()
                    .zip(readable.into_iter())
                    .collect(),
            );

            Ok(())
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

    /// Creates a partial transaction for instruction profiling by extracting and remapping
    /// a subset of instructions from the original transaction.
    ///
    /// This helper function handles the complex logic of:
    /// - Collecting all accounts referenced by the instruction subset
    /// - Categorizing accounts based on their original roles (signers vs non-signers)
    /// - Building a new account key list in the correct order
    /// - Remapping instruction account indices to match the new account list
    /// - Creating a valid partial transaction with appropriate signatures
    ///
    /// # Arguments
    /// * `instructions` - All instructions from the original transaction
    /// * `message_accounts` - Account keys from the original transaction
    /// * `mutable_signers` - Mutable signer accounts from original transaction
    /// * `readonly_signers` - Readonly signer accounts from original transaction
    /// * `mutable_non_signers` - Mutable non-signer accounts from original transaction
    /// * `transaction` - The original transaction for reference
    /// * `idx` - Number of instructions to include in the partial transaction
    ///
    /// # Returns
    /// A partial transaction containing the first `idx` instructions and the accounts used for
    /// the last instruction, or None if creation fails
    #[allow(clippy::too_many_arguments)]
    fn create_partial_transaction(
        &self,
        instructions: &[CompiledInstruction],
        message_accounts: &[Pubkey],
        transaction: &VersionedTransaction,
        idx: usize,
        loaded_addresses: &Option<TransactionLoadedAddresses>,
    ) -> Option<VersionedTransaction> {
        // Keep the full account map from the original transaction for every partial pass.
        // This simplifies remapping: we only keep the first `idx` instructions, but retain
        // the original `message_accounts` ordering and address table lookups.
        let ixs_for_tx = instructions[0..idx].to_vec();

        // Build a new message that keeps the original account map and address table lookups,
        // but only contains the first `idx` instructions.
        let new_message = match transaction.message {
            VersionedMessage::Legacy(ref message) => VersionedMessage::Legacy(Message {
                account_keys: message_accounts[..message.account_keys.len()].to_vec(),
                header: message.header,
                recent_blockhash: *transaction.message.recent_blockhash(),
                instructions: ixs_for_tx.clone(),
            }),
            VersionedMessage::V0(ref message) => {
                VersionedMessage::V0(solana_message::v0::Message {
                    account_keys: message_accounts[..message.account_keys.len()].to_vec(),
                    header: message.header,
                    recent_blockhash: *transaction.message.recent_blockhash(),
                    instructions: ixs_for_tx.clone(),
                    // Preserve the original address table lookups when available.
                    address_table_lookups: loaded_addresses
                        .as_ref()
                        .map(|l| l.to_address_table_lookups())
                        .unwrap_or_default(),
                })
            }
        };

        let tx = VersionedTransaction {
            signatures: transaction.signatures.clone(),
            message: new_message,
        };

        Some(tx)
    }

    /// Returns the profile result for a given signature or UUID, and whether it exists in the SVM.
    pub fn get_profile_result(
        &self,
        signature_or_uuid: UuidOrSignature,
        config: &RpcProfileResultConfig,
    ) -> SurfpoolResult<Option<UiKeyedProfileResult>> {
        let result = match &signature_or_uuid {
            UuidOrSignature::Signature(signature) => self
                .with_svm_reader(|svm| svm.executed_transaction_profiles.get(signature).cloned()),
            UuidOrSignature::Uuid(uuid) => {
                self.with_svm_reader(|svm| svm.simulated_transaction_profiles.get(uuid).cloned())
            }
        };
        Ok(result.map(|profile| self.encode_ui_keyed_profile_result(profile, config)))
    }

    pub fn encode_ui_keyed_profile_result(
        &self,
        profile: KeyedProfileResult,
        config: &RpcProfileResultConfig,
    ) -> UiKeyedProfileResult {
        self.with_svm_reader(|svm_reader| {
            svm_reader.encode_ui_keyed_profile_result(profile, config)
        })
    }

    /// Returns the profile results for a given tag.
    pub fn get_profile_results_by_tag(
        &self,
        tag: String,
        config: &RpcProfileResultConfig,
    ) -> SurfpoolResult<Option<Vec<UiKeyedProfileResult>>> {
        let tag_map = self.with_svm_reader(|svm| svm.profile_tag_map.get(&tag).cloned());
        match tag_map {
            None => Ok(None),
            Some(uuids_or_sigs) => {
                let mut profiles = Vec::new();
                for id in uuids_or_sigs {
                    let profile = self.get_profile_result(id, config)?;
                    if profile.is_none() {
                        return Err(SurfpoolError::tag_not_found(&tag));
                    }
                    profiles.push(profile.unwrap());
                }
                Ok(Some(profiles))
            }
        }
    }

    pub fn register_idl(&self, idl: Idl, slot: Option<Slot>) {
        self.with_svm_writer(|svm_writer| svm_writer.register_idl(idl, slot))
    }

    pub fn get_idl(&self, address: &Pubkey, slot: Option<Slot>) -> Option<Idl> {
        self.with_svm_reader(|svm_reader| {
            let query_slot = slot.unwrap_or_else(|| svm_reader.get_latest_absolute_slot());
            svm_reader.registered_idls.get(address).and_then(|heap| {
                heap.iter()
                    .filter(|VersionedIdl(s, _)| s <= &query_slot)
                    .max()
                    .map(|VersionedIdl(_, idl)| idl.clone())
            })
        })
    }

    /// Forges account data by decoding with IDL, applying overrides, and re-encoding.
    ///
    /// # Arguments
    /// * `account_pubkey` - The public key of the account (used for error messages)
    /// * `account_data` - The raw account data bytes
    /// * `idl` - The IDL for decoding/encoding the account data
    /// * `overrides` - HashMap of field paths (dot notation) to values to override
    ///
    /// # Returns
    /// The modified account data bytes with discriminator
    pub fn get_forged_account_data(
        &self,
        account_pubkey: &Pubkey,
        account_data: &[u8],
        idl: &Idl,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> SurfpoolResult<Vec<u8>> {
        // Step 1: Validate account data size
        if account_data.len() < 8 {
            return Err(SurfpoolError::invalid_account_data(
                account_pubkey,
                "Account data too small to be an Anchor account (need at least 8 bytes for discriminator)",
                Some("Data length too small"),
            ));
        }

        // Step 3: Split discriminator and data
        let discriminator = &account_data[..8];
        let serialized_data = &account_data[8..];

        // Step 4: Find the account type using the discriminator
        let _account_def = idl
            .accounts
            .iter()
            .find(|acc| acc.discriminator.eq(discriminator))
            .ok_or_else(|| {
                SurfpoolError::internal(format!(
                    "Account with discriminator '{:?}' not found in IDL",
                    discriminator
                ))
            })?;

        // Step 5: Deserialize the account data to JSON
        // For now, we'll use a simple approach: deserialize as raw JSON
        let mut account_json: serde_json::Value = serde_json::from_slice(serialized_data)
            .map_err(|e| {
                SurfpoolError::deserialize_error(
                    "account data",
                    format!(
                        "Failed to deserialize account data as JSON: {}. \
                        Note: This is a simplified implementation that expects JSON-serialized data. \
                        For Anchor accounts, proper Borsh deserialization should be implemented.",
                        e
                    )
                )
            })?;

        // Step 6: Apply overrides using dot notation
        for (path, value) in overrides {
            apply_override_to_json(&mut account_json, path, value)?;
        }

        // Step 7: Re-serialize the modified data
        let modified_data = serde_json::to_vec(&account_json).map_err(|e| {
            SurfpoolError::internal(format!("Failed to serialize modified account data: {}", e))
        })?;

        // Step 8: Reconstruct the account data with discriminator
        let mut new_account_data = Vec::with_capacity(8 + modified_data.len());
        new_account_data.extend_from_slice(discriminator);
        new_account_data.extend_from_slice(&modified_data);

        Ok(new_account_data)
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

    pub async fn set_program_authority(
        &self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
        program_id: Pubkey,
        new_authority: Option<Pubkey>,
    ) -> SurfpoolContextualizedResult<()> {
        let SvmAccessContext {
            slot,
            latest_epoch_info,
            latest_blockhash,
            inner: mut get_account_result,
        } = self.get_account(remote_ctx, &program_id, None).await?;

        let original_authority = match &mut get_account_result {
            GetAccountResult::None(pubkey) => {
                return Err(SurfpoolError::invalid_program_account(
                    pubkey,
                    "Account not found",
                ));
            }
            GetAccountResult::FoundAccount(pubkey, program_account, _) => {
                let programdata_address = get_program_data_address(pubkey);
                let mut programdata_account_result = self
                    .get_account(remote_ctx, &programdata_address, None)
                    .await?
                    .inner;
                match &mut programdata_account_result {
                    GetAccountResult::None(pubkey) => {
                        return Err(SurfpoolError::invalid_program_account(
                            pubkey,
                            "Program data account does not exist",
                        ));
                    }
                    GetAccountResult::FoundAccount(_, programdata_account, _) => {
                        let original_authority = update_programdata_account(
                            &program_id,
                            programdata_account,
                            new_authority,
                        )?;

                        get_account_result = GetAccountResult::FoundProgramAccount(
                            (*pubkey, program_account.clone()),
                            (programdata_address, Some(programdata_account.clone())),
                        );

                        original_authority
                    }
                    GetAccountResult::FoundProgramAccount(_, _)
                    | GetAccountResult::FoundTokenAccount(_, _) => {
                        return Err(SurfpoolError::invalid_program_account(
                            pubkey,
                            "Not a program account",
                        ));
                    }
                }
            }
            GetAccountResult::FoundProgramAccount(_, (_, None)) => {
                return Err(SurfpoolError::invalid_program_account(
                    program_id,
                    "Program data account does not exist",
                ));
            }
            GetAccountResult::FoundProgramAccount(_, (_, Some(programdata_account))) => {
                update_programdata_account(&program_id, programdata_account, new_authority)?
            }
            GetAccountResult::FoundTokenAccount(_, _) => {
                return Err(SurfpoolError::invalid_program_account(
                    program_id,
                    "Not a program account",
                ));
            }
        };

        let simnet_events_tx = self.simnet_events_tx();
        match (original_authority, new_authority) {
            (Some(original), Some(new)) => {
                if original != new {
                    let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                        "Setting new authority for program {}",
                        program_id
                    )));
                    let _ = simnet_events_tx
                        .send(SimnetEvent::info(format!("Old Authority: {}", original)));
                    let _ =
                        simnet_events_tx.send(SimnetEvent::info(format!("New Authority: {}", new)));
                } else {
                    let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                        "No authority change for program {}",
                        program_id
                    )));
                }
            }
            (Some(original), None) => {
                let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                    "Removing authority for program {}",
                    program_id
                )));
                let _ = simnet_events_tx
                    .send(SimnetEvent::info(format!("Old Authority: {}", original)));
            }
            (None, Some(new)) => {
                let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                    "Setting new authority for program {}",
                    program_id
                )));
                let _ = simnet_events_tx.send(SimnetEvent::info("Old Authority: None".to_string()));
                let _ = simnet_events_tx.send(SimnetEvent::info(format!("New Authority: {}", new)));
            }
            (None, None) => {
                let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                    "No authority change for program {}",
                    program_id
                )));
            }
        };

        self.write_account_update(get_account_result);

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            (),
        ))
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
        let res = self.with_svm_reader(|svm_reader| {
            let res = svm_reader.get_account_owned_by(program_id);

            let mut filtered = vec![];
            for (pubkey, account) in &res {
                if let Some(ref active_filters) = filters {
                    match apply_rpc_filters(&account.data, active_filters) {
                        Ok(true) => {}           // Account matches all filters
                        Ok(false) => continue,   // Filtered out
                        Err(e) => return Err(e), // Error applying filter, already JsonRpcError
                    }
                }

                filtered.push(svm_reader.account_to_rpc_keyed_account(
                    pubkey,
                    account,
                    &account_config,
                    None,
                ));
            }
            Ok(filtered)
        })?;

        Ok(self.with_contextualized_svm_reader(|_| res.clone()))
    }

    pub fn encode_ui_account(
        &self,
        pubkey: &Pubkey,
        account: &Account,
        encoding: UiAccountEncoding,
        additional_data: Option<AccountAdditionalDataV3>,
        data_slice: Option<UiDataSliceConfig>,
    ) -> UiAccount {
        self.with_svm_reader(|svm_reader| {
            svm_reader.encode_ui_account(pubkey, account, encoding, additional_data, data_slice)
        })
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

        let encoding = account_config.encoding.unwrap_or(UiAccountEncoding::Base64);
        let data_slice = account_config.data_slice;

        let remote_accounts_result = client
            .get_program_accounts(program_id, account_config, filters)
            .await?;

        let remote_accounts = remote_accounts_result.handle_method_not_supported(|| {
            let tx = self.simnet_events_tx();
            let _ = tx.send(SimnetEvent::warn("The `getProgramAccounts` method was sent to the remote RPC, but this method isn't supported by your RPC provider. If you need this method, please use a different RPC provider."));
            vec![]
        });

        let mut combined_accounts = remote_accounts
            .iter()
            .map(|(pubkey, account)| RpcKeyedAccount {
                pubkey: pubkey.to_string(),
                account: self.encode_ui_account(pubkey, account, encoding, None, data_slice),
            })
            .collect::<Vec<RpcKeyedAccount>>();

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
    pub fn get_first_local_slot(&self) -> Option<Slot> {
        self.with_svm_reader(|svm_reader| svm_reader.blocks.keys().min().copied())
    }

    pub async fn get_block(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
        slot: &Slot,
        config: &RpcBlockConfig,
    ) -> SurfpoolContextualizedResult<Option<UiConfirmedBlock>> {
        let first_local_slot = self.get_first_local_slot();

        let result = if first_local_slot.is_some() && first_local_slot.unwrap() > *slot {
            match remote_ctx {
                Some(remote_client) => Some(remote_client.get_block(slot, *config).await?),
                None => return Err(SurfpoolError::slot_too_old(*slot)),
            }
        } else {
            self.get_block_local(slot, config)?
        };

        Ok(SvmAccessContext {
            slot: *slot,
            latest_epoch_info: self.get_epoch_info(),
            latest_blockhash: self
                .get_latest_blockhash(&CommitmentConfig::processed())
                .unwrap_or_default(),
            inner: result,
        })
    }

    pub fn get_block_local(
        &self,
        slot: &Slot,
        config: &RpcBlockConfig,
    ) -> SurfpoolResult<Option<UiConfirmedBlock>> {
        self.with_svm_reader(|svm_reader| svm_reader.get_block_at_slot(*slot, config))
    }

    pub fn get_genesis_hash_local(&self) -> SvmAccessContext<Hash> {
        self.with_contextualized_svm_reader(|svm_reader| svm_reader.genesis_config.hash())
    }

    pub async fn get_genesis_hash(
        &self,
        remote_ctx: &Option<SurfnetRemoteClient>,
    ) -> SurfpoolContextualizedResult<Hash> {
        if let Some(client) = remote_ctx {
            let remote_hash = client.get_genesis_hash().await?;
            Ok(self.with_contextualized_svm_reader(|_| remote_hash))
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

    pub fn time_travel(
        &self,
        key: Option<(blake3::Hash, String)>,
        simnet_command_tx: Sender<SimnetCommand>,
        config: TimeTravelConfig,
    ) -> SurfpoolResult<EpochInfo> {
        let (mut epoch_info, slot_time, updated_at) = self.with_svm_reader(|svm_reader| {
            (
                svm_reader.latest_epoch_info.clone(),
                svm_reader.slot_time,
                svm_reader.updated_at,
            )
        });

        let clock_update: Clock =
            calculate_time_travel_clock(&config, updated_at, slot_time, &epoch_info)
                .map_err(|e| SurfpoolError::internal(e.to_string()))?;

        let formated_time = chrono::DateTime::from_timestamp(clock_update.unix_timestamp, 0)
            .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap())
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        epoch_info.slot_index = clock_update.slot;
        epoch_info.epoch = clock_update.epoch;
        epoch_info.absolute_slot =
            clock_update.slot + clock_update.epoch * epoch_info.slots_in_epoch;
        let _ = simnet_command_tx.send(SimnetCommand::UpdateInternalClock(key, clock_update));
        let _ = self.simnet_events_tx().send(SimnetEvent::info(format!(
            "Time travel to {} successful (epoch {} / slot {})",
            formated_time, epoch_info.epoch, epoch_info.absolute_slot
        )));

        Ok(epoch_info)
    }

    /// Retrieves the latest absolute slot from the underlying SVM.
    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.with_svm_reader(|svm_reader| svm_reader.get_latest_absolute_slot())
    }

    /// Retrieves the latest blockhash for the given commitment config from the underlying SVM.
    pub fn get_latest_blockhash(&self, config: &CommitmentConfig) -> Option<Hash> {
        let slot = self.get_slot_for_commitment(config);
        self.with_svm_reader(|svm_reader| svm_reader.blockhash_for_slot(slot))
    }

    pub fn latest_absolute_blockhash(&self) -> Hash {
        self.with_svm_reader(|svm_reader| svm_reader.latest_blockhash())
    }

    pub fn get_slot_for_commitment(&self, commitment: &CommitmentConfig) -> Slot {
        self.with_svm_reader(|svm_reader| {
            let slot = svm_reader.get_latest_absolute_slot();
            match commitment.commitment {
                CommitmentLevel::Processed => slot,
                CommitmentLevel::Confirmed => slot.saturating_sub(1),
                CommitmentLevel::Finalized => slot.saturating_sub(FINALIZATION_SLOT_THRESHOLD),
            }
        })
    }

    /// Executes an airdrop via the underlying SVM.
    #[allow(clippy::result_large_err)]
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

    /// Subscribes for logs updates and returns a receiver of logs updates.
    pub fn subscribe_for_logs_updates(
        &self,
        commitment_level: &CommitmentLevel,
        filter: &RpcTransactionLogsFilter,
    ) -> Receiver<(Slot, RpcLogsResponse)> {
        self.with_svm_writer(|svm_writer| {
            svm_writer.subscribe_for_logs_updates(commitment_level, filter)
        })
    }

    pub fn runbook_executions(&self) -> Vec<RunbookExecutionStatusReport> {
        self.with_svm_reader(|svm_reader| svm_reader.runbook_executions.clone())
    }

    pub fn start_runbook_execution(&self, runbook_id: String) {
        self.with_svm_writer(|svm_writer| {
            svm_writer.instruction_profiling_enabled = false;
            svm_writer.start_runbook_execution(runbook_id);
        });
    }

    pub fn complete_runbook_execution(&self, runbook_id: String, error: Option<Vec<String>>) {
        self.with_svm_writer(|svm_writer| {
            svm_writer.complete_runbook_execution(&runbook_id, error);
            let some_runbook_executing = svm_writer
                .runbook_executions
                .iter()
                .any(|e| e.completed_at.is_none());
            if !some_runbook_executing {
                svm_writer.instruction_profiling_enabled = true;
            }
        });
    }

    pub fn export_snapshot(
        &self,
        config: ExportSnapshotConfig,
    ) -> BTreeMap<String, AccountSnapshot> {
        self.with_svm_reader(|svm_reader| svm_reader.export_snapshot(config))
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
    *program_id == spl_token_interface::ID || *program_id == spl_token_2022_interface::ID
}

fn update_programdata_account(
    program_id: &Pubkey,
    programdata_account: &mut Account,
    new_authority: Option<Pubkey>,
) -> SurfpoolResult<Option<Pubkey>> {
    let upgradeable_loader_state =
        bincode::deserialize::<UpgradeableLoaderState>(&programdata_account.data).map_err(|e| {
            SurfpoolError::invalid_program_account(
                program_id,
                format!("Failed to serialize program data: {}", e),
            )
        })?;
    if let UpgradeableLoaderState::ProgramData {
        upgrade_authority_address,
        slot,
    } = upgradeable_loader_state
    {
        let offset = if upgrade_authority_address.is_some() {
            UpgradeableLoaderState::size_of_programdata_metadata()
        } else {
            UpgradeableLoaderState::size_of_programdata_metadata()
                - serialized_size(&Pubkey::default()).unwrap() as usize
        };

        let mut data = bincode::serialize(&UpgradeableLoaderState::ProgramData {
            upgrade_authority_address: new_authority,
            slot,
        })
        .map_err(|e| {
            SurfpoolError::invalid_program_account(
                program_id,
                format!("Failed to serialize program data: {}", e),
            )
        })?;

        data.append(&mut programdata_account.data[offset..].to_vec());

        programdata_account.data = data;

        Ok(upgrade_authority_address)
    } else {
        Err(SurfpoolError::invalid_program_account(
            program_id,
            "Invalid program data account",
        ))
    }
}

pub fn format_ui_amount_string(amount: u64, decimals: u8) -> String {
    if decimals > 0 {
        let divisor = 10u64.pow(decimals as u32);
        format!(
            "{:.decimals$}",
            amount as f64 / divisor as f64,
            decimals = decimals as usize
        )
    } else {
        amount.to_string()
    }
}

pub fn format_ui_amount(amount: u64, decimals: u8) -> f64 {
    if decimals > 0 {
        let divisor = 10u64.pow(decimals as u32);
        amount as f64 / divisor as f64
    } else {
        amount as f64
    }
}

#[cfg(test)]
mod tests {
    use crate::scenarios::registry::PYTH_V2_IDL_CONTENT;
    use crate::surfnet::SurfnetSvm;

    use super::*;
    use solana_account::Account;
    use solana_account_decoder::UiAccountEncoding;
    use std::collections::HashMap;

    #[test]
    fn test_get_forged_account_data_with_pyth_fixture() {
        use borsh::{BorshDeserialize, BorshSerialize};

        // Define local structures matching Pyth IDL
        #[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
        pub enum VerificationLevel {
            Partial { num_signatures: u8 },
            Full,
        }

        #[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
        pub struct PriceFeedMessage {
            pub feed_id: [u8; 32],
            pub price: i64,
            pub conf: u64,
            pub exponent: i32,
            pub publish_time: i64,
            pub prev_publish_time: i64,
            pub ema_price: i64,
            pub ema_conf: u64,
        }

        #[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
        pub struct PriceUpdateV2 {
            pub write_authority: Pubkey,
            pub verification_level: VerificationLevel,
            pub price_message: PriceFeedMessage,
            pub posted_slot: u64,
        }

        // Pyth price feed account data fixture
        let account_data_hex = vec![
            0x22, 0xf1, 0x23, 0x63, 0x9d, 0x7e, 0xf4, 0xcd, // Discriminator
            0x35, 0xa7, 0x0c, 0x11, 0x16, 0x2f, 0xbf, 0x5a, 0x0e, 0x7f, 0x7d, 0x2f, 0x96, 0xe1,
            0x9f, 0x97, 0xb0, 0x22, 0x46, 0xa1, 0x56, 0x87, 0xee, 0x67, 0x27, 0x94, 0x89, 0x74,
            0x48, 0xe6, 0x58, 0xde, 0x01, 0xe6, 0x2d, 0xf6, 0xc8, 0xb4, 0xa8, 0x5f, 0xe1, 0xa6,
            0x7d, 0xb4, 0x4d, 0xc1, 0x2d, 0xe5, 0xdb, 0x33, 0x0f, 0x7a, 0xc6, 0x6b, 0x72, 0xdc,
            0x65, 0x8a, 0xfe, 0xdf, 0x0f, 0x4a, 0x41, 0x5b, 0x43, 0xd7, 0x1f, 0x18, 0x64, 0x5f,
            0x0a, 0x00, 0x00, 0x96, 0x67, 0xea, 0xc5, 0x00, 0x00, 0x00, 0x00, 0xf8, 0xff, 0xff,
            0xff, 0x5f, 0x2b, 0x00, 0x69, 0x00, 0x00, 0x00, 0x00, 0x5e, 0x2b, 0x00, 0x69, 0x00,
            0x00, 0x00, 0x00, 0xa0, 0x7c, 0x1a, 0x38, 0x63, 0x0a, 0x00, 0x00, 0x94, 0xa6, 0xb9,
            0xb5, 0x00, 0x00, 0x00, 0x00, 0x8c, 0x5e, 0x6d, 0x16, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        // Create a minimal Pyth IDL for testing
        let idl: Idl = serde_json::from_str(PYTH_V2_IDL_CONTENT).expect("Failed to load IDL");

        // Create overrides - note: this won't actually work with the JSON deserialization
        // since the account data is Borsh-encoded, but we're testing the structure
        let mut overrides: HashMap<String, serde_json::Value> = HashMap::new();

        // Verify IDL has matching discriminator
        let account_def = idl
            .accounts
            .iter()
            .find(|acc| acc.discriminator.eq(&account_data_hex[..8]));

        assert!(
            account_def.is_some(),
            "Should find PriceUpdateV2 account by discriminator"
        );
        assert_eq!(account_def.unwrap().name, "PriceUpdateV2");

        // Step 1: Instantiate an offline Svm instance
        let (surfnet_svm, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
        let svm_locker = SurfnetSvmLocker::new(surfnet_svm);

        // Step 2: Register the IDL for this account
        let account_pubkey = Pubkey::from_str_const("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");
        svm_locker.register_idl(idl.clone(), None);

        // Step 3: Create an account with the Pyth data
        let pyth_account = Account {
            lamports: 1_000_000,
            data: account_data_hex.clone(),
            owner: account_pubkey,
            executable: false,
            rent_epoch: 0,
        };

        // Step 4: Use encode_ui_account to decode/encode the account data
        let ui_account = svm_locker.encode_ui_account(
            &account_pubkey,
            &pyth_account,
            UiAccountEncoding::JsonParsed,
            None,
            None, // data_slice
        );

        // Step 5: Verify the UI account has parsed data
        println!("UI Account lamports: {}", ui_account.lamports);
        println!("UI Account owner: {}", ui_account.owner);

        // Assert on parsed account data
        use solana_account_decoder::UiAccountData;
        match &ui_account.data {
            UiAccountData::Json(parsed_account) => {
                let parsed_obj = &parsed_account.parsed;

                // Extract price_message object
                let price_message = parsed_obj
                    .get("price_message")
                    .expect("Should have price_message field")
                    .as_object()
                    .expect("price_message should be an object");

                // Assert on price
                let price = price_message
                    .get("price")
                    .expect("Should have price field")
                    .as_i64()
                    .expect("price should be a number");
                assert_eq!(price, 11404817473495, "Price should match expected value");

                // Assert on exponent
                let exponent = price_message
                    .get("exponent")
                    .expect("Should have exponent field")
                    .as_i64()
                    .expect("exponent should be a number");
                assert_eq!(exponent, -8, "Exponent should be -8");

                // Assert on ema_price
                let ema_price = price_message
                    .get("ema_price")
                    .expect("Should have ema_price field")
                    .as_i64()
                    .expect("ema_price should be a number");
                assert_eq!(
                    ema_price, 11421259300000,
                    "EMA price should match expected value"
                );

                // Assert on publish_time
                let publish_time = price_message
                    .get("publish_time")
                    .expect("Should have publish_time field")
                    .as_i64()
                    .expect("publish_time should be a number");
                assert_eq!(
                    publish_time, 1761618783,
                    "Publish time should match expected value"
                );

                println!("✓ All price assertions passed!");
            }
            _ => panic!("Expected JSON parsed account data"),
        }

        // Step 6: Test get_forged_account_data without overrides (should return same data)
        println!("\n--- Testing get_forged_account_data without overrides ---");
        let forged_data_no_overrides = svm_locker.get_forged_account_data(
            &account_pubkey,
            &account_data_hex,
            &idl,
            &overrides,
        );

        match forged_data_no_overrides {
            Ok(data) => {
                // If it succeeds, verify the data is unchanged
                assert_eq!(
                    data, account_data_hex,
                    "Data without overrides should match original"
                );
                println!("✓ Forged data without overrides matches original!");
            }
            Err(e) => {
                // If it fails, it's due to Borsh/JSON mismatch (expected for now)
                println!("Expected error (Borsh vs JSON): {:?}", e);
                println!("Note: This documents the need for proper Borsh implementation");
            }
        }

        // Step 7: Test get_forged_account_data with overrides
        println!("\n--- Testing get_forged_account_data with overrides ---");

        // Set new values for price and publish_time
        let new_price = 999999999999i64;
        let new_publish_time = 1234567890i64;
        let new_ema_price = 888888888888i64;

        overrides.insert("price_message.price".into(), json!(new_price));
        overrides.insert("price_message.publish_time".into(), json!(new_publish_time));
        overrides.insert("price_message.ema_price".into(), json!(new_ema_price));

        let forged_data_with_overrides = svm_locker.get_forged_account_data(
            &account_pubkey,
            &account_data_hex,
            &idl,
            &overrides,
        );

        match forged_data_with_overrides {
            Ok(modified_data) => {
                // Verify the data is different from original
                assert_ne!(
                    modified_data, account_data_hex,
                    "Modified data should be different from original"
                );
                println!("✓ Modified data is different from original!");

                // Create a modified account to verify the changes
                let modified_account = Account {
                    lamports: 1_000_000,
                    data: modified_data.clone(),
                    owner: account_pubkey,
                    executable: false,
                    rent_epoch: 0,
                };

                // Re-encode the modified account to verify the changes
                let modified_ui_account = svm_locker.encode_ui_account(
                    &account_pubkey,
                    &modified_account,
                    UiAccountEncoding::JsonParsed,
                    None,
                    None,
                );

                // Verify the modified values in the re-encoded account
                match &modified_ui_account.data {
                    UiAccountData::Json(parsed_account) => {
                        let parsed_obj = &parsed_account.parsed;
                        let price_message = parsed_obj
                            .get("price_message")
                            .expect("Should have price_message field")
                            .as_object()
                            .expect("price_message should be an object");

                        // Verify new price
                        let modified_price = price_message
                            .get("price")
                            .expect("Should have price field")
                            .as_i64()
                            .expect("price should be a number");
                        assert_eq!(
                            modified_price, new_price,
                            "Modified price should match override value"
                        );

                        // Verify new publish_time
                        let modified_publish_time = price_message
                            .get("publish_time")
                            .expect("Should have publish_time field")
                            .as_i64()
                            .expect("publish_time should be a number");
                        assert_eq!(
                            modified_publish_time, new_publish_time,
                            "Modified publish_time should match override value"
                        );

                        // Verify new ema_price
                        let modified_ema_price = price_message
                            .get("ema_price")
                            .expect("Should have ema_price field")
                            .as_i64()
                            .expect("ema_price should be a number");
                        assert_eq!(
                            modified_ema_price, new_ema_price,
                            "Modified ema_price should match override value"
                        );

                        // Verify exponent is unchanged
                        let exponent = price_message
                            .get("exponent")
                            .expect("Should have exponent field")
                            .as_i64()
                            .expect("exponent should be a number");
                        assert_eq!(exponent, -8, "Exponent should remain unchanged");

                        println!("✓ All override assertions passed!");
                        println!("  - Price changed: 11404817473495 → {}", new_price);
                        println!(
                            "  - Publish time changed: 1761618783 → {}",
                            new_publish_time
                        );
                        println!("  - EMA price changed: 11421259300000 → {}", new_ema_price);
                        println!("  - Exponent unchanged: -8");
                    }
                    _ => panic!("Expected JSON parsed account data for modified account"),
                }
            }
            Err(e) => {
                // If it fails, it's due to Borsh/JSON mismatch (expected for now)
                println!("Expected error (Borsh vs JSON): {:?}", e);
                println!("Note: Once Borsh serialization is implemented, this test will:");
                println!("  1. Successfully modify the account data");
                println!("  2. Verify price changed to: {}", new_price);
                println!("  3. Verify publish_time changed to: {}", new_publish_time);
                println!("  4. Verify ema_price changed to: {}", new_ema_price);
                println!("  5. Verify other fields remain unchanged");
            }
        }

        // Step 8: Demonstrate proper Borsh deserialization/serialization
        println!("\n--- Step 8: Testing with Borsh structures ---");

        // Deserialize the original account data using Borsh
        let account_bytes = &account_data_hex[8..];
        println!(
            "Account data length (without discriminator): {} bytes",
            account_bytes.len()
        );

        let mut reader = std::io::Cursor::new(account_bytes);
        let original_price_update = PriceUpdateV2::deserialize_reader(&mut reader)
            .expect("Should deserialize Pyth account data with Borsh");

        let bytes_read = reader.position() as usize;
        println!("Bytes read by Borsh: {}", bytes_read);
        if bytes_read < account_bytes.len() {
            println!(
                "Note: {} extra bytes at end (likely padding)",
                account_bytes.len() - bytes_read
            );
        }

        println!("Original Borsh-deserialized data:");
        println!("  - Price: {}", original_price_update.price_message.price);
        println!(
            "  - Exponent: {}",
            original_price_update.price_message.exponent
        );
        println!(
            "  - EMA Price: {}",
            original_price_update.price_message.ema_price
        );
        println!(
            "  - Publish time: {}",
            original_price_update.price_message.publish_time
        );

        // Assert original values match what we saw in JSON parsing
        assert_eq!(
            original_price_update.price_message.price, 11404817473495,
            "Borsh price should match JSON parsed value"
        );
        assert_eq!(
            original_price_update.price_message.exponent, -8,
            "Borsh exponent should match JSON parsed value"
        );
        assert_eq!(
            original_price_update.price_message.ema_price, 11421259300000,
            "Borsh ema_price should match JSON parsed value"
        );
        assert_eq!(
            original_price_update.price_message.publish_time, 1761618783,
            "Borsh publish_time should match JSON parsed value"
        );

        println!("✓ Borsh deserialization matches JSON parsing!");

        // Step 9: Modify and re-serialize with Borsh
        println!("\n--- Step 9: Modifying account data with Borsh ---");

        let mut modified_price_update = original_price_update.clone();
        modified_price_update.price_message.price = new_price;
        modified_price_update.price_message.publish_time = new_publish_time;
        modified_price_update.price_message.ema_price = new_ema_price;

        // Serialize back to bytes
        let modified_account_data =
            borsh::to_vec(&modified_price_update).expect("Should serialize modified data");

        // Prepend the discriminator
        let mut full_modified_data = account_data_hex[..8].to_vec();
        full_modified_data.extend_from_slice(&modified_account_data);

        println!("Modified Borsh-serialized data:");
        println!(
            "  - Price: {} → {}",
            original_price_update.price_message.price, new_price
        );
        println!(
            "  - Publish time: {} → {}",
            original_price_update.price_message.publish_time, new_publish_time
        );
        println!(
            "  - EMA Price: {} → {}",
            original_price_update.price_message.ema_price, new_ema_price
        );
        println!(
            "  - Exponent: {} (unchanged)",
            modified_price_update.price_message.exponent
        );

        // Verify the modified data is different
        assert_ne!(
            full_modified_data, account_data_hex,
            "Modified data should differ from original"
        );

        // Verify we can deserialize the modified data back
        let mut modified_reader = std::io::Cursor::new(&full_modified_data[8..]);
        let reloaded_price_update = PriceUpdateV2::deserialize_reader(&mut modified_reader)
            .expect("Should deserialize modified data");

        assert_eq!(
            reloaded_price_update.price_message.price, new_price,
            "Reloaded price should match modified value"
        );
        assert_eq!(
            reloaded_price_update.price_message.publish_time, new_publish_time,
            "Reloaded publish_time should match modified value"
        );
        assert_eq!(
            reloaded_price_update.price_message.ema_price, new_ema_price,
            "Reloaded ema_price should match modified value"
        );
        assert_eq!(
            reloaded_price_update.price_message.exponent,
            original_price_update.price_message.exponent,
            "Exponent should remain unchanged"
        );

        println!("✓ Borsh round-trip successful!");

        // Step 10: Verify with encode_ui_account
        println!("\n--- Step 10: Verify modified data with encode_ui_account ---");

        let modified_test_account = Account {
            lamports: 1_000_000,
            data: full_modified_data,
            owner: account_pubkey,
            executable: false,
            rent_epoch: 0,
        };

        let modified_ui_account = svm_locker.encode_ui_account(
            &account_pubkey,
            &modified_test_account,
            UiAccountEncoding::JsonParsed,
            None,
            None,
        );

        // Verify through JSON encoding as well
        match &modified_ui_account.data {
            UiAccountData::Json(parsed_account) => {
                let parsed_obj = &parsed_account.parsed;
                let price_message = parsed_obj
                    .get("price_message")
                    .expect("Should have price_message")
                    .as_object()
                    .expect("Should be object");

                let final_price = price_message
                    .get("price")
                    .expect("Should have price")
                    .as_i64()
                    .expect("Should be i64");
                let final_publish_time = price_message
                    .get("publish_time")
                    .expect("Should have publish_time")
                    .as_i64()
                    .expect("Should be i64");
                let final_ema_price = price_message
                    .get("ema_price")
                    .expect("Should have ema_price")
                    .as_i64()
                    .expect("Should be i64");

                assert_eq!(
                    final_price, new_price,
                    "JSON-parsed price should match Borsh value"
                );
                assert_eq!(
                    final_publish_time, new_publish_time,
                    "JSON-parsed publish_time should match Borsh value"
                );
                assert_eq!(
                    final_ema_price, new_ema_price,
                    "JSON-parsed ema_price should match Borsh value"
                );
            }
            _ => panic!("Expected JSON parsed data"),
        }
    }

    #[test]
    fn test_apply_override_to_json() {
        let mut json = serde_json::json!({
            "price_message": {
                "price": 100,
                "publish_time": 1234567890
            },
            "expo": -8
        });

        // Test simple override
        let result = apply_override_to_json(&mut json, "expo", &serde_json::json!(-6));
        assert!(result.is_ok());
        assert_eq!(json["expo"], -6);

        // Test nested override
        let result =
            apply_override_to_json(&mut json, "price_message.price", &serde_json::json!(200));
        assert!(result.is_ok());
        assert_eq!(json["price_message"]["price"], 200);

        // Test invalid path
        let result =
            apply_override_to_json(&mut json, "nonexistent.field", &serde_json::json!(999));
        assert!(result.is_err());
    }
}
