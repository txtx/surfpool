#![allow(unused_imports, dead_code, unused_mut, unused_variables)]

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread::{JoinHandle, sleep},
    time::{Duration, Instant},
};

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaTransactionInfoV2, ReplicaTransactionInfoVersions,
};
use chrono::{Local, Utc};
use crossbeam::select;
use crossbeam_channel::{Receiver, Sender, unbounded};
use ipc_channel::{
    ipc::{IpcOneShotServer, IpcReceiver},
    router::RouterProxy,
};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use jsonrpc_pubsub::{PubSubHandler, Session};
use jsonrpc_ws_server::{RequestContext, ServerBuilder as WsServerBuilder};
use libloading::{Library, Symbol};
#[cfg(feature = "geyser-plugin")]
use solana_geyser_plugin_manager::geyser_plugin_manager::{
    GeyserPluginManager, LoadedGeyserPlugin,
};
use solana_message::SimpleAddressLoader;
use solana_sdk::transaction::MessageHash;
use solana_transaction::sanitized::SanitizedTransaction;
use surfpool_subgraph::SurfpoolSubgraphPlugin;
use surfpool_types::{
    BlockProductionMode, ClockCommand, ClockEvent, DataIndexingCommand, SimnetCommand, SimnetEvent,
    SubgraphCommand, SubgraphPluginConfig, SurfpoolConfig,
};
type PluginConstructor = unsafe fn() -> *mut dyn GeyserPlugin;
use txtx_addon_kit::helpers::fs::FileLocation;

use crate::{
    PluginManagerCommand,
    rpc::{
        self, RunloopContext, SurfpoolMiddleware, SurfpoolWebsocketMeta,
        SurfpoolWebsocketMiddleware, accounts_data::AccountsData, accounts_scan::AccountsScan,
        admin::AdminRpc, bank_data::BankData, full::Full, minimal::Minimal,
        surfnet_cheatcodes::SvmTricksRpc, ws::Rpc,
    },
    surfnet::{GeyserEvent, locker::SurfnetSvmLocker, remote::SurfnetRemoteClient},
};

const BLOCKHASH_SLOT_TTL: u64 = 75;

pub async fn start_local_surfnet_runloop(
    svm_locker: SurfnetSvmLocker,
    config: SurfpoolConfig,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_commands_tx: Sender<SimnetCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
    geyser_events_rx: Receiver<GeyserEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(simnet) = config.simnets.first() else {
        return Ok(());
    };
    let block_production_mode = simnet.block_production_mode.clone();

    let remote_rpc_client = Some(SurfnetRemoteClient::new(&simnet.remote_rpc_url));

    let _ = svm_locker.initialize(&remote_rpc_client).await?;

    svm_locker.airdrop_pubkeys(simnet.airdrop_token_amount, &simnet.airdrop_addresses);
    let simnet_events_tx_cc = svm_locker.simnet_events_tx();

    let (plugin_manager_commands_rx, _rpc_handle, _ws_handle) = start_rpc_servers_runloop(
        &config,
        &simnet_commands_tx,
        svm_locker.clone(),
        &remote_rpc_client,
    )
    .await?;

    let simnet_config = simnet.clone();

    match start_geyser_runloop(
        config.plugin_config_path.clone(),
        plugin_manager_commands_rx,
        subgraph_commands_tx.clone(),
        simnet_events_tx_cc.clone(),
        geyser_events_rx,
    ) {
        Ok(_) => {}
        Err(e) => {
            let _ =
                simnet_events_tx_cc.send(SimnetEvent::error(format!("Geyser plugin failed: {e}")));
        }
    };

    let (clock_event_rx, clock_command_tx) = start_clock_runloop(simnet_config.slot_time);

    let _ = simnet_events_tx_cc.send(SimnetEvent::Ready);

    start_block_production_runloop(
        clock_event_rx,
        clock_command_tx,
        simnet_commands_rx,
        simnet_commands_tx.clone(),
        svm_locker,
        block_production_mode,
        &remote_rpc_client,
        simnet_config.expiry.map(|e| e * 1000),
    )
    .await
}

