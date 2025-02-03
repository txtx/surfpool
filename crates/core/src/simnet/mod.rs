use chrono::{DateTime, Local, Utc};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    clock::Clock, epoch_info::EpochInfo, pubkey::Pubkey, transaction::VersionedTransaction,
};
use std::{
    sync::{mpsc::Sender, Arc, RwLock},
    thread::sleep,
    time::Duration,
};
use tokio::sync::broadcast;

use crate::rpc::{
    self, accounts_data::AccountsData, bank_data::BankData, full::Full, minimal::Minimal, Config,
    SurfpoolMiddleware,
};

const DEFAULT_SLOT_TIME: u64 = 400;

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
}

pub enum SimnetEvent {
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
    simnet_events_tx: Sender<SimnetEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let svm = LiteSVM::new();

    // Todo: should check config first
    let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com");
    let epoch_info = rpc_client.get_epoch_info().unwrap();
    let _ = simnet_events_tx.send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));
    // Question: can the value `slots_in_epoch` fluctuate over time?
    let slots_in_epoch = epoch_info.slots_in_epoch;

    let context = GlobalState {
        svm,
        transactions_processed: 0,
        epoch_info,
    };

    let context = Arc::new(RwLock::new(context));
    let (mempool_tx, mut mempool_rx) = broadcast::channel(1024);
    let middleware = SurfpoolMiddleware {
        context: context.clone(),
        mempool_tx,
        config: Config {},
    };

    let _handle = hiro_system_kit::thread_named("rpc handler").spawn(move || {
        let mut io = MetaIoHandler::with_middleware(middleware);
        io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
        io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
        io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
        io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());
        let res = ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .start_http(&"127.0.0.1:8899".parse().unwrap());
        let Ok(server) = res else {
            std::process::exit(1);
        };
        server.wait();
    });

    loop {
        sleep(Duration::from_millis(DEFAULT_SLOT_TIME));
        let unix_timestamp: i64 = Utc::now().timestamp();

        let Ok(mut ctx) = context.try_write() else {
            println!("unable to lock svm");
            continue;
        };

        while let Ok(tx) = mempool_rx.try_recv() {
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
