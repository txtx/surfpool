use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaTransactionInfoV2, ReplicaTransactionInfoVersions,
};
use chrono::{Local, Utc};
use crossbeam::select;
use crossbeam_channel::{unbounded, Receiver, Sender};
use ipc_channel::{
    ipc::{IpcOneShotServer, IpcReceiver},
    router::RouterProxy,
};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_feature_set::{disable_new_loader_v3_deployments, FeatureSet};
use solana_sdk::{
    clock::Clock,
    commitment_config::CommitmentConfig,
    epoch_info::EpochInfo,
    message::v0::LoadedAddresses,
    pubkey::Pubkey,
    transaction::{SanitizedTransaction, Transaction},
};
use solana_transaction_status::{InnerInstruction, InnerInstructions, TransactionStatusMeta};
use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, RwLock, RwLockWriteGuard},
    thread::{sleep, JoinHandle},
    time::{Duration, Instant},
};
use surfpool_subgraph::SurfpoolSubgraphPlugin;

use crate::{
    rpc::{
        self, accounts_data::AccountsData, accounts_scan::AccountsScan, admin::AdminRpc,
        bank_data::BankData, full::Full, minimal::Minimal,
        utils::convert_transaction_metadata_from_canonical, SurfpoolMiddleware,
    },
    types::{EntryStatus, GlobalState, TransactionWithStatusMeta},
    PluginManagerCommand,
};
use surfpool_types::{
    ClockCommand, ClockEvent, RunloopTriggerMode, SchemaDataSourcingEvent, SubgraphPluginConfig,
    TransactionConfirmationStatus, TransactionMetadata, TransactionStatusEvent,
};
use surfpool_types::{SimnetCommand, SimnetEvent, SubgraphCommand, SurfpoolConfig};

const BLOCKHASH_SLOT_TTL: u64 = 75;

// #[cfg(clippy)]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] = &[0];

// #[cfg(not(clippy))]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] =
//     include_bytes!("../../../../target/release/libsurfpool_subgraph.dylib");

fn initialize_lite_svm() -> LiteSVM {
    let mut feature_set = FeatureSet::all_enabled();

    // v2.2 of the solana_sdk deprecates the v3 loader, and enables the v4 loader by default.
    // In order to keep the v3 deployments enabled, we need to remove the
    // `disable_new_loader_v3_deployments` feature from the active set, and add it to the inactive set.
    let _ = feature_set
        .active
        .remove(&disable_new_loader_v3_deployments::id());
    feature_set
        .inactive
        .insert(disable_new_loader_v3_deployments::id());

    LiteSVM::new().with_feature_set(feature_set)
}

