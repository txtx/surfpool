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

use actix_web::dev::ServerHandle;
use crossbeam::channel::{Select, Sender};
use indicatif::{MultiProgress, ProgressBar};
use log::{debug, error, info, warn};
use notify::{
    Config, Event, EventKind, RecursiveMode, Result as NotifyResult, Watcher,
    event::{CreateKind, DataChange, ModifyKind},
};
use serde::{Deserialize, Serialize};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_core::{start_local_surfnet, surfnet::svm::SurfnetSvm};
use surfpool_types::{SanitizedConfig, SimnetCommand, SimnetEvent, SubgraphEvent};
use txtx_core::{
    kit::{
        channel::Receiver, futures::future::join_all, helpers::fs::FileLocation,
        types::frontend::BlockEvent,
    },
    manifest::WorkspaceManifest,
    types::RunbookSources,
};
use txtx_gql::kit::{indexmap::IndexMap, types::frontend::LogLevel, uuid::Uuid};

use super::{Context, ExecuteRunbook, StartSimnet};
use crate::{
    http::start_subgraph_and_explorer_server,
    runbook::{execute_in_memory_runbook, execute_on_disk_runbook, handle_log_event},
    scaffold::{detect_program_frameworks, scaffold_iac_layout, scaffold_in_memory_iac},
    tui::{self, simnet::DisplayedUrl},
};

#[derive(Debug, Serialize, Deserialize)]
struct CheckVersionResponse {
    pub latest: String,
    pub deprecation_notice: Option<String>,
}

