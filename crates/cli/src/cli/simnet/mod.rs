use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::sleep,
    time::Duration,
};

use crate::{
    http::start_server,
    runbook::execute_runbook,
    scaffold::{detect_program_frameworks, scaffold_iac_layout},
    tui,
};

use super::{Context, StartSimnet, DEFAULT_EXPLORER_PORT};
use crossbeam::channel::Select;
use notify::{
    event::{CreateKind, DataChange, ModifyKind},
    Config, Event, EventKind, RecursiveMode, Result as NotifyResult, Watcher,
};
use surfpool_core::{
    solana_sdk::{
        pubkey::Pubkey,
        signature::Keypair,
        signer::{EncodableKey, Signer},
    },
    start_simnet,
    types::{
        RpcConfig, RunloopTriggerMode, SimnetConfig, SimnetEvent, SubgraphConfig, SubgraphEvent,
        SurfpoolConfig,
    },
};
use txtx_core::kit::{
    channel::Receiver, futures::future::join_all, helpers::fs::FileLocation,
    types::frontend::BlockEvent,
};

pub async fn handle_start_simnet_command(cmd: &StartSimnet, ctx: &Context) -> Result<(), String> {
    // Check aidrop addresses
    let mut airdrop_addresses = vec![];
    for address in cmd.airdrop_addresses.iter() {
        let pubkey = Pubkey::from_str(&address).map_err(|e| e.to_string())?;
        airdrop_addresses.push(pubkey);
    }
    let breaker = if cmd.no_tui {
        None
    } else {
        let keypair = Keypair::new();
        airdrop_addresses.push(keypair.pubkey());
        Some(keypair)
    };

    for keypair_path in cmd.airdrop_keypair_path.iter() {
        let resolved = if keypair_path.starts_with("~") {
            format!(
                "{}{}",
                dirs::home_dir().unwrap().display(),
                keypair_path[1..].to_string()
            )
        } else {
            keypair_path.clone()
        };
        let path = PathBuf::from(resolved);
        let pubkey = Keypair::read_from_file(&path)
            .map_err(|e| format!("unable to read {}: {}", path.display(), e.to_string()))?
            .pubkey();
        airdrop_addresses.push(pubkey);
    }

    // Build config
    let config = SurfpoolConfig {
        rpc: RpcConfig {
            remote_rpc_url: cmd.rpc_url.clone(),
            bind_port: cmd.simnet_port,
            bind_host: cmd.network_host.clone(),
        },
        simnet: SimnetConfig {
            remote_rpc_url: cmd.rpc_url.clone(),
            slot_time: cmd.slot_time,
            runloop_trigger_mode: RunloopTriggerMode::Clock,
            airdrop_addresses,
            airdrop_token_amount: cmd.airdrop_token_amount,
        },
        subgraph: SubgraphConfig {},
        plugin_config_path: cmd
            .plugin_config_path
            .iter()
            .map(|f| PathBuf::from(f))
            .collect::<Vec<_>>(),
    };
    let remote_rpc_url = config.rpc.remote_rpc_url.clone();
    let local_rpc_url = config.rpc.get_socket_address();

    // We start the simnet as soon as possible, as it needs to be ready for deployments
    let (simnet_commands_tx, simnet_commands_rx) = crossbeam::channel::unbounded();
    let (simnet_events_tx, simnet_events_rx) = crossbeam::channel::unbounded();
    let (subgraph_commands_tx, subgraph_commands_rx) = crossbeam::channel::unbounded();
    let (subgraph_events_tx, subgraph_events_rx) = crossbeam::channel::unbounded();

    let network_binding = format!("{}:{}", cmd.network_host, DEFAULT_EXPLORER_PORT);
    let explorer_handle = start_server(
        network_binding,
        config.clone(),
        subgraph_events_tx.clone(),
        subgraph_commands_rx,
        &ctx.clone(),
    )
    .await
    .map_err(|e| format!("{}", e.to_string()))?;

    let ctx_copy = ctx.clone();
    let simnet_commands_tx_copy = simnet_commands_tx.clone();
    let config_copy = config.clone();
    let _handle = hiro_system_kit::thread_named("simnet")
        .spawn(move || {
            let future = start_simnet(
                config_copy,
                subgraph_commands_tx,
                simnet_events_tx,
                simnet_commands_tx_copy,
                simnet_commands_rx,
            );
            if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                error!(ctx_copy.expect_logger(), "{e}");
                sleep(Duration::from_millis(500));
                std::process::exit(1);
            }
            Ok::<(), String>(())
        })
        .map_err(|e| format!("{}", e))?;

    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Aborted(error)) => return Err(error),
            Ok(SimnetEvent::Shutdown) => return Ok(()),
            Ok(SimnetEvent::Ready) => break,
            _other => continue,
        }
    }

    let mut deploy_progress_rx = vec![];
    if !cmd.no_deploy {
        // Are we in a project directory?
        let deployment = match detect_program_frameworks(&cmd.manifest_path).await {
            Err(e) => {
                error!(ctx.expect_logger(), "{}", e);
                None
            }
            Ok(deployment) => deployment,
        };

        let (progress_tx, progress_rx) = crossbeam::channel::unbounded();
        deploy_progress_rx.push(progress_rx);

        if let Some((_framework, programs)) = deployment {
            // Is infrastructure-as-code (IaC) already setup?
            let base_location =
                FileLocation::from_path_string(&cmd.manifest_path)?.get_parent_location()?;
            let mut txtx_manifest_location = base_location.clone();
            txtx_manifest_location.append_path("txtx.yml")?;
            if !txtx_manifest_location.exists() {
                // Scaffold IaC
                scaffold_iac_layout(programs, &base_location)?;
            }

            let mut futures = vec![];
            let runbooks_ids_to_execute = cmd.runbooks.clone();
            for runbook_id in runbooks_ids_to_execute.iter() {
                futures.push(execute_runbook(
                    runbook_id.clone(),
                    progress_tx.clone(),
                    txtx_manifest_location.clone(),
                ));
            }

            let _handle = hiro_system_kit::thread_named("simnet")
                .spawn(move || {
                    let _ = hiro_system_kit::nestable_block_on(join_all(futures));
                    Ok::<(), String>(())
                })
                .map_err(|e| format!("{}", e))?;

            if cmd.watch {
                let _handle = hiro_system_kit::thread_named("watch filesystem")
                    .spawn(move || {
                        let mut target_path = base_location.clone();
                        let _ = target_path.append_path("target");
                        let _ = target_path.append_path("deploy");
                        let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
                        let mut watcher =
                            notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
                        watcher
                            .watch(
                                Path::new(&target_path.to_string()),
                                RecursiveMode::NonRecursive,
                            )
                            .map_err(|e| e.to_string())?;
                        let _ = watcher.configure(
                            Config::default()
                                .with_poll_interval(Duration::from_secs(1))
                                .with_compare_contents(true),
                        );
                        for res in rx {
                            // Disregard any event that would not create or modify a .so file
                            let mut found_candidates = false;
                            match res {
                                Ok(Event {
                                    kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                                    paths,
                                    attrs: _,
                                })
                                | Ok(Event {
                                    kind: EventKind::Create(CreateKind::File),
                                    paths,
                                    attrs: _,
                                }) => {
                                    for path in paths.iter() {
                                        if path.to_string_lossy().ends_with(".so") {
                                            found_candidates = true;
                                        }
                                    }
                                }
                                _ => continue,
                            }

                            if !found_candidates {
                                continue;
                            }

                            let mut futures = vec![];
                            for runbook_id in runbooks_ids_to_execute.iter() {
                                futures.push(execute_runbook(
                                    runbook_id.clone(),
                                    progress_tx.clone(),
                                    txtx_manifest_location.clone(),
                                ));
                            }
                            let _ = hiro_system_kit::nestable_block_on(join_all(futures));
                        }
                        Ok::<(), String>(())
                    })
                    .map_err(|e| format!("{}", e))
                    .unwrap();
            }
        }

        // clean-up state
    };

    // Start frontend - kept on main thread
    if cmd.no_tui {
        log_events(
            simnet_events_rx,
            subgraph_events_rx,
            cmd.debug,
            deploy_progress_rx,
            ctx,
        )?;
    } else {
        tui::simnet::start_app(
            simnet_events_rx,
            simnet_commands_tx,
            cmd.debug,
            deploy_progress_rx,
            &remote_rpc_url,
            &local_rpc_url,
            breaker,
        )
        .map_err(|e| format!("{}", e))?;
    }
    let _ = explorer_handle.stop(true);
    Ok(())
}