pub async fn start(
    config: SurfpoolConfig,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut svm = initialize_lite_svm();
    for recipient in config.simnet.airdrop_addresses.iter() {
        let _ = svm.airdrop(&recipient, config.simnet.airdrop_token_amount);
        let _ = simnet_events_tx.send(SimnetEvent::info(format!(
            "Genesis airdrop successful {}: {}",
            recipient.to_string(),
            config.simnet.airdrop_token_amount
        )));
    }

    // Todo: should check config first
    let rpc_client = Arc::new(RpcClient::new(config.simnet.remote_rpc_url.clone()));
    let epoch_info = rpc_client.get_epoch_info().await?;

    // Question: can the value `slots_in_epoch` fluctuate over time?
    let slots_in_epoch = epoch_info.slots_in_epoch;

    let context = GlobalState::new(svm, &epoch_info, rpc_client.clone());

    let context = Arc::new(RwLock::new(context));
    let (plugin_manager_commands_rx, _rpc_handle) = start_rpc_server_thread(
        &config,
        &simnet_events_tx,
        &simnet_commands_tx,
        context.clone(),
        &epoch_info,
    )?;

    let simnet_config = config.simnet.clone();
    let (plugins_data_tx, plugins_data_rx) = unbounded::<(Transaction, TransactionMetadata)>();

    if !config.plugin_config_path.is_empty() {
        match start_geyser_plugin_thread(
            plugin_manager_commands_rx,
            subgraph_commands_tx.clone(),
            simnet_events_tx.clone(),
            plugins_data_rx,
        ) {
            Ok(_) => {}
            Err(e) => {
                let _ =
                    simnet_events_tx.send(SimnetEvent::error(format!("Geyser plugin failed: {e}")));
            }
        };
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
                        SimnetCommand::TransactionReceived(key, transaction, status_tx, skip_preflight) => {
                            let signature = transaction.signatures[0].clone();
                            transactions_to_process.push((key, transaction, status_tx, skip_preflight));
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
        for (key, transaction, status_tx, skip_preflight) in transactions_to_process.drain(..) {
            transaction.verify_with_results();
            let transaction = transaction.into_legacy_transaction().unwrap();
            let message = &transaction.message;

            let accounts = message.account_keys.clone();
            for account_pubkey in accounts.iter() {
                match insert_account_from_remote(&mut ctx, account_pubkey, &rpc_client).await {
                    Ok(Some(event)) | Err(event) => {
                        let _ = simnet_events_tx.try_send(event);
                    }
                    Ok(None) => {}
                }
            }

            for instruction in &message.instructions {
                // The Transaction may not be sanitized at this point
                if instruction.program_id_index as usize >= message.account_keys.len() {
                    unreachable!();
                }
                let program_id = &message.account_keys[instruction.program_id_index as usize];
                if ctx.svm.get_account(&program_id).is_none() {
                    let res = rpc_client
                        .get_account_with_commitment(&program_id, CommitmentConfig::default())
                        .await;
                    if let Some(event) = match res {
                        Ok(res) => match res.value {
                            Some(account) => {
                                let _ = ctx.svm.set_account(*program_id, account);
                                Some(SimnetEvent::AccountUpdate(Local::now(), program_id.clone()))
                            }
                            None => None,
                        },
                        Err(e) => {
                            SimnetEvent::error(format!("unable to retrieve account: {}", e));
                            None
                        }
                    } {
                        let _ = simnet_events_tx.try_send(event);
                    }
                }
            }

            if !skip_preflight {
                let (meta, err) = match ctx.svm.simulate_transaction(transaction.clone()) {
                    Ok(res) => {
                        let transaction_meta =
                            convert_transaction_metadata_from_canonical(&res.meta);
                        let _ =
                            plugins_data_tx.send((transaction.clone(), transaction_meta.clone()));
                        (transaction_meta, None)
                    }
                    Err(res) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                            "Transaction simulation failed: {}",
                            res.err.to_string()
                        )));
                        (
                            convert_transaction_metadata_from_canonical(&res.meta),
                            Some(res.err),
                        )
                    }
                };

                if let Some(e) = &err {
                    let _ = status_tx
                        .try_send(TransactionStatusEvent::SimulationFailure((e.clone(), meta)));
                    continue;
                }
            }

            let (meta, err) = match ctx.svm.send_transaction(transaction.clone()) {
                Ok(res) => (convert_transaction_metadata_from_canonical(&res), None),
                Err(res) => {
                    let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                        "Transaction execution failed: {}",
                        res.err.to_string()
                    )));
                    (
                        convert_transaction_metadata_from_canonical(&res.meta),
                        Some(res.err),
                    )
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
                let _ = simnet_events_tx.try_send(SimnetEvent::transaction_processed(meta, err));
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

