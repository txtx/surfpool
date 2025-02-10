use chrono::{DateTime, Local, Utc};
use crossbeam::select;
use crossbeam_channel::{unbounded, Receiver, Sender};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    clock::Clock, epoch_info::EpochInfo, pubkey::Pubkey, transaction::VersionedTransaction,
};
use std::{
    net::SocketAddr,
    sync::{Arc, RwLock},
    thread::sleep,
    time::Duration,
};

use crate::{
    rpc::{
        self, accounts_data::AccountsData, bank_data::BankData, full::Full, minimal::Minimal,
        SurfpoolMiddleware,
    },
    types::{RunloopTriggerMode, SurfpoolConfig},
};

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
    pub rpc_client: Arc<RpcClient>,
}

#[derive(Debug)]
pub enum SimnetEvent {
    Ready,
    Aborted(String),
    Shutdown,
    ClockUpdate(Clock),
    EpochInfoUpdate(EpochInfo),
    BlockHashExpired,
    InfoLog(DateTime<Local>, String),
    ErrorLog(DateTime<Local>, String),
    WarnLog(DateTime<Local>, String),
    DebugLog(DateTime<Local>, String),
    TransactionReceived(DateTime<Local>, VersionedTransaction),
    AccountUpdate(DateTime<Local>, Pubkey),
}

pub enum SimnetCommand {
    SlotForward,
    SlotBackward,
    UpdateClock(ClockCommand),
    UpdateRunloopMode(RunloopTriggerMode),
}

pub enum ClockCommand {
    Pause,
    Resume,
    Toggle,
    UpdateSlotInterval(u64),
}

pub enum ClockEvent {
    Tick,
}

pub async fn start(
    config: &SurfpoolConfig,
    simnet_events_tx: Sender<SimnetEvent>,
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
    // Question: can the value `slots_in_epoch` fluctuate over time?
    let slots_in_epoch = epoch_info.slots_in_epoch;

    let context = GlobalState {
        svm,
        transactions_processed: 0,
        epoch_info: epoch_info.clone(),
        rpc_client: rpc_client.clone(),
    };

    let context = Arc::new(RwLock::new(context));
    let (mempool_tx, mempool_rx) = unbounded();
    let middleware = SurfpoolMiddleware {
        context: context.clone(),
        mempool_tx,
        config: config.rpc.clone(),
    };

    let simnet_events_tx_copy = simnet_events_tx.clone();
    let server_bind: SocketAddr = config.rpc.get_socket_address().parse()?;

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());

    let _handle = hiro_system_kit::thread_named("rpc handler").spawn(move || {
        let server = ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .start_http(&server_bind)
            .unwrap();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Ready);
        server.wait();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Shutdown);
    });

    let (clock_event_tx, clock_event_rx) = unbounded::<ClockEvent>();
    let (clock_command_tx, clock_command_rx) = unbounded::<ClockCommand>();

    let mut slot_time = config.simnet.slot_time;
    let _handle = hiro_system_kit::thread_named("clock").spawn(move || {
        let mut enabled = true;
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
            }
        }
    });

    let _ = simnet_events_tx.send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));
    let mut runloop_trigger_mode = config.simnet.runloop_trigger_mode.clone();
    let mut transactions_to_process = vec![];
    loop {
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
                    }
                },
                Err(_) => {},
            },
            recv(mempool_rx) -> msg => match msg {
                Ok(transaction) => {
                    transactions_to_process.push(transaction);
                },
                Err(_) => {},
            },
            default => {},
        }

        if !create_slot {
            continue;
        }

        // We will create a slot!
        let unix_timestamp: i64 = Utc::now().timestamp();
        let Ok(mut ctx) = context.write() else {
            continue;
        };

        // Handle the transactions accumulated
        for (_key, tx) in transactions_to_process.drain(..) {
            let _ =
                simnet_events_tx.send(SimnetEvent::TransactionReceived(Local::now(), tx.clone()));

            tx.verify_with_results();
            let tx = tx.into_legacy_transaction().unwrap();
            let message = &tx.message;

            // println!("Processing Transaction {:?}", tx);
            for instruction in &message.instructions {
                // The Transaction may not be sanitized at this point
                if instruction.program_id_index as usize >= message.account_keys.len() {
                    unreachable!();
                }
                let program_id = &message.account_keys[instruction.program_id_index as usize];
                if ctx.svm.get_account(&program_id).is_none() {
                    // println!("Retrieving account from Mainnet: {:?}", program_id);
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
                    let _ = simnet_events_tx.send(event);
                }
            }
            match ctx.svm.send_transaction(tx.clone()) {
                Ok(_) => {}
                Err(e) => {
                    let _ = simnet_events_tx.send(SimnetEvent::ErrorLog(
                        Local::now(),
                        format!("Error processing transaction: {}", e.err.to_string()),
                    ));
                }
            }
        }
        ctx.epoch_info.slot_index += 1;
        ctx.epoch_info.absolute_slot += 1;
        if ctx.epoch_info.slot_index > slots_in_epoch {
            ctx.epoch_info.slot_index = 0;
            ctx.epoch_info.epoch += 1;
        }
        ctx.svm.expire_blockhash();
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