pub async fn handle_start_local_surfnet_command(
    cmd: StartSimnet,
    ctx: &Context,
) -> Result<(), String> {
    if !cmd.plugin_config_path.is_empty() && !cfg!(feature = "geyser_plugin") {
        return Err(
            "Recompile surfpool and enable the feature 'geyser_plugin' to load geyser plugins"
                .to_string(),
        );
    }

    // We start the simnet as soon as possible, as it needs to be ready for deployments
    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = crossbeam::channel::unbounded();
    let (subgraph_commands_tx, subgraph_commands_rx) = crossbeam::channel::unbounded();
    let (subgraph_events_tx, subgraph_events_rx) = crossbeam::channel::unbounded();
    let simnet_events_tx = surfnet_svm.simnet_events_tx.clone();

    // Check aidrop addresses
    let (mut airdrop_addresses, airdrop_events) = cmd.get_airdrop_addresses();

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

    // Allow overriding public-facing URLs via environment variables
    // This is useful when running behind a reverse proxy (e.g., Caddy, nginx)
    let rpc_url = std::env::var("SURFPOOL_PUBLIC_RPC_URL")
        .unwrap_or_else(|_| format!("http://{}", config.rpc.get_rpc_base_url()));
    let ws_url = std::env::var("SURFPOOL_PUBLIC_WS_URL")
        .unwrap_or_else(|_| format!("ws://{}", config.rpc.get_ws_base_url()));
    let studio_url = std::env::var("SURFPOOL_PUBLIC_STUDIO_URL")
        .unwrap_or_else(|_| format!("http://{}", studio_binding_address));

    let graphql_query_route_url = format!("{}/workspace/v1/graphql", studio_url);
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

    let subgraph_database_path = cmd.subgraph_db.as_deref().unwrap_or(":memory:");
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
            error!("Failed to start subgraph and explorer server: {}", e);
            let _ = simnet_events_tx.send(SimnetEvent::warn(format!(
                "Failed to start subgraph and explorer server: {}",
                e
            )));
            let _ = simnet_events_tx.send(SimnetEvent::info("Continuing with simnet startup..."));
            None
        }
    };

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
                error!("Simnet exited with error: {e}");
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

    for event in airdrop_events {
        let _ = simnet_events_tx.send(event);
    }

    let mut deploy_progress_rx = vec![];
    if !cmd.no_deploy {
        match write_and_execute_iac(&cmd, &simnet_events_tx).await {
            Ok(rx) => deploy_progress_rx.push(rx),
            Err(e) => {
                let _ = simnet_events_tx.send(SimnetEvent::warn(format!(
                    "Automatic protocol deployment failed: {e}"
                )));
            }
        }
    };

    // Non blocking check for new versions
    #[cfg(feature = "version_check")]
    {
        let local_version = env!("CARGO_PKG_VERSION");
        let response = txtx_gql::kit::reqwest::get(format!(
            "{}/api/versions?v=/{}",
            super::DEFAULT_CLOUD_URL,
            local_version
        ))
        .await;
        if let Ok(response) = response {
            if let Ok(body) = response.json::<CheckVersionResponse>().await {
                if let Some(deprecation_notice) = body.deprecation_notice {
                    let _ =
                        simnet_events_tx.send(SimnetEvent::warn(deprecation_notice.to_string()));
                }
            }
        }
    }

    let cmd_cc = cmd.clone();
    let ctx_cc = ctx.clone();

    let runloop_terminator = Arc::new(AtomicBool::new(false));

    let _ = start_service(
        cmd_cc,
        simnet_events_rx,
        subgraph_events_rx,
        deploy_progress_rx,
        simnet_commands_tx,
        breaker,
        sanitized_config,
        explorer_handle,
        ctx_cc,
        Some(runloop_terminator),
    )
    .await;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_service(
    cmd: StartSimnet,
    simnet_events_rx: Receiver<SimnetEvent>,
    subgraph_events_rx: Receiver<SubgraphEvent>,
    deploy_progress_rx: Vec<Receiver<BlockEvent>>,
    simnet_commands_tx: Sender<SimnetCommand>,
    breaker: Option<Keypair>,
    sanitized_config: SanitizedConfig,
    explorer_handle: Option<ServerHandle>,
    _ctx: Context,
    runloop_terminator: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    let displayed_url = if cmd.no_studio {
        DisplayedUrl::Datasource(sanitized_config)
    } else {
        DisplayedUrl::Studio(sanitized_config)
    };

    // Start frontend - kept on main thread
    if cmd.daemon || cmd.no_tui {
        log_events(
            simnet_events_rx,
            subgraph_events_rx,
            cmd.debug,
            deploy_progress_rx,
            simnet_commands_tx,
            runloop_terminator.unwrap(),
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
    simnet_commands_tx: Sender<SimnetCommand>,
    runloop_terminator: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut deployment_completed = false;
    let do_stop_loop = runloop_terminator.clone();
    ctrlc::set_handler(move || {
        do_stop_loop.store(true, Ordering::Relaxed);
    })
    .expect("Error setting Ctrl-C handler");

    let log_filter = if include_debug_logs {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };
    let mut active_spinners: IndexMap<Uuid, ProgressBar> = IndexMap::new();
    let mut multi_progress = MultiProgress::new();

    loop {
        if runloop_terminator.load(Ordering::Relaxed) {
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
                        info!("{}", event.account_update_msg());
                    }
                    SimnetEvent::PluginLoaded(_) => {
                        info!("{}", event.plugin_loaded_msg());
                    }
                    SimnetEvent::EpochInfoUpdate(_) => {
                        info!("{}", event.epoch_info_update_msg());
                    }
                    SimnetEvent::SystemClockUpdated(_) => {
                        if include_debug_logs {
                            info!("{}", event.clock_update_msg());
                        }
                    }
                    SimnetEvent::ClockUpdate(_) => {}
                    SimnetEvent::ErrorLog(_dt, log) => {
                        error!("{}", log);
                    }
                    SimnetEvent::InfoLog(_dt, log) => {
                        info!("{}", log);
                    }
                    SimnetEvent::DebugLog(_dt, log) => {
                        if include_debug_logs {
                            debug!("{}", log);
                        }
                    }
                    SimnetEvent::WarnLog(_dt, log) => {
                        warn!("{}", log);
                    }
                    SimnetEvent::TransactionReceived(_dt, transaction) => {
                        if deployment_completed {
                            info!("Transaction received {}", transaction.signatures[0]);
                        }
                    }
                    SimnetEvent::TransactionProcessed(_dt, meta, _err) => {
                        if deployment_completed {
                            info!("Transaction processed {}", meta.signature);
                            for log in meta.logs {
                                info!("{}", log);
                            }
                        }
                    }
                    SimnetEvent::BlockHashExpired => {}
                    SimnetEvent::Aborted(error) => {
                        error!("{}", error);
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
                            "Profiled [{}]: {} CUs",
                            tag, result.transaction_profile.compute_units_consumed
                        );
                    }
                    SimnetEvent::RunbookStarted(runbook_id) => {
                        deployment_completed = false;
                        info!("Runbook '{}' execution started", runbook_id);
                        let _ = simnet_commands_tx
                            .send(SimnetCommand::StartRunbookExecution(runbook_id));
                    }
                    SimnetEvent::RunbookCompleted(runbook_id, errors) => {
                        deployment_completed = true;
                        info!("Runbook '{}' execution completed", runbook_id);
                        let _ = simnet_commands_tx
                            .send(SimnetCommand::CompleteRunbookExecution(runbook_id, errors));
                    }
                },
                Err(_e) => {
                    break;
                }
            },
            1 => match oper.recv(&subgraph_events_rx) {
                Ok(event) => match event {
                    SubgraphEvent::ErrorLog(_dt, log) => {
                        error!("{}", log);
                    }
                    SubgraphEvent::InfoLog(_dt, log) => {
                        info!("{}", log);
                    }
                    SubgraphEvent::DebugLog(_dt, log) => {
                        if include_debug_logs {
                            info!("{}", log);
                        }
                    }
                    SubgraphEvent::WarnLog(_dt, log) => {
                        warn!("{}", log);
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
                Ok(event) => {
                    if let BlockEvent::LogEvent(log) = event {
                        handle_log_event(
                            &mut multi_progress,
                            log,
                            &log_filter,
                            &mut active_spinners,
                            false,
                        )
                    }
                }
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
        let txtx_manifest_exists = txtx_manifest_location.exists();
        let do_write_scaffold = !cmd.autopilot && !txtx_manifest_exists;
        if do_write_scaffold {
            // Scaffold IaC
            scaffold_iac_layout(
                &framework,
                &programs,
                &base_location,
                cmd.skip_runbook_generation_prompts,
            )?;
        }

        // If there were existing on-disk runbooks, we'll execute those instead of in-memory ones
        // If there were no existing runbooks and the user requested autopilot, we'll generate and execute in-memory runbooks
        // If there were no existing runbooks and the user did not request autopilot, we'll generate and execute on-disk runbooks
        let do_execute_in_memory_runbooks = cmd.autopilot && !txtx_manifest_exists;

        let mut on_disk_runbook_data = None;
        let mut in_memory_runbook_data = None;
        if do_execute_in_memory_runbooks {
            in_memory_runbook_data = Some(scaffold_in_memory_iac(&framework, &programs)?);
        } else {
            let runbooks_ids_to_execute = cmd.runbooks.clone();
            on_disk_runbook_data = Some((txtx_manifest_location.clone(), runbooks_ids_to_execute));
        };
        let futures = assemble_runbook_execution_futures(
            &progress_tx,
            &simnet_events_tx,
            &on_disk_runbook_data,
            &in_memory_runbook_data,
        );

        let simnet_events_tx = simnet_events_tx.clone();
        let _handle = hiro_system_kit::thread_named("Deployment Runbook Executions")
            .spawn(move || {
                let _ = hiro_system_kit::nestable_block_on(join_all(futures.into_iter()));
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

                        let futures = assemble_runbook_execution_futures(
                            &progress_tx,
                            &simnet_events_tx,
                            &on_disk_runbook_data,
                            &in_memory_runbook_data,
                        );

                        let _ = hiro_system_kit::nestable_block_on(join_all(futures));
                    }
                    Ok::<(), String>(())
                })
                .map_err(|e| format!("Thread to watch filesystem exited: {}", e))?;
        }
    }
    Ok(progress_rx)
}