fn start_geyser_plugin_thread(
    plugin_manager_commands_rx: Receiver<PluginManagerCommand>,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    plugins_data_rx: Receiver<(Transaction, TransactionMetadata)>,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let handle = hiro_system_kit::thread_named("Geyser Plugins Handler").spawn(move || {
        let mut plugin_manager = vec![];

        let ipc_router = RouterProxy::new();
        // Note:
        // At the moment, surfpool-subgraph is the only plugin that we're mounting.
        // Please open an issue http://github.com/txtx/surfpool/issues/new if this is a feature you need!
        //
        // Proof of concept:
        //
        // let geyser_plugin_config_file = PathBuf::from("../../surfpool_subgraph_plugin.json");
        // let contents = "{\"name\": \"surfpool-subgraph\", \"libpath\": \"target/release/libsurfpool_subgraph.dylib\"}";
        // let result: serde_json::Value = json5::from_str(&contents).unwrap();
        // let libpath = result["libpath"]
        //     .as_str()
        //     .unwrap();
        // let mut libpath = PathBuf::from(libpath);
        // if libpath.is_relative() {
        //     let config_dir = geyser_plugin_config_file.parent().ok_or_else(|| {
        //         GeyserPluginManagerError::CannotOpenConfigFile(format!(
        //             "Failed to resolve parent of {geyser_plugin_config_file:?}",
        //         ))
        //     }).unwrap();
        //     libpath = config_dir.join(libpath);
        // }
        // let plugin_name = result["name"].as_str().map(|s| s.to_owned()).unwrap_or(format!("surfpool-subgraph"));
        // let (plugin, lib) = unsafe {
        //     let lib = match Library::new(&surfpool_subgraph_path) {
        //         Ok(lib) => lib,
        //         Err(e) => {
        //             let _ = simnet_events_tx_copy.send(SimnetEvent::ErrorLog(Local::now(), format!("Unable to load plugin {}: {}", plugin_name, e.to_string())));
        //             continue;
        //         }
        //     };
        //     let constructor: Symbol<PluginConstructor> = lib
        //         .get(b"_create_plugin")
        //         .map_err(|e| format!("{}", e.to_string()))?;
        //     let plugin_raw = constructor();
        //     (Box::from_raw(plugin_raw), lib)
        // };

        let err = loop {
            select! {
                recv(plugin_manager_commands_rx) -> msg => {
                    match msg {
                        Ok(event) => {
                            match event {
                                PluginManagerCommand::LoadConfig(uuid, config, notifier) => {
                                    let _ = subgraph_commands_tx.send(SubgraphCommand::CreateSubgraph(uuid.clone(), config.data.clone(), notifier));
                                    let mut plugin = SurfpoolSubgraphPlugin::default();

                                    let (server, ipc_token) = IpcOneShotServer::<IpcReceiver<SchemaDataSourcingEvent>>::new().expect("Failed to create IPC one-shot server.");
                                    let subgraph_plugin_config = SubgraphPluginConfig {
                                        uuid,
                                        ipc_token,
                                        subgraph_request: config.data.clone()
                                    };

                                    let config_file = match serde_json::to_string(&subgraph_plugin_config) {
                                        Ok(c) => c,
                                        Err(e) => {
                                            let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to serialize subgraph plugin config: {:?}", e)));
                                            continue;
                                        }
                                    };

                                    if let Err(e) = plugin.on_load(&config_file, false) {
                                        let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to load Geyser plugin: {:?}", e)));
                                    };
                                    if let Ok((_, rx)) = server.accept() {
                                        let subgraph_rx = ipc_router.route_ipc_receiver_to_new_crossbeam_receiver::<SchemaDataSourcingEvent>(rx);
                                        let _ = subgraph_commands_tx.send(SubgraphCommand::ObserveSubgraph(subgraph_rx));
                                    };
                                    let plugin: Box<dyn GeyserPlugin> = Box::new(plugin);
                                    plugin_manager.push(plugin);
                                    let _ = simnet_events_tx.send(SimnetEvent::PluginLoaded("surfpool-subgraph".into()));
                                }
                            }
                        },
                        Err(e) => {
                            break format!("Failed to read plugin manager command: {:?}", e);
                        },
                    }
                },
                recv(plugins_data_rx) -> msg => match msg {
                    Err(e) => {
                        break format!("Failed to read new transaction to send to Geyser plugin: {e}");
                    },
                    Ok((transaction, transaction_metadata)) => {
                        let mut inner_instructions = vec![];
                        for (i,inner) in transaction_metadata.inner_instructions.iter().enumerate() {
                            inner_instructions.push(
                                InnerInstructions {
                                    index: i as u8,
                                    instructions: inner.iter().map(|i| InnerInstruction {
                                        instruction: i.instruction.clone(),
                                        stack_height: Some(i.stack_height as u32)
                                    }).collect()
                                }
                            )
                        }

                        let transaction_status_meta = TransactionStatusMeta {
                            status: Ok(()),
                            fee: 0,
                            pre_balances: vec![],
                            post_balances: vec![],
                            inner_instructions: Some(inner_instructions),
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

                        let transaction = match SanitizedTransaction::try_from_legacy_transaction(transaction, &HashSet::new()) {
                            Ok(tx) => tx,
                            Err(e) => {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: failed to serialize transaction: {:?}", e)));
                                continue;
                            }
                        };

                        let transaction_replica = ReplicaTransactionInfoV2 {
                            signature: &transaction_metadata.signature,
                            is_vote: false,
                            transaction: &transaction,
                            transaction_status_meta: &transaction_status_meta,
                            index: 0
                        };
                        for plugin in plugin_manager.iter() {
                            if let Err(e) = plugin.notify_transaction(ReplicaTransactionInfoVersions::V0_0_2(&transaction_replica), 0) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: {:?}", e)));
                            };
                        }
                    }
                }
            }
        };
        Err(err)
    }).map_err(|e| format!("Failed to spawn Geyser Plugins Handler thread: {:?}", e))?;
    Ok(handle)
}

