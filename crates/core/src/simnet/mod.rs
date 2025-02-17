use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaTransactionInfoV2, ReplicaTransactionInfoVersions,
};
use base64::prelude::{Engine, BASE64_STANDARD};
use chrono::{Local, Utc};
use crossbeam::select;
use crossbeam_channel::{unbounded, Receiver, Sender};
use ipc_channel::{
    ipc::{IpcOneShotServer, IpcReceiver},
    router::RouterProxy,
};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::{types::TransactionMetadata, LiteSVM};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_geyser_plugin_manager::geyser_plugin_manager::{
    GeyserPluginManager, GeyserPluginManagerError, LoadedGeyserPlugin,
};
use solana_sdk::{
    clock::Clock,
    epoch_info::EpochInfo,
    message::v0::LoadedAddresses,
    signature::Signature,
    transaction::{SanitizedTransaction, Transaction, TransactionError, TransactionVersion},
};
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedConfirmedTransactionWithStatusMeta,
    EncodedTransaction, EncodedTransactionWithStatusMeta, TransactionConfirmationStatus,
    TransactionStatus, TransactionStatusMeta, UiCompiledInstruction, UiInnerInstructions,
    UiInstruction, UiMessage, UiRawMessage, UiReturnDataEncoding, UiTransaction,
    UiTransactionReturnData, UiTransactionStatusMeta,
};
use std::path::PathBuf;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::{Arc, RwLock},
    thread::sleep,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    rpc::{
        self, accounts_data::AccountsData, accounts_scan::AccountsScan, admin::AdminRpc,
        bank_data::BankData, full::Full, minimal::Minimal, SurfpoolMiddleware,
    },
    types::{
        ClockCommand, ClockEvent, PluginManagerCommand, RunloopTriggerMode,
        SchemaDatasourceingEvent, SimnetCommand, SimnetEvent, SubgraphCommand,
        SubgraphPluginConfig, SurfpoolConfig, TransactionStatusEvent,
    },
};

const BLOCKHASH_SLOT_TTL: u64 = 75;

#[derive(Debug, Clone)]
pub struct TransactionWithStatusMeta(
    pub u64,
    pub Transaction,
    pub TransactionMetadata,
    pub Option<TransactionError>,
);

impl TransactionWithStatusMeta {
    pub fn into_status(&self, current_slot: u64) -> TransactionStatus {
        TransactionStatus {
            slot: self.0,
            confirmations: Some((current_slot - self.0) as usize),
            status: match self.3.clone() {
                Some(err) => Err(err),
                None => Ok(()),
            },
            err: self.3.clone(),
            confirmation_status: Some(TransactionConfirmationStatus::Finalized),
        }
    }
}

impl Into<EncodedConfirmedTransactionWithStatusMeta> for TransactionWithStatusMeta {
    fn into(self) -> EncodedConfirmedTransactionWithStatusMeta {
        let slot = self.0;
        let tx = self.1;
        let meta = self.2;
        let err = self.3;
        EncodedConfirmedTransactionWithStatusMeta {
            slot,
            transaction: EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: tx.signatures.iter().map(|s| s.to_string()).collect(),
                    message: UiMessage::Raw(UiRawMessage {
                        header: tx.message.header,
                        account_keys: tx
                            .message
                            .account_keys
                            .iter()
                            .map(|pk| pk.to_string())
                            .collect(),
                        recent_blockhash: tx.message.recent_blockhash.to_string(),
                        instructions: tx
                            .message
                            .instructions
                            .iter()
                            // TODO: use stack height
                            .map(|ix| UiCompiledInstruction::from(ix, None))
                            .collect(),
                        address_table_lookups: None, // TODO: use lookup table
                    }),
                }),
                meta: Some(UiTransactionStatusMeta {
                    err: err.clone(),
                    status: match err {
                        Some(e) => Err(e),
                        None => Ok(()),
                    },
                    fee: 5000 * (tx.signatures.len() as u64), // TODO: fix calculation
                    pre_balances: vec![],
                    post_balances: vec![],
                    inner_instructions: OptionSerializer::Some(
                        meta.inner_instructions
                            .iter()
                            .enumerate()
                            .map(|(i, ixs)| UiInnerInstructions {
                                index: i as u8,
                                instructions: ixs
                                    .iter()
                                    .map(|ix| {
                                        UiInstruction::Compiled(UiCompiledInstruction {
                                            program_id_index: ix.instruction.program_id_index,
                                            accounts: ix.instruction.accounts.clone(),
                                            data: String::from_utf8(ix.instruction.data.clone())
                                                .unwrap(),
                                            stack_height: Some(ix.stack_height as u32),
                                        })
                                    })
                                    .collect(),
                            })
                            .collect(),
                    ),
                    log_messages: OptionSerializer::Some(meta.logs),
                    pre_token_balances: OptionSerializer::None,
                    post_token_balances: OptionSerializer::None,
                    rewards: OptionSerializer::None,
                    loaded_addresses: OptionSerializer::None,
                    return_data: OptionSerializer::Some(UiTransactionReturnData {
                        program_id: meta.return_data.program_id.to_string(),
                        data: (
                            BASE64_STANDARD.encode(meta.return_data.data),
                            UiReturnDataEncoding::Base64,
                        ),
                    }),
                    compute_units_consumed: OptionSerializer::Some(meta.compute_units_consumed),
                }),
                version: Some(TransactionVersion::Legacy(
                    solana_sdk::transaction::Legacy::Legacy,
                )),
            },
            block_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .ok(),
        }
    }
}

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions: HashMap<Signature, EntryStatus>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
    pub rpc_client: Arc<RpcClient>,
}