pub async fn start_block_production_runloop(
    clock_event_rx: Receiver<ClockEvent>,
    clock_command_tx: Sender<ClockCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
    simnet_commands_tx: Sender<SimnetCommand>,
    svm_locker: SurfnetSvmLocker,
    mut block_production_mode: BlockProductionMode,
    remote_rpc_client: &Option<SurfnetRemoteClient>,
    expiry_duration_ms: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut next_scheduled_expiry_check: Option<u64> =
        expiry_duration_ms.map(|expiry_val| Utc::now().timestamp_millis() as u64 + expiry_val);
    loop {
        let mut do_produce_block = false;

        select! {
            recv(clock_event_rx) -> msg => if let Ok(event) = msg {
                match event {
                    ClockEvent::Tick => {
                        if block_production_mode.eq(&BlockProductionMode::Clock) {
                            do_produce_block = true;
                        }

                        if let Some(expiry_ms) = expiry_duration_ms {
                            if let Some(scheduled_time_ref) = &mut next_scheduled_expiry_check {
                                let now_ms = Utc::now().timestamp_millis() as u64;
                                if now_ms >= *scheduled_time_ref {
                                    let svm = svm_locker.0.read().await;
                                    if svm.updated_at + expiry_ms < now_ms {
                                        let _ = simnet_commands_tx.send(SimnetCommand::Terminate(None));
                                    } else {
                                        *scheduled_time_ref = svm.updated_at + expiry_ms;
                                    }
                                }
                            }
                        }
                    }
                    ClockEvent::ExpireBlockHash => {
                        do_produce_block = true;
                    }
                }
            },
            recv(simnet_commands_rx) -> msg => if let Ok(event) = msg {
                match event {
                    SimnetCommand::SlotForward(_key) => {
                        block_production_mode = BlockProductionMode::Manual;
                        do_produce_block = true;
                    }
                    SimnetCommand::SlotBackward(_key) => {

                    }
                    SimnetCommand::UpdateClock(update) => {
                        let _ = clock_command_tx.send(update);
                        continue
                    }
                    SimnetCommand::UpdateBlockProductionMode(update) => {
                        block_production_mode = update;
                        continue
                    }
                    SimnetCommand::TransactionReceived(_key, transaction, status_tx, skip_preflight) => {
                       if let Err(e) = svm_locker.process_transaction(remote_rpc_client, transaction, status_tx, skip_preflight).await {
                            let _ = svm_locker.simnet_events_tx().send(SimnetEvent::error(format!("Failed to process transaction: {}", e)));
                       }
                    }
                    SimnetCommand::Terminate(_) => {
                        let _ = svm_locker.simnet_events_tx().send(SimnetEvent::Aborted("Terminated due to inactivity.".to_string()));
                        break;
                    }
                }
            },
        }

        {
            if do_produce_block {
                svm_locker.confirm_current_block()?;
            }
        }
    }
    Ok(())
}

pub fn start_clock_runloop(mut slot_time: u64) -> (Receiver<ClockEvent>, Sender<ClockCommand>) {
    let (clock_event_tx, clock_event_rx) = unbounded::<ClockEvent>();
    let (clock_command_tx, clock_command_rx) = unbounded::<ClockCommand>();

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

    (clock_event_rx, clock_command_tx)
}