fn start_rpc_server_thread(
    config: &SurfpoolConfig,
    simnet_events_tx: &Sender<SimnetEvent>,
    simnet_commands_tx: &Sender<SimnetCommand>,
    context: Arc<RwLock<GlobalState>>,
    epoch_info: &EpochInfo,
) -> Result<(Receiver<PluginManagerCommand>, JoinHandle<()>), String> {
    let (plugin_manager_commands_tx, plugin_manager_commands_rx) = unbounded();

    let middleware = SurfpoolMiddleware::new(
        context,
        simnet_commands_tx,
        &plugin_manager_commands_tx,
        &config.rpc,
    );
    let server_bind: SocketAddr = config
        .rpc
        .get_socket_address()
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;

    let simnet_events_tx = simnet_events_tx.clone();
    let epoch_info = epoch_info.clone();

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::accounts_scan::SurfpoolAccountsScanRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());

    if !config.plugin_config_path.is_empty() {
        io.extend_with(rpc::admin::SurfpoolAdminRpc.to_delegate());
    }

    let _ = std::net::TcpListener::bind(server_bind)
        .map_err(|e| format!("Failed to start RPC server: {}", e))?;

    let _handle = hiro_system_kit::thread_named("RPC Handler")
        .spawn(move || {
            let server = match ServerBuilder::new(io)
                .cors(DomainsValidation::Disabled)
                .start_http(&server_bind)
            {
                Ok(server) => server,
                Err(e) => {
                    let _ = simnet_events_tx.send(SimnetEvent::Aborted(format!(
                        "Failed to start RPC server: {:?}",
                        e
                    )));
                    return;
                }
            };
            let _ = simnet_events_tx.send(SimnetEvent::Ready);
            let _ = simnet_events_tx.send(SimnetEvent::EpochInfoUpdate(epoch_info));

            server.wait();
            let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
        })
        .map_err(|e| format!("Failed to spawn RPC Handler thread: {:?}", e))?;
    Ok((plugin_manager_commands_rx, _handle))
}

async fn insert_account_from_remote(
    ctx: &mut RwLockWriteGuard<'_, GlobalState>,
    account_pubkey: &Pubkey,
    rpc: &RpcClient,
) -> Result<Option<SimnetEvent>, SimnetEvent> {
    if ctx.svm.get_account(&account_pubkey).is_none() {
        let res = rpc
            .get_account_with_commitment(&account_pubkey, CommitmentConfig::default())
            .await;
        match res {
            Ok(res) => match res.value {
                Some(account) => {
                    let _ = ctx.svm.set_account(*account_pubkey, account);
                    return Ok(Some(SimnetEvent::AccountUpdate(
                        Local::now(),
                        account_pubkey.clone(),
                    )));
                }
                None => return Ok(None),
            },
            Err(e) => {
                return Err(SimnetEvent::error(format!(
                    "unable to retrieve account: {}",
                    e
                )));
            }
        }
    }
    Ok(None)
}
