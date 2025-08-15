use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::sleep,
    time::Duration,
};

use crossbeam::channel::{Select, Sender};
use notify::{
    Config, Event, EventKind, RecursiveMode, Result as NotifyResult, Watcher,
    event::{CreateKind, DataChange, ModifyKind},
};
use serde::{Deserialize, Serialize};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_core::{start_local_surfnet, surfnet::svm::SurfnetSvm};
use surfpool_types::{SanitizedConfig, SimnetEvent, SubgraphEvent};
use txtx_core::kit::{
    channel::Receiver, futures::future::join_all, helpers::fs::FileLocation,
    types::frontend::BlockEvent,
};
use txtx_gql::kit::reqwest;

use super::{Context, DEFAULT_CLOUD_URL, ExecuteRunbook, StartSimnet};
use crate::{
    http::start_subgraph_and_explorer_server,
    runbook::execute_runbook,
    scaffold::{detect_program_frameworks, scaffold_iac_layout},
    tui::{self, simnet::DisplayedUrl},
};

#[derive(Debug, Serialize, Deserialize)]
struct CheckVersionResponse {
    pub latest: String,
    pub deprecation_notice: Option<String>,
}

pub async fn handle_start_local_surfnet_command(
    cmd: &StartSimnet,
    ctx: &Context,
) -> Result<(), String> {
    // We start the simnet as soon as possible, as it needs to be ready for deployments
    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = crossbeam::channel::unbounded();
    let (subgraph_commands_tx, subgraph_commands_rx) = crossbeam::channel::unbounded();
    let (subgraph_events_tx, subgraph_events_rx) = crossbeam::channel::unbounded();
    let simnet_events_tx = surfnet_svm.simnet_events_tx.clone();

    // Check aidrop addresses
    let (mut airdrop_addresses, airdrop_errors) = cmd.get_airdrop_addresses();

    let breaker = if cmd.no_tui {
        None
    } else {
        let keypair = Keypair::new();
        airdrop_addresses.push(keypair.pubkey());
        Some(keypair)
    };

    // Build config
    let config = cmd.surfpool_config(airdrop_addresses);

    let studio_binding_address = config.studio.get_studio_base_url();
    let rpc_url = format!("http://{}", config.rpc.get_rpc_base_url());
    let ws_url = format!("ws://{}", config.rpc.get_ws_base_url());
    let studio_url = format!("http://{}", studio_binding_address);
    let graphql_query_route_url = format!("{}/gql/v1/graphql", studio_url);
    let rpc_datasource_url = config.simnets[0].get_sanitized_datasource_url();

    let sanitized_config = SanitizedConfig {
        rpc_url,
        ws_url,
        rpc_datasource_url,
        studio_url,
        graphql_query_route_url,
        version: env!("CARGO_PKG_VERSION").to_string(),
        workspace: None,
    };

    let subgraph_database_path = cmd
        .subgraph_database_path
        .as_ref()
        .map(|p| p.as_str())
        .unwrap_or(":memory:");
    let explorer_handle = match start_subgraph_and_explorer_server(
        studio_binding_address,
        subgraph_database_path,
        sanitized_config.clone(),
        subgraph_events_tx.clone(),
        subgraph_commands_rx,
        ctx,
    )
    .await
    {
        Ok((explorer_handle, _)) => Some(explorer_handle),
        Err(e) => {
            let _ = simnet_events_tx.send(SimnetEvent::warn(format!(
                "Failed to start subgraph {}server: {}",
                if !cmd.no_explorer {
                    "and explorer "
                } else {
                    ""
                },
                e
            )));
            let _ = simnet_events_tx.send(SimnetEvent::info("Continuing with simnet startup..."));
            None
        }
    };

    let ctx_copy = ctx.clone();
    let simnet_commands_tx_copy = simnet_commands_tx.clone();
    let config_copy = config.clone();

    let _handle = hiro_system_kit::thread_named("simnet")
        .spawn(move || {
            let future = start_local_surfnet(
                surfnet_svm,
                config_copy,
                subgraph_commands_tx,
                simnet_commands_tx_copy,
                simnet_commands_rx,
                geyser_events_rx,
            );
            if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                error!(ctx_copy.expect_logger(), "Simnet exited with error: {e}");
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
            Ok(SimnetEvent::Connected(_)) => break,
            Ok(SimnetEvent::Ready) => break,
            _other => continue,
        }
    }

    for error in airdrop_errors {
        let _ = simnet_events_tx.send(SimnetEvent::warn(error));
    }

    let mut deploy_progress_rx = vec![];
    if !cmd.no_deploy {
        match write_and_execute_iac(cmd, &simnet_events_tx).await {
            Ok(rx) => deploy_progress_rx.push(rx),
            Err(e) => {
                let _ = simnet_events_tx.send(SimnetEvent::warn(format!(
                    "Automatic protocol deployment failed: {e}"
                )));
            }
        }
    };

    // Non blocking check for new versions
    let local_version = env!("CARGO_PKG_VERSION");
    let response = reqwest::get(format!(
        "{}/api/versions?v=/{}",
        DEFAULT_CLOUD_URL, local_version
    ))
    .await;
    if let Ok(response) = response {
        if let Ok(body) = response.json::<CheckVersionResponse>().await {
            if let Some(deprecation_notice) = body.deprecation_notice {
                let _ = simnet_events_tx.send(SimnetEvent::warn(deprecation_notice.to_string()));
            }
        }
    }

    let displayed_url = if cmd.no_studio {
        DisplayedUrl::Datasource(sanitized_config)
    } else {
        DisplayedUrl::Studio(sanitized_config)
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
            displayed_url,
            breaker,
        )
        .map_err(|e| format!("{}", e))?;
    }
    if let Some(explorer_handle) = explorer_handle {
        let _ = explorer_handle.stop(true).await;
    }
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
                    SimnetEvent::AccountUpdate(_dt, _) => {
                        info!(ctx.expect_logger(), "{}", event.account_update_msg());
                    }
                    SimnetEvent::PluginLoaded(_) => {
                        info!(ctx.expect_logger(), "{}", event.plugin_loaded_msg());
                    }
                    SimnetEvent::EpochInfoUpdate(_) => {
                        info!(ctx.expect_logger(), "{}", event.epoch_info_update_msg());
                    }
                    SimnetEvent::SystemClockUpdated(_) => {
                        if include_debug_logs {
                            info!(ctx.expect_logger(), "{}", event.clock_update_msg());
                        }
                    }
                    SimnetEvent::ClockUpdate(_) => {}
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
                    SimnetEvent::TransactionReceived(_dt, transaction) => {
                        if deployment_completed {
                            info!(
                                ctx.expect_logger(),
                                "Transaction received {}", transaction.signatures[0]
                            );
                        }
                    }
                    SimnetEvent::TransactionProcessed(_dt, meta, _err) => {
                        if deployment_completed {
                            info!(
                                ctx.expect_logger(),
                                "Transaction processed {}", meta.signature
                            );
                            for log in meta.logs {
                                info!(ctx.expect_logger(), "{}", log);
                            }
                        }
                    }
                    SimnetEvent::BlockHashExpired => {}
                    SimnetEvent::Aborted(error) => {
                        error!(ctx.expect_logger(), "{}", error);
                        return Err(error);
                    }
                    SimnetEvent::Ready => {}
                    SimnetEvent::Connected(_rpc_url) => {}
                    SimnetEvent::Shutdown => {
                        break;
                    }
                    SimnetEvent::TaggedProfile {
                        result,
                        tag,
                        timestamp: _,
                    } => {
                        info!(
                            ctx.expect_logger(),
                            "Profiled [{}]: {} CUs",
                            tag,
                            result.transaction_profile.compute_units_consumed
                        );
                    }
                    SimnetEvent::RunbookStarted(runbook_id) => {
                        deployment_completed = false;
                        info!(
                            ctx.expect_logger(),
                            "Runbook '{}' execution started", runbook_id
                        );
                    }
                    SimnetEvent::RunbookCompleted(runbook_id) => {
                        deployment_completed = true;
                        info!(
                            ctx.expect_logger(),
                            "Runbook '{}' execution completed", runbook_id
                        );
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

async fn write_and_execute_iac(
    cmd: &StartSimnet,
    simnet_events_tx: &Sender<SimnetEvent>,
) -> Result<Receiver<BlockEvent>, String> {
    // Are we in a project directory?
    let deployment = detect_program_frameworks(&cmd.manifest_path)
        .await
        .map_err(|e| format!("Failed to detect project framework: {}", e))?;

    let (progress_tx, progress_rx) = crossbeam::channel::unbounded();

    if let Some((framework, programs)) = deployment {
        // Is infrastructure-as-code (IaC) already setup?
        let base_location =
            FileLocation::from_path_string(&cmd.manifest_path)?.get_parent_location()?;
        let mut txtx_manifest_location = base_location.clone();
        txtx_manifest_location.append_path("txtx.yml")?;
        if !txtx_manifest_location.exists() {
            // Scaffold IaC
            scaffold_iac_layout(&framework, programs, &base_location)?;
        }

        let mut futures = vec![];
        let runbooks_ids_to_execute = cmd.runbooks.clone();
        let simnet_events_tx_copy = simnet_events_tx.clone();
        for runbook_id in runbooks_ids_to_execute.iter() {
            futures.push(execute_runbook(
                progress_tx.clone(),
                simnet_events_tx_copy.clone(),
                ExecuteRunbook::default_localnet(runbook_id)
                    .with_manifest_path(txtx_manifest_location.to_string()),
            ));
        }

        let simnet_events_tx = simnet_events_tx.clone();
        let _handle = hiro_system_kit::thread_named("Deployment Runbook Executions")
            .spawn(move || {
                let _ = hiro_system_kit::nestable_block_on(join_all(futures));
                Ok::<(), String>(())
            })
            .map_err(|e| format!("Thread to execute runbooks exited: {}", e))?;

        if cmd.watch {
            let _handle = hiro_system_kit::thread_named("Watch Filesystem")
                .spawn(move || {
                    let mut target_path = base_location.clone();
                    let _ = target_path.append_path("target");
                    let _ = target_path.append_path("deploy");
                    let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
                    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
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
                                progress_tx.clone(),
                                simnet_events_tx.clone(),
                                ExecuteRunbook::default_localnet(runbook_id)
                                    .with_manifest_path(txtx_manifest_location.to_string()),
                            ));
                        }
                        let _ = hiro_system_kit::nestable_block_on(join_all(futures));
                    }
                    Ok::<(), String>(())
                })
                .map_err(|e| format!("Thread to watch filesystem exited: {}", e))?;
        }
    }
    Ok(progress_rx)
}