fn start_geyser_runloop(
    plugin_config_paths: Vec<PathBuf>,
    plugin_manager_commands_rx: Receiver<PluginManagerCommand>,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    geyser_events_rx: Receiver<GeyserEvent>,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let handle: JoinHandle<Result<(), String>> = hiro_system_kit::thread_named("Geyser Plugins Handler").spawn(move || {
        let mut indexing_enabled = false;

        #[cfg(feature = "geyser-plugin")]
        let mut plugin_manager = GeyserPluginManager::new();
        #[cfg(not(feature = "geyser-plugin"))]
        let mut plugin_manager = ();

        let mut surfpool_plugin_manager = vec![];

        #[cfg(feature = "geyser-plugin")]
        for plugin_config_path in plugin_config_paths.into_iter() {
            let plugin_manifest_location = FileLocation::from_path(plugin_config_path);
            let contents = plugin_manifest_location.read_content_as_utf8()?;
            let result: serde_json::Value = match json5::from_str(&contents) {
                Ok(res) => res,
                Err(e) => {
                    let error = format!("Unable to read manifest: {}", e);
                    let _ = simnet_events_tx.send(SimnetEvent::error(error.clone()));
                    return Err(error)
                }
            };
            let (plugin_name, plugin_dylib_path) = match (result.get("name").map(|p| p.as_str()), result.get("libpath").map(|p| p.as_str())) {
                (Some(Some(name)), Some(Some(libpath))) => (name, libpath),
                _ => {
                    let error = format!("Unable to retrieve dylib: {}", plugin_manifest_location);
                    let _ = simnet_events_tx.send(SimnetEvent::error(error.clone()));
                    return Err(error)
                }
            };

            let mut plugin_dylib_location = plugin_manifest_location.get_parent_location().expect("path invalid");
            plugin_dylib_location.append_path(&plugin_dylib_path).expect("path invalid");

            let (plugin, lib) = unsafe {
                let lib = match Library::new(&plugin_dylib_location.to_string()) {
                    Ok(lib) => lib,
                    Err(e) => {
                        let _ = simnet_events_tx.send(SimnetEvent::ErrorLog(Local::now(), format!("Unable to load plugin {}: {}", plugin_name, e.to_string())));
                        continue;
                    }
                };
                let constructor: Symbol<PluginConstructor> = lib
                    .get(b"_create_plugin")
                    .map_err(|e| format!("{}", e.to_string()))?;
                let plugin_raw = constructor();
                (Box::from_raw(plugin_raw), lib)
            };
            indexing_enabled = true;
            plugin_manager.plugins.push(LoadedGeyserPlugin::new(lib, plugin, Some(plugin_name.to_string())));
        }

        let ipc_router = RouterProxy::new();

        let err = loop {
            use agave_geyser_plugin_interface::geyser_plugin_interface::{ReplicaAccountInfoV3, ReplicaAccountInfoVersions};

            use crate::types::GeyserAccountUpdate;

            select! {
                recv(plugin_manager_commands_rx) -> msg => {
                    match msg {
                        Ok(event) => {
                            match event {
                                PluginManagerCommand::LoadConfig(uuid, config, notifier) => {
                                    let _ = subgraph_commands_tx.send(SubgraphCommand::CreateCollection(uuid, config.data.clone(), notifier));
                                    let mut plugin = SurfpoolSubgraphPlugin::default();

                                    let (server, ipc_token) = IpcOneShotServer::<IpcReceiver<DataIndexingCommand>>::new().expect("Failed to create IPC one-shot server.");
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
                                        let subgraph_rx = ipc_router.route_ipc_receiver_to_new_crossbeam_receiver::<DataIndexingCommand>(rx);
                                        let _ = subgraph_commands_tx.send(SubgraphCommand::ObserveCollection(subgraph_rx));
                                    };

                                    indexing_enabled = true;

                                    let plugin: Box<dyn GeyserPlugin> = Box::new(plugin);
                                    surfpool_plugin_manager.push(plugin);
                                    let _ = simnet_events_tx.send(SimnetEvent::PluginLoaded("surfpool-subgraph".into()));
                                }
                            }
                        },
                        Err(e) => {
                            break format!("Failed to read plugin manager command: {:?}", e);
                        },
                    }
                },
                recv(geyser_events_rx) -> msg => match msg {
                    Err(e) => {
                        break format!("Failed to read new transaction to send to Geyser plugin: {e}");
                    },
                    Ok(GeyserEvent::NotifyTransaction(transaction_with_status_meta, sanitized_transaction)) => {

                        if !indexing_enabled {
                            continue;
                        }

                        let transaction = match sanitized_transaction {
                            Some(tx) => tx,
                            None => {
                                let _ = simnet_events_tx.send(SimnetEvent::warn(format!("Unable to index sanitized transaction")));
                                continue;
                            }
                        };

                        let transaction_replica = ReplicaTransactionInfoV2 {
                            signature: transaction.signature(),
                            is_vote: false,
                            transaction: &transaction,
                            transaction_status_meta: &transaction_with_status_meta.meta,
                            index: 0
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.notify_transaction(ReplicaTransactionInfoVersions::V0_0_2(&transaction_replica), transaction_with_status_meta.slot) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: {:?}", e)));
                            };
                        }

                        #[cfg(feature = "geyser-plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.notify_transaction(ReplicaTransactionInfoVersions::V0_0_2(&transaction_replica), transaction_with_status_meta.slot) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: {:?}", e)));
                            };
                        }
                    }
                    Ok(GeyserEvent::UpdateAccount(account_update)) => {
                        let GeyserAccountUpdate {
                            pubkey,
                            account,
                            slot,
                            sanitized_transaction,
                            write_version,
                        } = account_update;

                        let account_replica = ReplicaAccountInfoV3 {
                            pubkey: pubkey.as_ref(),
                            lamports: account.lamports,
                            owner: account.owner.as_ref(),
                            executable: account.executable,
                            rent_epoch: account.rent_epoch,
                            data: account.data.as_ref(),
                            write_version,
                            txn: Some(&sanitized_transaction),
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.update_account(ReplicaAccountInfoVersions::V0_0_3(&account_replica), slot, false) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to update account in Geyser plugin: {:?}", e)));
                            }
                        }

                        #[cfg(feature = "geyser-plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.update_account(ReplicaAccountInfoVersions::V0_0_3(&account_replica), slot, false) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to update account in Geyser plugin: {:?}", e)));
                            }
                        }
                    }
                }
            }
        };
        Err(err)
    }).map_err(|e| format!("Failed to spawn Geyser Plugins Handler thread: {:?}", e))?;
    Ok(handle)
}

