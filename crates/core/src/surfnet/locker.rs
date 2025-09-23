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
use solana_message::{
    Message, MessageHeader, SimpleAddressLoader, VersionedMessage,
    v0::{LoadedAddresses, MessageAddressTableLookup},
};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_sdk::{
    bpf_loader_upgradeable::{UpgradeableLoaderState, get_program_data_address},
    instruction::CompiledInstruction,
    transaction::{SanitizedTransaction, TransactionVersion, VersionedTransaction},
};
use solana_signature::Signature;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta,
    TransactionConfirmationStatus as SolanaTransactionConfirmationStatus, UiConfirmedBlock,
    UiTransactionEncoding,
};
use surfpool_types::{
    ComputeUnitsEstimationResult, ExecutionCapture, Idl, KeyedProfileResult, ProfileResult,
    ResetAccountConfig, RpcProfileResultConfig, SimnetCommand, SimnetEvent,
    TransactionConfirmationStatus, TransactionStatusEvent, UiKeyedProfileResult, UuidOrSignature,
    VersionedIdl,
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
                Some(account) => {
                    //TODO: when LiteSVM updates `set_account` to remove accounts if 0 lamports, we can remove this check because the account will be removed from the store
                    if account.eq(&Account::default()) {
                        // If the account is default, it means it was deleted but still exists in our litesvm store
                        return GetAccountResult::None(*pubkey);
                    }
                    GetAccountResult::FoundAccount(
                        *pubkey, account,
                        // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                        false,
                    )
                }
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
                    Some(account) => {
                        //TODO: when LiteSVM updates `set_account` to remove accounts if 0 lamports, we can remove this check because the account will be removed from the store
                        if account.eq(&Account::default()) {
                            // If the account is default, it means it was deleted but still exists in our litesvm store
                            GetAccountResult::None(*pubkey)
                        } else {
                            GetAccountResult::FoundAccount(
                                *pubkey, account,
                                // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                                false,
                            )
                        }
                    }
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
                            Err(e) => Some(e.clone()),
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

        let account_updates = self
            .get_multiple_accounts(remote_ctx, &transaction_accounts, None)
            .await?
            .inner;

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
                    .filter_map(|(i, p)| {
                        svm_reader
                            .token_accounts
                            .get(&p)
                            .cloned()
                            .map(|a| (i, a))
                            .clone()
                    })
                    .collect::<Vec<_>>();

                let token_programs = token_accounts_before
                    .iter()
                    .filter_map(
                        |(i, _)| match svm_reader.get_account(&transaction_accounts[*i]) {
                            Some(account) => Some(account.owner),
                            None => {
                                warn!(
                                    "unable to retrieve token account at index {} ({})",
                                    i, transaction_accounts[*i]
                                );
                                None
                            }
                        },
                    )
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
        let mutable_loaded_addresses = loaded_addresses
            .as_ref()
            .map(|l| l.writable_len())
            .unwrap_or(0);
        let readonly_loaded_addresses = loaded_addresses
            .as_ref()
            .map(|l| l.readonly_len())
            .unwrap_or(0);
        let loaded_address_count = mutable_loaded_addresses + readonly_loaded_addresses;
        let last_signer_index = transaction.message.header().num_required_signatures as usize;
        let last_mutable_signer_index =
            last_signer_index - transaction.message.header().num_readonly_signed_accounts as usize;
        let last_mutable_non_signer_index = transaction_accounts.len()
            - transaction.message.header().num_readonly_unsigned_accounts as usize
            - loaded_address_count;
        let last_readonly_non_signer_index = transaction_accounts.len() - loaded_address_count;
        let last_mutable_loaded_index = transaction_accounts.len() - readonly_loaded_addresses;

        let mutable_signers = &transaction_accounts[0..last_mutable_signer_index];
        let readonly_signers = &transaction_accounts[last_mutable_signer_index..last_signer_index];
        let mutable_non_signers =
            &transaction_accounts[last_signer_index..last_mutable_non_signer_index];
        let readonly_non_signers =
            &transaction_accounts[last_mutable_non_signer_index..last_readonly_non_signer_index];
        let mutable_loaded =
            &transaction_accounts[last_readonly_non_signer_index..last_mutable_loaded_index];
        let readonly_loaded = &transaction_accounts[last_mutable_loaded_index..];

        let mut ix_profile_results: Vec<ProfileResult> = vec![];

        for idx in 1..=ix_count {
            if let Some((partial_tx, all_required_accounts_for_last_ix)) = self
                .create_partial_transaction(
                    instructions,
                    &transaction_accounts,
                    mutable_signers,
                    readonly_signers,
                    mutable_non_signers,
                    readonly_non_signers,
                    mutable_loaded,
                    readonly_loaded,
                    &transaction,
                    idx,
                    loaded_addresses,
                )
            {
                let (mut previous_execution_capture, previous_cus, previous_log_count) = {
                    let mut previous_execution_captures = ExecutionCapture::new();
                    let mut previous_cus = 0;
                    let mut previous_log_count = 0;
                    for result in ix_profile_results.iter() {
                        previous_execution_captures.extend(result.post_execution_capture.clone());
                        previous_cus += result.compute_units_consumed;
                        previous_log_count +=
                            result.log_messages.as_ref().map(|m| m.len()).unwrap_or(0);
                    }
                    (
                        previous_execution_captures,
                        previous_cus,
                        previous_log_count,
                    )
                };

                let skip_preflight = true;
                let sigverify = false;
                let do_propagate = false;

                let pre_execution_capture = {
                    let mut capture = pre_execution_capture.clone();

                    // If a pre-execution capture was provided, any pubkeys that are in the capture
                    // that we just took should be replaced with those from the pre-execution capture.
                    let capture_keys: Vec<_> = pre_execution_capture.keys().cloned().collect();
                    for pubkey in capture_keys.into_iter() {
                        if let Some(pre_account) = previous_execution_capture.remove(&pubkey) {
                            // Replace the account with the pre-execution one
                            capture.insert(pubkey, pre_account);
                        }
                    }
                    capture
                };

                let mut profile_result = {
                    let mut svm_clone = self.with_svm_reader(|svm_reader| svm_reader.clone());

                    let (dummy_simnet_tx, _) = crossbeam_channel::bounded(1);
                    let (dummy_geyser_tx, _) = crossbeam_channel::bounded(1);
                    svm_clone.simnet_events_tx = dummy_simnet_tx;
                    svm_clone.geyser_events_tx = dummy_geyser_tx;

                    let svm_locker = SurfnetSvmLocker::new(svm_clone);
                    svm_locker
                        .process_transaction_internal(
                            partial_tx,
                            skip_preflight,
                            sigverify,
                            transaction_accounts,
                            &loaded_addresses.as_ref().map(|l| l.loaded_addresses()),
                            accounts_before,
                            token_accounts_before,
                            token_programs,
                            pre_execution_capture,
                            status_tx,
                            do_propagate,
                        )
                        .await?
                };

                profile_result
                    .pre_execution_capture
                    .retain(|pubkey, _| all_required_accounts_for_last_ix.contains(pubkey));
                profile_result
                    .post_execution_capture
                    .retain(|pubkey, _| all_required_accounts_for_last_ix.contains(pubkey));

                profile_result.compute_units_consumed = profile_result
                    .compute_units_consumed
                    .saturating_sub(previous_cus);
                profile_result.log_messages = profile_result.log_messages.map(|logs| {
                    logs.into_iter()
                        .skip(previous_log_count)
                        .collect::<Vec<_>>()
                });

                ix_profile_results.push(profile_result);
            } else {
                return Ok(None);
                // panic!("No partial transaction created for instruction {}", idx);
            }
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
                meta_canonical,
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
            });

            self.with_svm_writer(|svm_writer| {
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
                let token_account = svm_writer.token_accounts.get(&pubkey).cloned();
                post_execution_capture.insert(*pubkey, account.clone());

                if let Some(token_account) = token_account {
                    token_accounts_after.push((i, token_account));
                    post_token_program_ids
                        .push(account.as_ref().map(|a| a.owner).unwrap_or(spl_token::id()));
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
                svm_writer
                    .transactions_queued_for_confirmation
                    .push_back((transaction.clone(), status_tx.clone()));

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
                    &loaded_addresses,
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
                &loaded_addresses,
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
                .map_err(ProcessTransactionResult::ExecutionFailure)
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
    pub fn reset_account(&self, pubkey: Pubkey, config: ResetAccountConfig) -> SurfpoolResult<()> {
        let cascade_to_owned = config.recursive.unwrap_or_default();
        self.with_svm_writer(move |svm_writer| {
            if let Some(account) = svm_writer.get_account(&pubkey) {
                // Check if this is an executable account (program)
                if account.executable {
                    // Handle upgradeable program - also reset the program data account
                    if account.owner == solana_sdk_ids::bpf_loader_upgradeable::id() {
                        let program_data_address =
                            solana_sdk::bpf_loader_upgradeable::get_program_data_address(&pubkey);

                        // Reset the program data account first
                        svm_writer.reset_account(&program_data_address)?;
                    }
                }
                if cascade_to_owned {
                    let owned_accounts = svm_writer.get_account_owned_by(pubkey);
                    for (owned_pubkey, _) in owned_accounts {
                        // Avoid infinite recursion by not cascading further
                        svm_writer.reset_account(&owned_pubkey)?;
                    }
                }
                // Reset the account itself
                svm_writer.reset_account(&pubkey)?;
            }
            Ok(())
        })
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

                // acc_keys.append(&mut alt_pubkeys);
                if let Some(loaded_addresses) = all_transaction_lookup_table_addresses {
                    acc_keys.extend(loaded_addresses.into_iter());
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
                let alts = message.address_table_lookups.clone();
                if alts.is_empty() {
                    return Ok(None);
                }
                let mut loaded = TransactionLoadedAddresses::new();
                for alt in alts {
                    self.get_lookup_table_addresses(remote_ctx, &alt, &mut loaded)
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
    fn create_partial_transaction(
        &self,
        instructions: &[CompiledInstruction],
        message_accounts: &[Pubkey],
        mutable_signers: &[Pubkey],
        readonly_signers: &[Pubkey],
        mutable_non_signers: &[Pubkey],
        readonly_non_signers: &[Pubkey],
        mutable_loaded_addresses: &[Pubkey],
        readonly_loaded_addresses: &[Pubkey],
        transaction: &VersionedTransaction,
        idx: usize,
        loaded_addresses: &Option<TransactionLoadedAddresses>,
    ) -> Option<(VersionedTransaction, IndexSet<Pubkey>)> {
        let ixs_for_tx = instructions[0..idx].to_vec();

        // Collect all required accounts for the partial transaction
        let mut all_required_accounts = IndexSet::new();
        for ix in &ixs_for_tx {
            // Add instruction accounts
            for &account_idx in &ix.accounts {
                all_required_accounts.insert(message_accounts[account_idx as usize]);
            }
            // If we still don't have any accounts, it means our instruction doesn't have any accounts.
            // Some instructions don't need to read/write any accounts, so the account list is empty.
            // However, we're still using an account to sign, so we add that here.
            {
                if all_required_accounts.len() == 0 {
                    let mut mutable_signers =
                        mutable_signers.iter().cloned().collect::<IndexSet<_>>();
                    all_required_accounts.append(&mut mutable_signers);
                }
                if all_required_accounts.len() == 0 {
                    let mut readonly_signers =
                        readonly_signers.iter().cloned().collect::<IndexSet<_>>();
                    all_required_accounts.append(&mut readonly_signers);
                }
            }
            // Add program ID
            all_required_accounts.insert(message_accounts[ix.program_id_index as usize]);
        }

        // Profiling our partial transaction is really about knowing the impacts of the _last_
        // instruction, so we want to have a separate list of all of the accounts that are
        // used in the last instruction.
        let mut all_required_accounts_for_last_ix = IndexSet::new();
        let last = ixs_for_tx.last().unwrap();
        for &account_idx in &last.accounts {
            all_required_accounts_for_last_ix.insert(message_accounts[account_idx as usize]);
        }
        {
            if all_required_accounts_for_last_ix.len() == 0 {
                let mut mutable_signers = mutable_signers.iter().cloned().collect::<IndexSet<_>>();
                all_required_accounts_for_last_ix.append(&mut mutable_signers);
            }
            if all_required_accounts_for_last_ix.len() == 0 {
                let mut readonly_signers =
                    readonly_signers.iter().cloned().collect::<IndexSet<_>>();
                all_required_accounts_for_last_ix.append(&mut readonly_signers);
            }
        }
        all_required_accounts_for_last_ix.insert(message_accounts[last.program_id_index as usize]);

        // Categorize accounts based on their original positions
        let mut new_mutable_signers = HashSet::new();
        let mut new_readonly_signers = HashSet::new();
        let mut new_mutable_non_signers = HashSet::new();
        let mut new_readonly_non_signers = HashSet::new();
        let mut new_mutable_loaded = HashSet::new();
        let mut new_readonly_loaded = HashSet::new();

        for &account in &all_required_accounts {
            if let Some(idx) = message_accounts.iter().position(|pk| pk == &account) {
                match idx {
                    i if i < mutable_signers.len() => {
                        new_mutable_signers.insert(account);
                    }
                    i if i < mutable_signers.len() + readonly_signers.len() => {
                        new_readonly_signers.insert(account);
                    }
                    i if i < mutable_signers.len()
                        + readonly_signers.len()
                        + mutable_non_signers.len() =>
                    {
                        new_mutable_non_signers.insert(account);
                    }
                    i if i < mutable_signers.len()
                        + readonly_signers.len()
                        + mutable_non_signers.len()
                        + readonly_non_signers.len() =>
                    {
                        new_readonly_non_signers.insert(account);
                    }
                    i if i < mutable_signers.len()
                        + readonly_signers.len()
                        + mutable_non_signers.len()
                        + readonly_non_signers.len()
                        + mutable_loaded_addresses.len() =>
                    {
                        new_mutable_loaded.insert(account);
                    }
                    i if i < mutable_signers.len()
                        + readonly_signers.len()
                        + mutable_non_signers.len()
                        + readonly_non_signers.len()
                        + mutable_loaded_addresses.len()
                        + readonly_loaded_addresses.len() =>
                    {
                        new_readonly_loaded.insert(account);
                    }
                    _ => {}
                };
            }
        }

        // Build account keys in correct order: signers first, then non-signers
        let mut new_account_keys = Vec::new();
        for &account in &all_required_accounts {
            if new_mutable_signers.contains(&account) {
                new_account_keys.push(account);
            }
        }
        for &account in &all_required_accounts {
            if new_readonly_signers.contains(&account) {
                new_account_keys.push(account);
            }
        }
        for &account in &all_required_accounts {
            if new_mutable_non_signers.contains(&account) {
                new_account_keys.push(account);
            }
        }
        for &account in &all_required_accounts {
            if new_readonly_non_signers.contains(&account) {
                new_account_keys.push(account);
            }
        }

        // Create account index mapping
        let mut account_index_mapping = HashMap::new();
        {
            for (new_idx, pubkey) in new_account_keys.iter().enumerate() {
                if let Some(old_idx) = message_accounts.iter().position(|pk| pk == pubkey) {
                    account_index_mapping.insert(old_idx, new_idx);
                }
            }

            // Accounts loaded from an ALT shouldn't be in the overall message accounts, but their
            // indices should be remapped correctly.
            let non_loaded_address_len = mutable_signers.len()
                + readonly_signers.len()
                + mutable_non_signers.len()
                + readonly_non_signers.len();
            for pubkey in new_mutable_loaded.iter() {
                if let Some(old_idx) = message_accounts.iter().position(|pk| pk == pubkey) {
                    let placement = old_idx - non_loaded_address_len;
                    account_index_mapping.insert(old_idx, new_account_keys.len() + placement);
                }
            }

            for pubkey in new_readonly_loaded.iter() {
                if let Some(old_idx) = message_accounts.iter().position(|pk| pk == pubkey) {
                    let placement =
                        old_idx - non_loaded_address_len - mutable_loaded_addresses.len();
                    account_index_mapping.insert(
                        old_idx,
                        new_account_keys.len() + new_mutable_loaded.len() + placement,
                    );
                }
            }
        }

        // Remap instructions
        let mut remapped_instructions = Vec::new();
        for ix in ixs_for_tx {
            let mut remapped_accounts = Vec::new();
            for &account_idx in &ix.accounts {
                if let Some(&new_idx) = account_index_mapping.get(&(account_idx as usize)) {
                    remapped_accounts.push(new_idx as u8);
                } else {
                    continue; // Skip instructions with unmappable accounts, this should be an unreachable path
                }
            }

            if remapped_accounts.len() == ix.accounts.len() {
                let new_program_id_idx = account_index_mapping
                    .get(&(ix.program_id_index as usize))
                    .copied()
                    .unwrap_or(0) as u8;

                remapped_instructions.push(CompiledInstruction {
                    program_id_index: new_program_id_idx,
                    accounts: remapped_accounts,
                    data: ix.data.clone(),
                });
            }
        }

        if remapped_instructions.is_empty() {
            // panic!("No valid instructions after remapping, skipping partial transaction creation.");
            return None;
        }

        // Create new message
        let num_required_signatures = new_mutable_signers.len() + new_readonly_signers.len();
        let num_readonly_signed_accounts = new_readonly_signers.len();
        let num_readonly_unsigned_accounts = new_readonly_non_signers.len();

        let new_message = match transaction.version() {
            TransactionVersion::Legacy(_) => VersionedMessage::Legacy(Message {
                header: MessageHeader {
                    num_required_signatures: num_required_signatures as u8,
                    num_readonly_signed_accounts: num_readonly_signed_accounts as u8,
                    num_readonly_unsigned_accounts: num_readonly_unsigned_accounts as u8,
                },
                account_keys: new_account_keys,
                recent_blockhash: *transaction.message.recent_blockhash(),
                instructions: remapped_instructions,
            }),
            TransactionVersion::Number(_) => VersionedMessage::V0(solana_message::v0::Message {
                header: MessageHeader {
                    num_required_signatures: num_required_signatures as u8,
                    num_readonly_signed_accounts: num_readonly_signed_accounts as u8,
                    num_readonly_unsigned_accounts: num_readonly_unsigned_accounts as u8,
                },
                account_keys: new_account_keys,
                recent_blockhash: *transaction.message.recent_blockhash(),
                instructions: remapped_instructions,
                address_table_lookups: loaded_addresses
                    .as_ref()
                    .map(|l| {
                        l.filter_from_members(&new_mutable_loaded, &new_readonly_loaded)
                            .to_address_table_lookups()
                    })
                    .unwrap_or_default(),
            }),
        };

        // Create partial transaction with appropriate signatures
        let signatures_to_use = transaction.signatures[0..num_required_signatures].to_vec();

        let tx = VersionedTransaction {
            signatures: signatures_to_use,
            message: new_message,
        };

        Some((tx, all_required_accounts_for_last_ix))
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
                    let profile = self.get_profile_result(id.clone(), config)?;
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
                            (pubkey.clone(), program_account.clone()),
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
                    &program_id,
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
                let _ = simnet_events_tx.send(SimnetEvent::info(format!("Old Authority: None")));
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
            let res = svm_reader.get_account_owned_by(*program_id);

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

fn update_programdata_account(
    program_id: &Pubkey,
    programdata_account: &mut Account,
    new_authority: Option<Pubkey>,
) -> SurfpoolResult<Option<Pubkey>> {
    let upgradeable_loader_state =
        bincode::deserialize::<UpgradeableLoaderState>(&programdata_account.data).map_err(|e| {
            SurfpoolError::invalid_program_account(
                &program_id,
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
                &program_id,
                format!("Failed to serialize program data: {}", e),
            )
        })?;

        data.append(&mut programdata_account.data[offset..].to_vec());

        programdata_account.data = data;

        Ok(upgrade_authority_address)
    } else {
        return Err(SurfpoolError::invalid_program_account(
            &program_id,
            "Invalid program data account",
        ));
    }
}

pub fn format_ui_amount_string(amount: u64, decimals: u8) -> String {
    let formatted_amount = if decimals > 0 {
        let divisor = 10u64.pow(decimals as u32);
        format!(
            "{:.decimals$}",
            amount as f64 / divisor as f64,
            decimals = decimals as usize
        )
    } else {
        amount.to_string()
    };
    formatted_amount
}

pub fn format_ui_amount(amount: u64, decimals: u8) -> f64 {
    if decimals > 0 {
        let divisor = 10u64.pow(decimals as u32);
        amount as f64 / divisor as f64
    } else {
        amount as f64
    }
}
