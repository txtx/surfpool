use chrono::Utc;
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{clock::Clock, epoch_info::EpochInfo};
use std::{
    sync::{Arc, RwLock},
    thread::sleep,
    time::Duration,
};
use tokio::sync::broadcast;

use crate::rpc::{self, full::Full, minimal::Minimal, Config, SurfpoolMiddleware};

const DEFAULT_SLOT_TIME: u64 = 2000;

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
}

pub async fn start() -> Result<(), Box<dyn std::error::Error>> {
    let svm = LiteSVM::new();

    // Todo: should check config first
    let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com");
    let epoch_info = rpc_client.get_epoch_info().unwrap();
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
        let server = ServerBuilder::new(io)
            .cors(DomainsValidation::Disabled)
            .start_http(&"127.0.0.1:8899".parse().unwrap())
            .expect("Unable to start RPC server");
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
            tx.verify_with_results();
            let tx = tx.into_legacy_transaction().unwrap();
            let message = &tx.message;
            println!("Processing Transaction {:?}", tx);
            for instruction in &message.instructions {
                // The Transaction may not be sanitized at this point
                if instruction.program_id_index as usize >= message.account_keys.len() {
                    unreachable!();
                }
                let program_id = &message.account_keys[instruction.program_id_index as usize];

                // let mut pt = solana_program_test::ProgramTest::default();
                // add_program(&read_counter_program(), program_id, &mut pt);
                // let mut ctx = pt.start_with_context().await;

                if ctx.svm.get_account(&program_id).is_none() {
                    println!("Retrieving account from Mainnet: {:?}", program_id);
                    let mainnet_account = rpc_client.get_account(&program_id).unwrap();
                    let _ = ctx.svm.set_account(*program_id, mainnet_account);
                    println!("Injecting {:?}", program_id);
                }
            }
            let res = ctx.svm.send_transaction(tx);
            println!("{:?}", res);
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
        ctx.svm.set_sysvar(&clock);
        println!("{:?} / {:?}", clock, ctx.svm.latest_blockhash());
    }
}