fn assemble_runbook_execution_futures(
    progress_tx: &Sender<BlockEvent>,
    simnet_events_tx: &Sender<SimnetEvent>,
    on_disk_runbook_data: &Option<(FileLocation, Vec<String>)>,
    in_memory_runbook_data: &Option<(String, RunbookSources, WorkspaceManifest)>,
) -> Vec<std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>> {
    let mut futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>,
    > = vec![];
    let simnet_events_tx_copy = simnet_events_tx.clone();
    let do_setup_logger = false;
    if let Some((runbook_id, runbook_sources, manifest)) = in_memory_runbook_data {
        // Clone owned values so all arguments are 'static
        let runbook_id_owned = runbook_id.clone();
        let runbook_sources_owned = runbook_sources.clone();
        let manifest_owned = manifest.clone();
        futures.push(Box::pin(execute_in_memory_runbook(
            progress_tx.clone(),
            simnet_events_tx_copy.clone(),
            ExecuteRunbook::default_localnet(&runbook_id_owned),
            do_setup_logger,
            runbook_id_owned,
            manifest_owned,
            runbook_sources_owned,
        )));
    }

    if let Some((file_location, runbooks_ids_to_execute)) = on_disk_runbook_data {
        let file_location_owned = file_location.clone();
        let runbooks_ids_to_execute_owned = runbooks_ids_to_execute.clone();
        let simnet_events_tx_copy = simnet_events_tx.clone();
        for runbook_id in runbooks_ids_to_execute_owned.iter() {
            let runbook_id_owned = runbook_id.clone();
            futures.push(Box::pin(execute_on_disk_runbook(
                progress_tx.clone(),
                simnet_events_tx_copy.clone(),
                ExecuteRunbook::default_localnet(&runbook_id_owned)
                    .with_manifest_path(file_location_owned.to_string()),
                do_setup_logger,
            )));
        }
    }
    futures
}
