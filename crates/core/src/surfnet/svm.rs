use std::{
    cmp::max,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    str::FromStr,
    time::SystemTime,
};

use agave_feature_set::{
    FeatureSet, abort_on_invalid_curve, blake3_syscall_enabled, curve25519_syscall_enabled,
    deplete_cu_meter_on_vm_failure, deprecate_legacy_vote_ixs,
    disable_deploy_of_alloc_free_syscall, disable_fees_sysvar, disable_sbpf_v0_execution,
    disable_zk_elgamal_proof_program, enable_alt_bn128_compression_syscall,
    enable_alt_bn128_syscall, enable_big_mod_exp_syscall,
    enable_bpf_loader_set_authority_checked_ix, enable_extend_program_checked,
    enable_get_epoch_stake_syscall, enable_loader_v4, enable_poseidon_syscall,
    enable_sbpf_v1_deployment_and_execution, enable_sbpf_v2_deployment_and_execution,
    enable_sbpf_v3_deployment_and_execution, fix_alt_bn128_multiplication_input_length,
    formalize_loaded_transaction_data_size, get_sysvar_syscall_enabled,
    increase_tx_account_lock_limit, last_restart_slot_sysvar,
    mask_out_rent_epoch_in_vm_serialization, move_precompile_verification_to_svm,
    move_stake_and_move_lamports_ixs, raise_cpi_nesting_limit_to_8, reenable_sbpf_v0_execution,
    reenable_zk_elgamal_proof_program, remaining_compute_units_syscall_enabled,
    remove_bpf_loader_incorrect_program_id, simplify_alt_bn128_syscall_error_codes,
    stake_raise_minimum_delegation_to_1_sol, stricter_abi_and_runtime_constraints,
};
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::Utc;
use convert_case::Casing;
use crossbeam_channel::{Receiver, Sender, unbounded};
use litesvm::types::{
    FailedTransactionMetadata, SimulatedTransactionInfo, TransactionMetadata, TransactionResult,
};
use solana_account::{Account, AccountSharedData, ReadableAccount};
use solana_account_decoder::{
    UiAccount, UiAccountData, UiAccountEncoding, UiDataSliceConfig, encode_ui_account,
    parse_account_data::{AccountAdditionalDataV3, ParsedAccount, SplTokenAdditionalDataV2},
};
use solana_client::{
    rpc_client::SerializableTransaction,
    rpc_config::{RpcAccountInfoConfig, RpcBlockConfig, RpcTransactionLogsFilter},
    rpc_response::{RpcKeyedAccount, RpcLogsResponse, RpcPerfSample},
};
use solana_clock::{Clock, Slot};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_epoch_info::EpochInfo;
use solana_feature_gate_interface::Feature;
use solana_genesis_config::GenesisConfig;
use solana_hash::Hash;
use solana_inflation::Inflation;
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_message::{
    Message, VersionedMessage, inline_nonce::is_advance_nonce_instruction_data, v0::LoadedAddresses,
};
use solana_program_option::COption;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::SlotInfo;
use solana_sdk_ids::{bpf_loader, system_program};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{TransactionDetails, TransactionStatusMeta, UiConfirmedBlock};
use spl_token_2022_interface::extension::{
    BaseStateWithExtensions, StateWithExtensions, interest_bearing_mint::InterestBearingConfig,
    scaled_ui_amount::ScaledUiAmountConfig,
};
use surfpool_types::{
    AccountChange, AccountProfileState, AccountSnapshot, DEFAULT_PROFILING_MAP_CAPACITY,
    DEFAULT_SLOT_TIME_MS, ExportSnapshotConfig, ExportSnapshotScope, FifoMap, Idl,
    OverrideInstance, ProfileResult, RpcProfileDepth, RpcProfileResultConfig,
    RunbookExecutionStatusReport, SimnetEvent, SvmFeature, SvmFeatureConfig,
    TransactionConfirmationStatus, TransactionStatusEvent, UiAccountChange, UiAccountProfileState,
    UiProfileResult, VersionedIdl,
    types::{
        ComputeUnitsEstimationResult, KeyedProfileResult, UiKeyedProfileResult, UuidOrSignature,
    },
};
use txtx_addon_kit::{
    indexmap::IndexMap,
    types::types::{AddonJsonConverter, Value},
};
use txtx_addon_network_svm::codec::idl::borsh_encode_value_to_idl_type;
use txtx_addon_network_svm_types::subgraph::idl::{
    parse_bytes_to_value_with_expected_idl_type_def_ty,
    parse_bytes_to_value_with_expected_idl_type_def_ty_with_leftover_bytes,
};
use uuid::Uuid;

use super::{
    AccountSubscriptionData, BlockHeader, BlockIdentifier, FINALIZATION_SLOT_THRESHOLD,
    GetAccountResult, GeyserEvent, SLOTS_PER_EPOCH, SignatureSubscriptionData,
    SignatureSubscriptionType, remote::SurfnetRemoteClient,
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    rpc::utils::convert_transaction_metadata_from_canonical,
    scenarios::TemplateRegistry,
    storage::{Storage, new_kv_store},
    surfnet::{
        LogsSubscriptionData, locker::is_supported_token_program, surfnet_lite_svm::SurfnetLiteSvm,
    },
    types::{
        GeyserAccountUpdate, MintAccount, SurfnetTransactionStatus, SyntheticBlockhash,
        TokenAccount, TransactionWithStatusMeta,
    },
};

/// Helper function to apply an override to a decoded account value using dot notation
pub fn apply_override_to_decoded_account(
    decoded_value: &mut Value,
    path: &str,
    value: &serde_json::Value,
) -> SurfpoolResult<()> {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.is_empty() {
        return Err(SurfpoolError::internal("Empty path provided for override"));
    }

    // Navigate to the parent of the target field
    let mut current = decoded_value;
    for part in &parts[..parts.len() - 1] {
        match current {
            Value::Object(map) => {
                current = map.get_mut(&part.to_string()).ok_or_else(|| {
                    SurfpoolError::internal(format!(
                        "Path segment '{}' not found in decoded account",
                        part
                    ))
                })?;
            }
            _ => {
                return Err(SurfpoolError::internal(format!(
                    "Cannot navigate through field '{}' - not an object",
                    part
                )));
            }
        }
    }

    // Set the final field
    let final_key = parts[parts.len() - 1];
    match current {
        Value::Object(map) => {
            // Convert serde_json::Value to txtx Value
            let txtx_value = json_to_txtx_value(value)?;
            map.insert(final_key.to_string(), txtx_value);
            Ok(())
        }
        _ => Err(SurfpoolError::internal(format!(
            "Cannot set field '{}' - parent is not an object",
            final_key
        ))),
    }
}

/// Helper function to convert serde_json::Value to txtx Value
fn json_to_txtx_value(json: &serde_json::Value) -> SurfpoolResult<Value> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i as i128))
            } else if let Some(u) = n.as_u64() {
                Ok(Value::Integer(u as i128))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err(SurfpoolError::internal(format!(
                    "Unable to convert number: {}",
                    n
                )))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let txtx_arr: Result<Vec<Value>, _> = arr.iter().map(json_to_txtx_value).collect();
            Ok(Value::Array(Box::new(txtx_arr?)))
        }
        serde_json::Value::Object(obj) => {
            let mut txtx_obj = IndexMap::new();
            for (k, v) in obj.iter() {
                txtx_obj.insert(k.clone(), json_to_txtx_value(v)?);
            }
            Ok(Value::Object(txtx_obj))
        }
    }
}

pub type AccountOwner = Pubkey;

#[allow(deprecated)]
use solana_sysvar::recent_blockhashes::MAX_ENTRIES;

#[allow(deprecated)]
pub const MAX_RECENT_BLOCKHASHES_STANDARD: usize = MAX_ENTRIES;

pub fn get_txtx_value_json_converters() -> Vec<AddonJsonConverter<'static>> {
    vec![
        Box::new(move |value: &txtx_addon_kit::types::types::Value| {
            txtx_addon_network_svm_types::SvmValue::to_json(value)
        }) as AddonJsonConverter<'static>,
    ]
}

/// `SurfnetSvm` provides a lightweight Solana Virtual Machine (SVM) for testing and simulation.
///
/// It supports a local in-memory blockchain state,
/// remote RPC connections, transaction processing, and account management.
///
/// It also exposes channels to listen for simulation events (`SimnetEvent`) and Geyser plugin events (`GeyserEvent`).
#[derive(Clone)]
pub struct SurfnetSvm {
    pub inner: SurfnetLiteSvm,
    pub remote_rpc_url: Option<String>,
    pub chain_tip: BlockIdentifier,
    pub blocks: Box<dyn Storage<u64, BlockHeader>>,
    pub transactions: Box<dyn Storage<String, SurfnetTransactionStatus>>,
    pub transactions_queued_for_confirmation: VecDeque<(
        VersionedTransaction,
        Sender<TransactionStatusEvent>,
        Option<TransactionError>,
    )>,
    pub transactions_queued_for_finalization: VecDeque<(
        Slot,
        VersionedTransaction,
        Sender<TransactionStatusEvent>,
        Option<TransactionError>,
    )>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub latest_epoch_info: EpochInfo,
    pub simnet_events_tx: Sender<SimnetEvent>,
    pub geyser_events_tx: Sender<GeyserEvent>,
    pub signature_subscriptions: HashMap<Signature, Vec<SignatureSubscriptionData>>,
    pub account_subscriptions: AccountSubscriptionData,
    pub slot_subscriptions: Vec<Sender<SlotInfo>>,
    pub profile_tag_map: Box<dyn Storage<String, Vec<UuidOrSignature>>>,
    pub simulated_transaction_profiles: HashMap<Uuid, KeyedProfileResult>,
    pub executed_transaction_profiles: FifoMap<Signature, KeyedProfileResult>,
    pub logs_subscriptions: Vec<LogsSubscriptionData>,
    pub updated_at: u64,
    pub slot_time: u64,
    pub start_time: SystemTime,
    pub accounts_by_owner: Box<dyn Storage<String, Vec<String>>>,
    pub account_associated_data: HashMap<Pubkey, AccountAdditionalDataV3>,
    pub token_accounts: Box<dyn Storage<String, TokenAccount>>,
    pub token_mints: Box<dyn Storage<String, MintAccount>>,
    pub token_accounts_by_owner: Box<dyn Storage<String, Vec<String>>>,
    pub token_accounts_by_delegate: Box<dyn Storage<String, Vec<String>>>,
    pub token_accounts_by_mint: Box<dyn Storage<String, Vec<String>>>,
    pub total_supply: u64,
    pub circulating_supply: u64,
    pub non_circulating_supply: u64,
    pub non_circulating_accounts: Vec<String>,
    pub genesis_config: GenesisConfig,
    pub inflation: Inflation,
    /// A global monotonically increasing atomic number, which can be used to tell the order of the account update.
    /// For example, when an account is updated in the same slot multiple times,
    /// the update with higher write_version should supersede the one with lower write_version.
    pub write_version: u64,
    pub registered_idls: Box<dyn Storage<String, Vec<VersionedIdl>>>,
    pub feature_set: FeatureSet,
    pub instruction_profiling_enabled: bool,
    pub max_profiles: usize,
    pub runbook_executions: Vec<RunbookExecutionStatusReport>,
    pub account_update_slots: HashMap<Pubkey, Slot>,
    pub streamed_accounts: Box<dyn Storage<String, bool>>,
    pub recent_blockhashes: VecDeque<(SyntheticBlockhash, i64)>,
    pub scheduled_overrides: Box<dyn Storage<u64, Vec<OverrideInstance>>>,
    /// Tracks accounts that have been explicitly closed by the user.
    /// These accounts will not be fetched from mainnet even if they don't exist in the local cache.
    pub closed_accounts: HashSet<Pubkey>,
}

pub const FEATURE: Feature = Feature {
    activated_at: Some(0),
};

impl SurfnetSvm {
    pub fn new() -> (Self, Receiver<SimnetEvent>, Receiver<GeyserEvent>) {
        Self::_new(None, 0).unwrap()
    }

    pub fn new_with_db(
        database_url: Option<&str>,
        surfnet_id: u32,
    ) -> SurfpoolResult<(Self, Receiver<SimnetEvent>, Receiver<GeyserEvent>)> {
        Self::_new(database_url, surfnet_id)
    }

    /// Explicitly shutdown the SVM, performing cleanup like WAL checkpoint for SQLite.
    /// This should be called before the application exits to ensure data is persisted.
    pub fn shutdown(&self) {
        self.inner.shutdown();
        self.blocks.shutdown();
        self.transactions.shutdown();
        self.token_accounts.shutdown();
        self.token_mints.shutdown();
        self.accounts_by_owner.shutdown();
        self.token_accounts_by_owner.shutdown();
        self.token_accounts_by_delegate.shutdown();
        self.token_accounts_by_mint.shutdown();
        self.streamed_accounts.shutdown();
        self.scheduled_overrides.shutdown();
        self.registered_idls.shutdown();
        self.profile_tag_map.shutdown();
    }

