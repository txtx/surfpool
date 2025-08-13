use std::{collections::HashMap, sync::Mutex};

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
    ReplicaEntryInfoVersions, ReplicaTransactionInfoV2, ReplicaTransactionInfoVersions,
    Result as PluginResult, SlotStatus,
};
use ipc_channel::ipc::IpcSender;
use solana_clock::Slot;
use surfpool_types::{DataIndexingCommand, SubgraphPluginConfig};
use txtx_addon_kit::types::types::Value;
use txtx_addon_network_svm::Pubkey;
use txtx_addon_network_svm_types::subgraph::{
    IndexedSubgraphSourceType, PdaSubgraphSource, SubgraphRequest,
};
use uuid::Uuid;

#[derive(Default, Debug)]
pub struct SurfpoolSubgraphPlugin {
    pub uuid: Uuid,
    subgraph_indexing_event_tx: Mutex<Option<IpcSender<DataIndexingCommand>>>,
    subgraph_request: Option<SubgraphRequest>,
    pda_mappings: Mutex<PdaMapping>,
    account_update_purgatory: Mutex<AccountPurgatory>,
}

impl GeyserPlugin for SurfpoolSubgraphPlugin {
    fn name(&self) -> &'static str {
        "surfpool-subgraph"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        let config = serde_json::from_str::<SubgraphPluginConfig>(config_file)
            .map_err(|e| GeyserPluginError::ConfigFileReadError { msg: e.to_string() })?;
        let oneshot_tx = IpcSender::connect(config.ipc_token).map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to connect IPC sender: {}", e).into())
        })?;
        let (tx, rx) = ipc_channel::ipc::channel().map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to create IPC channel: {}", e).into())
        })?;
        let _ = tx.send(DataIndexingCommand::ProcessCollection(config.uuid));
        let _ = oneshot_tx.send(rx);
        self.uuid = config.uuid;
        self.subgraph_indexing_event_tx = Mutex::new(Some(tx));
        self.subgraph_request = Some(config.subgraph_request);
        Ok(())
    }

    fn on_unload(&mut self) {}

    fn notify_end_of_startup(&self) -> PluginResult<()> {
        Ok(())
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions,
        slot: Slot,
        _is_startup: bool,
    ) -> PluginResult<()> {
        match account {
            ReplicaAccountInfoVersions::V0_0_1(_info) => {
                return Err(GeyserPluginError::Custom(
                    "ReplicaAccountInfoVersions::V0_0_1 is not supported, skipping account update"
                        .into(),
                ));
            }
            ReplicaAccountInfoVersions::V0_0_2(_info) => {
                return Err(GeyserPluginError::Custom(
                    "ReplicaAccountInfoVersions::V0_0_2 is not supported, skipping account update"
                        .into(),
                ));
            }
            ReplicaAccountInfoVersions::V0_0_3(info) => {
                if info.txn.is_some() {
                    return Ok(()); // We only care about account updates _without_ a transaction, indicating it's a post-block update rather than post-transaction
                }
            }
        }
        let Ok(tx) = self.subgraph_indexing_event_tx.lock() else {
            return Err(GeyserPluginError::Custom(
                "Failed to lock subgraph indexing sender".into(),
            ));
        };
        let tx = tx.as_ref().ok_or_else(|| {
            GeyserPluginError::Custom("Failed to lock subgraph indexing sender".into())
        })?;

        let Some(ref subgraph_request) = self.subgraph_request else {
            return Ok(());
        };
        let mut entries = vec![];

        match account {
            ReplicaAccountInfoVersions::V0_0_3(info) => {
                let pubkey_bytes: [u8; 32] =
                    info.pubkey.try_into().expect("pubkey must be 32 bytes");
                let pubkey = Pubkey::new_from_array(pubkey_bytes);
                let owner_bytes: [u8; 32] = info
                    .owner
                    .try_into()
                    .expect("owner pubkey must be 32 bytes");
                let owner = Pubkey::new_from_array(owner_bytes);

                probe_account(
                    &self.account_update_purgatory,
                    &self.pda_mappings,
                    subgraph_request,
                    pubkey,
                    owner,
                    info.data.to_vec(),
                    slot,
                    info.lamports,
                    &mut entries,
                )
                .map_err(|e| GeyserPluginError::AccountsUpdateError {
                    msg: format!("{} at slot {} for account {}", e, pubkey, slot),
                })?;
            }
            _ => unreachable!(),
        };

        if !entries.is_empty() {
            let data = serde_json::to_vec(&entries).unwrap();
            let _ = tx.send(DataIndexingCommand::ProcessCollectionEntriesPack(
                self.uuid, data,
            ));
        }
        Ok(())
    }

    fn update_slot_status(
        &self,
        _slot: Slot,
        _parent: Option<u64>,
        _status: &SlotStatus,
    ) -> PluginResult<()> {
        Ok(())
    }

    fn notify_transaction(
        &self,
        transaction: ReplicaTransactionInfoVersions,
        slot: Slot,
    ) -> PluginResult<()> {
        let Ok(tx) = self.subgraph_indexing_event_tx.lock() else {
            return Err(GeyserPluginError::Custom(
                "Failed to lock subgraph indexing sender".into(),
            ));
        };
        let tx = tx.as_ref().ok_or_else(|| {
            GeyserPluginError::Custom("Failed to lock subgraph indexing sender".into())
        })?;

        let Some(ref subgraph_request) = self.subgraph_request else {
            return Ok(());
        };

        let mut entries = vec![];
        match transaction {
            ReplicaTransactionInfoVersions::V0_0_2(data) => {
                probe_transaction(
                    &self.account_update_purgatory,
                    &self.pda_mappings,
                    subgraph_request,
                    &data,
                    slot,
                    &mut entries,
                )
                .map_err(|e| GeyserPluginError::TransactionUpdateError {
                    msg: format!("{} at slot {}", e, slot),
                })?;
            }
            ReplicaTransactionInfoVersions::V0_0_1(_) => {
                return Err(GeyserPluginError::Custom(
                    "ReplicaTransactionInfoVersions::V0_0_1 is not supported, skipping transaction"
                        .into(),
                ));
            }
        };
        if !entries.is_empty() {
            let data = serde_json::to_vec(&entries).unwrap();
            let _ = tx.send(DataIndexingCommand::ProcessCollectionEntriesPack(
                self.uuid, data,
            ));
        }
        Ok(())
    }

    fn notify_entry(&self, _entry: ReplicaEntryInfoVersions) -> PluginResult<()> {
        Ok(())
    }

    fn notify_block_metadata(&self, _blockinfo: ReplicaBlockInfoVersions) -> PluginResult<()> {
        Ok(())
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn transaction_notifications_enabled(&self) -> bool {
        false
    }

    fn entry_notifications_enabled(&self) -> bool {
        false
    }
}