#[derive(Debug, Clone)]
pub enum EntryStatus {
    Received,
    Processed(TransactionWithStatusMeta),
}

impl EntryStatus {
    pub fn expect_processed(&self) -> &TransactionWithStatusMeta {
        match &self {
            EntryStatus::Received => unreachable!(),
            EntryStatus::Processed(status) => status,
        }
    }
}

type PluginConstructor = unsafe fn() -> *mut dyn GeyserPlugin;
use libloading::{Library, Symbol};

pub async fn start(
    config: SurfpoolConfig,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut svm = LiteSVM::new();
    for recipient in config.simnet.airdrop_addresses.iter() {
        let _ = svm.airdrop(&recipient, config.simnet.airdrop_token_amount);
        let _ = simnet_events_tx.send(SimnetEvent::InfoLog(
            Local::now(),
            format!(
                "Genesis airdrop successful {}: {}",
                recipient.to_string(),
                config.simnet.airdrop_token_amount
            ),
        ));
    }

    // Todo: should check config first
    let rpc_client = Arc::new(RpcClient::new(config.simnet.remote_rpc_url.clone()));
    let epoch_info = rpc_client.get_epoch_info().await?;
    // let epoch_info = EpochInfo {
    //     epoch: 0,
    //     slot_index: 0,
    //     slots_in_epoch: 0,
    //     absolute_slot: 0,
    //     block_height: 0,
    //     transaction_count: None,
    // };

    // Question: can the value `slots_in_epoch` fluctuate over time?
    let slots_in_epoch = epoch_info.slots_in_epoch;

    let context = GlobalState {
        svm,
        transactions: HashMap::new(),
        perf_samples: VecDeque::new(),
        transactions_processed: 0,
        epoch_info: epoch_info.clone(),
        rpc_client: rpc_client.clone(),
    };

    let (plugin_manager_commands_tx, plugin_manager_commands_rx) = unbounded();

    let context = Arc::new(RwLock::new(context));
    let middleware = SurfpoolMiddleware {
        context: context.clone(),
        simnet_commands_tx: simnet_commands_tx.clone(),
        plugin_manager_commands_tx,
        config: config.rpc.clone(),
    };

    let simnet_events_tx_copy = simnet_events_tx.clone();
    let server_bind: SocketAddr = config.rpc.get_socket_address().parse()?;

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::accounts_scan::SurfpoolAccountsScanRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());
    io.extend_with(rpc::admin::SurfpoolAdminRpc.to_delegate());

    let _handle = hiro_system_kit::thread_named("rpc handler").spawn(move || {
        let server = ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .start_http(&server_bind)
            .unwrap();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Ready);
        let _ = simnet_events_tx_copy.send(SimnetEvent::EpochInfoUpdate(epoch_info));

        server.wait();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Shutdown);
    });

    let simnet_config = config.simnet.clone();
    let simnet_events_tx_copy = simnet_events_tx.clone();
    let (plugins_data_tx, plugins_data_rx) = unbounded::<(Transaction, TransactionMetadata)>();

    let ipc_router = RouterProxy::new();

    if !config.plugin_config_path.is_empty() {
        let _handle = hiro_system_kit::thread_named("geyser plugins handler").spawn(move || {
            let mut plugin_manager = GeyserPluginManager::new();

            // for (i, geyser_plugin_config_file) in config.plugin_config_path.iter().enumerate() {
                // let mut file = match File::open(geyser_plugin_config_file) {
                //     Ok(file) => file,
                //     Err(err) => {
                //         return Err(GeyserPluginManagerError::CannotOpenConfigFile(format!(
                //             "Failed to open the plugin config file {geyser_plugin_config_file:?}, error: {err:?}"
                //         )));
                //     }
                // };
                // let mut contents = String::new();
                // if let Err(err) = file.read_to_string(&mut contents) {
                //     return Err(GeyserPluginManagerError::CannotReadConfigFile(format!(
                //         "Failed to read the plugin config file {geyser_plugin_config_file:?}, error: {err:?}"
                //     )));
                // }
                // let result: serde_json::Value = match json5::from_str(&contents) {
                //     Ok(value) => value,
                //     Err(err) => {
                //         return Err(GeyserPluginManagerError::InvalidConfigFileFormat(format!(
                //             "The config file {geyser_plugin_config_file:?} is not in a valid Json5 format, error: {err:?}"
                //         )));
                //     }
                // };

                // let _config_file = geyser_plugin_config_file
                //     .as_os_str()
                //     .to_str()
                //     .ok_or(GeyserPluginManagerError::InvalidPluginPath)?;


                let geyser_plugin_config_file = PathBuf::from("../../surfpool_subgraph_plugin.json");

                let contents = "{\"name\": \"surfpool-subgraph\", \"libpath\": \"target/release/libsurfpool_subgraph.dylib\"}";
                let result: serde_json::Value = json5::from_str(&contents).unwrap();

                let libpath = result["libpath"]
                    .as_str()
                    .unwrap();
                let mut libpath = PathBuf::from(libpath);
                if libpath.is_relative() {
                    let config_dir = geyser_plugin_config_file.parent().ok_or_else(|| {
                        GeyserPluginManagerError::CannotOpenConfigFile(format!(
                            "Failed to resolve parent of {geyser_plugin_config_file:?}",
                        ))
                    }).unwrap();
                    libpath = config_dir.join(libpath);
                }

                let plugin_name = result["name"].as_str().map(|s| s.to_owned()).unwrap_or(format!("surfpool-subgraph"));

                loop {
                    select! {
                        recv(plugin_manager_commands_rx) -> msg => match msg {
                            Ok(event) => {
                                match event {
                                    PluginManagerCommand::LoadConfig(uuid, config, notifier) => {
                                        let _ = subgraph_commands_tx.send(SubgraphCommand::CreateSubgraph(uuid.clone(), config.data.clone(), notifier));

                                        let (mut plugin, lib) = unsafe {
                                            let lib = match Library::new(&libpath) {
                                                Ok(lib) => lib,
                                                Err(e) => {
                                                    let _ = simnet_events_tx_copy.send(SimnetEvent::ErrorLog(Local::now(), format!("Unable to load plugin {}: {}", plugin_name, e.to_string())));
                                                    continue;
                                                }
                                            };
                                            let constructor: Symbol<PluginConstructor> = lib
                                                .get(b"_create_plugin")
                                                .map_err(|e| format!("{}", e.to_string()))?;
                                            let plugin_raw = constructor();
                                            (Box::from_raw(plugin_raw), lib)
                                        };

                                        let (server, ipc_token) = IpcOneShotServer::<IpcReceiver<SchemaDatasourceingEvent>>::new().expect("Failed to create IPC one-shot server.");
                                        let subgraph_plugin_config = SubgraphPluginConfig {
                                            uuid,
                                            ipc_token,
                                            subgraph_request: config.data.clone()
                                        };
                                        let config_file = serde_json::to_string(&subgraph_plugin_config).unwrap();
                                        let _res = plugin.on_load(&config_file, false);
                                        if let Ok((_, rx)) = server.accept() {
                                            let subgraph_rx = ipc_router.route_ipc_receiver_to_new_crossbeam_receiver(rx);
                                            let _ = subgraph_commands_tx.send(SubgraphCommand::ObserveSubgraph(subgraph_rx));
                                        };
                                        plugin_manager.plugins.push(LoadedGeyserPlugin::new(lib, plugin, Some(plugin_name.clone())));
                                        let _ = simnet_events_tx_copy.send(SimnetEvent::PluginLoaded("surfpool-subgraph".into()));
                                    }
                                }
                            },
                            Err(_) => {},
                        },
                        recv(plugins_data_rx) -> msg => match msg {
                            Err(_) => unreachable!(),
                            Ok((transaction, transaction_metadata)) => {
                                let transaction_status_meta = TransactionStatusMeta {
                                    status: Ok(()),
                                    fee: 0,
                                    pre_balances: vec![],
                                    post_balances: vec![],
                                    inner_instructions: None,
                                    log_messages: Some(transaction_metadata.logs.clone()),
                                    pre_token_balances: None,
                                    post_token_balances: None,
                                    rewards: None,
                                    loaded_addresses: LoadedAddresses {
                                        writable: vec![],
                                        readonly: vec![],
                                    },
                                    return_data: Some(transaction_metadata.return_data.clone()),
                                    compute_units_consumed: Some(transaction_metadata.compute_units_consumed),
                                };

                                let transaction = SanitizedTransaction::try_from_legacy_transaction(transaction, &HashSet::new())
                                    .unwrap();

                                let transaction_replica = ReplicaTransactionInfoV2 {
                                    signature: &transaction_metadata.signature,
                                    is_vote: false,
                                    transaction: &transaction,
                                    transaction_status_meta: &transaction_status_meta,
                                    index: 0
                                };
                                for plugin in plugin_manager.plugins.iter() {
                                    plugin.notify_transaction(ReplicaTransactionInfoVersions::V0_0_2(&transaction_replica), 0).unwrap();
                                }
                            }
                        }
                    }
                }
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        });
    }

    let (clock_event_tx, clock_event_rx) = unbounded::<ClockEvent>();
    let (clock_command_tx, clock_command_rx) = unbounded::<ClockCommand>();

    let mut slot_time = simnet_config.slot_time;
    let _handle = hiro_system_kit::thread_named("clock").spawn(move || {
        let mut enabled = true;
        let mut block_hash_timeout = Instant::now();

        loop {
            match clock_command_rx.try_recv() {
                Ok(ClockCommand::Pause) => {
                    enabled = false;
                }
                Ok(ClockCommand::Resume) => {
                    enabled = true;
                }
                Ok(ClockCommand::Toggle) => {
                    enabled = !enabled;
                }
                Ok(ClockCommand::UpdateSlotInterval(updated_slot_time)) => {
                    slot_time = updated_slot_time;
                }
                Err(_e) => {}
            }
            sleep(Duration::from_millis(slot_time));
            if enabled {
                let _ = clock_event_tx.send(ClockEvent::Tick);
                // Todo: the block expiration is not completely accurate.
                if block_hash_timeout.elapsed()
                    > Duration::from_millis(BLOCKHASH_SLOT_TTL * slot_time)
                {
                    let _ = clock_event_tx.send(ClockEvent::ExpireBlockHash);
                    block_hash_timeout = Instant::now();
                }
            }
        }
    });

    let mut runloop_trigger_mode = config.simnet.runloop_trigger_mode.clone();
    let mut transactions_to_process = vec![];
    let mut num_transactions = 0;
    loop {
        let mut transactions_processed = vec![];
        let mut create_slot = false;

        select! {
            recv(clock_event_rx) -> msg => match msg {
                Ok(event) => {
                    match event {
                        ClockEvent::Tick => {
                            if runloop_trigger_mode.eq(&RunloopTriggerMode::Clock) {
                                create_slot = true;
                            }
                        }
                        ClockEvent::ExpireBlockHash => {
                            if let Ok(mut ctx) = context.write() {
                                ctx.svm.expire_blockhash();
                            }
                        }
                    }
                },
                Err(_) => {},
            },
            recv(simnet_commands_rx) -> msg => match msg {
                Ok(event) => {
                    match event {
                        SimnetCommand::SlotForward => {
                            runloop_trigger_mode = RunloopTriggerMode::Manual;
                            create_slot = true;
                        }
                        SimnetCommand::SlotBackward => {

                        }
                        SimnetCommand::UpdateClock(update) => {
                            let _ = clock_command_tx.send(update);
                            continue
                        }
                        SimnetCommand::UpdateRunloopMode(update) => {
                            runloop_trigger_mode = update;
                            continue
                        }
                        SimnetCommand::TransactionReceived(key, transaction, status_tx, config) => {
                            let signature = transaction.signatures[0].clone();
                            transactions_to_process.push((key, transaction, status_tx, config));
                            if let Ok(mut ctx) = context.write() {
                                ctx.transactions.insert(
                                    signature,
                                    EntryStatus::Received,
                                );
                            }
                        }
                    }
                },
                Err(_) => {},
            },
        }

        // We will create a slot!
        let unix_timestamp: i64 = Utc::now().timestamp();
        let Ok(mut ctx) = context.write() else {
            continue;
        };

        // Handle the transactions accumulated
        for (key, transaction, status_tx, tx_config) in transactions_to_process.drain(..) {
            transaction.verify_with_results();
            let transaction = transaction.into_legacy_transaction().unwrap();
            let message = &transaction.message;

            for instruction in &message.instructions {
                // The Transaction may not be sanitized at this point
                if instruction.program_id_index as usize >= message.account_keys.len() {
                    unreachable!();
                }
                let program_id = &message.account_keys[instruction.program_id_index as usize];
                if ctx.svm.get_account(&program_id).is_none() {
                    let res = rpc_client.get_account(&program_id).await;
                    let event = match res {
                        Ok(account) => {
                            let _ = ctx.svm.set_account(*program_id, account);
                            SimnetEvent::AccountUpdate(Local::now(), program_id.clone())
                        }
                        Err(e) => SimnetEvent::ErrorLog(
                            Local::now(),
                            format!("unable to retrieve account: {}", e),
                        ),
                    };
                    let _ = simnet_events_tx.try_send(event);
                }
            }

            if !tx_config.skip_preflight {
                let (meta, err) = match ctx.svm.simulate_transaction(transaction.clone()) {
                    Ok(res) => (res.meta, None),
                    Err(e) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::ErrorLog(
                            Local::now(),
                            format!("Transaction simulation failed: {}", e.err.to_string()),
                        ));
                        (e.meta, Some(e.err))
                    }
                };

                if let Some(e) = &err {
                    let _ = status_tx
                        .try_send(TransactionStatusEvent::SimulationFailure((e.clone(), meta)));
                    continue;
                }
            }

            let (meta, err) = match ctx.svm.send_transaction(transaction.clone()) {
                Ok(res) => {
                    let _ = plugins_data_tx.try_send((transaction.clone(), res.clone()));
                    (res, None)
                }
                Err(e) => {
                    let _ = simnet_events_tx.try_send(SimnetEvent::ErrorLog(
                        Local::now(),
                        format!("Transaction execution failed: {}", e.err.to_string()),
                    ));
                    (e.meta, Some(e.err))
                }
            };
            let slot = ctx.epoch_info.absolute_slot;

            ctx.transactions.insert(
                transaction.signatures[0],
                EntryStatus::Processed(TransactionWithStatusMeta(
                    slot,
                    transaction.clone(),
                    meta.clone(),
                    err.clone(),
                )),
            );
            if let Some(e) = &err {
                let _ =
                    status_tx.try_send(TransactionStatusEvent::ExecutionFailure((e.clone(), meta)));
            } else {
                let _ = status_tx.try_send(TransactionStatusEvent::Success(
                    TransactionConfirmationStatus::Processed,
                ));
                let _ = simnet_events_tx.try_send(SimnetEvent::TransactionProcessed(
                    Local::now(),
                    meta,
                    err,
                ));
            }
            transactions_processed.push((key, transaction, status_tx));
            num_transactions += 1;
        }

        if !create_slot {
            continue;
        }

        for (_key, _transaction, status_tx) in transactions_processed.iter() {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
        }

        ctx.epoch_info.slot_index += 1;
        ctx.epoch_info.absolute_slot += 1;
        let slot = ctx.epoch_info.slot_index;

        if ctx.perf_samples.len() > 30 {
            ctx.perf_samples.pop_back();
        }
        ctx.perf_samples.push_front(RpcPerfSample {
            slot,
            num_slots: 1,
            sample_period_secs: 1,
            num_transactions,
            num_non_vote_transactions: None,
        });
        num_transactions = 0;

        if ctx.epoch_info.slot_index > slots_in_epoch {
            ctx.epoch_info.slot_index = 0;
            ctx.epoch_info.epoch += 1;
        }
        let clock: Clock = Clock {
            slot: ctx.epoch_info.slot_index,
            epoch: ctx.epoch_info.epoch,
            unix_timestamp,
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };
        let _ = simnet_events_tx.send(SimnetEvent::ClockUpdate(clock.clone()));
        ctx.svm.set_sysvar(&clock);
    }
}