    /// Creates a new instance of `SurfnetSvm`.
    ///
    /// Returns a tuple containing the SVM instance, a receiver for simulation events, and a receiver for Geyser plugin events.
    fn _new(
        database_url: Option<&str>,
        surfnet_id: u32,
    ) -> SurfpoolResult<(Self, Receiver<SimnetEvent>, Receiver<GeyserEvent>)> {
        let (simnet_events_tx, simnet_events_rx) = crossbeam_channel::bounded(1024);
        let (geyser_events_tx, geyser_events_rx) = crossbeam_channel::bounded(1024);

        let mut feature_set = FeatureSet::all_enabled();

        // todo: remove once txtx deployments upgrade solana dependencies.
        // todo: consider making this configurable via config
        feature_set.deactivate(&enable_extend_program_checked::id());

        let inner =
            SurfnetLiteSvm::new().initialize(feature_set.clone(), database_url, surfnet_id)?;

        let native_mint_account = inner
            .get_account(&spl_token_interface::native_mint::ID)?
            .unwrap();
        let parsed_mint_account = MintAccount::unpack(&native_mint_account.data).unwrap();

        // Load native mint into owned account and token mint indexes
        let mut accounts_by_owner_db: Box<dyn Storage<String, Vec<String>>> =
            new_kv_store(&database_url, "accounts_by_owner", surfnet_id)?;
        accounts_by_owner_db.store(
            native_mint_account.owner.to_string(),
            vec![spl_token_interface::native_mint::ID.to_string()],
        )?;
        let blocks_db = new_kv_store(&database_url, "blocks", surfnet_id)?;
        let transactions_db = new_kv_store(&database_url, "transactions", surfnet_id)?;
        let token_accounts_db = new_kv_store(&database_url, "token_accounts", surfnet_id)?;
        let mut token_mints_db: Box<dyn Storage<String, MintAccount>> =
            new_kv_store(&database_url, "token_mints", surfnet_id)?;
        token_mints_db.store(
            spl_token_interface::native_mint::ID.to_string(),
            parsed_mint_account,
        )?;
        let token_accounts_by_owner_db: Box<dyn Storage<String, Vec<String>>> =
            new_kv_store(&database_url, "token_accounts_by_owner", surfnet_id)?;
        let token_accounts_by_delegate_db: Box<dyn Storage<String, Vec<String>>> =
            new_kv_store(&database_url, "token_accounts_by_delegate", surfnet_id)?;
        let token_accounts_by_mint_db: Box<dyn Storage<String, Vec<String>>> =
            new_kv_store(&database_url, "token_accounts_by_mint", surfnet_id)?;
        let streamed_accounts_db: Box<dyn Storage<String, bool>> =
            new_kv_store(&database_url, "streamed_accounts", surfnet_id)?;
        let scheduled_overrides_db: Box<dyn Storage<u64, Vec<OverrideInstance>>> =
            new_kv_store(&database_url, "scheduled_overrides", surfnet_id)?;
        let registered_idls_db: Box<dyn Storage<String, Vec<VersionedIdl>>> =
            new_kv_store(&database_url, "registered_idls", surfnet_id)?;
        let profile_tag_map_db: Box<dyn Storage<String, Vec<UuidOrSignature>>> =
            new_kv_store(&database_url, "profile_tag_map", surfnet_id)?;

        let chain_tip = if let Some((_, block)) = blocks_db
            .into_iter()
            .unwrap()
            .max_by_key(|(slot, _): &(u64, BlockHeader)| *slot)
        {
            BlockIdentifier {
                index: block.block_height,
                hash: block.hash,
            }
        } else {
            BlockIdentifier::zero()
        };

        let mut svm = Self {
            inner,
            remote_rpc_url: None,
            chain_tip,
            blocks: blocks_db,
            transactions: transactions_db,
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
            profile_tag_map: profile_tag_map_db,
            simulated_transaction_profiles: HashMap::new(),
            executed_transaction_profiles: FifoMap::default(),
            logs_subscriptions: Vec::new(),
            updated_at: Utc::now().timestamp_millis() as u64,
            slot_time: DEFAULT_SLOT_TIME_MS,
            start_time: SystemTime::now(),
            accounts_by_owner: accounts_by_owner_db,
            account_associated_data: HashMap::new(),
            token_accounts: token_accounts_db,
            token_mints: token_mints_db,
            token_accounts_by_owner: token_accounts_by_owner_db,
            token_accounts_by_delegate: token_accounts_by_delegate_db,
            token_accounts_by_mint: token_accounts_by_mint_db,
            total_supply: 0,
            circulating_supply: 0,
            non_circulating_supply: 0,
            non_circulating_accounts: Vec::new(),
            genesis_config: GenesisConfig::default(),
            inflation: Inflation::default(),
            write_version: 0,
            registered_idls: registered_idls_db,
            feature_set,
            instruction_profiling_enabled: true,
            max_profiles: DEFAULT_PROFILING_MAP_CAPACITY,
            runbook_executions: Vec::new(),
            account_update_slots: HashMap::new(),
            streamed_accounts: streamed_accounts_db,
            recent_blockhashes: VecDeque::new(),
            scheduled_overrides: scheduled_overrides_db,
            closed_accounts: HashSet::new(),
        };

        // Generate the initial synthetic blockhash
        svm.chain_tip = svm.new_blockhash();

        Ok((svm, simnet_events_rx, geyser_events_rx))
    }

    /// Applies the SVM feature configuration to the internal feature set.
    ///
    /// This method enables or disables specific SVM features based on the provided configuration.
    /// Features explicitly listed in `enable` will be activated, and features in `disable` will be deactivated.
    ///
    /// # Arguments
    /// * `config` - The feature configuration specifying which features to enable/disable.
    pub fn apply_feature_config(&mut self, config: &SvmFeatureConfig) {
        // Apply explicit enables
        for feature in &config.enable {
            if let Some(id) = Self::feature_to_id(feature) {
                self.feature_set.activate(&id, 0);
            }
        }

        // Apply explicit disables
        for feature in &config.disable {
            if let Some(id) = Self::feature_to_id(feature) {
                self.feature_set.deactivate(&id);
            }
        }

        // Rebuild inner VM with updated feature set
        self.inner.apply_feature_config(self.feature_set.clone());
    }

    /// Maps an SvmFeature enum variant to its corresponding feature ID (Pubkey).
    fn feature_to_id(feature: &SvmFeature) -> Option<Pubkey> {
        match feature {
            SvmFeature::MovePrecompileVerificationToSvm => {
                Some(move_precompile_verification_to_svm::id())
            }
            SvmFeature::StricterAbiAndRuntimeConstraints => {
                Some(stricter_abi_and_runtime_constraints::id())
            }
            SvmFeature::EnableBpfLoaderSetAuthorityCheckedIx => {
                Some(enable_bpf_loader_set_authority_checked_ix::id())
            }
            SvmFeature::EnableLoaderV4 => Some(enable_loader_v4::id()),
            SvmFeature::DepleteCuMeterOnVmFailure => Some(deplete_cu_meter_on_vm_failure::id()),
            SvmFeature::AbortOnInvalidCurve => Some(abort_on_invalid_curve::id()),
            SvmFeature::Blake3SyscallEnabled => Some(blake3_syscall_enabled::id()),
            SvmFeature::Curve25519SyscallEnabled => Some(curve25519_syscall_enabled::id()),
            SvmFeature::DisableDeployOfAllocFreeSyscall => {
                Some(disable_deploy_of_alloc_free_syscall::id())
            }
            SvmFeature::DisableFeesSysvar => Some(disable_fees_sysvar::id()),
            SvmFeature::DisableSbpfV0Execution => Some(disable_sbpf_v0_execution::id()),
            SvmFeature::EnableAltBn128CompressionSyscall => {
                Some(enable_alt_bn128_compression_syscall::id())
            }
            SvmFeature::EnableAltBn128Syscall => Some(enable_alt_bn128_syscall::id()),
            SvmFeature::EnableBigModExpSyscall => Some(enable_big_mod_exp_syscall::id()),
            SvmFeature::EnableGetEpochStakeSyscall => Some(enable_get_epoch_stake_syscall::id()),
            SvmFeature::EnablePoseidonSyscall => Some(enable_poseidon_syscall::id()),
            SvmFeature::EnableSbpfV1DeploymentAndExecution => {
                Some(enable_sbpf_v1_deployment_and_execution::id())
            }
            SvmFeature::EnableSbpfV2DeploymentAndExecution => {
                Some(enable_sbpf_v2_deployment_and_execution::id())
            }
            SvmFeature::EnableSbpfV3DeploymentAndExecution => {
                Some(enable_sbpf_v3_deployment_and_execution::id())
            }
            SvmFeature::GetSysvarSyscallEnabled => Some(get_sysvar_syscall_enabled::id()),
            SvmFeature::LastRestartSlotSysvar => Some(last_restart_slot_sysvar::id()),
            SvmFeature::ReenableSbpfV0Execution => Some(reenable_sbpf_v0_execution::id()),
            SvmFeature::RemainingComputeUnitsSyscallEnabled => {
                Some(remaining_compute_units_syscall_enabled::id())
            }
            SvmFeature::RemoveBpfLoaderIncorrectProgramId => {
                Some(remove_bpf_loader_incorrect_program_id::id())
            }
            SvmFeature::MoveStakeAndMoveLamportsIxs => Some(move_stake_and_move_lamports_ixs::id()),
            SvmFeature::StakeRaiseMinimumDelegationTo1Sol => {
                Some(stake_raise_minimum_delegation_to_1_sol::id())
            }
            SvmFeature::DeprecateLegacyVoteIxs => Some(deprecate_legacy_vote_ixs::id()),
            SvmFeature::MaskOutRentEpochInVmSerialization => {
                Some(mask_out_rent_epoch_in_vm_serialization::id())
            }
            SvmFeature::SimplifyAltBn128SyscallErrorCodes => {
                Some(simplify_alt_bn128_syscall_error_codes::id())
            }
            SvmFeature::FixAltBn128MultiplicationInputLength => {
                Some(fix_alt_bn128_multiplication_input_length::id())
            }
            SvmFeature::IncreaseTxAccountLockLimit => Some(increase_tx_account_lock_limit::id()),
            SvmFeature::EnableExtendProgramChecked => Some(enable_extend_program_checked::id()),
            SvmFeature::FormalizeLoadedTransactionDataSize => {
                Some(formalize_loaded_transaction_data_size::id())
            }
            SvmFeature::DisableZkElgamalProofProgram => {
                Some(disable_zk_elgamal_proof_program::id())
            }
            SvmFeature::ReenableZkElgamalProofProgram => {
                Some(reenable_zk_elgamal_proof_program::id())
            }
            SvmFeature::RaiseCpiNestingLimitTo8 => Some(raise_cpi_nesting_limit_to_8::id()),
        }
    }

    pub fn increment_write_version(&mut self) -> u64 {
        self.write_version += 1;
        self.write_version
    }

    /// Initializes the SVM with the provided epoch info and optionally notifies about remote connection.
    ///
    /// Updates the internal epoch info, sends connection and epoch update events, and sets the clock sysvar.
    ///
    /// # Arguments
    /// * `epoch_info` - The epoch information to initialize with.
    /// * `remote_ctx` - Optional remote client context for event notification.
    ///
    pub fn initialize(
        &mut self,
        epoch_info: EpochInfo,
        slot_time: u64,
        remote_ctx: &Option<SurfnetRemoteClient>,
        do_profile_instructions: bool,
        log_bytes_limit: Option<usize>,
    ) {
        self.chain_tip = self.new_blockhash();
        self.latest_epoch_info = epoch_info.clone();
        self.updated_at = Utc::now().timestamp_millis() as u64;
        self.slot_time = slot_time;
        self.instruction_profiling_enabled = do_profile_instructions;
        self.set_profiling_map_capacity(self.max_profiles);
        self.inner.set_log_bytes_limit(log_bytes_limit);

        let registry = TemplateRegistry::new();
        for (_, template) in registry.templates.into_iter() {
            let _ = self.register_idl(template.idl, None);
        }

        if let Some(remote_client) = remote_ctx {
            let _ = self
                .simnet_events_tx
                .send(SimnetEvent::Connected(remote_client.client.url()));
        }
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::EpochInfoUpdate(epoch_info));

        let clock: Clock = Clock {
            slot: self.latest_epoch_info.absolute_slot,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp: self.updated_at as i64 / 1_000,
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };

        self.inner.set_sysvar(&clock);
    }

    pub fn set_profile_instructions(&mut self, do_profile_instructions: bool) {
        self.instruction_profiling_enabled = do_profile_instructions;
    }

    pub fn set_profiling_map_capacity(&mut self, capacity: usize) {
        let clamped_capacity = max(1, capacity);
        self.max_profiles = clamped_capacity;
        self.executed_transaction_profiles = FifoMap::new(clamped_capacity);
    }

    /// Airdrops a specified amount of lamports to a single public key.
    ///
    /// # Arguments
    /// * `pubkey` - The recipient public key.
    /// * `lamports` - The amount of lamports to airdrop.
    ///
    /// # Returns
    /// A `TransactionResult` indicating success or failure.
    #[allow(clippy::result_large_err)]
    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> SurfpoolResult<TransactionResult> {
        let res = self.inner.airdrop(pubkey, lamports);
        let (status_tx, _rx) = unbounded();
        if let Ok(ref tx_result) = res {
            let airdrop_keypair = Keypair::new();
            let slot = self.latest_epoch_info.absolute_slot;
            let account = self.get_account(pubkey)?.unwrap();

            let mut tx = VersionedTransaction::try_new(
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
            .unwrap();

            // we need the airdrop tx to store in our transactions list,
            // but for it to be properly processed we need its signature to match
            // the actual underlying transaction
            tx.signatures[0] = tx_result.signature;

            let system_lamports = self
                .get_account(&system_program::id())?
                .map(|a| a.lamports())
                .unwrap_or(1);
            self.transactions.store(
                tx.get_signature().to_string(),
                SurfnetTransactionStatus::processed(
                    TransactionWithStatusMeta {
                        slot,
                        transaction: tx.clone(),
                        meta: TransactionStatusMeta {
                            status: Ok(()),
                            fee: 5000,
                            pre_balances: vec![
                                account.lamports,
                                account.lamports.saturating_sub(lamports),
                                system_lamports,
                            ],
                            post_balances: vec![
                                account.lamports.saturating_sub(lamports + 5000),
                                account.lamports,
                                system_lamports,
                            ],
                            inner_instructions: Some(vec![]),
                            log_messages: Some(tx_result.logs.clone()),
                            pre_token_balances: Some(vec![]),
                            post_token_balances: Some(vec![]),
                            rewards: Some(vec![]),
                            loaded_addresses: LoadedAddresses::default(),
                            return_data: Some(tx_result.return_data.clone()),
                            compute_units_consumed: Some(tx_result.compute_units_consumed),
                            cost_units: None,
                        },
                    },
                    HashSet::from([*pubkey]),
                ),
            )?;
            self.notify_signature_subscribers(
                SignatureSubscriptionType::processed(),
                tx.get_signature(),
                slot,
                None,
            );
            self.notify_logs_subscribers(
                tx.get_signature(),
                None,
                tx_result.logs.clone(),
                CommitmentLevel::Processed,
            );
            self.transactions_queued_for_confirmation
                .push_back((tx, status_tx.clone(), None));
            let account = self.get_account(pubkey)?.unwrap();
            self.set_account(pubkey, account)?;
        }
        Ok(res)
    }

    /// Airdrops a specified amount of lamports to a list of public keys.
    ///
    /// # Arguments
    /// * `lamports` - The amount of lamports to airdrop.
    /// * `addresses` - Slice of recipient public keys.
    pub fn airdrop_pubkeys(&mut self, lamports: u64, addresses: &[Pubkey]) {
        for recipient in addresses {
            match self.airdrop(recipient, lamports) {
                Ok(_) => {
                    let _ = self.simnet_events_tx.send(SimnetEvent::info(format!(
                        "Genesis airdrop successful {}: {}",
                        recipient, lamports
                    )));
                }
                Err(e) => {
                    let _ = self.simnet_events_tx.send(SimnetEvent::error(format!(
                        "Genesis airdrop failed {}: {}",
                        recipient, e
                    )));
                }
            };
        }
    }

    /// Returns the latest known absolute slot from the local epoch info.
    pub const fn get_latest_absolute_slot(&self) -> Slot {
        self.latest_epoch_info.absolute_slot
    }

    /// Returns the latest blockhash known by the SVM.
    pub fn latest_blockhash(&self) -> solana_hash::Hash {
        Hash::from_str(&self.chain_tip.hash).expect("Invalid blockhash")
    }

    /// Returns the latest epoch info known by the `SurfnetSvm`.
    pub fn latest_epoch_info(&self) -> EpochInfo {
        self.latest_epoch_info.clone()
    }

    pub fn get_account_from_feature_set(&self, pubkey: &Pubkey) -> Option<Account> {
        self.feature_set.active().get(pubkey).map(|_| {
            let feature_bytes = bincode::serialize(&FEATURE).unwrap();
            let lamports = self
                .inner
                .minimum_balance_for_rent_exemption(feature_bytes.len());
            Account {
                lamports,
                data: feature_bytes,
                owner: solana_sdk_ids::feature::id(),
                executable: false,
                rent_epoch: 0,
            }
        })
    }

    /// Generates and sets a new blockhash, updating the RecentBlockhashes sysvar.
    ///
    /// # Returns
    /// A new `BlockIdentifier` for the updated blockhash.
    #[allow(deprecated)]
    fn new_blockhash(&mut self) -> BlockIdentifier {
        use solana_slot_hashes::SlotHashes;
        use solana_sysvar::recent_blockhashes::{IterItem, RecentBlockhashes};
        // Backup the current block hashes
        let recent_blockhashes_backup = self.inner.get_sysvar::<RecentBlockhashes>();
        let num_blockhashes_expected = recent_blockhashes_backup
            .len()
            .min(MAX_RECENT_BLOCKHASHES_STANDARD);
        // Invalidate the current block hash.
        // LiteSVM bug / feature: calling this method empties `sysvar::<RecentBlockhashes>()`
        self.inner.expire_blockhash();
        // Rebuild recent blockhashes
        let mut recent_blockhashes = Vec::with_capacity(num_blockhashes_expected);
        let recent_blockhashes_overriden = self.inner.get_sysvar::<RecentBlockhashes>();
        let latest_entry = recent_blockhashes_overriden
            .first()
            .expect("Latest blockhash not found");

        let new_synthetic_blockhash = SyntheticBlockhash::new(self.chain_tip.index);
        let new_synthetic_blockhash_str = new_synthetic_blockhash.to_string();

        recent_blockhashes.push(IterItem(
            0,
            new_synthetic_blockhash.hash(),
            latest_entry.fee_calculator.lamports_per_signature,
        ));

        // Append the previous blockhashes, ignoring the first one
        for (index, entry) in recent_blockhashes_backup.iter().enumerate() {
            if recent_blockhashes.len() >= MAX_RECENT_BLOCKHASHES_STANDARD {
                break;
            }
            recent_blockhashes.push(IterItem(
                (index + 1) as u64,
                &entry.blockhash,
                entry.fee_calculator.lamports_per_signature,
            ));
        }

        self.inner
            .set_sysvar(&RecentBlockhashes::from_iter(recent_blockhashes));

        let mut slot_hashes = self.inner.get_sysvar::<SlotHashes>();
        slot_hashes.add(
            self.get_latest_absolute_slot() + 1,
            *new_synthetic_blockhash.hash(),
        );
        self.inner.set_sysvar(&SlotHashes::new(&slot_hashes));

        BlockIdentifier::new(
            self.chain_tip.index + 1,
            new_synthetic_blockhash_str.as_str(),
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
            .get_sysvar::<solana_sysvar::recent_blockhashes::RecentBlockhashes>()
            .iter()
            .any(|entry| entry.blockhash == *recent_blockhash)
    }

    /// Validates the blockhash of a transaction, considering nonce accounts if present.
    /// If the transaction uses a nonce account, the blockhash is validated against the nonce account's stored blockhash.
    /// Otherwise, it is validated against the RecentBlockhashes sysvar.
    ///
    /// # Arguments
    /// * `tx` - The transaction to validate.
    ///
    /// # Returns
    /// `true` if the transaction blockhash is valid, `false` otherwise.
    pub fn validate_transaction_blockhash(&self, tx: &VersionedTransaction) -> bool {
        let recent_blockhash = tx.message.recent_blockhash();

        let some_nonce_account_index = tx
            .message
            .instructions()
            .get(solana_nonce::NONCED_TX_MARKER_IX_INDEX as usize)
            .filter(|instruction| {
                matches!(
                    tx.message.static_account_keys().get(instruction.program_id_index as usize),
                    Some(program_id) if system_program::check_id(program_id)
                ) && is_advance_nonce_instruction_data(&instruction.data)
            })
            .map(|instruction| {
                // nonce account is the first account in the instruction
                instruction.accounts.get(0)
            });

        debug!(
            "Validating tx blockhash: {}; is nonce tx?: {}",
            recent_blockhash,
            some_nonce_account_index.is_some()
        );

        if let Some(nonce_account_index) = some_nonce_account_index {
            trace!(
                "Nonce tx detected. Nonce account index: {:?}",
                nonce_account_index
            );
            let Some(nonce_account_index) = nonce_account_index else {
                return false;
            };

            let Some(nonce_account_pubkey) = tx
                .message
                .static_account_keys()
                .get(*nonce_account_index as usize)
            else {
                return false;
            };

            trace!("Nonce account pubkey: {:?}", nonce_account_pubkey,);

            // Here we're swallowing errors in the storage - if we fail to fetch the account because of a storage error,
            // we're just considering the blockhash to be invalid.
            let Ok(Some(nonce_account)) = self.get_account(nonce_account_pubkey) else {
                return false;
            };
            trace!("Nonce account: {:?}", nonce_account);

            let Some(nonce_data) =
                bincode::deserialize::<solana_nonce::versions::Versions>(&nonce_account.data).ok()
            else {
                return false;
            };
            trace!("Nonce account data: {:?}", nonce_data);

            let nonce_state = nonce_data.state();
            let initialized_state = match nonce_state {
                solana_nonce::state::State::Uninitialized => return false,
                solana_nonce::state::State::Initialized(data) => data,
            };
            return initialized_state.blockhash() == *recent_blockhash;
        } else {
            self.check_blockhash_is_recent(recent_blockhash)
        }
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
        self.inner
            .set_account(*pubkey, account.clone())
            .map_err(|e| SurfpoolError::set_account(*pubkey, e))?;

        self.account_update_slots
            .insert(*pubkey, self.get_latest_absolute_slot());

        // Update the account registries and indexes
        self.update_account_registries(pubkey, &account)?;

        // Notify account subscribers
        self.notify_account_subscribers(pubkey, &account);

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::account_update(*pubkey));
        Ok(())
    }

    pub fn update_account_registries(
        &mut self,
        pubkey: &Pubkey,
        account: &Account,
    ) -> SurfpoolResult<()> {
        let is_deleted_account = account == &Account::default();

        // When this function is called after processing a transaction, the account is already updated
        // in the inner SVM. However, the database hasn't been updated yet, so we need to manually update the db.
        if is_deleted_account {
            // This amounts to deleting the account from the db if the account is deleted in the SVM
            self.inner.delete_account_in_db(pubkey)?;
        } else {
            // Or updating the db account to match the SVM account if not deleted
            self.inner
                .set_account_in_db(*pubkey, account.clone().into())?;
        }

        if is_deleted_account {
            self.closed_accounts.insert(*pubkey);
            if let Some(old_account) = self.get_account(pubkey)? {
                self.remove_from_indexes(pubkey, &old_account)?;
            }
            return Ok(());
        }

        // only update our indexes if the account exists in the svm accounts db
        if let Some(old_account) = self.get_account(pubkey)? {
            self.remove_from_indexes(pubkey, &old_account)?;
        }
        // add to owner index (check for duplicates)
        let owner_key = account.owner.to_string();
        let pubkey_str = pubkey.to_string();
        let mut owner_accounts = self
            .accounts_by_owner
            .get(&owner_key)
            .ok()
            .flatten()
            .unwrap_or_default();
        if !owner_accounts.contains(&pubkey_str) {
            owner_accounts.push(pubkey_str.clone());
            self.accounts_by_owner.store(owner_key, owner_accounts)?;
        }

        // if it's a token account, update token-specific indexes
        if is_supported_token_program(&account.owner) {
            if let Ok(token_account) = TokenAccount::unpack(&account.data) {
                // index by owner -> check for duplicates
                let owner_key = token_account.owner().to_string();
                let mut token_owner_accounts = self
                    .token_accounts_by_owner
                    .get(&owner_key)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !token_owner_accounts.contains(&pubkey_str) {
                    token_owner_accounts.push(pubkey_str.clone());
                    self.token_accounts_by_owner
                        .store(owner_key, token_owner_accounts)?;
                }

                // index by mint -> check for duplicates
                let mint_key = token_account.mint().to_string();
                let mut mint_accounts = self
                    .token_accounts_by_mint
                    .get(&mint_key)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !mint_accounts.contains(&pubkey_str) {
                    mint_accounts.push(pubkey_str.clone());
                    self.token_accounts_by_mint.store(mint_key, mint_accounts)?;
                }

                if let COption::Some(delegate) = token_account.delegate() {
                    let delegate_key = delegate.to_string();
                    let mut delegate_accounts = self
                        .token_accounts_by_delegate
                        .get(&delegate_key)
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    if !delegate_accounts.contains(&pubkey_str) {
                        delegate_accounts.push(pubkey_str);
                        self.token_accounts_by_delegate
                            .store(delegate_key, delegate_accounts)?;
                    }
                }
                self.token_accounts
                    .store(pubkey.to_string(), token_account)?;
            }

            if let Ok(mint_account) = MintAccount::unpack(&account.data) {
                self.token_mints.store(pubkey.to_string(), mint_account)?;
            }

            if let Ok(mint) =
                StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&account.data)
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
        Ok(())
    }

    fn remove_from_indexes(
        &mut self,
        pubkey: &Pubkey,
        old_account: &Account,
    ) -> SurfpoolResult<()> {
        let owner_key = old_account.owner.to_string();
        let pubkey_str = pubkey.to_string();
        if let Some(mut accounts) = self.accounts_by_owner.get(&owner_key).ok().flatten() {
            accounts.retain(|pk| pk != &pubkey_str);
            if accounts.is_empty() {
                self.accounts_by_owner.take(&owner_key)?;
            } else {
                self.accounts_by_owner.store(owner_key, accounts)?;
            }
        }

        // if it was a token account, remove from token indexes
        if is_supported_token_program(&old_account.owner) {
            if let Some(old_token_account) = self.token_accounts.take(&pubkey.to_string())? {
                let owner_key = old_token_account.owner().to_string();
                if let Some(mut accounts) =
                    self.token_accounts_by_owner.get(&owner_key).ok().flatten()
                {
                    accounts.retain(|pk| pk != &pubkey_str);
                    if accounts.is_empty() {
                        self.token_accounts_by_owner.take(&owner_key)?;
                    } else {
                        self.token_accounts_by_owner.store(owner_key, accounts)?;
                    }
                }

                let mint_key = old_token_account.mint().to_string();
                if let Some(mut accounts) =
                    self.token_accounts_by_mint.get(&mint_key).ok().flatten()
                {
                    accounts.retain(|pk| pk != &pubkey_str);
                    if accounts.is_empty() {
                        self.token_accounts_by_mint.take(&mint_key)?;
                    } else {
                        self.token_accounts_by_mint.store(mint_key, accounts)?;
                    }
                }

                if let COption::Some(delegate) = old_token_account.delegate() {
                    let delegate_key = delegate.to_string();
                    if let Some(mut accounts) = self
                        .token_accounts_by_delegate
                        .get(&delegate_key)
                        .ok()
                        .flatten()
                    {
                        accounts.retain(|pk| pk != &pubkey_str);
                        if accounts.is_empty() {
                            self.token_accounts_by_delegate.take(&delegate_key)?;
                        } else {
                            self.token_accounts_by_delegate
                                .store(delegate_key, accounts)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn reset_network(&mut self) -> SurfpoolResult<()> {
        self.inner.reset(self.feature_set.clone())?;

        let native_mint_account = self
            .inner
            .get_account(&spl_token_interface::native_mint::ID)?
            .unwrap();
        let parsed_mint_account = MintAccount::unpack(&native_mint_account.data).unwrap();

        self.blocks.clear()?;
        self.transactions.clear()?;
        self.transactions_queued_for_confirmation.clear();
        self.transactions_queued_for_finalization.clear();
        self.perf_samples.clear();
        self.transactions_processed = 0;
        self.profile_tag_map.clear()?;
        self.simulated_transaction_profiles.clear();
        self.accounts_by_owner.clear()?;
        self.accounts_by_owner.store(
            native_mint_account.owner.to_string(),
            vec![spl_token_interface::native_mint::ID.to_string()],
        )?;
        self.account_associated_data.clear();
        self.token_accounts.clear()?;
        self.token_mints.clear()?;
        self.token_mints.store(
            spl_token_interface::native_mint::ID.to_string(),
            parsed_mint_account,
        )?;
        self.token_accounts_by_owner.clear()?;
        self.token_accounts_by_delegate.clear()?;
        self.token_accounts_by_mint.clear()?;
        self.non_circulating_accounts.clear();
        self.registered_idls.clear()?;
        self.runbook_executions.clear();
        self.streamed_accounts.clear()?;
        self.scheduled_overrides.clear()?;
        Ok(())
    }

    pub fn reset_account(
        &mut self,
        pubkey: &Pubkey,
        include_owned_accounts: bool,
    ) -> SurfpoolResult<()> {
        let Some(account) = self.get_account(pubkey)? else {
            return Ok(());
        };

        if account.executable {
            // Handle upgradeable program - also reset the program data account
            if account.owner == solana_sdk_ids::bpf_loader_upgradeable::id() {
                let program_data_pubkey =
                    solana_loader_v3_interface::get_program_data_address(pubkey);

                // Reset the program data account first
                self.purge_account_from_cache(&account, &program_data_pubkey)?;
            }
        }
        if include_owned_accounts {
            let owned_accounts = self.get_account_owned_by(pubkey)?;
            for (owned_pubkey, _) in owned_accounts {
                // Avoid infinite recursion by not cascading further
                self.purge_account_from_cache(&account, &owned_pubkey)?;
            }
        }
        // Reset the account itself
        self.purge_account_from_cache(&account, pubkey)?;
        Ok(())
    }

    fn purge_account_from_cache(
        &mut self,
        account: &Account,
        pubkey: &Pubkey,
    ) -> SurfpoolResult<()> {
        self.remove_from_indexes(pubkey, account)?;

        self.inner.delete_account(pubkey)?;

        Ok(())
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
    #[allow(clippy::result_large_err)]
    pub fn send_transaction(
        &mut self,
        tx: VersionedTransaction,
        cu_analysis_enabled: bool,
        sigverify: bool,
    ) -> TransactionResult {
        if sigverify && tx.verify_with_results().iter().any(|valid| !*valid) {
            return Err(FailedTransactionMetadata {
                err: TransactionError::SignatureFailure,
                meta: TransactionMetadata::default(),
            });
        }

        if cu_analysis_enabled {
            let estimation_result = self.estimate_compute_units(&tx);
            let _ = self.simnet_events_tx.try_send(SimnetEvent::info(format!(
                "CU Estimation for tx: {} | Consumed: {} | Success: {} | Logs: {:?} | Error: {:?}",
                tx.signatures
                    .first()
                    .map_or_else(|| "N/A".to_string(), |s| s.to_string()),
                estimation_result.compute_units_consumed,
                estimation_result.success,
                estimation_result.log_messages,
                estimation_result.error_message
            )));
        }
        self.transactions_processed += 1;

        if !self.validate_transaction_blockhash(&tx) {
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

        match self.inner.send_transaction(tx.clone()) {
            Ok(res) => Ok(res),
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
        if !self.validate_transaction_blockhash(transaction) {
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
    #[allow(clippy::result_large_err)]
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

        if !self.validate_transaction_blockhash(&tx) {
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
    fn confirm_transactions(&mut self) -> Result<(Vec<Signature>, HashSet<Pubkey>), SurfpoolError> {
        let mut confirmed_transactions = vec![];
        let slot = self.latest_epoch_info.slot_index;
        let current_slot = self.latest_epoch_info.absolute_slot;

        let mut all_mutated_account_keys = HashSet::new();

        while let Some((tx, status_tx, error)) =
            self.transactions_queued_for_confirmation.pop_front()
        {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
            let signature = tx.signatures[0];
            let finalized_at = self.latest_epoch_info.absolute_slot + FINALIZATION_SLOT_THRESHOLD;
            self.transactions_queued_for_finalization.push_back((
                finalized_at,
                tx,
                status_tx,
                error.clone(),
            ));

            self.notify_signature_subscribers(
                SignatureSubscriptionType::confirmed(),
                &signature,
                slot,
                error,
            );

            let Some(SurfnetTransactionStatus::Processed(tx_data)) =
                self.transactions.get(&signature.to_string()).ok().flatten()
            else {
                continue;
            };
            let (tx_with_status_meta, mutated_account_keys) = tx_data.as_ref();
            all_mutated_account_keys.extend(mutated_account_keys);

            for pubkey in mutated_account_keys {
                self.account_update_slots.insert(*pubkey, current_slot);
            }

            self.notify_logs_subscribers(
                &signature,
                None,
                tx_with_status_meta
                    .meta
                    .log_messages
                    .clone()
                    .unwrap_or(vec![]),
                CommitmentLevel::Confirmed,
            );
            confirmed_transactions.push(signature);
        }

        Ok((confirmed_transactions, all_mutated_account_keys))
    }

    /// Finalizes transactions queued for finalization, sending finalized events as needed.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(SurfpoolError)` on error.
    fn finalize_transactions(&mut self) -> Result<(), SurfpoolError> {
        let current_slot = self.latest_epoch_info.absolute_slot;
        let mut requeue = VecDeque::new();
        while let Some((finalized_at, tx, status_tx, error)) =
            self.transactions_queued_for_finalization.pop_front()
        {
            if current_slot >= finalized_at {
                let _ = status_tx.try_send(TransactionStatusEvent::Success(
                    TransactionConfirmationStatus::Finalized,
                ));
                let signature = &tx.signatures[0];
                self.notify_signature_subscribers(
                    SignatureSubscriptionType::finalized(),
                    signature,
                    self.latest_epoch_info.absolute_slot,
                    error,
                );
                let Some(SurfnetTransactionStatus::Processed(tx_data)) =
                    self.transactions.get(&signature.to_string()).ok().flatten()
                else {
                    continue;
                };
                let (tx_with_status_meta, _) = tx_data.as_ref();
                let logs = tx_with_status_meta
                    .meta
                    .log_messages
                    .clone()
                    .unwrap_or(vec![]);
                self.notify_logs_subscribers(signature, None, logs, CommitmentLevel::Finalized);
            } else {
                requeue.push_back((finalized_at, tx, status_tx, error));
            }
        }
        // Requeue any transactions that are not yet finalized
        self.transactions_queued_for_finalization
            .append(&mut requeue);

        Ok(())
    }

    /// Writes account updates to the SVM state based on the provided account update result.
    ///
    /// # Arguments
    /// * `account_update` - The account update result to process.
    pub fn write_account_update(&mut self, account_update: GetAccountResult) {
        let init_programdata_account = |program_account: &Account| {
            if !program_account.executable {
                return None;
            }
            if !program_account
                .owner
                .eq(&solana_sdk_ids::bpf_loader_upgradeable::id())
            {
                return None;
            }
            let Ok(UpgradeableLoaderState::Program {
                programdata_address,
            }) = bincode::deserialize::<UpgradeableLoaderState>(&program_account.data)
            else {
                return None;
            };

            let programdata_state = UpgradeableLoaderState::ProgramData {
                upgrade_authority_address: Some(system_program::id()),
                slot: self.get_latest_absolute_slot(),
            };
            let mut data = bincode::serialize(&programdata_state).unwrap();

            data.extend_from_slice(&include_bytes!("../tests/assets/minimum_program.so").to_vec());
            let lamports = self.inner.minimum_balance_for_rent_exemption(data.len());
            Some((
                programdata_address,
                Account {
                    lamports,
                    data,
                    owner: solana_sdk_ids::bpf_loader_upgradeable::id(),
                    executable: false,
                    rent_epoch: 0,
                },
            ))
        };
        match account_update {
            GetAccountResult::FoundAccount(pubkey, account, do_update_account) => {
                if do_update_account {
                    if let Some((programdata_address, programdata_account)) =
                        init_programdata_account(&account)
                    {
                        match self.get_account(&programdata_address) {
                            Ok(None) => {
                                if let Err(e) =
                                    self.set_account(&programdata_address, programdata_account)
                                {
                                    let _ = self
                                        .simnet_events_tx
                                        .send(SimnetEvent::error(e.to_string()));
                                }
                            }
                            Ok(Some(_)) => {}
                            Err(e) => {
                                let _ = self
                                    .simnet_events_tx
                                    .send(SimnetEvent::error(e.to_string()));
                            }
                        }
                    }
                    if let Err(e) = self.set_account(&pubkey, account.clone()) {
                        let _ = self
                            .simnet_events_tx
                            .send(SimnetEvent::error(e.to_string()));
                    }
                }
            }
            GetAccountResult::FoundProgramAccount((pubkey, account), (_, None)) => {
                if let Some((programdata_address, programdata_account)) =
                    init_programdata_account(&account)
                {
                    match self.get_account(&programdata_address) {
                        Ok(None) => {
                            if let Err(e) =
                                self.set_account(&programdata_address, programdata_account)
                            {
                                let _ = self
                                    .simnet_events_tx
                                    .send(SimnetEvent::error(e.to_string()));
                            }
                        }
                        Ok(Some(_)) => {}
                        Err(e) => {
                            let _ = self
                                .simnet_events_tx
                                .send(SimnetEvent::error(e.to_string()));
                        }
                    }
                }
                if let Err(e) = self.set_account(&pubkey, account.clone()) {
                    let _ = self
                        .simnet_events_tx
                        .send(SimnetEvent::error(e.to_string()));
                }
            }
            GetAccountResult::FoundTokenAccount((pubkey, account), (_, None)) => {
                if let Err(e) = self.set_account(&pubkey, account.clone()) {
                    let _ = self
                        .simnet_events_tx
                        .send(SimnetEvent::error(e.to_string()));
                }
            }
            GetAccountResult::FoundProgramAccount(
                (pubkey, account),
                (coupled_pubkey, Some(coupled_account)),
            )
            | GetAccountResult::FoundTokenAccount(
                (pubkey, account),
                (coupled_pubkey, Some(coupled_account)),
            ) => {
                // The data account _must_ be set first, as the program account depends on it.
                if let Err(e) = self.set_account(&coupled_pubkey, coupled_account.clone()) {
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

    pub fn confirm_current_block(&mut self) -> SurfpoolResult<()> {
        let slot = self.get_latest_absolute_slot();
        let previous_chain_tip = self.chain_tip.clone();
        if slot % 100 == 0 {
            debug!("Clearing liteSVM cache at slot {}", slot);
            self.inner.garbage_collect(self.feature_set.clone());
        }
        self.chain_tip = self.new_blockhash();
        // Confirm processed transactions
        let (confirmed_signatures, all_mutated_account_keys) = self.confirm_transactions()?;
        let write_version = self.increment_write_version();

        // Notify Geyser plugin of account updates
        for pubkey in all_mutated_account_keys {
            let Some(account) = self.inner.get_account(&pubkey)? else {
                continue;
            };
            self.geyser_events_tx
                .send(GeyserEvent::UpdateAccount(
                    GeyserAccountUpdate::block_update(pubkey, account, slot, write_version),
                ))
                .ok();
        }

        let num_transactions = confirmed_signatures.len() as u64;
        self.updated_at += self.slot_time;

        self.blocks.store(
            slot,
            BlockHeader {
                hash: self.chain_tip.hash.clone(),
                previous_blockhash: previous_chain_tip.hash,
                block_time: self.updated_at as i64 / 1_000,
                block_height: self.chain_tip.index,
                parent_slot: slot,
                signatures: confirmed_signatures,
            },
        )?;
        if self.perf_samples.len() > 30 {
            self.perf_samples.pop_back();
        }
        self.perf_samples.push_front(RpcPerfSample {
            slot,
            num_slots: 1,
            sample_period_secs: 1,
            num_transactions,
            num_non_vote_transactions: Some(num_transactions),
        });

        self.latest_epoch_info.slot_index += 1;
        self.latest_epoch_info.block_height = self.chain_tip.index;
        self.latest_epoch_info.absolute_slot += 1;
        if self.latest_epoch_info.slot_index > self.latest_epoch_info.slots_in_epoch {
            self.latest_epoch_info.slot_index = 0;
            self.latest_epoch_info.epoch += 1;
        }
        let total_transactions = self.latest_epoch_info.transaction_count.unwrap_or(0);
        self.latest_epoch_info.transaction_count = Some(total_transactions + num_transactions);

        let parent_slot = self.latest_epoch_info.absolute_slot.saturating_sub(1);
        let new_slot = self.latest_epoch_info.absolute_slot;
        let root = new_slot.saturating_sub(FINALIZATION_SLOT_THRESHOLD);
        self.notify_slot_subscribers(new_slot, parent_slot, root);

        let clock: Clock = Clock {
            slot: self.latest_epoch_info.absolute_slot,
            epoch: self.latest_epoch_info.epoch,
            unix_timestamp: self.updated_at as i64 / 1_000,
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::SystemClockUpdated(clock.clone()));
        self.inner.set_sysvar(&clock);

        self.finalize_transactions()?;

        // Evict the accounts marked as streamed from cache to enforce them to be fetched again
        let accounts_to_reset: Vec<_> = self.streamed_accounts.into_iter()?.collect();
        for (pubkey_str, include_owned_accounts) in accounts_to_reset {
            let pubkey = Pubkey::from_str(&pubkey_str)
                .map_err(|e| SurfpoolError::invalid_pubkey(&pubkey_str, e.to_string()))?;
            self.reset_account(&pubkey, include_owned_accounts)?;
        }

        Ok(())
    }

    /// Materializes scheduled overrides for the current slot
    ///
    /// This function:
    /// 1. Dequeues overrides scheduled for the current slot
    /// 2. Resolves account addresses (Pubkey or PDA)
    /// 3. Optionally fetches fresh account data from remote if `fetch_before_use` is enabled
    /// 4. Applies the overrides to the account data
    /// 5. Updates the SVM state
    pub async fn materialize_overrides(
        &mut self,
        remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
    ) -> SurfpoolResult<()> {
        let current_slot = self.latest_epoch_info.absolute_slot;

        // Remove and get overrides for this slot
        let Some(overrides) = self.scheduled_overrides.take(&current_slot)? else {
            // No overrides for this slot
            return Ok(());
        };

        debug!(
            "Materializing {} override(s) for slot {}",
            overrides.len(),
            current_slot
        );

        for override_instance in overrides {
            if !override_instance.enabled {
                debug!("Skipping disabled override: {}", override_instance.id);
                continue;
            }

            // Resolve account address
            let account_pubkey = match &override_instance.account {
                surfpool_types::AccountAddress::Pubkey(pubkey_str) => {
                    match Pubkey::from_str(pubkey_str) {
                        Ok(pubkey) => pubkey,
                        Err(e) => {
                            warn!(
                                "Failed to parse pubkey '{}' for override {}: {}",
                                pubkey_str, override_instance.id, e
                            );
                            continue;
                        }
                    }
                }
                surfpool_types::AccountAddress::Pda {
                    program_id: _,
                    seeds: _,
                } => unimplemented!(),
            };

            debug!(
                "Processing override {} for account {} (label: {:?})",
                override_instance.id, account_pubkey, override_instance.label
            );

            // Fetch fresh account data from remote if requested
            if override_instance.fetch_before_use {
                if let Some((client, _)) = remote_ctx {
                    debug!(
                        "Fetching fresh account data for {} from remote",
                        account_pubkey
                    );

                    match client
                        .get_account(&account_pubkey, CommitmentConfig::confirmed())
                        .await
                    {
                        Ok(GetAccountResult::FoundAccount(_pubkey, remote_account, _)) => {
                            debug!(
                                "Fetched account {} from remote: {} lamports, {} bytes",
                                account_pubkey,
                                remote_account.lamports(),
                                remote_account.data().len()
                            );

                            // Set the fresh account data in the SVM
                            if let Err(e) = self.inner.set_account(account_pubkey, remote_account) {
                                warn!(
                                    "Failed to set account {} from remote: {}",
                                    account_pubkey, e
                                );
                            }
                        }
                        Ok(GetAccountResult::None(_)) => {
                            debug!("Account {} not found on remote", account_pubkey);
                        }
                        Ok(_) => {
                            debug!("Account {} fetched (other variant)", account_pubkey);
                        }
                        Err(e) => {
                            warn!(
                                "Failed to fetch account {} from remote: {}",
                                account_pubkey, e
                            );
                        }
                    }
                } else {
                    debug!(
                        "fetch_before_use enabled but no remote client available for override {}",
                        override_instance.id
                    );
                }
            }

            // Apply the override values to the account data
            if !override_instance.values.is_empty() {
                debug!(
                    "Override {} applying {} field modification(s) to account {}",
                    override_instance.id,
                    override_instance.values.len(),
                    account_pubkey
                );

                // Get the account from the SVM
                let Some(account) = self.inner.get_account(&account_pubkey)? else {
                    warn!(
                        "Account {} not found in SVM for override {}, skipping modifications",
                        account_pubkey, override_instance.id
                    );
                    continue;
                };

                // Get the account owner (program ID)
                let owner_program_id = account.owner();

                // Look up the IDL for the owner program
                let idl_versions = match self.registered_idls.get(&owner_program_id.to_string()) {
                    Ok(Some(versions)) => versions,
                    Ok(None) => {
                        warn!(
                            "No IDL registered for program {} (owner of account {}), skipping override {}",
                            owner_program_id, account_pubkey, override_instance.id
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to get IDL for program {}: {}, skipping override {}",
                            owner_program_id, e, override_instance.id
                        );
                        continue;
                    }
                };

                // Get the latest IDL version (first in the sorted Vec)
                let Some(versioned_idl) = idl_versions.first() else {
                    warn!(
                        "IDL versions empty for program {}, skipping override {}",
                        owner_program_id, override_instance.id
                    );
                    continue;
                };

                let idl = &versioned_idl.1;

                // Get account data
                let account_data = account.data();

                // Use get_forged_account_data to apply the overrides
                let new_account_data = match self.get_forged_account_data(
                    &account_pubkey,
                    account_data,
                    idl,
                    &override_instance.values,
                ) {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(
                            "Failed to forge account data for {} (override {}): {}",
                            account_pubkey, override_instance.id, e
                        );
                        continue;
                    }
                };

                // Create a new account with modified data
                let modified_account = Account {
                    lamports: account.lamports(),
                    data: new_account_data,
                    owner: *account.owner(),
                    executable: account.executable(),
                    rent_epoch: account.rent_epoch(),
                };

                // Update the account in the SVM
                if let Err(e) = self.inner.set_account(account_pubkey, modified_account) {
                    warn!(
                        "Failed to set modified account {} in SVM: {}",
                        account_pubkey, e
                    );
                } else {
                    debug!(
                        "Successfully applied {} override(s) to account {} (override {})",
                        override_instance.values.len(),
                        account_pubkey,
                        override_instance.id
                    );
                }
            }
        }

        Ok(())
    }

    /// Forges account data by applying overrides to existing account data
    ///
    /// This function:
    /// 1. Validates account data size (must be at least 8 bytes for discriminator)
    /// 2. Splits discriminator and serialized data
    /// 3. Finds the account type in the IDL using the discriminator
    /// 4. Deserializes the account data
    /// 5. Applies field overrides using dot notation
    /// 6. Re-serializes the modified data
    /// 7. Reconstructs the account data with the original discriminator
    ///
    /// # Arguments
    /// * `account_pubkey` - The account address (for error messages)
    /// * `account_data` - The original account data bytes
    /// * `idl` - The IDL for the account's program
    /// * `overrides` - Map of field paths to new values
    ///
    /// # Returns
    /// The forged account data as bytes, or an error
    pub fn get_forged_account_data(
        &self,
        account_pubkey: &Pubkey,
        account_data: &[u8],
        idl: &Idl,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> SurfpoolResult<Vec<u8>> {
        // Validate account data size
        if account_data.len() < 8 {
            return Err(SurfpoolError::invalid_account_data(
                account_pubkey,
                "Account data too small to be an Anchor account (need at least 8 bytes for discriminator)",
                Some("Data length too small"),
            ));
        }

        // Split discriminator and data
        let discriminator = &account_data[..8];
        let serialized_data = &account_data[8..];

        // Find the account type using the discriminator
        let account_def = idl
            .accounts
            .iter()
            .find(|acc| acc.discriminator.eq(discriminator))
            .ok_or_else(|| {
                SurfpoolError::internal(format!(
                    "Account with discriminator '{:?}' not found in IDL",
                    discriminator
                ))
            })?;

        // Find the corresponding type definition
        let account_type = idl
            .types
            .iter()
            .find(|t| t.name == account_def.name)
            .ok_or_else(|| {
                SurfpoolError::internal(format!(
                    "Type definition for account '{}' not found in IDL",
                    account_def.name
                ))
            })?;

        // Set up generics for parsing
        let empty_vec = vec![];
        let idl_type_def_generics = idl
            .types
            .iter()
            .find(|t| t.name == account_type.name)
            .map(|t| &t.generics);

        // Deserialize the account data using proper Borsh deserialization
        // Use the version that returns leftover bytes to preserve any trailing padding
        let (mut parsed_value, leftover_bytes) =
            parse_bytes_to_value_with_expected_idl_type_def_ty_with_leftover_bytes(
                serialized_data,
                &account_type.ty,
                &idl.types,
                &vec![],
                idl_type_def_generics.unwrap_or(&empty_vec),
            )
            .map_err(|e| {
                SurfpoolError::deserialize_error(
                    "account data",
                    format!("Failed to deserialize account data using Borsh: {}", e),
                )
            })?;

        // Apply overrides to the decoded value
        for (path, value) in overrides {
            apply_override_to_decoded_account(&mut parsed_value, path, value)?;
        }

        // Construct an IdlType::Defined that references the account type
        // This is needed because borsh_encode_value_to_idl_type expects IdlType, not IdlTypeDefTy
        use anchor_lang_idl::types::{IdlGenericArg, IdlType};
        let defined_type = IdlType::Defined {
            name: account_type.name.clone(),
            generics: account_type
                .generics
                .iter()
                .map(|_| IdlGenericArg::Type {
                    ty: IdlType::String,
                })
                .collect(),
        };

        // Re-encode the value using Borsh
        let re_encoded_data =
            borsh_encode_value_to_idl_type(&parsed_value, &defined_type, &idl.types, None)
                .map_err(|e| {
                    SurfpoolError::internal(format!(
                        "Failed to re-encode account data using Borsh: {}",
                        e
                    ))
                })?;

        // Reconstruct the account data with discriminator and preserve any trailing bytes
        let mut new_account_data =
            Vec::with_capacity(8 + re_encoded_data.len() + leftover_bytes.len());
        new_account_data.extend_from_slice(discriminator);
        new_account_data.extend_from_slice(&re_encoded_data);
        new_account_data.extend_from_slice(leftover_bytes);

        Ok(new_account_data)
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
                let config = RpcAccountInfoConfig {
                    encoding,
                    ..Default::default()
                };
                let account = self
                    .account_to_rpc_keyed_account(account_updated_pubkey, account, &config, None)
                    .account;
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
    /// * `config` - The configuration for the block retrieval.
    ///
    /// # Returns
    /// `Some(UiConfirmedBlock)` if found, or `None` if not present.
    pub fn get_block_at_slot(
        &self,
        slot: Slot,
        config: &RpcBlockConfig,
    ) -> SurfpoolResult<Option<UiConfirmedBlock>> {
        let Some(block) = self.blocks.get(&slot)? else {
            return Ok(None);
        };

        let show_rewards = config.rewards.unwrap_or(true);
        let transaction_details = config
            .transaction_details
            .unwrap_or(TransactionDetails::Full);

        let transactions = match transaction_details {
            TransactionDetails::Full => Some(
                block
                    .signatures
                    .iter()
                    .filter_map(|sig| self.transactions.get(&sig.to_string()).ok().flatten())
                    .map(|tx_with_meta| {
                        let (meta, _) = tx_with_meta.expect_processed();
                        meta.encode(
                            config.encoding.unwrap_or(
                                solana_transaction_status::UiTransactionEncoding::JsonParsed,
                            ),
                            config.max_supported_transaction_version,
                            show_rewards,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(SurfpoolError::from)?,
            ),
            TransactionDetails::Signatures => None,
            TransactionDetails::None => None,
            TransactionDetails::Accounts => Some(
                block
                    .signatures
                    .iter()
                    .filter_map(|sig| self.transactions.get(&sig.to_string()).ok().flatten())
                    .map(|tx_with_meta| {
                        let (meta, _) = tx_with_meta.expect_processed();
                        meta.to_json_accounts(
                            config.max_supported_transaction_version,
                            show_rewards,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(SurfpoolError::from)?,
            ),
        };

        let signatures = match transaction_details {
            TransactionDetails::Signatures => {
                Some(block.signatures.iter().map(|t| t.to_string()).collect())
            }
            TransactionDetails::Full | TransactionDetails::Accounts | TransactionDetails::None => {
                None
            }
        };

        let block = UiConfirmedBlock {
            previous_blockhash: block.previous_blockhash.clone(),
            blockhash: block.hash.clone(),
            parent_slot: block.parent_slot,
            transactions,
            signatures,
            rewards: if show_rewards { Some(vec![]) } else { None },
            num_reward_partitions: None,
            block_time: Some(block.block_time / 1000),
            block_height: Some(block.block_height),
        };
        Ok(Some(block))
    }

    /// Returns the blockhash for a given slot, if available.
    pub fn blockhash_for_slot(&self, slot: Slot) -> Option<Hash> {
        self.blocks
            .get(&slot)
            .unwrap()
            .and_then(|header| header.hash.parse().ok())
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
    pub fn get_account_owned_by(
        &self,
        program_id: &Pubkey,
    ) -> SurfpoolResult<Vec<(Pubkey, Account)>> {
        let account_pubkeys = self
            .accounts_by_owner
            .get(&program_id.to_string())
            .ok()
            .flatten()
            .unwrap_or_default();

        account_pubkeys
            .iter()
            .filter_map(|pk_str| {
                let pk = Pubkey::from_str(pk_str).ok()?;
                self.get_account(&pk)
                    .map(|res| res.map(|account| (pk, account.clone())))
                    .transpose()
            })
            .collect::<Result<Vec<_>, SurfpoolError>>()
    }

    fn get_additional_data(
        &self,
        pubkey: &Pubkey,
        token_mint: Option<Pubkey>,
    ) -> Option<AccountAdditionalDataV3> {
        let token_mint = if let Some(mint) = token_mint {
            Some(mint)
        } else {
            self.token_accounts
                .get(&pubkey.to_string())
                .ok()
                .flatten()
                .map(|ta| ta.mint())
        };

        token_mint.and_then(|mint| self.account_associated_data.get(&mint).cloned())
    }

    pub fn account_to_rpc_keyed_account<T: ReadableAccount>(
        &self,
        pubkey: &Pubkey,
        account: &T,
        config: &RpcAccountInfoConfig,
        token_mint: Option<Pubkey>,
    ) -> RpcKeyedAccount {
        let additional_data = self.get_additional_data(pubkey, token_mint);

        RpcKeyedAccount {
            pubkey: pubkey.to_string(),
            account: self.encode_ui_account(
                pubkey,
                account,
                config.encoding.unwrap_or(UiAccountEncoding::Base64),
                additional_data,
                config.data_slice,
            ),
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
        if let Some(account_pubkeys) = self
            .token_accounts_by_delegate
            .get(&delegate.to_string())
            .ok()
            .flatten()
        {
            account_pubkeys
                .iter()
                .filter_map(|pk_str| {
                    let pk = Pubkey::from_str(pk_str).ok()?;
                    self.token_accounts
                        .get(pk_str)
                        .ok()
                        .flatten()
                        .map(|ta| (pk, ta))
                })
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
        if let Some(account_pubkeys) = self
            .token_accounts_by_owner
            .get(&owner.to_string())
            .ok()
            .flatten()
        {
            account_pubkeys
                .iter()
                .filter_map(|pk_str| {
                    let pk = Pubkey::from_str(pk_str).ok()?;
                    self.token_accounts
                        .get(pk_str)
                        .ok()
                        .flatten()
                        .map(|ta| (pk, ta))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_token_accounts_by_owner(
        &self,
        owner: &Pubkey,
    ) -> SurfpoolResult<Vec<(Pubkey, Account)>> {
        let account_pubkeys = self
            .token_accounts_by_owner
            .get(&owner.to_string())
            .ok()
            .flatten()
            .unwrap_or_default();

        account_pubkeys
            .iter()
            .filter_map(|pk_str| {
                let pk = Pubkey::from_str(pk_str).ok()?;
                self.get_account(&pk)
                    .map(|res| res.map(|account| (pk, account.clone())))
                    .transpose()
            })
            .collect::<Result<Vec<_>, SurfpoolError>>()
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
        if let Some(account_pubkeys) = self
            .token_accounts_by_mint
            .get(&mint.to_string())
            .ok()
            .flatten()
        {
            account_pubkeys
                .iter()
                .filter_map(|pk_str| {
                    let pk = Pubkey::from_str(pk_str).ok()?;
                    self.token_accounts
                        .get(pk_str)
                        .ok()
                        .flatten()
                        .map(|ta| (pk, ta))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn subscribe_for_slot_updates(&mut self) -> Receiver<SlotInfo> {
        let (tx, rx) = unbounded();
        self.slot_subscriptions.push(tx);
        rx
    }

    pub fn notify_slot_subscribers(&mut self, slot: Slot, parent: Slot, root: Slot) {
        self.slot_subscriptions
            .retain(|tx| tx.send(SlotInfo { slot, parent, root }).is_ok());
    }

    pub fn write_simulated_profile_result(
        &mut self,
        uuid: Uuid,
        tag: Option<String>,
        profile_result: KeyedProfileResult,
    ) -> SurfpoolResult<()> {
        self.simulated_transaction_profiles
            .insert(uuid, profile_result);

        let tag = tag.unwrap_or_else(|| uuid.to_string());
        let mut tags = self
            .profile_tag_map
            .get(&tag)
            .ok()
            .flatten()
            .unwrap_or_default();
        tags.push(UuidOrSignature::Uuid(uuid));
        self.profile_tag_map.store(tag, tags)?;
        Ok(())
    }

    pub fn write_executed_profile_result(
        &mut self,
        signature: Signature,
        profile_result: KeyedProfileResult,
    ) -> SurfpoolResult<()> {
        self.executed_transaction_profiles
            .insert(signature, profile_result);
        let tag = signature.to_string();
        let mut tags = self
            .profile_tag_map
            .get(&tag)
            .ok()
            .flatten()
            .unwrap_or_default();
        tags.push(UuidOrSignature::Signature(signature));
        self.profile_tag_map.store(tag, tags)?;
        Ok(())
    }

    pub fn subscribe_for_logs_updates(
        &mut self,
        commitment_level: &CommitmentLevel,
        filter: &RpcTransactionLogsFilter,
    ) -> Receiver<(Slot, RpcLogsResponse)> {
        let (tx, rx) = unbounded();
        self.logs_subscriptions
            .push((*commitment_level, filter.clone(), tx));
        rx
    }

    pub fn notify_logs_subscribers(
        &mut self,
        signature: &Signature,
        err: Option<TransactionError>,
        logs: Vec<String>,
        commitment_level: CommitmentLevel,
    ) {
        for (expected_level, filter, tx) in self.logs_subscriptions.iter() {
            if !expected_level.eq(&commitment_level) {
                continue; // Skip if commitment level is not expected
            }

            let should_notify = match filter {
                RpcTransactionLogsFilter::All | RpcTransactionLogsFilter::AllWithVotes => true,

                RpcTransactionLogsFilter::Mentions(mentioned_accounts) => {
                    // Get the tx accounts including loaded addresses
                    let transaction_accounts =
                        if let Some(SurfnetTransactionStatus::Processed(tx_data)) =
                            self.transactions.get(&signature.to_string()).ok().flatten()
                        {
                            let (tx_meta, _) = tx_data.as_ref();
                            let mut accounts = match &tx_meta.transaction.message {
                                VersionedMessage::Legacy(msg) => msg.account_keys.clone(),
                                VersionedMessage::V0(msg) => msg.account_keys.clone(),
                            };

                            accounts.extend(&tx_meta.meta.loaded_addresses.writable);
                            accounts.extend(&tx_meta.meta.loaded_addresses.readonly);
                            Some(accounts)
                        } else {
                            None
                        };

                    let Some(accounts) = transaction_accounts else {
                        continue;
                    };

                    mentioned_accounts.iter().any(|filtered_acc| {
                        if let Ok(filtered_pubkey) = Pubkey::from_str(&filtered_acc) {
                            accounts.contains(&filtered_pubkey)
                        } else {
                            false
                        }
                    })
                }
            };

            if should_notify {
                let message = RpcLogsResponse {
                    signature: signature.to_string(),
                    err: err.clone().map(|e| e.into()),
                    logs: logs.clone(),
                };
                let _ = tx.send((self.get_latest_absolute_slot(), message));
            }
        }
    }

    pub fn register_idl(&mut self, idl: Idl, slot: Option<Slot>) -> SurfpoolResult<()> {
        let slot = slot.unwrap_or(self.latest_epoch_info.absolute_slot);
        let program_id = Pubkey::from_str_const(&idl.address);
        let program_id_str = program_id.to_string();
        let mut idl_versions = self
            .registered_idls
            .get(&program_id_str)
            .ok()
            .flatten()
            .unwrap_or_default();
        idl_versions.push(VersionedIdl(slot, idl));
        // Sort by slot descending so the latest IDL is first
        idl_versions.sort_by(|a, b| b.0.cmp(&a.0));
        self.registered_idls.store(program_id_str, idl_versions)?;
        Ok(())
    }

    fn encode_ui_account_profile_state(
        &self,
        pubkey: &Pubkey,
        account_profile_state: AccountProfileState,
        encoding: &UiAccountEncoding,
    ) -> UiAccountProfileState {
        let additional_data = self.get_additional_data(pubkey, None);

        match account_profile_state {
            AccountProfileState::Readonly => UiAccountProfileState::Readonly,
            AccountProfileState::Writable(account_change) => {
                let change = match account_change {
                    AccountChange::Create(account) => UiAccountChange::Create(
                        self.encode_ui_account(pubkey, &account, *encoding, additional_data, None),
                    ),
                    AccountChange::Update(account_before, account_after) => {
                        UiAccountChange::Update(
                            self.encode_ui_account(
                                pubkey,
                                &account_before,
                                *encoding,
                                additional_data,
                                None,
                            ),
                            self.encode_ui_account(
                                pubkey,
                                &account_after,
                                *encoding,
                                additional_data,
                                None,
                            ),
                        )
                    }
                    AccountChange::Delete(account) => UiAccountChange::Delete(
                        self.encode_ui_account(pubkey, &account, *encoding, additional_data, None),
                    ),
                    AccountChange::Unchanged(account) => {
                        UiAccountChange::Unchanged(account.map(|account| {
                            self.encode_ui_account(
                                pubkey,
                                &account,
                                *encoding,
                                additional_data,
                                None,
                            )
                        }))
                    }
                };
                UiAccountProfileState::Writable(change)
            }
        }
    }

    fn encode_ui_profile_result(
        &self,
        profile_result: ProfileResult,
        readonly_accounts: &[Pubkey],
        encoding: &UiAccountEncoding,
    ) -> UiProfileResult {
        let ProfileResult {
            pre_execution_capture,
            post_execution_capture,
            compute_units_consumed,
            log_messages,
            error_message,
        } = profile_result;

        let account_states = pre_execution_capture
            .into_iter()
            .zip(post_execution_capture)
            .map(|((pubkey, pre_account), (_, post_account))| {
                // if pubkey != post {
                //     panic!(
                //         "Pre-execution pubkey {} does not match post-execution pubkey {}",
                //         pubkey, post
                //     );
                // }
                let state =
                    AccountProfileState::new(pubkey, pre_account, post_account, readonly_accounts);
                (
                    pubkey,
                    self.encode_ui_account_profile_state(&pubkey, state, encoding),
                )
            })
            .collect::<IndexMap<Pubkey, UiAccountProfileState>>();

        UiProfileResult {
            account_states,
            compute_units_consumed,
            log_messages,
            error_message,
        }
    }

    pub fn encode_ui_keyed_profile_result(
        &self,
        keyed_profile_result: KeyedProfileResult,
        config: &RpcProfileResultConfig,
    ) -> UiKeyedProfileResult {
        let KeyedProfileResult {
            slot,
            key,
            instruction_profiles,
            transaction_profile,
            readonly_account_states,
        } = keyed_profile_result;

        let encoding = config.encoding.unwrap_or(UiAccountEncoding::JsonParsed);

        let readonly_accounts = readonly_account_states.keys().cloned().collect::<Vec<_>>();

        let default = RpcProfileDepth::default();
        let instruction_profiles = match *config.depth.as_ref().unwrap_or(&default) {
            RpcProfileDepth::Transaction => None,
            RpcProfileDepth::Instruction => instruction_profiles.map(|instruction_profiles| {
                instruction_profiles
                    .into_iter()
                    .map(|p| self.encode_ui_profile_result(p, &readonly_accounts, &encoding))
                    .collect()
            }),
        };

        let transaction_profile =
            self.encode_ui_profile_result(transaction_profile, &readonly_accounts, &encoding);

        let readonly_account_states = readonly_account_states
            .into_iter()
            .map(|(pubkey, account)| {
                let account = self.encode_ui_account(&pubkey, &account, encoding, None, None);
                (pubkey, account)
            })
            .collect();

        UiKeyedProfileResult {
            slot,
            key,
            instruction_profiles,
            transaction_profile,
            readonly_account_states,
        }
    }

    pub fn encode_ui_account<T: ReadableAccount>(
        &self,
        pubkey: &Pubkey,
        account: &T,
        encoding: UiAccountEncoding,
        additional_data: Option<AccountAdditionalDataV3>,
        data_slice_config: Option<UiDataSliceConfig>,
    ) -> UiAccount {
        let owner_program_id = account.owner();

        let filter_slot = self.latest_epoch_info.absolute_slot; // todo: consider if we should pass in a slot
        if encoding == UiAccountEncoding::JsonParsed {
            if let Ok(Some(registered_idls)) =
                self.registered_idls.get(&owner_program_id.to_string())
            {
                // IDLs are stored sorted by slot descending (most recent first)
                let ordered_available_idls = registered_idls
                    .iter()
                    // only get IDLs that are active (their slot is before the latest slot)
                    .filter_map(|VersionedIdl(slot, idl)| {
                        if *slot <= filter_slot {
                            Some(idl)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                // if we have none in this loop, it means the only IDLs registered for this pubkey are for a
                // future slot, for some reason. if we have some, we'll try each one in this loop, starting
                // with the most recent one, to see if the account data can be parsed to the IDL type
                for idl in &ordered_available_idls {
                    // If we have a valid IDL, use it to parse the account data
                    let data = account.data();
                    let discriminator = &data[..8];
                    if let Some(matching_account) = idl
                        .accounts
                        .iter()
                        .find(|a| a.discriminator.eq(&discriminator))
                    {
                        // If we found a matching account, we can look up the type to parse the account
                        if let Some(account_type) =
                            idl.types.iter().find(|t| t.name == matching_account.name)
                        {
                            let empty_vec = vec![];
                            let idl_type_def_generics = idl
                                .types
                                .iter()
                                .find(|t| t.name == account_type.name)
                                .map(|t| &t.generics);

                            // If we found a matching account type, we can use it to parse the account data
                            let rest = data[8..].as_ref();
                            if let Ok(parsed_value) =
                                parse_bytes_to_value_with_expected_idl_type_def_ty(
                                    rest,
                                    &account_type.ty,
                                    &idl.types,
                                    &vec![],
                                    idl_type_def_generics.unwrap_or(&empty_vec),
                                )
                            {
                                return UiAccount {
                                    lamports: account.lamports(),
                                    data: UiAccountData::Json(ParsedAccount {
                                        program: idl
                                            .metadata
                                            .name
                                            .to_string()
                                            .to_case(convert_case::Case::Kebab),
                                        parsed: parsed_value
                                            .to_json(Some(&get_txtx_value_json_converters())),
                                        space: data.len() as u64,
                                    }),
                                    owner: account.owner().to_string(),
                                    executable: account.executable(),
                                    rent_epoch: account.rent_epoch(),
                                    space: Some(account.data().len() as u64),
                                };
                            }
                        }
                    }
                }
            }
        }

        // Fall back to the default encoding
        encode_ui_account(
            pubkey,
            account,
            encoding,
            additional_data,
            data_slice_config,
        )
    }

    pub fn get_account(&self, pubkey: &Pubkey) -> SurfpoolResult<Option<Account>> {
        self.inner.get_account(pubkey)
    }

    pub fn get_all_accounts(&self) -> SurfpoolResult<Vec<(Pubkey, AccountSharedData)>> {
        self.inner.get_all_accounts()
    }

    pub fn get_transaction(
        &self,
        signature: &Signature,
    ) -> SurfpoolResult<Option<SurfnetTransactionStatus>> {
        Ok(self.transactions.get(&signature.to_string())?)
    }

    pub fn start_runbook_execution(&mut self, runbook_id: String) {
        self.runbook_executions
            .push(RunbookExecutionStatusReport::new(runbook_id));
    }

    pub fn complete_runbook_execution(&mut self, runbook_id: &str, error: Option<Vec<String>>) {
        if let Some(execution) = self
            .runbook_executions
            .iter_mut()
            .find(|e| e.runbook_id.eq(runbook_id) && e.completed_at.is_none())
        {
            execution.mark_completed(error);
        }
    }

    /// Export all accounts to a JSON file suitable for test fixtures
    ///
    /// # Arguments
    /// * `encoding` - The encoding to use for account data (Base64, JsonParsed, etc.)
    ///
    /// # Returns
    /// A BTreeMap of pubkey -> AccountFixture that can be serialized to JSON.
    pub fn export_snapshot(
        &self,
        config: ExportSnapshotConfig,
    ) -> SurfpoolResult<BTreeMap<String, AccountSnapshot>> {
        let mut fixtures = BTreeMap::new();
        let encoding = if config.include_parsed_accounts.unwrap_or_default() {
            UiAccountEncoding::JsonParsed
        } else {
            UiAccountEncoding::Base64
        };
        let filter = config.filter.unwrap_or_default();
        let include_program_accounts = filter.include_program_accounts.unwrap_or(false);
        let include_accounts = filter.include_accounts.unwrap_or_default();
        let exclude_accounts = filter.exclude_accounts.unwrap_or_default();

        fn is_program_account(pubkey: &Pubkey) -> bool {
            pubkey == &bpf_loader::id()
                || pubkey == &solana_sdk_ids::bpf_loader_deprecated::id()
                || pubkey == &solana_sdk_ids::bpf_loader_upgradeable::id()
        }

        // Helper function to process an account and add it to fixtures
        let mut process_account = |pubkey: &Pubkey, account: &Account| {
            let is_include_account = include_accounts.iter().any(|k| k.eq(&pubkey.to_string()));
            let is_exclude_account = exclude_accounts.iter().any(|k| k.eq(&pubkey.to_string()));
            let is_program_account = is_program_account(&account.owner);
            if is_exclude_account
                || ((is_program_account && !include_program_accounts) && !is_include_account)
            {
                return;
            }

            // For token accounts, we need to provide the mint additional data
            let additional_data = if account.owner == spl_token_interface::id()
                || account.owner == spl_token_2022_interface::id()
            {
                if let Ok(token_account) = TokenAccount::unpack(&account.data) {
                    self.account_associated_data
                        .get(&token_account.mint())
                        .cloned()
                } else {
                    self.account_associated_data.get(pubkey).cloned()
                }
            } else {
                self.account_associated_data.get(pubkey).cloned()
            };

            let ui_account =
                self.encode_ui_account(pubkey, account, encoding, additional_data, None);

            let (base64, parsed_data) = match ui_account.data {
                UiAccountData::Json(parsed_account) => {
                    (BASE64_STANDARD.encode(account.data()), Some(parsed_account))
                }
                UiAccountData::Binary(base64, _) => (base64, None),
                UiAccountData::LegacyBinary(_) => unreachable!(),
            };

            let account_snapshot = AccountSnapshot::new(
                account.lamports,
                account.owner.to_string(),
                account.executable,
                account.rent_epoch,
                base64,
                parsed_data,
            );

            fixtures.insert(pubkey.to_string(), account_snapshot);
        };

        match &config.scope {
            ExportSnapshotScope::Network => {
                // Export all network accounts (current behavior)
                for (pubkey, account_shared_data) in self.get_all_accounts()? {
                    let account = Account::from(account_shared_data.clone());
                    process_account(&pubkey, &account);
                }
            }
            ExportSnapshotScope::PreTransaction(signature_str) => {
                // Export accounts from a specific transaction's pre-execution state
                if let Ok(signature) = Signature::from_str(signature_str) {
                    if let Some(profile) = self.executed_transaction_profiles.get(&signature) {
                        // Collect accounts from pre-execution capture only
                        // This gives us the account state BEFORE the transaction executed
                        for (pubkey, account_opt) in
                            &profile.transaction_profile.pre_execution_capture
                        {
                            if let Some(account) = account_opt {
                                process_account(pubkey, account);
                            }
                        }

                        // Also collect readonly account states (these don't change)
                        for (pubkey, account) in &profile.readonly_account_states {
                            process_account(pubkey, account);
                        }
                    }
                }
            }
        }

        Ok(fixtures)
    }

    /// Registers a scenario for execution by scheduling its overrides
    ///
    /// The `slot` parameter is the base slot from which relative override slot heights are calculated.
    /// If not provided, uses the current slot.
    pub fn register_scenario(
        &mut self,
        scenario: surfpool_types::Scenario,
        slot: Option<Slot>,
    ) -> SurfpoolResult<()> {
        // Use provided slot or current slot as the base for relative slot heights
        let base_slot = slot.unwrap_or(self.latest_epoch_info.absolute_slot);

        info!(
            "Registering scenario: {} ({}) with {} overrides at base slot {}",
            scenario.name,
            scenario.id,
            scenario.overrides.len(),
            base_slot
        );

        // Schedule overrides by adding base slot to their scenario-relative slots
        for override_instance in scenario.overrides {
            let scenario_relative_slot = override_instance.scenario_relative_slot;
            let absolute_slot = base_slot + scenario_relative_slot;

            debug!(
                "Scheduling override at absolute slot {} (base {} + relative {})",
                absolute_slot, base_slot, scenario_relative_slot
            );

            let mut slot_overrides = self
                .scheduled_overrides
                .get(&absolute_slot)
                .ok()
                .flatten()
                .unwrap_or_default();
            slot_overrides.push(override_instance);
            self.scheduled_overrides
                .store(absolute_slot, slot_overrides)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose};
    use borsh::BorshSerialize;
    // use test_log::test; // uncomment to get logs from litesvm
    use solana_account::Account;
    use solana_loader_v3_interface::get_program_data_address;
    use solana_program_pack::Pack;
    use spl_token_interface::state::{Account as TokenAccount, AccountState};
    use test_case::test_case;

    use crate::storage::tests::TestType;

    use super::*;

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_synthetic_blockhash_generation(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Test with different chain tip indices
        let test_cases = vec![0, 1, 42, 255, 1000, 0x12345678];

        for index in test_cases {
            svm.chain_tip = BlockIdentifier::new(index, "test_hash");

            // Generate the synthetic blockhash
            let new_blockhash = svm.new_blockhash();

            // Verify the blockhash string contains our expected pattern
            let blockhash_str = new_blockhash.hash.clone();
            println!("Index {} -> Blockhash: {}", index, blockhash_str);

            // The blockhash should be a valid base58 string
            assert!(!blockhash_str.is_empty());
            assert!(blockhash_str.len() > 20); // Base58 encoded 32 bytes should be around 44 chars

            // Verify it's deterministic - same index should produce same blockhash
            svm.chain_tip = BlockIdentifier::new(index, "test_hash");
            let new_blockhash2 = svm.new_blockhash();
            assert_eq!(new_blockhash.hash, new_blockhash2.hash);
        }
    }

    #[test]
    fn test_synthetic_blockhash_base58_encoding() {
        // Test the base58 encoding logic directly
        let test_index = 42u64;
        let index_hex = format!("{:08x}", test_index)
            .replace('0', "x")
            .replace('O', "x");

        let target_length = 43;
        let padding_needed = target_length - SyntheticBlockhash::PREFIX.len() - index_hex.len();
        let padding = "x".repeat(padding_needed.max(0));
        let target_string = format!("{}{}{}", SyntheticBlockhash::PREFIX, padding, index_hex);

        println!("Target string: {}", target_string);

        // Verify the string is valid base58
        let decoded_bytes = bs58::decode(&target_string).into_vec();
        assert!(decoded_bytes.is_ok(), "String should be valid base58");

        let bytes = decoded_bytes.unwrap();
        assert!(bytes.len() <= 32, "Decoded bytes should fit in 32 bytes");

        // Test that we can create a hash from these bytes
        let mut blockhash_bytes = [0u8; 32];
        blockhash_bytes[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
        let hash = Hash::new_from_array(blockhash_bytes);

        // Verify the hash can be converted back to string
        let hash_str = hash.to_string();
        assert!(!hash_str.is_empty());
        println!("Generated hash: {}", hash_str);
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_blockhash_consistency_across_calls(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Set a specific chain tip
        svm.chain_tip = BlockIdentifier::new(123, "initial_hash");

        // Generate multiple blockhashes and verify they're consistent
        let mut previous_hash: Option<BlockIdentifier> = None;
        for i in 0..5 {
            let new_blockhash = svm.new_blockhash();
            println!(
                "Call {}: index={}, hash={}",
                i, new_blockhash.index, new_blockhash.hash
            );

            if let Some(prev) = previous_hash {
                // Each call should increment the index
                assert_eq!(new_blockhash.index, prev.index + 1);
                // But the hash should be different (since index changed)
                assert_ne!(new_blockhash.hash, prev.hash);
            } else {
                // First call should increment from the initial chain tip
                assert_eq!(new_blockhash.index, svm.chain_tip.index + 1);
            }

            previous_hash = Some(new_blockhash.clone());
            // Update the chain tip for the next iteration
            svm.chain_tip = new_blockhash;
        }
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_token_account_indexing(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

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
            owner: spl_token_interface::id(),
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&token_account_pubkey, account).unwrap();

        // test all indexes were created correctly
        assert_eq!(svm.token_accounts.keys().unwrap().len(), 1);

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

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_account_update_removes_old_indexes(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

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
            owner: spl_token_interface::id(),
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
            owner: spl_token_interface::id(),
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

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_non_token_accounts_not_indexed(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

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
        assert_eq!(svm.token_accounts.keys().unwrap().len(), 0);
        assert_eq!(svm.token_accounts_by_owner.keys().unwrap().len(), 0);
        assert_eq!(svm.token_accounts_by_delegate.keys().unwrap().len(), 0);
        assert_eq!(svm.token_accounts_by_mint.keys().unwrap().len(), 0);
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
                    assert_eq!(
                        svm.get_account(&pubkey).unwrap().as_ref(),
                        Some(expected_account)
                    );
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

    fn _expect_error_event(events_rx: &Receiver<SimnetEvent>, expected_error: &str) -> bool {
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
            data: bincode::serialize(
                &solana_loader_v3_interface::state::UpgradeableLoaderState::Program {
                    programdata_address: program_data_address,
                },
            )
            .unwrap(),
            owner: solana_sdk_ids::bpf_loader_upgradeable::ID,
            executable: true,
            rent_epoch: 10000000000000,
        };

        let mut bin = include_bytes!("../tests/assets/metaplex_program.bin").to_vec();
        let mut data = bincode::serialize(
            &solana_loader_v3_interface::state::UpgradeableLoaderState::ProgramData {
                slot: 0,
                upgrade_authority_address: Some(Pubkey::new_unique()),
            },
        )
        .unwrap();
        data.append(&mut bin); // push our binary after the state data
        let program_data_account = Account {
            lamports: 10000000000000,
            data,
            owner: solana_sdk_ids::bpf_loader_upgradeable::ID,
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

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_inserting_account_updates(test_type: TestType) {
        let (mut svm, events_rx, _geyser_rx) = test_type.initialize_svm();

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
            let index_before = svm.get_all_accounts().unwrap();
            let empty_update = GetAccountResult::None(pubkey);
            svm.write_account_update(empty_update);
            assert_eq!(svm.get_all_accounts().unwrap(), index_before);
        }

        // GetAccountResult::FoundAccount with `DoUpdateSvm` flag to false should be a noop
        {
            let index_before = svm.get_all_accounts().unwrap();
            let found_update = GetAccountResult::FoundAccount(pubkey, account.clone(), false);
            svm.write_account_update(found_update);
            assert_eq!(svm.get_all_accounts().unwrap(), index_before);
        }

        // GetAccountResult::FoundAccount with `DoUpdateSvm` flag to true should update the account
        {
            let index_before = svm.get_all_accounts().unwrap();
            let found_update = GetAccountResult::FoundAccount(pubkey, account.clone(), true);
            svm.write_account_update(found_update);
            assert_eq!(
                svm.get_all_accounts().unwrap().len(),
                index_before.len() + 1
            );
            if !expect_account_update_event(&events_rx, &svm, &pubkey, &account) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundAccount update"
                );
            }
        }

        // GetAccountResult::FoundProgramAccount with no program account inserts a default programdata account
        {
            let (program_address, program_account, program_data_address, _) =
                create_program_accounts();

            let mut data = bincode::serialize(
                &solana_loader_v3_interface::state::UpgradeableLoaderState::ProgramData {
                    slot: svm.get_latest_absolute_slot(),
                    upgrade_authority_address: Some(system_program::id()),
                },
            )
            .unwrap();

            let mut bin = include_bytes!("../tests/assets/minimum_program.so").to_vec();
            data.append(&mut bin); // push our binary after the state data
            let lamports = svm.inner.minimum_balance_for_rent_exemption(data.len());
            let default_program_data_account = Account {
                lamports,
                data,
                owner: solana_sdk_ids::bpf_loader_upgradeable::ID,
                executable: false,
                rent_epoch: 0,
            };

            let index_before = svm.get_all_accounts().unwrap();
            let found_program_account_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, None),
            );
            svm.write_account_update(found_program_account_update);

            if !expect_account_update_event(
                &events_rx,
                &svm,
                &program_data_address,
                &default_program_data_account,
            ) {
                panic!(
                    "Expected account update event not received after inserting default program data account"
                );
            }

            if !expect_account_update_event(&events_rx, &svm, &program_address, &program_account) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundProgramAccount update for program pubkey"
                );
            }
            assert_eq!(
                svm.get_all_accounts().unwrap().len(),
                index_before.len() + 2
            );
        }

        // GetAccountResult::FoundProgramAccount with program account + program data account inserts two accounts
        {
            let (program_address, program_account, program_data_address, program_data_account) =
                create_program_accounts();

            let index_before = svm.get_all_accounts().unwrap();
            let found_program_account_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, Some(program_data_account.clone())),
            );
            svm.write_account_update(found_program_account_update);
            assert_eq!(
                svm.get_all_accounts().unwrap().len(),
                index_before.len() + 2
            );
            if !expect_account_update_event(
                &events_rx,
                &svm,
                &program_data_address,
                &program_data_account,
            ) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundProgramAccount update for program data pubkey"
                );
            }

            if !expect_account_update_event(&events_rx, &svm, &program_address, &program_account) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundProgramAccount update for program pubkey"
                );
            }
        }

        // If we insert the program data account ahead of time, then have a GetAccountResult::FoundProgramAccount with just the program data account,
        // we should get one insert
        {
            let (program_address, program_account, program_data_address, program_data_account) =
                create_program_accounts();

            let index_before = svm.get_all_accounts().unwrap();
            let found_update = GetAccountResult::FoundAccount(
                program_data_address,
                program_data_account.clone(),
                true,
            );
            svm.write_account_update(found_update);
            assert_eq!(
                svm.get_all_accounts().unwrap().len(),
                index_before.len() + 1
            );
            if !expect_account_update_event(
                &events_rx,
                &svm,
                &program_data_address,
                &program_data_account,
            ) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundAccount update"
                );
            }

            let index_before = svm.get_all_accounts().unwrap();
            let program_account_found_update = GetAccountResult::FoundProgramAccount(
                (program_address, program_account.clone()),
                (program_data_address, None),
            );
            svm.write_account_update(program_account_found_update);
            assert_eq!(
                svm.get_all_accounts().unwrap().len(),
                index_before.len() + 1
            );
            if !expect_account_update_event(&events_rx, &svm, &program_address, &program_account) {
                panic!(
                    "Expected account update event not received after GetAccountResult::FoundAccount update"
                );
            }
        }
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_encode_ui_account(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        let idl_v1: Idl =
            serde_json::from_slice(&include_bytes!("../tests/assets/idl_v1.json").to_vec())
                .unwrap();

        svm.register_idl(idl_v1.clone(), Some(0)).unwrap();

        let account_pubkey = Pubkey::new_unique();

        #[derive(borsh::BorshSerialize)]
        pub struct CustomAccount {
            pub my_custom_data: u64,
            pub another_field: String,
            pub bool: bool,
            pub pubkey: Pubkey,
        }

        // Account data not matching IDL schema should use default encoding
        {
            let account_data = vec![0; 100];
            let base64_data = general_purpose::STANDARD.encode(&account_data);
            let expected_data = UiAccountData::Binary(base64_data, UiAccountEncoding::Base64);
            let account = Account {
                lamports: 1000,
                data: account_data,
                owner: idl_v1.address.parse().unwrap(),
                executable: false,
                rent_epoch: 0,
            };

            let ui_account = svm.encode_ui_account(
                &account_pubkey,
                &account,
                UiAccountEncoding::JsonParsed,
                None,
                None,
            );
            let expected_account = UiAccount {
                lamports: 1000,
                data: expected_data,
                owner: idl_v1.address.clone(),
                executable: false,
                rent_epoch: 0,
                space: Some(account.data.len() as u64),
            };
            assert_eq!(ui_account, expected_account);
        }

        // valid account data matching IDL schema should be parsed
        {
            let mut account_data = idl_v1.accounts[0].discriminator.clone();
            let pubkey = Pubkey::new_unique();
            CustomAccount {
                my_custom_data: 42,
                another_field: "test".to_string(),
                bool: true,
                pubkey,
            }
            .serialize(&mut account_data)
            .unwrap();

            let account = Account {
                lamports: 1000,
                data: account_data,
                owner: idl_v1.address.parse().unwrap(),
                executable: false,
                rent_epoch: 0,
            };

            let ui_account = svm.encode_ui_account(
                &account_pubkey,
                &account,
                UiAccountEncoding::JsonParsed,
                None,
                None,
            );
            let expected_account = UiAccount {
                lamports: 1000,
                data: UiAccountData::Json(ParsedAccount {
                    program: format!("{}", idl_v1.metadata.name).to_case(convert_case::Case::Kebab),
                    parsed: serde_json::json!({
                        "my_custom_data": 42,
                        "another_field": "test",
                        "bool": true,
                        "pubkey": pubkey.to_string(),
                    }),
                    space: account.data.len() as u64,
                }),
                owner: idl_v1.address.clone(),
                executable: false,
                rent_epoch: 0,
                space: Some(account.data.len() as u64),
            };
            assert_eq!(ui_account, expected_account);
        }

        let idl_v2: Idl =
            serde_json::from_slice(&include_bytes!("../tests/assets/idl_v2.json").to_vec())
                .unwrap();

        svm.register_idl(idl_v2.clone(), Some(100)).unwrap();

        // even though we have a new IDL that is more recent, we should be able to match with the old IDL
        {
            let mut account_data = idl_v1.accounts[0].discriminator.clone();
            let pubkey = Pubkey::new_unique();
            CustomAccount {
                my_custom_data: 42,
                another_field: "test".to_string(),
                bool: true,
                pubkey,
            }
            .serialize(&mut account_data)
            .unwrap();

            let account = Account {
                lamports: 1000,
                data: account_data,
                owner: idl_v1.address.parse().unwrap(),
                executable: false,
                rent_epoch: 0,
            };

            let ui_account = svm.encode_ui_account(
                &account_pubkey,
                &account,
                UiAccountEncoding::JsonParsed,
                None,
                None,
            );
            let expected_account = UiAccount {
                lamports: 1000,
                data: UiAccountData::Json(ParsedAccount {
                    program: format!("{}", idl_v1.metadata.name).to_case(convert_case::Case::Kebab),
                    parsed: serde_json::json!({
                        "my_custom_data": 42,
                        "another_field": "test",
                        "bool": true,
                        "pubkey": pubkey.to_string(),
                    }),
                    space: account.data.len() as u64,
                }),
                owner: idl_v1.address.clone(),
                executable: false,
                rent_epoch: 0,
                space: Some(account.data.len() as u64),
            };
            assert_eq!(ui_account, expected_account);
        }

        // valid account data matching IDL v2 schema should be parsed, if svm slot reaches IDL registration slot
        {
            // use the v2 shape of the custom account
            #[derive(borsh::BorshSerialize)]
            pub struct CustomAccount {
                pub my_custom_data: u64,
                pub another_field: String,
                pub pubkey: Pubkey,
            }
            let mut account_data = idl_v1.accounts[0].discriminator.clone();
            let pubkey = Pubkey::new_unique();
            CustomAccount {
                my_custom_data: 42,
                another_field: "test".to_string(),
                pubkey,
            }
            .serialize(&mut account_data)
            .unwrap();

            let account = Account {
                lamports: 1000,
                data: account_data.clone(),
                owner: idl_v1.address.parse().unwrap(),
                executable: false,
                rent_epoch: 0,
            };

            let ui_account = svm.encode_ui_account(
                &account_pubkey,
                &account,
                UiAccountEncoding::JsonParsed,
                None,
                None,
            );
            let base64_data = general_purpose::STANDARD.encode(&account_data);
            let expected_data = UiAccountData::Binary(base64_data, UiAccountEncoding::Base64);
            let expected_account = UiAccount {
                lamports: 1000,
                data: expected_data,
                owner: idl_v1.address.clone(),
                executable: false,
                rent_epoch: 0,
                space: Some(account.data.len() as u64),
            };
            assert_eq!(ui_account, expected_account);

            svm.latest_epoch_info.absolute_slot = 100; // simulate reaching the slot where IDL v2 was registered

            let ui_account = svm.encode_ui_account(
                &account_pubkey,
                &account,
                UiAccountEncoding::JsonParsed,
                None,
                None,
            );
            let expected_account = UiAccount {
                lamports: 1000,
                data: UiAccountData::Json(ParsedAccount {
                    program: format!("{}", idl_v1.metadata.name).to_case(convert_case::Case::Kebab),
                    parsed: serde_json::json!({
                        "my_custom_data": 42,
                        "another_field": "test",
                        "pubkey": pubkey.to_string(),
                    }),
                    space: account.data.len() as u64,
                }),
                owner: idl_v1.address.clone(),
                executable: false,
                rent_epoch: 0,
                space: Some(account.data.len() as u64),
            };
            assert_eq!(ui_account, expected_account);
        }
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_profiling_map_capacity_default(test_type: TestType) {
        let (svm, _events_rx, _geyser_rx) = test_type.initialize_svm();
        assert_eq!(
            svm.executed_transaction_profiles.capacity(),
            DEFAULT_PROFILING_MAP_CAPACITY
        );
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_profiling_map_capacity_set(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();
        svm.set_profiling_map_capacity(10);
        assert_eq!(svm.executed_transaction_profiles.capacity(), 10);
    }

    // Feature configuration tests

    #[test]
    fn test_feature_to_id_all_features_have_mapping() {
        // Ensure every SvmFeature variant has a valid mapping to a feature ID
        for feature in SvmFeature::all() {
            let id = SurfnetSvm::feature_to_id(&feature);
            assert!(
                id.is_some(),
                "Feature {:?} should have a valid ID mapping",
                feature
            );
        }
    }

    #[test]
    fn test_feature_to_id_returns_valid_pubkeys() {
        // Spot check a few known features
        let loader_v4_id = SurfnetSvm::feature_to_id(&SvmFeature::EnableLoaderV4);
        assert!(loader_v4_id.is_some());
        assert_ne!(loader_v4_id.unwrap(), Pubkey::default());

        let disable_fees_id = SurfnetSvm::feature_to_id(&SvmFeature::DisableFeesSysvar);
        assert!(disable_fees_id.is_some());
        assert_ne!(disable_fees_id.unwrap(), Pubkey::default());

        // Different features should have different IDs
        assert_ne!(loader_v4_id, disable_fees_id);
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_empty(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();
        let config = SvmFeatureConfig::new();

        // Should not panic with empty config
        svm.apply_feature_config(&config);
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_enable_feature(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Disable a feature first
        let feature_id = enable_loader_v4::id();
        svm.feature_set.deactivate(&feature_id);
        assert!(!svm.feature_set.is_active(&feature_id));

        // Now enable it via config
        let config = SvmFeatureConfig::new().enable(SvmFeature::EnableLoaderV4);
        svm.apply_feature_config(&config);

        assert!(svm.feature_set.is_active(&feature_id));
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_disable_feature(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Feature should be active by default (all_enabled)
        let feature_id = disable_fees_sysvar::id();
        assert!(svm.feature_set.is_active(&feature_id));

        // Now disable it via config
        let config = SvmFeatureConfig::new().disable(SvmFeature::DisableFeesSysvar);
        svm.apply_feature_config(&config);

        assert!(!svm.feature_set.is_active(&feature_id));
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_mainnet_defaults(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();
        let config = SvmFeatureConfig::default_mainnet_features();

        svm.apply_feature_config(&config);

        // Features disabled on mainnet should now be inactive
        assert!(!svm.feature_set.is_active(&enable_loader_v4::id()));
        assert!(
            !svm.feature_set
                .is_active(&enable_extend_program_checked::id())
        );
        assert!(!svm.feature_set.is_active(&blake3_syscall_enabled::id()));
        assert!(
            !svm.feature_set
                .is_active(&enable_sbpf_v1_deployment_and_execution::id())
        );
        assert!(
            !svm.feature_set
                .is_active(&formalize_loaded_transaction_data_size::id())
        );
        assert!(
            !svm.feature_set
                .is_active(&move_precompile_verification_to_svm::id())
        );

        // Features active on mainnet should still be active
        assert!(svm.feature_set.is_active(&disable_fees_sysvar::id()));
        assert!(svm.feature_set.is_active(&curve25519_syscall_enabled::id()));
        assert!(
            svm.feature_set
                .is_active(&enable_sbpf_v2_deployment_and_execution::id())
        );
        assert!(
            svm.feature_set
                .is_active(&enable_sbpf_v3_deployment_and_execution::id())
        );
        assert!(
            svm.feature_set
                .is_active(&raise_cpi_nesting_limit_to_8::id())
        );
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_mainnet_with_override(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Start with mainnet defaults, but enable loader v4
        let config =
            SvmFeatureConfig::default_mainnet_features().enable(SvmFeature::EnableLoaderV4);

        svm.apply_feature_config(&config);

        // Loader v4 should be enabled despite mainnet defaults
        assert!(svm.feature_set.is_active(&enable_loader_v4::id()));

        // Other mainnet-disabled features should still be disabled
        assert!(!svm.feature_set.is_active(&blake3_syscall_enabled::id()));
        assert!(
            !svm.feature_set
                .is_active(&enable_extend_program_checked::id())
        );
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_multiple_changes(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .enable(SvmFeature::EnableSbpfV2DeploymentAndExecution)
            .disable(SvmFeature::DisableFeesSysvar)
            .disable(SvmFeature::Blake3SyscallEnabled);

        svm.apply_feature_config(&config);

        assert!(svm.feature_set.is_active(&enable_loader_v4::id()));
        assert!(
            svm.feature_set
                .is_active(&enable_sbpf_v2_deployment_and_execution::id())
        );
        assert!(!svm.feature_set.is_active(&disable_fees_sysvar::id()));
        assert!(!svm.feature_set.is_active(&blake3_syscall_enabled::id()));
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_preserves_native_mint(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        // Native mint should exist before
        assert!(
            svm.inner
                .get_account(&spl_token_interface::native_mint::ID)
                .unwrap()
                .is_some()
        );

        let config = SvmFeatureConfig::new().disable(SvmFeature::DisableFeesSysvar);
        svm.apply_feature_config(&config);

        // Native mint should still exist after (re-added in apply_feature_config)
        assert!(
            svm.inner
                .get_account(&spl_token_interface::native_mint::ID)
                .unwrap()
                .is_some()
        );
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_apply_feature_config_idempotent(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .disable(SvmFeature::DisableFeesSysvar);

        // Apply twice
        svm.apply_feature_config(&config);
        svm.apply_feature_config(&config);

        // State should be the same
        assert!(svm.feature_set.is_active(&enable_loader_v4::id()));
        assert!(!svm.feature_set.is_active(&disable_fees_sysvar::id()));
    }

    // Garbage collection tests

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_garbage_collected_account_tracking(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        let owner = Pubkey::new_unique();
        let account_pubkey = Pubkey::new_unique();

        let account = Account {
            lamports: 1000000,
            data: vec![1, 2, 3, 4, 5],
            owner,
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&account_pubkey, account.clone()).unwrap();

        assert!(svm.get_account(&account_pubkey).unwrap().is_some());
        assert!(!svm.closed_accounts.contains(&account_pubkey));
        assert_eq!(svm.get_account_owned_by(&owner).unwrap().len(), 1);

        let empty_account = Account::default();
        svm.update_account_registries(&account_pubkey, &empty_account)
            .unwrap();

        assert!(svm.closed_accounts.contains(&account_pubkey));

        assert_eq!(svm.get_account_owned_by(&owner).unwrap().len(), 0);

        let owned_accounts = svm.get_account_owned_by(&owner).unwrap();
        assert!(!owned_accounts.iter().any(|(pk, _)| *pk == account_pubkey));
    }

    #[test_case(TestType::sqlite(); "with on-disk sqlite db")]
    #[test_case(TestType::in_memory(); "with in-memory sqlite db")]
    #[test_case(TestType::no_db(); "with no db")]
    #[cfg_attr(feature = "postgres", test_case(TestType::postgres(); "with postgres db"))]
    fn test_garbage_collected_token_account_cleanup(test_type: TestType) {
        let (mut svm, _events_rx, _geyser_rx) = test_type.initialize_svm();

        let token_owner = Pubkey::new_unique();
        let delegate = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let token_account_pubkey = Pubkey::new_unique();

        let mut token_account_data = [0u8; TokenAccount::LEN];
        let token_account = TokenAccount {
            mint,
            owner: token_owner,
            amount: 1000,
            delegate: COption::Some(delegate),
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 500,
            close_authority: COption::None,
        };
        token_account.pack_into_slice(&mut token_account_data);

        let account = Account {
            lamports: 2000000,
            data: token_account_data.to_vec(),
            owner: spl_token_interface::id(),
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&token_account_pubkey, account).unwrap();

        assert_eq!(
            svm.get_token_accounts_by_owner(&token_owner).unwrap().len(),
            1
        );
        assert_eq!(svm.get_token_accounts_by_delegate(&delegate).len(), 1);
        assert!(!svm.closed_accounts.contains(&token_account_pubkey));

        let empty_account = Account::default();
        svm.update_account_registries(&token_account_pubkey, &empty_account)
            .unwrap();

        assert!(svm.closed_accounts.contains(&token_account_pubkey));

        assert_eq!(
            svm.get_token_accounts_by_owner(&token_owner).unwrap().len(),
            0
        );
        assert_eq!(svm.get_token_accounts_by_delegate(&delegate).len(), 0);
        assert!(
            svm.token_accounts
                .get(&token_account_pubkey.to_string())
                .unwrap()
                .is_none()
        );
    }
}
