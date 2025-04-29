use {
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
        ReplicaEntryInfoVersions, ReplicaTransactionInfoVersions, Result as PluginResult,
        SlotStatus,
    },
    ipc_channel::ipc::IpcSender,
    solana_program::clock::Slot,
    std::{collections::HashMap, sync::Mutex},
    surfpool_types::{SchemaDataSourcingEvent, SubgraphPluginConfig},
    txtx_addon_network_svm::codec::idl::parse_bytes_to_value_with_expected_idl_type,
    txtx_addon_network_svm_types::subgraph::{IndexedSubgraphSourceType, SubgraphRequest},
    uuid::Uuid,
};

#[derive(Default, Debug)]
pub struct SurfpoolSubgraphPlugin {
    pub uuid: Uuid,
    subgraph_indexing_event_tx: Mutex<Option<IpcSender<SchemaDataSourcingEvent>>>,
    subgraph_request: Option<SubgraphRequest>,
}

impl GeyserPlugin for SurfpoolSubgraphPlugin {
    fn name(&self) -> &'static str {
        "surfpool-subgraph"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        let config = serde_json::from_str::<SubgraphPluginConfig>(&config_file).unwrap();
        let oneshot_tx = IpcSender::connect(config.ipc_token).unwrap();
        let (tx, rx) = ipc_channel::ipc::channel().unwrap();
        let _ = tx.send(SchemaDataSourcingEvent::Rountrip(config.uuid.clone()));
        let _ = oneshot_tx.send(rx);
        self.uuid = config.uuid.clone();
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
        _slot: Slot,
        _is_startup: bool,
    ) -> PluginResult<()> {
        let account_info = match account {
            ReplicaAccountInfoVersions::V0_0_1(_info) => {
                unreachable!("ReplicaAccountInfoVersions::V0_0_1 is not supported")
            }
            ReplicaAccountInfoVersions::V0_0_2(_info) => {
                unreachable!("ReplicaAccountInfoVersions::V0_0_2 is not supported")
            }
            ReplicaAccountInfoVersions::V0_0_3(info) => info,
        };

        println!("lamports: {}", account_info.lamports);

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
        let tx_hash = match transaction {
            ReplicaTransactionInfoVersions::V0_0_2(data) => {
                if data.is_vote {
                    return Ok(());
                }

                let Some(ref inner_instructions) = data.transaction_status_meta.inner_instructions
                else {
                    return Ok(());
                };
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

                        match &subgraph_request.data_source {
                            IndexedSubgraphSourceType::Instruction(
                                _instruction_subgraph_source,
                            ) => {
                                continue;
                            }
                            IndexedSubgraphSourceType::Event(event_subgraph_source) => {
                                if event_subgraph_source
                                    .event
                                    .discriminator
                                    .eq(eight_bytes.as_slice())
                                {
                                    let parsed_value = parse_bytes_to_value_with_expected_idl_type(
                                        &rest,
                                        &event_subgraph_source.ty.ty,
                                    )
                                    .unwrap();

                                    let obj = parsed_value.as_object().unwrap().clone();
                                    let mut entry = HashMap::new();
                                    for field in subgraph_request.fields.iter() {
                                        let v = obj.get(&field.source_key).unwrap().clone();
                                        entry.insert(field.display_name.clone(), v);
                                    }
                                    entries.push(entry);
                                }
                            }
                        }
                    }
                }
                data.transaction.message_hash()
            }
            ReplicaTransactionInfoVersions::V0_0_1(_) => {
                todo!()
            }
        };
        if !entries.is_empty() {
            let data = serde_json::to_vec(&entries).unwrap();
            let _ = tx.send(SchemaDataSourcingEvent::ApplyEntry(
                self.uuid,
                data,
                slot,
                tx_hash.to_bytes(),
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

#[no_mangle]
#[allow(improper_ctypes_definitions)]

/// # Safety
///
/// This function returns the Plugin pointer as trait GeyserPlugin.
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    let plugin: Box<dyn GeyserPlugin> = Box::<SurfpoolSubgraphPlugin>::default();
    Box::into_raw(plugin)
}