#[unsafe(no_mangle)]
#[allow(improper_ctypes_definitions)]
/// # Safety
///
/// This function returns the Plugin pointer as trait GeyserPlugin.
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    let plugin: Box<dyn GeyserPlugin> = Box::<SurfpoolSubgraphPlugin>::default();
    Box::into_raw(plugin)
}

pub fn probe_account(
    purgatory: &Mutex<AccountPurgatory>,
    pda_mappings: &Mutex<PdaMapping>,
    subgraph_request: &SubgraphRequest,
    pubkey: Pubkey,
    owner: Pubkey,
    data: Vec<u8>,
    slot: Slot,
    lamports: u64,
    entries: &mut Vec<HashMap<String, Value>>,
) -> Result<(), String> {
    if let Some(pda_source) = PdaMapping::get(&pda_mappings, &pubkey).unwrap() {
        pda_source.evaluate_account_update(
            &data,
            subgraph_request,
            slot,
            pubkey,
            owner,
            lamports,
            entries,
        )
    } else {
        AccountPurgatory::banish(&purgatory, &pubkey, slot, data, owner, lamports)
    }
    .map_err(|e| {
        format!(
            "Failed to evaluate account update for PDA {}: {}",
            pubkey, e
        )
    })
}

pub fn probe_transaction(
    purgatory: &Mutex<AccountPurgatory>,
    pda_mappings: &Mutex<PdaMapping>,
    subgraph_request: &SubgraphRequest,
    data: &ReplicaTransactionInfoV2<'_>,
    slot: Slot,
    entries: &mut Vec<HashMap<String, Value>>,
) -> Result<(), String> {
    let SubgraphRequest::V0(subgraph_request_v0) = subgraph_request;
    if data.is_vote {
        return Ok(());
    }

    let transaction = data.transaction;
    let account_keys = transaction.message().account_keys();
    let account_pubkeys = account_keys.iter().cloned().collect::<Vec<_>>();
    let is_program_id_match = transaction.message().instructions().iter().any(|ix| {
        ix.program_id(account_pubkeys.as_ref())
            .eq(&subgraph_request_v0.program_id)
    });
    if !is_program_id_match {
        return Ok(());
    }

    match &subgraph_request_v0.data_source {
        IndexedSubgraphSourceType::Instruction(_) => return Ok(()),
        IndexedSubgraphSourceType::Event(event_source) =>
        // Check inner instructions
        {
            if let Some(ref inner_instructions) = data.transaction_status_meta.inner_instructions {
                event_source
                    .evaluate_inner_instructions(
                        inner_instructions,
                        subgraph_request,
                        slot,
                        transaction.signature().clone(),
                        entries,
                    )
                    .map_err(|e| {
                        format!(
                            "Failed to evaluate inner instructions for event source: {}",
                            e
                        )
                    })?;
            }
        }
        IndexedSubgraphSourceType::Pda(pda_source) => {
            for instruction in transaction.message().instructions() {
                let Some(pda) = pda_source.evaluate_instruction(instruction, &account_pubkeys)
                else {
                    continue;
                };

                let Some(AccountPurgatoryData {
                    slot,
                    account_data,
                    owner,
                    lamports,
                }) = AccountPurgatory::release(purgatory, pda_mappings, pda, pda_source.clone())?
                else {
                    continue;
                };

                pda_source
                    .evaluate_account_update(
                        &account_data,
                        subgraph_request,
                        slot,
                        pda,
                        owner,
                        lamports,
                        entries,
                    )
                    .map_err(|e| {
                        format!("Failed to evaluate account update for PDA {}: {}", pda, e)
                    })?;
            }
        }
        IndexedSubgraphSourceType::TokenAccount(token_account_source) => {
            let mut already_found_token_accounts = vec![];
            for instruction in transaction.message().instructions() {
                token_account_source
                    .evaluate_instruction(
                        instruction,
                        &account_pubkeys,
                        data.transaction_status_meta,
                        slot,
                        transaction.signature().clone(),
                        subgraph_request,
                        &mut already_found_token_accounts,
                        entries,
                    )
                    .map_err(|e| {
                        format!(
                            "Failed to evaluate instruction for token account source: {}",
                            e
                        )
                    })?;
            }
        }
    }
    Ok(())
}

