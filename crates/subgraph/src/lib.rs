use {
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
        ReplicaEntryInfoVersions, ReplicaTransactionInfoVersions, Result as PluginResult,
        SlotStatus,
    },
    ipc_channel::ipc::IpcSender,
    solana_program::clock::Slot,
    std::sync::Mutex,
    surfpool_core::types::{SubgraphIndexingEvent, SubgraphPluginConfig},
};

#[derive(Default, Debug)]
pub struct SurfpoolSubgraph {
    pub id: String,
    subgraph_indexing_event_tx: Mutex<Option<IpcSender<SubgraphIndexingEvent>>>,
}

impl GeyserPlugin for SurfpoolSubgraph {
    fn name(&self) -> &'static str {
        "surfpool-subgraph"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        let config = serde_json::from_str::<SubgraphPluginConfig>(&config_file).unwrap();
        let oneshot_tx = IpcSender::connect(config.ipc_token).unwrap();
        let (tx, rx) = ipc_channel::ipc::channel().unwrap();
        let _ = oneshot_tx.send(rx);
        self.subgraph_indexing_event_tx = Mutex::new(Some(tx));
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
        match transaction {
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
                        println!("{:?}", instruction);
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
    let plugin: Box<dyn GeyserPlugin> = Box::<SurfpoolSubgraph>::default();
    Box::into_raw(plugin)
}