fn log_events(
    simnet_events_rx: Receiver<SimnetEvent>,
    subgraph_events_rx: Receiver<SubgraphEvent>,
    include_debug_logs: bool,
    deploy_progress_rx: Vec<Receiver<BlockEvent>>,
    ctx: &Context,
) -> Result<(), String> {
    let mut deployment_completed = false;
    let stop_loop = Arc::new(AtomicBool::new(false));
    let do_stop_loop = stop_loop.clone();
    ctrlc::set_handler(move || {
        stop_loop.store(true, Ordering::Relaxed);
    })
    .expect("Error setting Ctrl-C handler");

    loop {
        if do_stop_loop.load(Ordering::Relaxed) {
            break;
        }
        let mut selector = Select::new();
        let mut handles = vec![];

        selector.recv(&simnet_events_rx);
        selector.recv(&subgraph_events_rx);

        if !deployment_completed {
            for rx in deploy_progress_rx.iter() {
                handles.push(selector.recv(rx));
            }
        }

        let oper = selector.select();

        match oper.index() {
            0 => match oper.recv(&simnet_events_rx) {
                Ok(event) => match event {
                    SimnetEvent::AccountUpdate(_dt, account) => {
                        info!(
                            ctx.expect_logger(),
                            "Account {} retrieved from Mainnet", account
                        );
                    }
                    SimnetEvent::PluginLoaded(plugin_name) => {
                        info!(
                            ctx.expect_logger(),
                            "Plugin {} successfully loaded", plugin_name
                        );
                    }
                    SimnetEvent::EpochInfoUpdate(epoch_info) => {
                        info!(
                            ctx.expect_logger(),
                            "Connection established. Epoch {}, Slot {}.",
                            epoch_info.epoch,
                            epoch_info.slot_index
                        );
                    }
                    SimnetEvent::ClockUpdate(clock) => {
                        if include_debug_logs {
                            info!(
                                ctx.expect_logger(),
                                "Clock ticking (epoch {}, slot {})", clock.epoch, clock.slot
                            );
                        }
                    }
                    SimnetEvent::ErrorLog(_dt, log) => {
                        error!(ctx.expect_logger(), "{}", log);
                    }
                    SimnetEvent::InfoLog(_dt, log) => {
                        info!(ctx.expect_logger(), "{}", log);
                    }
                    SimnetEvent::DebugLog(_dt, log) => {
                        if include_debug_logs {
                            info!(ctx.expect_logger(), "{}", log);
                        }
                    }
                    SimnetEvent::WarnLog(_dt, log) => {
                        warn!(ctx.expect_logger(), "{}", log);
                    }
                    SimnetEvent::TransactionSimulated(_dt, transaction) => {
                        if deployment_completed {
                            info!(
                                ctx.expect_logger(),
                                "Transaction received {}", transaction.signatures[0]
                            );
                        }
                    }
                    SimnetEvent::TransactionProcessed(_dt, _meta, _err) => {
                        if deployment_completed {
                            info!(
                                ctx.expect_logger(),
                                "Transaction processed {}", _meta.signature
                            );
                        }
                    }
                    SimnetEvent::BlockHashExpired => {}
                    SimnetEvent::Aborted(error) => {
                        error!(ctx.expect_logger(), "{}", error);
                        return Err(error);
                    }
                    SimnetEvent::Ready => {}
                    SimnetEvent::Shutdown => {
                        break;
                    }
                },
                Err(_e) => {
                    break;
                }
            },
            1 => match oper.recv(&subgraph_events_rx) {
                Ok(event) => match event {
                    SubgraphEvent::ErrorLog(_dt, log) => {
                        error!(ctx.expect_logger(), "{}", log);
                    }
                    SubgraphEvent::InfoLog(_dt, log) => {
                        info!(ctx.expect_logger(), "{}", log);
                    }
                    SubgraphEvent::DebugLog(_dt, log) => {
                        if include_debug_logs {
                            info!(ctx.expect_logger(), "{}", log);
                        }
                    }
                    SubgraphEvent::WarnLog(_dt, log) => {
                        warn!(ctx.expect_logger(), "{}", log);
                    }
                    SubgraphEvent::EndpointReady => {}
                    SubgraphEvent::Shutdown => {
                        break;
                    }
                },
                Err(_e) => {
                    break;
                }
            },
            i => match oper.recv(&deploy_progress_rx[i - 2]) {
                Ok(event) => match event {
                    BlockEvent::UpdateProgressBarStatus(update) => {
                        debug!(
                            ctx.expect_logger(),
                            "{}",
                            format!(
                                "{}: {}",
                                update.new_status.status, update.new_status.message
                            )
                        );
                    }
                    BlockEvent::RunbookCompleted => {
                        info!(ctx.expect_logger(), "{}", format!("Deployment executed",));
                    }
                    _ => {}
                },
                Err(_e) => {
                    deployment_completed = true;
                }
            },
        }
    }
    Ok(())
}
