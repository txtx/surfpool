#![allow(unused_imports, dead_code, unused_mut, unused_variables)]

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{JoinHandle, sleep},
    time::{Duration, Instant},
};

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaBlockInfoV4, ReplicaBlockInfoVersions, ReplicaEntryInfoV2,
    ReplicaEntryInfoVersions, ReplicaTransactionInfoV3, ReplicaTransactionInfoVersions, SlotStatus,
};
use chrono::{Local, Utc};
use crossbeam_channel::{Receiver, Sender, unbounded};
use ipc_channel::{
    ipc::{IpcOneShotServer, IpcReceiver},
    router::RouterProxy,
};
use itertools::Itertools;
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use jsonrpc_pubsub::{PubSubHandler, Session};
use jsonrpc_ws_server::{RequestContext, ServerBuilder as WsServerBuilder};
use libloading::{Library, Symbol};
use serde::Serialize;
use solana_commitment_config::CommitmentConfig;
#[cfg(feature = "geyser_plugin")]
use solana_geyser_plugin_manager::geyser_plugin_manager::{
    GeyserPluginManager, LoadedGeyserPlugin,
};
use solana_message::SimpleAddressLoader;
use solana_transaction::sanitized::{MessageHash, SanitizedTransaction};
use solana_transaction_status::RewardsAndNumPartitions;
#[cfg(feature = "subgraph")]
use surfpool_subgraph::SurfpoolSubgraphPlugin;
use surfpool_types::{
    BlockProductionMode, ClockCommand, ClockEvent, DEFAULT_MAINNET_RPC_URL, DataIndexingCommand,
    SimnetCommand, SimnetConfig, SimnetEvent, SubgraphCommand, SubgraphPluginConfig,
    SurfpoolConfig,
};
type PluginConstructor = unsafe fn() -> *mut dyn GeyserPlugin;
use txtx_addon_kit::helpers::fs::FileLocation;

use crate::{
    PluginManagerCommand,
    rpc::{
        self, RunloopContext, SurfpoolMiddleware, SurfpoolWebsocketMeta,
        SurfpoolWebsocketMiddleware, accounts_data::AccountsData, accounts_scan::AccountsScan,
        admin::AdminRpc, bank_data::BankData, full::Full, minimal::Minimal,
        surfnet_cheatcodes::SurfnetCheatcodes, ws::Rpc,
    },
    surfnet::{GeyserEvent, locker::SurfnetSvmLocker, remote::SurfnetRemoteClient},
};

const BLOCKHASH_SLOT_TTL: u64 = 75;