async fn start_rpc_servers_runloop(
    config: &SurfpoolConfig,
    simnet_commands_tx: &Sender<SimnetCommand>,
    svm_locker: SurfnetSvmLocker,
    remote_rpc_client: &Option<SurfnetRemoteClient>,
) -> Result<
    (
        Receiver<PluginManagerCommand>,
        JoinHandle<()>,
        JoinHandle<()>,
    ),
    String,
> {
    let (plugin_manager_commands_tx, plugin_manager_commands_rx) = unbounded();
    let simnet_events_tx = svm_locker.simnet_events_tx();

    let middleware = SurfpoolMiddleware::new(
        svm_locker,
        simnet_commands_tx,
        &plugin_manager_commands_tx,
        &config.rpc,
        remote_rpc_client,
    );

    let rpc_handle =
        start_http_rpc_server_runloop(config, middleware.clone(), simnet_events_tx.clone()).await?;
    let ws_handle = start_ws_rpc_server_runloop(config, middleware, simnet_events_tx).await?;
    Ok((plugin_manager_commands_rx, rpc_handle, ws_handle))
}

async fn start_http_rpc_server_runloop(
    config: &SurfpoolConfig,
    middleware: SurfpoolMiddleware,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<JoinHandle<()>, String> {
    let server_bind: SocketAddr = config
        .rpc
        .get_rpc_base_url()
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::accounts_scan::SurfpoolAccountsScanRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());
    io.extend_with(rpc::surfnet_cheatcodes::SurfnetCheatcodesRpc.to_delegate());
    io.extend_with(rpc::admin::SurfpoolAdminRpc.to_delegate());

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

            server.wait();
            let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
        })
        .map_err(|e| format!("Failed to spawn RPC Handler thread: {:?}", e))?;

    Ok(_handle)
}
async fn start_ws_rpc_server_runloop(
    config: &SurfpoolConfig,
    middleware: SurfpoolMiddleware,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<JoinHandle<()>, String> {
    let ws_server_bind: SocketAddr = config
        .rpc
        .get_ws_base_url()
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;

    let uid = std::sync::atomic::AtomicUsize::new(0);
    let ws_middleware = SurfpoolWebsocketMiddleware::new(middleware.clone(), None);

    let mut rpc_io = PubSubHandler::new(MetaIoHandler::with_middleware(ws_middleware));

    let _ws_handle = hiro_system_kit::thread_named("WebSocket RPC Handler")
        .spawn(move || {
            // The pubsub handler needs to be able to run async tasks, so we create a Tokio runtime here
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build Tokio runtime");

            let tokio_handle = runtime.handle();
            rpc_io.extend_with(
                rpc::ws::SurfpoolWsRpc {
                    uid,
                    signature_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    account_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    slot_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    logs_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    tokio_handle: tokio_handle.clone(),
                }
                .to_delegate(),
            );
            runtime.block_on(async move {
                let server = match WsServerBuilder::new(rpc_io)
                    .session_meta_extractor(move |ctx: &RequestContext| {
                        // Create meta from context + session
                        let runloop_context = RunloopContext {
                            id: None,
                            svm_locker: middleware.surfnet_svm.clone(),
                            simnet_commands_tx: middleware.simnet_commands_tx.clone(),
                            plugin_manager_commands_tx: middleware
                                .plugin_manager_commands_tx
                                .clone(),
                            remote_rpc_client: middleware.remote_rpc_client.clone(),
                        };
                        Some(SurfpoolWebsocketMeta::new(
                            runloop_context,
                            Some(Arc::new(Session::new(ctx.sender()))),
                        ))
                    })
                    .start(&ws_server_bind)
                {
                    Ok(server) => server,
                    Err(e) => {
                        let _ = simnet_events_tx.send(SimnetEvent::Aborted(format!(
                            "Failed to start WebSocket RPC server: {:?}",
                            e
                        )));
                        return;
                    }
                };
                // The server itself is blocking, so spawn it in a separate thread if needed
                tokio::task::spawn_blocking(move || {
                    server.wait().unwrap();
                })
                .await
                .ok();

                let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
            });
        })
        .map_err(|e| format!("Failed to spawn WebSocket RPC Handler thread: {:?}", e))?;
    Ok(_ws_handle)
}
