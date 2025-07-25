use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use bincode::serialized_size;
use crossbeam_channel::{Receiver, Sender};
use itertools::Itertools;
use litesvm::types::{FailedTransactionMetadata, SimulatedTransactionInfo, TransactionResult};
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
use solana_clock::Slot;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_epoch_info::EpochInfo;
use solana_hash::Hash;
use solana_message::{
    SimpleAddressLoader, VersionedMessage,
    v0::{LoadedAddresses, MessageAddressTableLookup},
};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_sdk::{
    bpf_loader_upgradeable::{UpgradeableLoaderState, get_program_data_address},
    transaction::{SanitizedTransaction, VersionedTransaction},
};
use solana_signature::Signature;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta,
    TransactionConfirmationStatus as SolanaTransactionConfirmationStatus, UiConfirmedBlock,
    UiTransactionEncoding,
};
use surfpool_types::{
    ComputeUnitsEstimationResult, Idl, ProfileResult, ProfileState, SimnetEvent,
    TransactionConfirmationStatus, TransactionStatusEvent, UuidOrSignature,
};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::{
    AccountFactory, GetAccountResult, GetTransactionResult, GeyserEvent, SignatureSubscriptionType,
    SurfnetSvm,
    remote::{SomeRemoteCtx, SurfnetRemoteClient},
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::{convert_transaction_metadata_from_canonical, verify_pubkey},
    surfnet::FINALIZATION_SLOT_THRESHOLD,
    types::{
        GeyserAccountUpdate, RemoteRpcResult, SurfnetTransactionStatus, TransactionWithStatusMeta,
    },
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

            let mut before_slot = None;
            let mut until_slot = None;

            let sigs: Vec<_> = svm_reader
                .transactions
                .iter()
                .filter_map(|(sig, status)| {
                    let TransactionWithStatusMeta {
                        slot,
                        transaction,
                        meta,
                    } = status.expect_processed();

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
                    let is_signer = transaction
                        .message
                        .static_account_keys()
                        .iter()
                        .position(|pk| pk == pubkey)
                        .map(|i| transaction.message.is_signer(i))
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

            let transaction_with_status_meta = entry.expect_processed();
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
        let loaded_addresses = self
            .get_loaded_addresses(remote_ctx, &transaction.message)
            .await?;
        let pubkeys_from_message = self
            .get_pubkeys_from_message(&transaction.message, loaded_addresses.clone())
            .clone();

        let account_updates = self
            .get_multiple_accounts(remote_ctx, &pubkeys_from_message, None)
            .await?
            .inner;

        let pre_execution_capture = {
            let mut capture = BTreeMap::new();
            for account_update in account_updates.iter() {
                self.snapshot_get_account_result(
                    &mut capture,
                    account_update.clone(),
                    Some(UiAccountEncoding::JsonParsed),
                );
            }
            capture
        };

        self.with_svm_writer(|svm_writer| {
            for update in &account_updates {
                svm_writer.write_account_update(update.clone());
            }

            let accounts_before = pubkeys_from_message
                .iter()
                .map(|p| svm_writer.inner.get_account(p))
                .collect::<Vec<Option<Account>>>();

            let token_accounts_before = pubkeys_from_message
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    svm_writer
                        .token_accounts
                        .get(&p)
                        .cloned()
                        .map(|a| (i, a))
                        .clone()
                })
                .collect::<Vec<_>>();

            let token_programs = token_accounts_before
                .iter()
                .map(|(i, _)| {
                    svm_writer
                        .accounts_registry
                        .get(&pubkeys_from_message[*i])
                        .unwrap()
                        .owner
                })
                .collect::<Vec<_>>()
                .clone();
            let mut logs = vec![];

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
                        logs = meta.logs.clone();
                        let _ = status_tx.try_send(TransactionStatusEvent::SimulationFailure((
                            res.err.clone(),
                            meta,
                        )));
                        svm_writer.notify_signature_subscribers(
                            SignatureSubscriptionType::processed(),
                            &signature,
                            latest_absolute_slot,
                            Some(res.err.clone()),
                        );
                        svm_writer.notify_logs_subscribers(
                            &signature,
                            Some(res.err),
                            logs,
                            CommitmentLevel::Processed,
                        );
                        return Ok::<(), SurfpoolError>(());
                    }
                }
            }
            // send the transaction to the SVM
            let err = match svm_writer
                .send_transaction(transaction.clone(), false /* cu_analysis_enabled */)
            {
                Ok(res) => {
                    logs = res.logs.clone();
                    let accounts_after = pubkeys_from_message
                        .iter()
                        .map(|p| svm_writer.inner.get_account(p))
                        .collect::<Vec<Option<Account>>>();

                    let sanitized_transaction = SanitizedTransaction::try_create(
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
                    .ok();

                    for (pubkey, (before, after)) in pubkeys_from_message
                        .iter()
                        .zip(accounts_before.clone().iter().zip(accounts_after.clone()))
                    {
                        if before.ne(&after) {
                            if let Some(after) = &after {
                                svm_writer.update_account_registries(pubkey, after);
                                let write_version = svm_writer.increment_write_version();

                                if let Some(sanitized_transaction) = sanitized_transaction.clone() {
                                    let _ = svm_writer.geyser_events_tx.send(
                                        GeyserEvent::UpdateAccount(GeyserAccountUpdate::new(
                                            *pubkey,
                                            after.clone(),
                                            svm_writer.get_latest_absolute_slot(),
                                            sanitized_transaction.clone(),
                                            write_version,
                                        )),
                                    );
                                }
                            }
                            svm_writer
                                .notify_account_subscribers(pubkey, &after.unwrap_or_default());
                        }
                    }

                    let mut token_accounts_after = vec![];
                    let mut post_execution_capture = BTreeMap::new();
                    for (i, (pubkey, account)) in pubkeys_from_message
                        .iter()
                        .zip(accounts_after.iter())
                        .enumerate()
                    {
                        let token_account = svm_writer.token_accounts.get(&pubkey).cloned();

                        let ui_account = if let Some(account) = account {
                            let ui_account = svm_writer
                                .account_to_rpc_keyed_account(
                                    &pubkey,
                                    account,
                                    &RpcAccountInfoConfig {
                                        encoding: Some(UiAccountEncoding::JsonParsed),
                                        ..Default::default()
                                    },
                                    token_account.map(|a| a.mint()),
                                )
                                .account;
                            Some(ui_account)
                        } else {
                            None
                        };
                        post_execution_capture.insert(*pubkey, ui_account);

                        if let Some(token_account) = token_account {
                            token_accounts_after.push((i, token_account));
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

                    let profile_result = ProfileResult::success(
                        res.compute_units_consumed,
                        res.logs.clone(),
                        pre_execution_capture.clone(),
                        post_execution_capture,
                        latest_absolute_slot,
                        Some(Uuid::new_v4()),
                    );
                    svm_writer.write_executed_profile_result(signature, profile_result);

                    let transaction_meta = convert_transaction_metadata_from_canonical(&res);

                    let transaction_with_status_meta = TransactionWithStatusMeta::new(
                        svm_writer.get_latest_absolute_slot(),
                        transaction.clone(),
                        res,
                        accounts_before,
                        accounts_after,
                        token_accounts_before,
                        token_accounts_after,
                        token_mints,
                        token_programs,
                        loaded_addresses.clone().unwrap_or_default(),
                    );
                    svm_writer.transactions.insert(
                        transaction_meta.signature,
                        SurfnetTransactionStatus::Processed(Box::new(
                            transaction_with_status_meta.clone(),
                        )),
                    );

                    let _ = svm_writer
                        .simnet_events_tx
                        .try_send(SimnetEvent::transaction_processed(transaction_meta, None));

                    let _ = svm_writer
                        .geyser_events_tx
                        .send(GeyserEvent::NotifyTransaction(transaction_with_status_meta));

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
                err.clone(),
            );
            svm_writer.notify_logs_subscribers(&signature, err, logs, CommitmentLevel::Processed);
            Ok(())
        })?;

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
                        TokenAccountsFilter::Mint(mint) => token_account.mint().eq(mint),
                        TokenAccountsFilter::ProgramId(program_id) => account.owner.eq(program_id),
                    } {
                        Some(svm_reader.account_to_rpc_keyed_account(
                            pubkey,
                            account,
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
                    let account = svm_reader.accounts_registry.get(pubkey)?;
                    let include = match filter {
                        TokenAccountsFilter::Mint(mint) => token_account.mint() == *mint,
                        TokenAccountsFilter::ProgramId(program_id) => {
                            account.owner == *program_id && is_supported_token_program(program_id)
                        }
                    };

                    if include {
                        Some(svm_reader.account_to_rpc_keyed_account(
                            pubkey,
                            account,
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
        loaded_addresses: Option<LoadedAddresses>,
    ) -> Vec<Pubkey> {
        match message {
            VersionedMessage::Legacy(message) => message.account_keys.clone(),
            VersionedMessage::V0(message) => {
                let alts = message.address_table_lookups.clone();
                let mut acc_keys = message.account_keys.clone();
                let mut alt_pubkeys = alts.iter().map(|msg| msg.account_key).collect::<Vec<_>>();

                acc_keys.append(&mut alt_pubkeys);
                if let Some(mut loaded_addresses) = loaded_addresses {
                    acc_keys.append(&mut loaded_addresses.readonly);
                    acc_keys.append(&mut loaded_addresses.writable);
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
    ) -> SurfpoolResult<Option<LoadedAddresses>> {
        match message {
            VersionedMessage::Legacy(_) => Ok(None),
            VersionedMessage::V0(message) => {
                let alts = message.address_table_lookups.clone();
                if alts.is_empty() {
                    return Ok(None);
                }
                let mut combined = LoadedAddresses::default();
                for alt in alts {
                    let mut loaded_addresses = self
                        .get_lookup_table_addresses(remote_ctx, &alt)
                        .await?
                        .inner;
                    combined.readonly.append(&mut loaded_addresses.readonly);
                    combined.writable.append(&mut loaded_addresses.writable);
                }

                Ok(Some(combined))
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
        transaction: VersionedTransaction,
        encoding: Option<UiAccountEncoding>,
        tag: Option<String>,
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

        let loaded_addresses = svm_locker
            .get_loaded_addresses(&remote_ctx_with_config, &transaction.message)
            .await?;
        let account_keys =
            svm_locker.get_pubkeys_from_message(&transaction.message, loaded_addresses);

        let pre_execution_capture = {
            let mut capture = BTreeMap::new();
            for pubkey in &account_keys {
                let account = svm_locker
                    .get_account(&remote_ctx_with_config, pubkey, None)
                    .await?
                    .inner;

                svm_locker.snapshot_get_account_result(&mut capture, account, encoding);
            }
            capture
        };

        let compute_units_estimation_result = svm_locker.estimate_compute_units(&transaction).inner;

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let signature = transaction.signatures[0];
        let _ = svm_locker
            .process_transaction(remote_ctx, transaction, status_tx, true)
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
                                signature, err
                            ),
                        ));
                        return Err(SurfpoolError::internal(format!(
                            "Transaction {} failed during snapshot simulation: {}",
                            signature, err
                        )));
                    }
                    TransactionStatusEvent::SimulationFailure(_) => unreachable!(),
                    TransactionStatusEvent::VerificationFailure(_) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::WarnLog(
                            chrono::Local::now(),
                            format!(
                                "Transaction {} verification failed during snapshot simulation",
                                signature
                            ),
                        ));
                        return Err(SurfpoolError::internal(format!(
                            "Transaction {} verification failed during snapshot simulation",
                            signature
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

                svm_locker.snapshot_get_account_result(&mut capture, account, encoding);
            }
            capture
        };

        let uuid = Uuid::new_v4();
        let profile_result = ProfileResult {
            compute_units: compute_units_estimation_result,
            state: ProfileState::new(pre_execution_capture, post_execution_capture),
            slot,
            uuid: Some(uuid),
        };

        self.with_svm_writer(|svm| {
            svm.simulated_transaction_profiles
                .insert(uuid, profile_result.clone());
            svm.profile_tag_map
                .entry(tag.clone().unwrap_or(uuid.to_string()))
                .or_default()
                .push(UuidOrSignature::Uuid(uuid));
        });

        Ok(SvmAccessContext::new(
            slot,
            latest_epoch_info,
            latest_blockhash,
            profile_result,
        ))
    }

    /// Returns the profile result for a given signature or UUID, and whether it exists in the SVM.
    pub fn get_profile_result(
        &self,
        signature_or_uuid: UuidOrSignature,
    ) -> SurfpoolResult<Option<ProfileResult>> {
        match &signature_or_uuid {
            UuidOrSignature::Signature(signature) => {
                let profile = self.with_svm_reader(|svm| {
                    svm.executed_transaction_profiles.get(signature).cloned()
                });
                let transaction_exists =
                    self.with_svm_reader(|svm| svm.transactions.contains_key(signature));
                if profile.is_none() && transaction_exists {
                    Err(SurfpoolError::transaction_not_found_in_svm(signature))
                } else {
                    Ok(profile)
                }
            }
            UuidOrSignature::Uuid(uuid) => {
                let profile = self
                    .with_svm_reader(|svm| svm.simulated_transaction_profiles.get(uuid).cloned());
                Ok(profile)
            }
        }
    }

    /// Returns the profile results for a given tag.
    pub fn get_profile_results_by_tag(
        &self,
        tag: String,
    ) -> SurfpoolResult<Option<Vec<ProfileResult>>> {
        let tag_map = self.with_svm_reader(|svm| svm.profile_tag_map.get(&tag).cloned());
        match tag_map {
            None => Ok(None),
            Some(uuids_or_sigs) => {
                let mut profiles = Vec::new();
                for id in uuids_or_sigs {
                    let profile = self.get_profile_result(id.clone())?;
                    if profile.is_none() {
                        return Err(SurfpoolError::tag_not_found(&tag));
                    }
                    profiles.push(profile.unwrap());
                }
                Ok(Some(profiles))
            }
        }
    }

    pub fn register_idl(&self, idl: Idl) -> SurfpoolResult<()> {
        self.with_svm_writer(|svm| {
            svm.idl_registry.insert(idl.address.clone(), idl);
            Ok::<(), SurfpoolError>(())
        })
    }

    pub fn get_idl(&self, address: &str) -> SurfpoolResult<Option<Idl>> {
        self.with_svm_reader(|svm| Ok(svm.idl_registry.get(address).cloned()))
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

        let remote_accounts = client
            .get_program_accounts(program_id, account_config, filters)
            .await
            .map(|accounts| {
                accounts
                    .iter()
                    .map(|(pubkey, account)| RpcKeyedAccount {
                        pubkey: pubkey.to_string(),
                        account: self
                            .encode_ui_account(pubkey, account, encoding, None, data_slice),
                    })
                    .collect::<Vec<RpcKeyedAccount>>()
            })?;

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

    fn snapshot_get_account_result(
        &self,
        capture: &mut BTreeMap<Pubkey, Option<UiAccount>>,
        result: GetAccountResult,
        encoding: Option<UiAccountEncoding>,
    ) {
        let config = RpcAccountInfoConfig {
            encoding,
            ..Default::default()
        };
        match result {
            GetAccountResult::None(pubkey) => {
                capture.insert(pubkey, None);
            }
            GetAccountResult::FoundAccount(pubkey, account, _) => {
                let rpc_keyed_account = self.with_svm_reader(|svm_reader| {
                    svm_reader.account_to_rpc_keyed_account(&pubkey, &account, &config, None)
                });
                capture.insert(pubkey, Some(rpc_keyed_account.account));
            }
            GetAccountResult::FoundTokenAccount((pubkey, account), (mint_pubkey, mint_account)) => {
                let rpc_keyed_account = self.with_svm_reader(|svm_reader| {
                    svm_reader.account_to_rpc_keyed_account(
                        &pubkey,
                        &account,
                        &config,
                        Some(mint_pubkey),
                    )
                });
                capture.insert(pubkey, Some(rpc_keyed_account.account));
                if let Some(mint_account) = mint_account {
                    let rpc_keyed_account = self.with_svm_reader(|svm_reader| {
                        svm_reader.account_to_rpc_keyed_account(
                            &mint_pubkey,
                            &mint_account,
                            &config,
                            None,
                        )
                    });
                    capture.insert(mint_pubkey, Some(rpc_keyed_account.account));
                }
            }
            GetAccountResult::FoundProgramAccount(
                (pubkey, account),
                (data_pubkey, data_account),
            ) => {
                let rpc_keyed_account = self.with_svm_reader(|svm_reader| {
                    svm_reader.account_to_rpc_keyed_account(&pubkey, &account, &config, None)
                });
                capture.insert(pubkey, Some(rpc_keyed_account.account));
                if let Some(data_account) = data_account {
                    let rpc_keyed_account = self.with_svm_reader(|svm_reader| {
                        svm_reader.account_to_rpc_keyed_account(
                            &data_pubkey,
                            &data_account,
                            &config,
                            None,
                        )
                    });
                    capture.insert(data_pubkey, Some(rpc_keyed_account.account));
                }
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
