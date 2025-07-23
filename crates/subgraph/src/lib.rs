use std::{collections::HashMap, sync::Mutex};

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions, ReplicaEntryInfoVersions,
    ReplicaTransactionInfoVersions, Result as PluginResult, SlotStatus,
};
use ipc_channel::ipc::IpcSender;
use solana_program::clock::Slot;
use solana_signature::Signature;
use surfpool_types::{DataIndexingCommand, SubgraphPluginConfig};
use txtx_addon_kit::types::types::Value as TxtxValue;
use txtx_addon_network_svm::{
    Pubkey, codec::idl::parse_bytes_to_value_with_expected_idl_type_def_ty,
};
use txtx_addon_network_svm_types::{
    anchor::types::IdlTypeDefTy,
    subgraph::{
        IndexedSubgraphField, IndexedSubgraphSourceType, ParsablePdaData, SubgraphRequest,
        match_idl_accounts,
    },
};
use uuid::Uuid;

#[derive(Default, Debug)]
pub struct SurfpoolSubgraphPlugin {
    pub uuid: Uuid,
    subgraph_indexing_event_tx: Mutex<Option<IpcSender<DataIndexingCommand>>>,
    subgraph_request: Option<SubgraphRequest>,
    pda_mappings: Mutex<HashMap<Pubkey, ParsablePdaData>>,
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
        pubkey: &Pubkey,
        parsable_data: ParsablePdaData,
        subgraph_request: &SubgraphRequest,
        tx_signature: Signature,
        tx: &IpcSender<DataIndexingCommand>,
    ) -> Result<(), String> {
        self.pda_mappings
            .lock()
            .unwrap()
            .insert(*pubkey, parsable_data.clone());

        let Some(AccountPurgatoryData {
            slot,
            account_data,
            owner,
            lamports,
            write_version,
        }) = self.account_update_purgatory.lock().unwrap().remove(pubkey)
        else {
            return Ok(());
        };

        let entry = match SurfpoolSubgraphPlugin::parse_account_data(
            &account_data,
            &parsable_data.account.discriminator,
            &parsable_data.account_type.ty,
            &subgraph_request.defined_fields,
        ) {
            Ok(entry) => entry,
            Err(_) => None,
        };
        let Some(mut entry) = entry else {
            return Ok(());
        };

        subgraph_request.intrinsic_fields.iter().for_each(|field| {
            if let Some((entry_key, entry_value)) = field.extract_intrinsic(
                Some(slot),
                Some(tx_signature),
                Some(*pubkey),
                Some(owner),
                Some(lamports),
                Some(write_version),
            ) {
                entry.insert(entry_key, entry_value);
            }
        });

        if !entry.is_empty() {
            let data = serde_json::to_vec(&vec![entry]).unwrap();
            let _ = tx.send(DataIndexingCommand::ProcessCollectionEntriesPack(
                self.uuid, data,
            ));
        }
        Ok(())
    }

    fn parse_account_data(
        data: &[u8],
        account_discriminator: &[u8],
        account_type_def_ty: &IdlTypeDefTy,
        defined_fields: &Vec<IndexedSubgraphField>,
    ) -> Result<Option<HashMap<String, TxtxValue>>, String> {
        let actual_account_discriminator = data[0..8].to_vec();
        if actual_account_discriminator != account_discriminator {
            // This is not the expected account, so we skip it
            return Ok(None);
        }
        let rest = data[8..].to_vec();
        let parsed_value =
            parse_bytes_to_value_with_expected_idl_type_def_ty(&rest, account_type_def_ty)?;

        let obj = parsed_value.as_object().unwrap().clone();
        let mut entry = HashMap::new();
        for field in defined_fields.iter() {
            let v = obj.get(&field.source_key).unwrap().clone();
            entry.insert(field.display_name.clone(), v);
        }
        Ok(Some(entry))
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

                if let Some(parsable_data) = self.pda_mappings.lock().unwrap().get(&pubkey) {
                    let Some(mut entry) = SurfpoolSubgraphPlugin::parse_account_data(
                        info.data,
                        &parsable_data.account.discriminator,
                        &parsable_data.account_type.ty,
                        &subgraph_request.defined_fields,
                    )
                    .unwrap() else {
                        return Ok(());
                    };

                    subgraph_request.intrinsic_fields.iter().for_each(|field| {
                        if let Some((entry_key, entry_value)) = field.extract_intrinsic(
                            Some(slot),
                            Some(txn.signature().clone()),
                            Some(pubkey),
                            Some(owner),
                            Some(info.lamports),
                            Some(info.write_version),
                        ) {
                            entry.insert(entry_key, entry_value);
                        }
                    });

                    entries.push(entry);
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
                        .eq(&subgraph_request.program_id)
                });
                if !is_program_id_match {
                    return Ok(());
                }

                match &subgraph_request.data_source {
                    IndexedSubgraphSourceType::Instruction(_) => return Ok(()),
                    IndexedSubgraphSourceType::Event(event_source) =>
                    // Check inner instructions
                    {
                        if let Some(ref inner_instructions) =
                            data.transaction_status_meta.inner_instructions
                        {
                            for inner_instructions in inner_instructions.iter() {
                                for instruction in inner_instructions.instructions.iter() {
                                    let instruction = &instruction.instruction;
                                    // it's not valid cpi event data if there isn't an 8-byte signature
                                    // well, that ^ is what I thought, but it looks like the _second_ 8 bytes
                                    // are matching the discriminator
                                    if instruction.data.len() < 16 {
                                        continue;
                                    }

                                    let eight_bytes = instruction.data[8..16].to_vec();
                                    let rest = instruction.data[16..].to_vec();

                                    if event_source.event.discriminator.eq(eight_bytes.as_slice()) {
                                        let parsed_value =
                                            parse_bytes_to_value_with_expected_idl_type_def_ty(
                                                &rest,
                                                &event_source.ty.ty,
                                            )
                                            .unwrap();

                                        let obj = parsed_value.as_object().unwrap().clone();
                                        let mut entry = HashMap::new();
                                        for field in subgraph_request.defined_fields.iter() {
                                            if let Some(v) = obj.get(&field.source_key) {
                                                entry.insert(field.display_name.clone(), v.clone());
                                            }
                                        }

                                        subgraph_request.intrinsic_fields.iter().for_each(
                                            |field| {
                                                if let Some((entry_key, entry_value)) = field
                                                    .extract_intrinsic(
                                                        Some(slot),
                                                        Some(transaction.signature().clone()),
                                                        None,
                                                        None,
                                                        None,
                                                        None,
                                                    )
                                                {
                                                    entry.insert(entry_key, entry_value);
                                                }
                                            },
                                        );

                                        entries.push(entry);
                                    }
                                }
                            }
                        }
                    }
                    IndexedSubgraphSourceType::Pda(pda_source) => {
                        for instruction in transaction.message().instructions() {
                            let Some((matching_idl_instruction, idl_instruction_account)) =
                                pda_source.instruction_accounts.iter().find_map(
                                    |(ix, ix_account)| {
                                        if instruction.data.starts_with(&ix.discriminator) {
                                            Some((ix, ix_account))
                                        } else {
                                            None
                                        }
                                    },
                                )
                            else {
                                continue;
                            };

                            let idl_accounts = match_idl_accounts(
                                matching_idl_instruction,
                                &instruction.accounts,
                                &account_pubkeys,
                            );
                            let Some(pda) = idl_accounts.iter().find_map(|(name, pubkey)| {
                                if idl_instruction_account.name.eq(name) {
                                    Some(pubkey)
                                } else {
                                    None
                                }
                            }) else {
                                continue;
                            };

                            self.release_account(
                                pda,
                                ParsablePdaData {
                                    account: pda_source.account.clone(),
                                    account_type: pda_source.account_type.clone(),
                                },
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
