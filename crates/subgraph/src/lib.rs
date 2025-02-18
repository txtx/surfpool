use {
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
        ReplicaEntryInfoVersions, ReplicaTransactionInfoVersions, Result as PluginResult,
        SlotStatus,
    },
    ipc_channel::ipc::IpcSender,
    solana_program::clock::Slot,
    std::sync::Mutex,
    surfpool_types::{SchemaDatasourceingEvent, SubgraphPluginConfig},
    txtx_addon_network_svm::codec::subgraph::{IndexedSubgraphSourceType, SubgraphRequest},
    uuid::Uuid,
};

#[derive(Default, Debug)]
pub struct SurfpoolSubgraphPlugin {
    pub uuid: Uuid,
    subgraph_indexing_event_tx: Mutex<Option<IpcSender<SchemaDatasourceingEvent>>>,
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
        let _ = tx.send(SchemaDatasourceingEvent::Rountrip(config.uuid.clone()));
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
        _slot: Slot,
    ) -> PluginResult<()> {
        let Ok(tx) = self.subgraph_indexing_event_tx.lock() else {
            return Ok(());
        };
        let tx = tx.as_ref().unwrap();
        let Some(ref subgraph_request) = self.subgraph_request else {
            return Ok(());
        };
        match transaction {
            ReplicaTransactionInfoVersions::V0_0_2(data) => {
                let _ = tx.send(SchemaDatasourceingEvent::ApplyEntry(
                    self.uuid,
                    data.signature.to_string(),
                    // subgraph_request.clone(),
                    // slot,
                ));
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
                        let decoded_data = bs58::decode(&instruction.data)
                            .into_vec()
                            .map_err(|e| GeyserPluginError::Custom(Box::new(e)))?;
                        // it's not valid cpi event data if there isn't an 8-byte signature
                        if decoded_data.len() < 8 {
                            continue;
                        }
                        let eight_bytes = decoded_data[0..8].to_vec();
                        let decoded_signature = bs58::decode(eight_bytes)
                            .into_vec()
                            .map_err(|e| GeyserPluginError::Custom(Box::new(e)))?;
                        for field in subgraph_request.fields.iter() {
                            match &field.data_source {
                                IndexedSubgraphSourceType::Instruction(
                                    _instruction_subgraph_source,
                                ) => {
                                    continue;
                                }
                                IndexedSubgraphSourceType::Event(event_subgraph_source) => {
                                    if event_subgraph_source
                                        .event
                                        .discriminator
                                        .eq(decoded_signature.as_slice())
                                    {
                                        println!(
                                            "found event with match!!!: {:?}",
                                            event_subgraph_source.event.name
                                        );
                                        let _ = tx.send(SchemaDatasourceingEvent::ApplyEntry(
                                            self.uuid,
                                            data.signature.to_string(),
                                            // subgraph_request.clone(),
                                            // slot,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ReplicaTransactionInfoVersions::V0_0_1(_) => {}
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
