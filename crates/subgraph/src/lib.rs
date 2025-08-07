use std::{collections::HashMap, sync::Mutex};

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions, ReplicaEntryInfoVersions,
    ReplicaTransactionInfoVersions, Result as PluginResult, SlotStatus,
};
use ipc_channel::ipc::IpcSender;
use solana_clock::Slot;
use solana_signature::Signature;
use surfpool_types::{DataIndexingCommand, SubgraphPluginConfig};
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
    pda_mappings: Mutex<HashMap<Pubkey, PdaSubgraphSource>>,
    account_update_purgatory: Mutex<HashMap<Pubkey, AccountPurgatoryData>>,
}

#[derive(Default, Debug)]
pub struct AccountPurgatoryData {
    slot: Slot,
    account_data: Vec<u8>,
    owner: Pubkey,
    lamports: u64,
    write_version: u64,
}

impl SurfpoolSubgraphPlugin {
    fn send_to_purgatory(
        &self,
        pubkey: &Pubkey,
        slot: Slot,
        account_data: Vec<u8>,
        owner: Pubkey,
        lamports: u64,
        write_version: u64,
    ) {
        self.account_update_purgatory.lock().unwrap().insert(
            *pubkey,
            AccountPurgatoryData {
                slot,
                account_data,
                owner,
                lamports,
                write_version,
            },
        );
    }

    fn release_account(
        &self,
        pubkey: Pubkey,
        pda_source: PdaSubgraphSource,
        subgraph_request: &SubgraphRequest,
        tx_signature: Signature,
        tx: &IpcSender<DataIndexingCommand>,
    ) -> Result<(), String> {
        self.pda_mappings
            .lock()
            .unwrap()
            .insert(pubkey, pda_source.clone());

        let Some(AccountPurgatoryData {
            slot,
            account_data,
            owner,
            lamports,
            write_version,
        }) = self
            .account_update_purgatory
            .lock()
            .unwrap()
            .remove(&pubkey)
        else {
            return Ok(());
        };
        let mut entries = vec![];

        pda_source
            .evaluate_account_update(
                &account_data,
                subgraph_request,
                slot,
                tx_signature,
                pubkey,
                owner,
                lamports,
                write_version,
                &mut entries,
            )
            .unwrap();

        if !entries.is_empty() {
            let data = serde_json::to_vec(&entries).unwrap();
            let _ = tx.send(DataIndexingCommand::ProcessCollectionEntriesPack(
                self.uuid, data,
            ));
        }
        Ok(())
    }
}

impl GeyserPlugin for SurfpoolSubgraphPlugin {
    fn name(&self) -> &'static str {
        "surfpool-subgraph"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        let config = serde_json::from_str::<SubgraphPluginConfig>(config_file).unwrap();
        let oneshot_tx = IpcSender::connect(config.ipc_token).unwrap();
        let (tx, rx) = ipc_channel::ipc::channel().unwrap();
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
        let Ok(tx) = self.subgraph_indexing_event_tx.lock() else {
            return Ok(());
        };
        let tx = tx.as_ref().unwrap();

        let Some(ref subgraph_request) = self.subgraph_request else {
            return Ok(());
        };
        let mut entries = vec![];

        match account {
            ReplicaAccountInfoVersions::V0_0_1(_info) => {
                unreachable!("ReplicaAccountInfoVersions::V0_0_1 is not supported")
            }
            ReplicaAccountInfoVersions::V0_0_2(_info) => {
                unreachable!("ReplicaAccountInfoVersions::V0_0_2 is not supported")
            }
            ReplicaAccountInfoVersions::V0_0_3(info) => {
                let Some(txn) = info.txn else {
                    return Ok(());
                };
                let pubkey_bytes: [u8; 32] =
                    info.pubkey.try_into().expect("pubkey must be 32 bytes");
                let pubkey = Pubkey::new_from_array(pubkey_bytes);
                let owner_bytes: [u8; 32] = info
                    .owner
                    .try_into()
                    .expect("owner pubkey must be 32 bytes");
                let owner = Pubkey::new_from_array(owner_bytes);

                if let Some(pda_source) = self.pda_mappings.lock().unwrap().get(&pubkey) {
                    pda_source
                        .evaluate_account_update(
                            info.data,
                            subgraph_request,
                            slot,
                            txn.signature().clone(),
                            pubkey,
                            owner,
                            info.lamports,
                            info.write_version,
                            &mut entries,
                        )
                        .unwrap();
                } else {
                    self.send_to_purgatory(
                        &pubkey,
                        slot,
                        info.data.to_vec(),
                        owner,
                        info.lamports,
                        info.write_version,
                    );
                }
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
            return Ok(());
        };
        let tx = tx.as_ref().unwrap();
        let Some(ref subgraph_request) = self.subgraph_request else {
            return Ok(());
        };

        let SubgraphRequest::V0(subgraph_request_v0) = subgraph_request;

        let mut entries = vec![];
        match transaction {
            ReplicaTransactionInfoVersions::V0_0_2(data) => {
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
                        if let Some(ref inner_instructions) =
                            data.transaction_status_meta.inner_instructions
                        {
                            event_source
                                .evaluate_inner_instructions(
                                    inner_instructions,
                                    subgraph_request,
                                    slot,
                                    transaction.signature().clone(),
                                    &mut entries,
                                )
                                .unwrap();
                        }
                    }
                    IndexedSubgraphSourceType::Pda(pda_source) => {
                        for instruction in transaction.message().instructions() {
                            let Some(pda) = pda_source
                                .evaluate_instruction(instruction, &account_pubkeys)
                                .unwrap()
                            else {
                                continue;
                            };

                            self.release_account(
                                pda,
                                pda_source.clone(),
                                subgraph_request,
                                transaction.signature().clone(),
                                tx,
                            )
                            .unwrap();
                        }
                    }
                }
            }
            ReplicaTransactionInfoVersions::V0_0_1(_) => {
                todo!()
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
