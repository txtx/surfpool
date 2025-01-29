use std::{path::PathBuf, thread::sleep, time::Duration};
use chrono::Utc;
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{AccessControlAllowOrigin, DomainsValidation, ServerBuilder};
use litesvm::LiteSVM;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{clock::Clock, transaction::Transaction};

use crate::rpc::{self, minimal::Minimal, Router};


// fn read_counter_program() -> Vec<u8> {
//     let mut so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
//     so_path.push("crates/test_programs/target/deploy/counter.so");
//     std::fs::read(so_path).unwrap()
// }

pub async fn start(svm: &mut LiteSVM) {
    let mut mempool: Vec<Transaction> = Vec::new();

    let handle = hiro_system_kit::thread_named("rpc handler").spawn(move || {
        let mut io = MetaIoHandler::default();
        io.extend_with(rpc::minimal::MinimalImpl.to_delegate());
        let server = ServerBuilder::new(io)
            .request_middleware(Router {})
            .cors(DomainsValidation::Disabled)
            .start_http(&"127.0.0.1:8899".parse().unwrap())
            .expect("Unable to start RPC server");
        server.wait();
    });

    // Start RPC
    let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com");

    // Main runloop
    let epoch_info = rpc_client.get_epoch_info().unwrap();
    let mut slot = epoch_info.slot_index;
    let mut epoch = epoch_info.epoch;
    let slots_in_epoch = epoch_info.slots_in_epoch;

    loop {
        sleep(Duration::from_millis(400));
        let unix_timestamp: i64 = Utc::now().timestamp();
    
        for tx in mempool.drain(..) {
            tx.verify().unwrap();
            let message = tx.message();
            for instruction in &message.instructions {
                // The Transaction may not be sanitized at this point
                if instruction.program_id_index as usize >= message.account_keys.len() {
                    unreachable!();
                }
                let program_id = &message.account_keys[instruction.program_id_index as usize];

                // let mut pt = solana_program_test::ProgramTest::default();
                // add_program(&read_counter_program(), program_id, &mut pt);
                // let mut ctx = pt.start_with_context().await;        
                
                if svm.get_account(&program_id).is_none() {
                    println!("Retrieving account from Mainnet: {:?}", program_id);
                    // solana_rpc_client::rpc_client::RpcClient::new(url)
                    let mainnet_account = rpc_client.get_account(&program_id).unwrap();
                    let _ = svm.set_account(*program_id, mainnet_account);
                    println!("Injecting {:?}", program_id);
                }
            }
            let res= svm.send_transaction(tx);
            println!("{:?}", res);
        }
        slot +=1;
        if slot > slots_in_epoch {
            slot = 0;
            epoch += 1;
        }
        svm.expire_blockhash();
        let clock: Clock = Clock {
            slot,
            epoch_start_timestamp: 0,
            epoch,
            leader_schedule_epoch: 0,
            unix_timestamp,
        };
        svm.set_sysvar(&clock);
        // println!("{:?} / {:?}", clock, svm.latest_blockhash());
    }
}