#[derive(Default, Debug)]
pub struct PdaMapping(pub HashMap<Pubkey, PdaSubgraphSource>);
impl PdaMapping {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert(&mut self, pubkey: Pubkey, pda_source: PdaSubgraphSource) {
        self.0.insert(pubkey, pda_source);
    }

    pub fn _get(&self, pubkey: &Pubkey) -> Option<&PdaSubgraphSource> {
        self.0.get(pubkey)
    }

    pub fn get(
        pda_mapping: &Mutex<Self>,
        pubkey: &Pubkey,
    ) -> Result<Option<PdaSubgraphSource>, String> {
        pda_mapping
            .lock()
            .map_err(|e| format!("Failed to lock PdaMapping: {}", e))
            .and_then(|mapping| Ok(mapping._get(pubkey).cloned()))
    }
}

#[derive(Default, Debug)]
pub struct AccountPurgatory(pub HashMap<Pubkey, AccountPurgatoryData>);

impl AccountPurgatory {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    fn insert(&mut self, pubkey: Pubkey, data: AccountPurgatoryData) {
        self.0.insert(pubkey, data);
    }

    fn remove(&mut self, pubkey: &Pubkey) -> Option<AccountPurgatoryData> {
        self.0.remove(pubkey)
    }

    pub fn banish(
        purgatory: &Mutex<Self>,
        pubkey: &Pubkey,
        slot: Slot,
        account_data: Vec<u8>,
        owner: Pubkey,
        lamports: u64,
    ) -> Result<(), String> {
        purgatory
            .lock()
            .map_err(|e| format!("Failed to lock AccountPurgatory: {}", e))
            .map(|mut purgatory| {
                purgatory.insert(
                    *pubkey,
                    AccountPurgatoryData {
                        slot,
                        account_data,
                        owner,
                        lamports,
                    },
                )
            })
    }

    pub fn release(
        purgatory: &Mutex<Self>,
        pda_mapping: &Mutex<PdaMapping>,
        pubkey: Pubkey,
        pda_source: PdaSubgraphSource,
    ) -> Result<Option<AccountPurgatoryData>, String> {
        pda_mapping
            .lock()
            .map_err(|e| format!("Failed to lock PdaMapping: {}", e))?
            .insert(pubkey, pda_source);

        purgatory
            .lock()
            .map_err(|e| format!("Failed to lock AccountPurgatory: {}", e))
            .map(|mut purgatory| purgatory.remove(&pubkey))
    }
}

#[derive(Default, Debug)]
pub struct AccountPurgatoryData {
    slot: Slot,
    account_data: Vec<u8>,
    owner: Pubkey,
    lamports: u64,
}