/// Checks if a port is available for binding.
fn check_port_availability(addr: SocketAddr, server_type: &str) -> Result<(), String> {
    match std::net::TcpListener::bind(addr) {
        Ok(_listener) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let msg = format!(
                "{} port {} is already in use. Try --port or --ws-port to use a different port.",
                server_type,
                addr.port()
            );
            eprintln!("Error: {}", msg);
            Err(msg)
        }
        Err(e) => {
            let msg = format!("Failed to bind {} server to {}: {}", server_type, addr, e);
            eprintln!("Error: {}", msg);
            Err(msg)
        }
    }
}

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

    let remote_rpc_client = match simnet.offline_mode {
        true => None,
        false => SurfnetRemoteClient::new_unsafe(
            simnet
                .remote_rpc_url
                .as_ref()
                .unwrap_or(&DEFAULT_MAINNET_RPC_URL.to_string()),
        ),
    };

    svm_locker
        .initialize(
            simnet.slot_time,
            &remote_rpc_client,
            simnet.instruction_profiling_enabled,
            simnet.log_bytes_limit,
        )
        .await?;

    svm_locker.airdrop_pubkeys(simnet.airdrop_token_amount, &simnet.airdrop_addresses);

    // Load snapshot accounts if provided
    if !simnet.snapshot.is_empty() {
        match svm_locker
            .load_snapshot(
                &simnet.snapshot,
                remote_rpc_client.as_ref(),
                CommitmentConfig::confirmed(),
            )
            .await
        {
            Ok(loaded_count) => {
                let _ = svm_locker.with_svm_reader(|svm| {
                    svm.simnet_events_tx.send(SimnetEvent::info(format!(
                        "Preloaded {} accounts from snapshot(s) into SVM",
                        loaded_count
                    )))
                });
            }
            Err(e) => {
                let _ = svm_locker.with_svm_reader(|svm| {
                    svm.simnet_events_tx.send(SimnetEvent::warn(format!(
                        "Error loading snapshot accounts: {}",
                        e
                    )))
                });
            }
        }
    }

    let simnet_events_tx_cc = svm_locker.simnet_events_tx();

    let (
        plugin_manager_commands_tx,
        plugin_manager_commands_rx,
        rpc_handle,
        ws_handle,
        rpc_close_handle,
        ws_close_handle,
    ) = start_rpc_servers_runloop(
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

    let (clock_event_rx, clock_command_tx) =
        start_clock_runloop(simnet_config.slot_time, Some(simnet_events_tx_cc.clone()));

    // Emit TransactionProcessed events for each stored transaction before Ready
    let initial_transaction_count = svm_locker.with_svm_reader(|svm| {
        let iter_result = svm.transactions.into_iter();
        let mut count: u64 = 0;

        if let Ok(iter) = iter_result {
            let mut events = vec![];
            for (_, status) in iter {
                if let Some((tx_meta, _updated_accounts)) = status.as_processed() {
                    let signature = tx_meta.transaction.signatures[0];
                    let err = tx_meta.meta.status.clone().err();

                    // Build TransactionMetadata from stored data
                    let meta = surfpool_types::TransactionMetadata {
                        signature,
                        logs: tx_meta.meta.log_messages.clone().unwrap_or_default(),
                        inner_instructions: tx_meta
                            .meta
                            .inner_instructions
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|inner_ixs| {
                                inner_ixs
                                    .instructions
                                    .into_iter()
                                    .map(|ix| solana_message::inner_instruction::InnerInstruction {
                                        instruction: ix.instruction,
                                        stack_height: ix.stack_height.unwrap_or(1) as u8,
                                    })
                                    .collect()
                            })
                            .collect(),
                        compute_units_consumed: tx_meta.meta.compute_units_consumed.unwrap_or(0),
                        return_data: tx_meta.meta.return_data.clone().unwrap_or_default(),
                        fee: tx_meta.meta.fee,
                    };

                    events.push((
                        tx_meta.slot,
                        SimnetEvent::TransactionProcessed(Local::now(), meta, err.clone()),
                    ));

                    count += 1;
                }
            }
            for (_, event) in events
                .into_iter()
                .sorted_by(|(a_slot, _), (b_slot, _)| a_slot.cmp(b_slot))
            {
                let _ = svm.simnet_events_tx.send(event);
            }
        }

        count
    });
    let _ = simnet_events_tx_cc.send(SimnetEvent::Ready(initial_transaction_count));

    // Notify geyser plugins that startup is complete
    let _ = svm_locker.with_svm_reader(|svm| svm.geyser_events_tx.send(GeyserEvent::EndOfStartup));

    start_block_production_runloop(
        clock_event_rx,
        clock_command_tx,
        simnet_commands_rx,
        simnet_commands_tx.clone(),
        svm_locker,
        block_production_mode,
        &remote_rpc_client,
        simnet_config.expiry.map(|e| e * 1000),
        &simnet_config,
        Some(rpc_close_handle),
        Some(ws_close_handle),
        Some(plugin_manager_commands_tx),
        Some(subgraph_commands_tx),
        rpc_handle,
        ws_handle,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn start_block_production_runloop(
    clock_event_rx: Receiver<ClockEvent>,
    clock_command_tx: Sender<ClockCommand>,
    simnet_commands_rx: Receiver<SimnetCommand>,
    simnet_commands_tx: Sender<SimnetCommand>,
    svm_locker: SurfnetSvmLocker,
    mut block_production_mode: BlockProductionMode,
    remote_rpc_client: &Option<SurfnetRemoteClient>,
    expiry_duration_ms: Option<u64>,
    simnet_config: &SimnetConfig,
    mut rpc_close_handle: Option<jsonrpc_http_server::CloseHandle>,
    mut ws_close_handle: Option<jsonrpc_ws_server::CloseHandle>,
    mut plugin_manager_commands_tx: Option<Sender<PluginManagerCommand>>,
    mut subgraph_commands_tx: Option<Sender<SubgraphCommand>>,
    rpc_handle: JoinHandle<()>,
    ws_handle: JoinHandle<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let remote_client_with_commitment = remote_rpc_client.as_ref().map(|c| {
        (
            c.clone(),
            solana_commitment_config::CommitmentConfig::confirmed(),
        )
    });
    let mut next_scheduled_expiry_check: Option<u64> =
        expiry_duration_ms.map(|expiry_val| Utc::now().timestamp_millis() as u64 + expiry_val);
    let global_skip_sig_verify = simnet_config.skip_signature_verification;
    // Shutdown helper: terminates geyser, closes RPC servers, signals subgraph, stops clock, and shuts down storage.
    // Extracted to avoid duplicating the shutdown sequence in multiple code paths.
    macro_rules! run_shutdown {
        ($plugin_manager_commands_tx:expr, $rpc_close_handle:expr, $ws_close_handle:expr,
         $subgraph_commands_tx:expr, $clock_command_tx:expr, $svm_locker:expr) => {{
            // Terminate geyser thread first so it doesn't spin on closed channels
            if let Some(tx) = $plugin_manager_commands_tx.take() {
                let _ = tx.send(PluginManagerCommand::Terminate);
            }
            // Close RPC servers on a blocking thread to avoid dropping
            // their internal tokio runtimes from within an async context.
            let rpc_close = $rpc_close_handle.take();
            let ws_close = $ws_close_handle.take();
            let close_task = tokio::task::spawn_blocking(move || {
                if let Some(h) = rpc_close {
                    h.close();
                }
                if let Some(h) = ws_close {
                    h.close();
                }
            });
            // Wait for RPC servers to close before continuing shutdown
            let _ = close_task.await;
            // Signal subgraph runloop to shutdown
            if let Some(tx) = $subgraph_commands_tx.take() {
                let _ = tx.send(SubgraphCommand::Shutdown);
            }
            // Stop clock thread
            let _ = $clock_command_tx.send(ClockCommand::Terminate);
            // Explicitly shutdown storage to trigger WAL checkpoint before exiting
            $svm_locker.shutdown();
        }};
    }

    loop {
        let mut do_produce_block = false;

        // Use timeout instead of blocking select to allow checking for channel disconnection.
        // This prevents the loop from blocking forever when the terminal closes abruptly.
        let mut selector = crossbeam::channel::Select::new();
        selector.recv(&clock_event_rx);
        selector.recv(&simnet_commands_rx);

        // Holds a clock event or simnet command consumed during the disconnect check
        // so it can be processed normally instead of being dropped.
        let mut pending_clock_event: Option<ClockEvent> = None;
        let mut pending_simnet_command: Option<SimnetCommand> = None;
        // Set to true when a channel disconnects during timeout; deferred so any
        // consumed messages are processed before the loop exits.
        let mut channel_disconnected = false;

        match selector.select_timeout(std::time::Duration::from_millis(100)) {
            Ok(oper) => {
                // select returned a ready channel — consume the message via recv()
                match oper.index() {
                    0 => match oper.recv(&clock_event_rx) {
                        Ok(event) => pending_clock_event = Some(event),
                        Err(_) => break, // Clock channel closed, exit runloop
                    },
                    1 => match oper.recv(&simnet_commands_rx) {
                        Ok(cmd) => pending_simnet_command = Some(cmd),
                        Err(_) => break, // Command channel closed, exit runloop
                    },
                    _ => unreachable!(),
                }
            }
            Err(_) => {
                // Timeout — check if channels are disconnected.
                // try_recv() is the only way to detect disconnection on crossbeam channels,
                // but it may consume a pending message. We capture any consumed message
                // and process it normally below to avoid dropping commands.
                // We defer the break so both channels are drained first.
                match clock_event_rx.try_recv() {
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        channel_disconnected = true
                    }
                    Ok(event) => pending_clock_event = Some(event),
                    Err(crossbeam_channel::TryRecvError::Empty) => {}
                }
                match simnet_commands_rx.try_recv() {
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        channel_disconnected = true
                    }
                    Ok(cmd) => pending_simnet_command = Some(cmd),
                    Err(crossbeam_channel::TryRecvError::Empty) => {}
                }
                if channel_disconnected
                    && pending_clock_event.is_none()
                    && pending_simnet_command.is_none()
                {
                    break;
                }
            }
        }

        // Process clock event — either from select or from the disconnect-check try_recv
        if let Some(event) = pending_clock_event {
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
        }

        // Process simnet command — either from select or from the disconnect-check try_recv
        let simnet_command = pending_simnet_command;

        if let Some(event) = simnet_command {
            match event {
                SimnetCommand::SlotForward(_key) => {
                    block_production_mode = BlockProductionMode::Manual;
                    do_produce_block = true;
                }
                SimnetCommand::SlotBackward(_key) => {}
                SimnetCommand::CommandClock(_, update) => {
                    if let ClockCommand::UpdateSlotInterval(updated_slot_time) = update {
                        svm_locker.with_svm_writer(|svm_writer| {
                            svm_writer.slot_time = updated_slot_time;
                        });
                    }

                    // Handle PauseWithConfirmation specially
                    if let ClockCommand::PauseWithConfirmation(response_tx) = update {
                        // Get current slot and slot_time before pausing
                        let (current_slot, slot_time) = svm_locker.with_svm_reader(|svm_reader| {
                            (
                                svm_reader.latest_epoch_info.absolute_slot,
                                svm_reader.slot_time,
                            )
                        });

                        // Send Pause to clock runloop
                        let _ = clock_command_tx.send(ClockCommand::Pause);

                        // Give the clock time to process the pause command
                        tokio::time::sleep(tokio::time::Duration::from_millis(slot_time / 2)).await;

                        // Loop and check if the slot has stopped advancing
                        let max_attempts = 10;
                        let mut attempts = 0;
                        loop {
                            tokio::time::sleep(tokio::time::Duration::from_millis(slot_time)).await;

                            let new_slot = svm_locker.with_svm_reader(|svm_reader| {
                                svm_reader.latest_epoch_info.absolute_slot
                            });

                            // If slot hasn't changed, clock has stopped
                            if new_slot == current_slot || attempts >= max_attempts {
                                break;
                            }

                            attempts += 1;
                        }

                        // Read epoch info after clock has stopped
                        let epoch_info = svm_locker
                            .with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone());
                        // Send response
                        let _ = response_tx.send(epoch_info);
                    } else {
                        let _ = clock_command_tx.send(update);
                    }
                    continue;
                }
                SimnetCommand::UpdateInternalClock(_, clock) => {
                    // Confirm the current block to materialize any scheduled overrides for this slot
                    if let Err(e) = svm_locker
                        .confirm_current_block(&remote_client_with_commitment)
                        .await
                    {
                        let _ = svm_locker
                            .simnet_events_tx()
                            .send(SimnetEvent::error(format!(
                                "Failed to confirm block after time travel: {}",
                                e
                            )));
                    }

                    svm_locker.with_svm_writer(|svm_writer| {
                        svm_writer.inner.set_sysvar(&clock);
                        svm_writer.updated_at = clock.unix_timestamp as u64 * 1_000;
                        svm_writer.latest_epoch_info.absolute_slot = clock.slot;
                        svm_writer.latest_epoch_info.epoch = clock.epoch;
                        svm_writer.latest_epoch_info.slot_index = clock.slot;
                        svm_writer.latest_epoch_info.epoch = clock.epoch;
                        svm_writer.latest_epoch_info.absolute_slot =
                            clock.slot + clock.epoch * svm_writer.latest_epoch_info.slots_in_epoch;
                        let _ = svm_writer
                            .simnet_events_tx
                            .send(SimnetEvent::SystemClockUpdated(clock));
                    });
                }
                SimnetCommand::UpdateInternalClockWithConfirmation(_, clock, response_tx) => {
                    // Confirm the current block to materialize any scheduled overrides for this slot
                    if let Err(e) = svm_locker
                        .confirm_current_block(&remote_client_with_commitment)
                        .await
                    {
                        let _ = svm_locker
                            .simnet_events_tx()
                            .send(SimnetEvent::error(format!(
                                "Failed to confirm block after time travel: {}",
                                e
                            )));
                    }

                    let epoch_info = svm_locker.with_svm_writer(|svm_writer| {
                        svm_writer.inner.set_sysvar(&clock);
                        svm_writer.updated_at = clock.unix_timestamp as u64 * 1_000;
                        svm_writer.latest_epoch_info.absolute_slot = clock.slot;
                        svm_writer.latest_epoch_info.epoch = clock.epoch;
                        svm_writer.latest_epoch_info.slot_index = clock.slot;
                        svm_writer.latest_epoch_info.epoch = clock.epoch;
                        svm_writer.latest_epoch_info.absolute_slot =
                            clock.slot + clock.epoch * svm_writer.latest_epoch_info.slots_in_epoch;
                        let _ = svm_writer
                            .simnet_events_tx
                            .send(SimnetEvent::SystemClockUpdated(clock));
                        svm_writer.latest_epoch_info.clone()
                    });

                    // Send confirmation back
                    let _ = response_tx.send(epoch_info);
                }
                SimnetCommand::UpdateBlockProductionMode(update) => {
                    block_production_mode = update;
                    continue;
                }
                SimnetCommand::ProcessTransaction(
                    _key,
                    transaction,
                    status_tx,
                    skip_preflight,
                    skip_sig_verify_override,
                ) => {
                    let skip_sig_verify =
                        skip_sig_verify_override.unwrap_or(global_skip_sig_verify);
                    let sigverify = !skip_sig_verify;
                    if let Err(e) = svm_locker
                        .process_transaction(
                            &remote_client_with_commitment,
                            transaction,
                            status_tx,
                            skip_preflight,
                            sigverify,
                        )
                        .await
                    {
                        let _ = svm_locker
                            .simnet_events_tx()
                            .send(SimnetEvent::error(format!(
                                "Failed to process transaction: {}",
                                e
                            )));
                    }
                    if block_production_mode.eq(&BlockProductionMode::Transaction) {
                        do_produce_block = true;
                    }
                }
                SimnetCommand::Terminate(_) => {
                    run_shutdown!(
                        plugin_manager_commands_tx,
                        rpc_close_handle,
                        ws_close_handle,
                        subgraph_commands_tx,
                        clock_command_tx,
                        svm_locker
                    );
                    break;
                }
                SimnetCommand::StartRunbookExecution(runbook_id) => {
                    svm_locker.start_runbook_execution(runbook_id);
                }
                SimnetCommand::CompleteRunbookExecution(runbook_id, error) => {
                    svm_locker.complete_runbook_execution(runbook_id, error);
                }
                SimnetCommand::FetchRemoteAccounts(pubkeys, remote_url) => {
                    let remote_client = SurfnetRemoteClient::new_unsafe(&remote_url);
                    if let Some(remote_client) = remote_client {
                        match svm_locker
                            .get_multiple_accounts_with_remote_fallback(
                                &remote_client,
                                &pubkeys,
                                CommitmentConfig::confirmed(),
                            )
                            .await
                        {
                            Ok(account_updates) => {
                                svm_locker.write_multiple_account_updates(&account_updates.inner);
                            }
                            Err(e) => {
                                svm_locker
                                    .simnet_events_tx()
                                    .try_send(SimnetEvent::error(format!(
                                        "Failed to fetch remote accounts {:?}: {}",
                                        pubkeys, e
                                    )))
                                    .ok();
                            }
                        };
                    }
                }
                SimnetCommand::AirdropProcessed => {
                    if block_production_mode.eq(&BlockProductionMode::Transaction) {
                        do_produce_block = true;
                    }
                }
            }
        }

        {
            if do_produce_block {
                svm_locker
                    .confirm_current_block(&remote_client_with_commitment)
                    .await?;
            }
        }

        // Break after processing any consumed messages from disconnected channels
        if channel_disconnected {
            break;
        }
    }

    // Ensure shutdown runs even if the loop exited via channel disconnection
    // rather than SimnetCommand::Terminate. The macro uses .take() on all
    // handles and senders, so this is a no-op if shutdown already ran above.
    run_shutdown!(
        plugin_manager_commands_tx,
        rpc_close_handle,
        ws_close_handle,
        subgraph_commands_tx,
        clock_command_tx,
        svm_locker
    );

    // Wait for RPC server threads to complete shutdown gracefully
    // WS handler has a 5s shutdown_timeout, so we wait slightly longer
    let shutdown_wait_start = std::time::Instant::now();
    let max_shutdown_wait = Duration::from_secs(6);

    // Poll until WS thread finishes or timeout
    while !ws_handle.is_finished() {
        if shutdown_wait_start.elapsed() > max_shutdown_wait {
            break; // Give up - process exit will clean up
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Join handles only if they finished. After a timeout, join() would block forever
    // since the thread didn't respond to the close signal within the deadline.
    if ws_handle.is_finished() {
        let _ = ws_handle.join();
    }
    if rpc_handle.is_finished() {
        let _ = rpc_handle.join();
    }

    Ok(())
}

pub fn start_clock_runloop(
    mut slot_time: u64,
    simnet_events_tx: Option<Sender<SimnetEvent>>,
) -> (Receiver<ClockEvent>, Sender<ClockCommand>) {
    let (clock_event_tx, clock_event_rx) = unbounded::<ClockEvent>();
    let (clock_command_tx, clock_command_rx) = unbounded::<ClockCommand>();

    let _handle = hiro_system_kit::thread_named("clock").spawn(move || {
        let mut enabled = true;
        let mut block_hash_timeout = Instant::now();

        loop {
            match clock_command_rx.try_recv() {
                Ok(ClockCommand::Terminate) => {
                    break;
                }
                Ok(ClockCommand::Pause) => {
                    enabled = false;
                    if let Some(ref simnet_events_tx) = simnet_events_tx {
                        let _ =
                            simnet_events_tx.send(SimnetEvent::ClockUpdate(ClockCommand::Pause));
                    }
                }
                Ok(ClockCommand::Resume) => {
                    enabled = true;
                    if let Some(ref simnet_events_tx) = simnet_events_tx {
                        let _ =
                            simnet_events_tx.send(SimnetEvent::ClockUpdate(ClockCommand::Resume));
                    }
                }
                Ok(ClockCommand::Toggle) => {
                    enabled = !enabled;
                    if let Some(ref simnet_events_tx) = simnet_events_tx {
                        let _ =
                            simnet_events_tx.send(SimnetEvent::ClockUpdate(ClockCommand::Toggle));
                    }
                }
                Ok(ClockCommand::UpdateSlotInterval(updated_slot_time)) => {
                    slot_time = updated_slot_time;
                }
                Ok(ClockCommand::PauseWithConfirmation(_)) => {
                    // This should be handled in the block production runloop, not here
                    // If it reaches here, just treat it as a regular Pause
                    enabled = false;
                    if let Some(ref simnet_events_tx) = simnet_events_tx {
                        let _ =
                            simnet_events_tx.send(SimnetEvent::ClockUpdate(ClockCommand::Pause));
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    // Channel closed, exit thread
                    break;
                }
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
    let handle: JoinHandle<Result<(), String>> = hiro_system_kit::thread_named(
        "Geyser Plugins Handler",
    )
    .spawn(move || {
        let mut indexing_enabled = false;

        #[cfg(feature = "geyser_plugin")]
        let mut plugin_manager = GeyserPluginManager::new();
        #[cfg(not(feature = "geyser_plugin"))]
        let mut plugin_manager = ();

        let mut surfpool_plugin_manager: Vec<Box<dyn GeyserPlugin>> = vec![];

        // Map between each plugin's UUID to its entry (index, plugin_name)
        let mut plugin_map: HashMap<crate::Uuid, (usize, String)> = HashMap::new();

        // helper to log errors that can't be propagated
        let log_error = |msg: String| {
            let _ = simnet_events_tx.send(SimnetEvent::error(msg));
        };

        let log_warn = |msg: String| {
            let _ = simnet_events_tx.send(SimnetEvent::warn(msg));
        };

        let log_info = |msg: String| {
            let _ = simnet_events_tx.send(SimnetEvent::info(msg));
        };

        #[cfg(feature = "geyser_plugin")]
        for plugin_config_path in plugin_config_paths.into_iter() {
            let plugin_manifest_location = FileLocation::from_path(plugin_config_path);
            let config_file = plugin_manifest_location.read_content_as_utf8()?;
            let result: serde_json::Value = match json5::from_str(&config_file) {
                Ok(res) => res,
                Err(e) => {
                    let error = format!("Unable to read manifest: {}", e);
                    let _ = simnet_events_tx.send(SimnetEvent::error(error.clone()));
                    return Err(error);
                }
            };

            let plugin_dylib_path = match result.get("libpath").map(|p| p.as_str()) {
                Some(Some(name)) => name,
                _ => {
                    let error = format!(
                        "Plugin config file should include a 'libpath' field: {}",
                        plugin_manifest_location
                    );
                    let _ = simnet_events_tx.send(SimnetEvent::error(error.clone()));
                    return Err(error);
                }
            };

            let mut plugin_dylib_location = plugin_manifest_location
                .get_parent_location()
                .expect("path invalid");
            plugin_dylib_location
                .append_path(&plugin_dylib_path)
                .expect("path invalid");

            let (plugin, lib) = unsafe {
                let lib = match Library::new(&plugin_dylib_location.to_string()) {
                    Ok(lib) => lib,
                    Err(e) => {
                        log_error(format!(
                            "Unable to load plugin {}: {}",
                            plugin_dylib_location.to_string(),
                            e.to_string()
                        ));
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

            let mut plugin = LoadedGeyserPlugin::new(lib, plugin, None);
            if let Err(e) = plugin.on_load(&plugin_manifest_location.to_string(), false) {
                let error = format!("Unable to load plugin:: {}", e.to_string());
                let _ = simnet_events_tx.send(SimnetEvent::error(error.clone()));
                return Err(error);
            }

            plugin_manager.plugins.push(plugin);
        }

        // Helper function to load a subgraph plugin
        #[cfg(feature = "subgraph")]
        let load_subgraph_plugin =
            |uuid: uuid::Uuid,
             config: txtx_addon_network_svm_types::subgraph::PluginConfig,
             notifier: crossbeam_channel::Sender<String>,
             surfpool_plugin_manager: &mut Vec<Box<dyn GeyserPlugin>>,
             plugin_map: &mut HashMap<uuid::Uuid, (usize, String)>,
             indexing_enabled: &mut bool|
             -> Result<(), String> {
                if let Err(e) = subgraph_commands_tx.send(SubgraphCommand::CreateCollection(
                    uuid,
                    config.data.clone(),
                    notifier,
                )) {
                    return Err(format!("Failed to send CreateCollection command: {:?}", e));
                };

                let mut plugin = SurfpoolSubgraphPlugin::default();

                let (server, ipc_token) =
                    IpcOneShotServer::<IpcReceiver<DataIndexingCommand>>::new()
                        .expect("Failed to create IPC one-shot server.");
                let subgraph_plugin_config = SubgraphPluginConfig {
                    uuid,
                    ipc_token,
                    subgraph_request: config.data.clone(),
                };

                let config_file = serde_json::to_string(&subgraph_plugin_config)
                    .map_err(|e| format!("Failed to serialize subgraph plugin config: {:?}", e))?;

                plugin
                    .on_load(&config_file, false)
                    .map_err(|e| format!("Failed to load Geyser plugin: {:?}", e))?;

                match server.accept() {
                    Ok((_, rx)) => {
                        let subgraph_rx = RouterProxy::new()
                            .route_ipc_receiver_to_new_crossbeam_receiver::<DataIndexingCommand>(
                                rx,
                            );
                        if let Err(e) = subgraph_commands_tx
                            .send(SubgraphCommand::ObserveCollection(subgraph_rx))
                        {
                            return Err(format!(
                                "Failed to send ObserveCollection command: {:?}",
                                e
                            ));
                        }
                    }
                    Err(e) => {
                        return Err(format!(
                            "Failed to accept IPC connection for subgraph {}: {:?}",
                            uuid, e
                        ));
                    }
                };

                *indexing_enabled = true;

                let plugin: Box<dyn GeyserPlugin> = Box::new(plugin);
                let plugin_index = surfpool_plugin_manager.len();
                surfpool_plugin_manager.push(plugin);
                plugin_map.insert(uuid, (plugin_index, config.plugin_name.to_string()));

                Ok(())
            };

        // Helper function to unload a plugin by UUID
        #[cfg(feature = "subgraph")]
        let unload_plugin_by_uuid = |uuid: uuid::Uuid,
                                     surfpool_plugin_manager: &mut Vec<Box<dyn GeyserPlugin>>,
                                     plugin_map: &mut HashMap<uuid::Uuid, (usize, String)>,
                                     indexing_enabled: &mut bool|
         -> Result<(), String> {
            let plugin_index = plugin_map
                .get(&uuid)
                .ok_or_else(|| format!("Plugin {} not found", uuid))?
                .0;

            if plugin_index >= surfpool_plugin_manager.len() {
                return Err(format!("Plugin index {} out of bounds", plugin_index));
            }

            // Destroy database/schema for this collection
            if let Err(e) = subgraph_commands_tx.send(SubgraphCommand::DestroyCollection(uuid)) {
                return Err(format!(
                    "Failed to send DestroyCollection command for {}: {:?}",
                    uuid, e
                ));
            }

            // Unload the plugin
            surfpool_plugin_manager[plugin_index].on_unload();

            // Remove from tracking structures
            surfpool_plugin_manager.remove(plugin_index);
            plugin_map.remove(&uuid);

            // Adjust indices after removal
            for (index, _) in plugin_map.values_mut() {
                if *index > plugin_index {
                    *index -= 1;
                }
            }

            // Disable indexing if no plugins remain
            if surfpool_plugin_manager.is_empty() {
                *indexing_enabled = false;
                //  Add Logging When Indexing Disabled
                log_info("All plugins unloaded,indexing disabled".to_string())
            }

            Ok(())
        };

        let err = loop {
            use agave_geyser_plugin_interface::geyser_plugin_interface::{
                ReplicaAccountInfoV3, ReplicaAccountInfoVersions,
            };

            use crate::types::GeyserAccountUpdate;

            // Use timeout instead of blocking select to allow checking for channel disconnection
            let mut selector = crossbeam::channel::Select::new();
            selector.recv(&plugin_manager_commands_rx);
            selector.recv(&geyser_events_rx);

            // Holds messages consumed during the disconnect check so they aren't dropped.
            let mut pending_plugin_cmd: Option<PluginManagerCommand> = None;
            let mut pending_geyser_event: Option<GeyserEvent> = None;
            // Set when a channel disconnects during timeout; deferred so any
            // consumed messages are processed before the loop exits.
            let mut disconnect_reason: Option<String> = None;

            match selector.select_timeout(Duration::from_millis(100)) {
                Ok(oper) => {
                    // select returned a ready channel — consume via recv()
                    match oper.index() {
                        0 => match oper.recv(&plugin_manager_commands_rx) {
                            Ok(cmd) => pending_plugin_cmd = Some(cmd),
                            Err(_) => {
                                break "Failed to read plugin manager command".to_string();
                            }
                        },
                        1 => match oper.recv(&geyser_events_rx) {
                            Ok(event) => pending_geyser_event = Some(event),
                            Err(e) => {
                                break format!(
                                    "Failed to read new transaction to send to Geyser plugin: {e}"
                                );
                            }
                        },
                        _ => unreachable!(),
                    }
                }
                Err(_) => {
                    // Timeout — check if channels are disconnected.
                    // Capture any consumed messages to process them below.
                    // We defer the break so both channels are drained first.
                    match plugin_manager_commands_rx.try_recv() {
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            disconnect_reason =
                                Some("Plugin manager command channel disconnected".to_string());
                        }
                        Ok(cmd) => pending_plugin_cmd = Some(cmd),
                        Err(crossbeam_channel::TryRecvError::Empty) => {}
                    }
                    match geyser_events_rx.try_recv() {
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            disconnect_reason
                                .get_or_insert("Geyser events channel disconnected".to_string());
                        }
                        Ok(event) => pending_geyser_event = Some(event),
                        Err(crossbeam_channel::TryRecvError::Empty) => {}
                    }
                    if disconnect_reason.is_some()
                        && pending_plugin_cmd.is_none()
                        && pending_geyser_event.is_none()
                    {
                        break disconnect_reason.unwrap();
                    }
                }
            }

            // Process plugin manager command — either from select or from disconnect-check try_recv
            let plugin_cmd = pending_plugin_cmd;

            if let Some(event) = plugin_cmd {
                match event {
                    PluginManagerCommand::Terminate => {
                        break "Geyser thread terminated".to_string();
                    }
                    #[cfg(not(feature = "subgraph"))]
                    PluginManagerCommand::LoadConfig(_, _, _) => {
                        continue;
                    }
                    #[cfg(feature = "subgraph")]
                    PluginManagerCommand::LoadConfig(uuid, config, notifier) => {
                        if let Err(e) = load_subgraph_plugin(
                            uuid,
                            config,
                            notifier,
                            &mut surfpool_plugin_manager,
                            &mut plugin_map,
                            &mut indexing_enabled,
                        ) {
                            let _ = simnet_events_tx
                                .send(SimnetEvent::error(format!("Failed to load plugin: {}", e)));
                        }
                    }
                    #[cfg(not(feature = "subgraph"))]
                    PluginManagerCommand::UnloadPlugin(_, _) => {
                        continue;
                    }
                    #[cfg(feature = "subgraph")]
                    PluginManagerCommand::UnloadPlugin(uuid, notifier) => {
                        match unload_plugin_by_uuid(
                            uuid,
                            &mut surfpool_plugin_manager,
                            &mut plugin_map,
                            &mut indexing_enabled,
                        ) {
                            Ok(_) => {
                                log_info(format!(
                                    "Successfully unloaded plugin with UUID {}",
                                    uuid
                                ));
                                let _ = notifier.send(Ok(()));
                            }
                            Err(e) => {
                                log_error(format!("Failed to unload plugin {}: {}", uuid, e));
                                let _ = notifier.send(Err(e));
                            }
                        }
                    }
                    #[cfg(not(feature = "subgraph"))]
                    PluginManagerCommand::ReloadPlugin(_, _, _) => {
                        continue;
                    }
                    #[cfg(feature = "subgraph")]
                    PluginManagerCommand::ReloadPlugin(uuid, config, notifier) => {
                        // Unload the old plugin
                        match unload_plugin_by_uuid(
                            uuid,
                            &mut surfpool_plugin_manager,
                            &mut plugin_map,
                            &mut indexing_enabled,
                        ) {
                            Ok(_) => {
                                log_info(format!("Unloaded plugin with UUID {} for reload", uuid));

                                // Load the new plugin with the same UUID
                                match load_subgraph_plugin(
                                    uuid,
                                    config,
                                    notifier.clone(),
                                    &mut surfpool_plugin_manager,
                                    &mut plugin_map,
                                    &mut indexing_enabled,
                                ) {
                                    Ok(_) => {
                                        log_info(format!(
                                            "Successfully reloaded plugin with UUID {}",
                                            uuid
                                        ));
                                        let _ = notifier
                                            .send(format!("Plugin {} reloaded successfully", uuid));
                                    }
                                    Err(e) => {
                                        let error_msg =
                                            format!("Failed to reload plugin {}: {}", uuid, e);
                                        log_error(error_msg.clone());
                                        let _ = notifier.send(error_msg);
                                    }
                                }
                            }
                            Err(e) => {
                                let error_msg = format!(
                                    "Failed to unload plugin {} during reload: {}",
                                    uuid, e
                                );
                                log_error(error_msg.clone());
                                let _ = notifier.send(error_msg);
                            }
                        }
                    }
                    PluginManagerCommand::ListPlugins(notifier) => {
                        let plugin_list: Vec<crate::PluginInfo> = plugin_map
                            .iter()
                            .map(|(uuid, (_, plugin_name))| crate::PluginInfo {
                                plugin_name: plugin_name.clone(),
                                uuid: uuid.to_string(),
                            })
                            .collect();
                        let _ = notifier.send(plugin_list);
                    }
                }
            }

            // Process geyser event — either from select or from disconnect-check try_recv
            let geyser_event = pending_geyser_event;

            if let Some(geyser_event) = geyser_event {
                match geyser_event {
                    GeyserEvent::NotifyTransaction(
                        transaction_with_status_meta,
                        versioned_transaction,
                    ) => {
                        if !indexing_enabled {
                            continue;
                        }

                        let transaction = match versioned_transaction {
                            Some(tx) => tx,
                            None => {
                                log_warn("Unable to index sanitized transaction".to_string());
                                continue;
                            }
                        };

                        let transaction_replica = ReplicaTransactionInfoV3 {
                            signature: &transaction.signatures[0],
                            is_vote: false,
                            transaction: &transaction,
                            transaction_status_meta: &transaction_with_status_meta.meta,
                            index: 0,
                            message_hash: &transaction.message.hash(),
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.notify_transaction(
                                ReplicaTransactionInfoVersions::V0_0_3(&transaction_replica),
                                transaction_with_status_meta.slot,
                            ) {
                                log_error(format!(
                                    "Failed to notify Geyser plugin of new transaction: {:?}",
                                    e
                                ))
                            };
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.notify_transaction(
                                ReplicaTransactionInfoVersions::V0_0_3(&transaction_replica),
                                transaction_with_status_meta.slot,
                            ) {
                                log_error(format!(
                                    "Failed to notify Geyser plugin of new transaction: {:?}",
                                    e
                                ))
                            };
                        }
                    }
                    GeyserEvent::UpdateAccount(account_update) => {
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
                            txn: sanitized_transaction.as_ref(),
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.update_account(
                                ReplicaAccountInfoVersions::V0_0_3(&account_replica),
                                slot,
                                false,
                            ) {
                                log_error(format!(
                                    "Failed to update account in Geyser plugin: {:?}",
                                    e
                                ));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.update_account(
                                ReplicaAccountInfoVersions::V0_0_3(&account_replica),
                                slot,
                                false,
                            ) {
                                log_error(format!(
                                    "Failed to update account in Geyser plugin: {:?}",
                                    e
                                ))
                            }
                        }
                    }
                    GeyserEvent::StartupAccountUpdate(account_update) => {
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
                            txn: sanitized_transaction.as_ref(),
                        };

                        // Send startup account updates with is_startup=true
                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.update_account(
                                ReplicaAccountInfoVersions::V0_0_3(&account_replica),
                                slot,
                                true,
                            ) {
                                log_error(format!(
                                    "Failed to send startup account update to Geyser plugin: {:?}",
                                    e
                                ));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.update_account(
                                ReplicaAccountInfoVersions::V0_0_3(&account_replica),
                                slot,
                                true,
                            ) {
                                log_error(format!(
                                    "Failed to send startup account update to Geyser plugin: {:?}",
                                    e
                                ))
                            }
                        }
                    }
                    GeyserEvent::EndOfStartup => {
                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.notify_end_of_startup() {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify end of startup to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.notify_end_of_startup() {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify end of startup to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }
                    }
                    GeyserEvent::UpdateSlotStatus {
                        slot,
                        parent,
                        status,
                    } => {
                        let slot_status = match status {
                            crate::surfnet::GeyserSlotStatus::Processed => SlotStatus::Processed,
                            crate::surfnet::GeyserSlotStatus::Confirmed => SlotStatus::Confirmed,
                            crate::surfnet::GeyserSlotStatus::Rooted => SlotStatus::Rooted,
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.update_slot_status(slot, parent, &slot_status) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to update slot status in Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.update_slot_status(slot, parent, &slot_status) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to update slot status in Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }
                    }
                    GeyserEvent::NotifyBlockMetadata(block_metadata) => {
                        let rewards = RewardsAndNumPartitions {
                            rewards: vec![],
                            num_partitions: None,
                        };

                        let block_info = ReplicaBlockInfoV4 {
                            slot: block_metadata.slot,
                            blockhash: &block_metadata.blockhash,
                            rewards: &rewards,
                            block_time: block_metadata.block_time,
                            block_height: block_metadata.block_height,
                            parent_slot: block_metadata.parent_slot,
                            parent_blockhash: &block_metadata.parent_blockhash,
                            executed_transaction_count: block_metadata.executed_transaction_count,
                            entry_count: block_metadata.entry_count,
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin.notify_block_metadata(
                                ReplicaBlockInfoVersions::V0_0_4(&block_info),
                            ) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify block metadata to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin.notify_block_metadata(
                                ReplicaBlockInfoVersions::V0_0_4(&block_info),
                            ) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify block metadata to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }
                    }
                    GeyserEvent::NotifyEntry(entry_info) => {
                        let entry_replica = ReplicaEntryInfoV2 {
                            slot: entry_info.slot,
                            index: entry_info.index,
                            num_hashes: entry_info.num_hashes,
                            hash: &entry_info.hash,
                            executed_transaction_count: entry_info.executed_transaction_count,
                            starting_transaction_index: entry_info.starting_transaction_index,
                        };

                        for plugin in surfpool_plugin_manager.iter() {
                            if let Err(e) = plugin
                                .notify_entry(ReplicaEntryInfoVersions::V0_0_2(&entry_replica))
                            {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify entry to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }

                        #[cfg(feature = "geyser_plugin")]
                        for plugin in plugin_manager.plugins.iter() {
                            if let Err(e) = plugin
                                .notify_entry(ReplicaEntryInfoVersions::V0_0_2(&entry_replica))
                            {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                                    "Failed to notify entry to Geyser plugin: {:?}",
                                    e
                                )));
                            }
                        }
                    }
                }
            }

            // Break after processing any consumed messages from disconnected channels
            if let Some(reason) = disconnect_reason {
                break reason;
            }
        };
        Err(err)
    })
    .map_err(|e| format!("Failed to spawn Geyser Plugins Handler thread: {:?}", e))?;
    Ok(handle)
}

async fn start_rpc_servers_runloop(
    config: &SurfpoolConfig,
    simnet_commands_tx: &Sender<SimnetCommand>,
    svm_locker: SurfnetSvmLocker,
    remote_rpc_client: &Option<SurfnetRemoteClient>,
) -> Result<
    (
        Sender<PluginManagerCommand>,
        Receiver<PluginManagerCommand>,
        JoinHandle<()>,
        JoinHandle<()>,
        jsonrpc_http_server::CloseHandle,
        jsonrpc_ws_server::CloseHandle,
    ),
    String,
> {
    let rpc_addr: SocketAddr = config
        .rpc
        .get_rpc_base_url()
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let ws_addr: SocketAddr = config
        .rpc
        .get_ws_base_url()
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;

    check_port_availability(rpc_addr, "RPC")?;
    check_port_availability(ws_addr, "WebSocket")?;

    let (plugin_manager_commands_tx, plugin_manager_commands_rx) = unbounded();
    let simnet_events_tx = svm_locker.simnet_events_tx();

    let middleware = SurfpoolMiddleware::new(
        svm_locker,
        simnet_commands_tx,
        &plugin_manager_commands_tx,
        &config.rpc,
        remote_rpc_client,
    );

    let (rpc_handle, rpc_close_handle) =
        start_http_rpc_server_runloop(config, middleware.clone(), simnet_events_tx.clone()).await?;
    let (ws_handle, ws_close_handle) =
        start_ws_rpc_server_runloop(config, middleware, simnet_events_tx).await?;
    Ok((
        plugin_manager_commands_tx,
        plugin_manager_commands_rx,
        rpc_handle,
        ws_handle,
        rpc_close_handle,
        ws_close_handle,
    ))
}

async fn start_http_rpc_server_runloop(
    config: &SurfpoolConfig,
    middleware: SurfpoolMiddleware,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(JoinHandle<()>, jsonrpc_http_server::CloseHandle), String> {
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

    // Build the server on a blocking thread to avoid tokio runtime conflict.
    // jsonrpc_http_server::start_http() creates an internal tokio runtime which
    // panics if called from within an existing async context (nestable_block_on).
    let server = tokio::task::spawn_blocking(move || {
        ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .threads(6)
            .start_http(&server_bind)
    })
    .await
    .map_err(|e| format!("Failed to spawn server builder: {:?}", e))?
    .map_err(|e| format!("Failed to start RPC server: {:?}", e))?;

    let close_handle = server.close_handle();

    let handle = hiro_system_kit::thread_named("RPC Handler")
        .spawn(move || {
            server.wait();
            let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
        })
        .map_err(|e| format!("Failed to spawn RPC Handler thread: {:?}", e))?;

    Ok((handle, close_handle))
}
async fn start_ws_rpc_server_runloop(
    config: &SurfpoolConfig,
    middleware: SurfpoolMiddleware,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(JoinHandle<()>, jsonrpc_ws_server::CloseHandle), String> {
    let ws_server_bind: SocketAddr = config
        .rpc
        .get_ws_base_url()
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;

    // Use a oneshot channel to receive the close handle from the spawned thread
    let (close_handle_tx, close_handle_rx) =
        std::sync::mpsc::sync_channel::<jsonrpc_ws_server::CloseHandle>(1);

    // Create termination flag for subscription loops
    let ws_terminate = Arc::new(AtomicBool::new(false));
    let ws_terminate_for_shutdown = Arc::clone(&ws_terminate);

    let ws_handle = hiro_system_kit::thread_named("WebSocket RPC Handler")
        .spawn(move || {
            // The pubsub handler needs its own Tokio runtime
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build Tokio runtime");

            let tokio_handle = runtime.handle().clone();

            let uid = std::sync::atomic::AtomicUsize::new(0);
            let ws_middleware = SurfpoolWebsocketMiddleware::new(middleware.clone(), None);

            let mut rpc_io = PubSubHandler::new(MetaIoHandler::with_middleware(ws_middleware));

            rpc_io.extend_with(
                rpc::ws::SurfpoolWsRpc {
                    uid,
                    signature_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    account_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    slot_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    logs_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    snapshot_subscription_map: Arc::new(RwLock::new(HashMap::new())),
                    tokio_handle,
                    terminate: ws_terminate,
                }
                .to_delegate(),
            );

            let server = match WsServerBuilder::new(rpc_io)
                .session_meta_extractor(move |ctx: &RequestContext| {
                    let runloop_context = RunloopContext {
                        id: None,
                        svm_locker: middleware.surfnet_svm.clone(),
                        simnet_commands_tx: middleware.simnet_commands_tx.clone(),
                        plugin_manager_commands_tx: middleware.plugin_manager_commands_tx.clone(),
                        remote_rpc_client: middleware.remote_rpc_client.clone(),
                        rpc_config: middleware.config.clone(),
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

            // Send the close handle back to the main thread
            let _ = close_handle_tx.send(server.close_handle());

            server.wait().ok();

            // Signal termination to subscription tasks
            ws_terminate_for_shutdown.store(true, Ordering::Release);

            // Give subscription tasks time to exit, then force shutdown
            runtime.shutdown_timeout(Duration::from_secs(5));

            let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
        })
        .map_err(|e| format!("Failed to spawn WebSocket RPC Handler thread: {:?}", e))?;

    // Wait for the close handle from the spawned thread
    let close_handle = close_handle_rx
        .recv()
        .map_err(|_| "Failed to receive WebSocket server close handle")?;

    Ok((ws_handle, close_handle))
}
