use chrono::{DateTime, Local, Utc};
use crossbeam_channel::Sender;
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    clock::Clock, epoch_info::EpochInfo, pubkey::Pubkey, transaction::VersionedTransaction,
};
use std::{
    net::SocketAddr,
    sync::{mpsc::channel, Arc, RwLock},
    thread::sleep,
    time::Duration,
};
use tokio::sync::broadcast;

use crate::{
    rpc::{
        self, accounts_data::AccountsData, bank_data::BankData, full::Full, minimal::Minimal,
        SurfpoolMiddleware,
    },
    types::SurfpoolConfig,
};

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
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

pub async fn start(
    config: &SurfpoolConfig,
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let svm = LiteSVM::new();

    // Todo: should check config first
    let rpc_client = RpcClient::new(&config.simnet.remote_rpc_url);
    let epoch_info = rpc_client.get_epoch_info()?;
    // Question: can the value `slots_in_epoch` fluctuate over time?
    let slots_in_epoch = epoch_info.slots_in_epoch;

    let context = GlobalState {
        svm,
        transactions_processed: 0,
        epoch_info: epoch_info.clone(),
    };

    let context = Arc::new(RwLock::new(context));
    let (mempool_tx, mut mempool_rx) = broadcast::channel(1024);
    let middleware = SurfpoolMiddleware {
        context: context.clone(),
        mempool_tx,
        config: config.rpc.clone(),
    };

    let simnet_events_tx_copy = simnet_events_tx.clone();
    let server_bind: SocketAddr =
        format!("{}:{}", config.rpc.bind_address, config.rpc.bind_port).parse()?;

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());

    let _handle = hiro_system_kit::thread_named("rpc handler").spawn(move || {
        let server = ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .start_http(&server_bind).unwrap();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Ready);
        server.wait();
        let _ = simnet_events_tx_copy.send(SimnetEvent::Shutdown);
    });

    let _ = simnet_events_tx.send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));

    loop {
        sleep(Duration::from_millis(config.simnet.slot_time));
        let unix_timestamp: i64 = Utc::now().timestamp();

        let Ok(mut ctx) = context.try_write() else {
            println!("unable to lock svm");
            continue;
        };

        while let Ok((_, tx)) = mempool_rx.try_recv() {
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
                    let res = rpc_client.get_account(&program_id);
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
            let res = ctx.svm.send_transaction(tx);
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
