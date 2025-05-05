use crossbeam_channel::unbounded;
use jsonrpc_core::{
    futures::future::{self, join_all},
    Error,
};
use jsonrpc_core_client::transports::http;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{
    compiled_instruction::CompiledInstruction, v0::MessageAddressTableLookup,v0,Message,
    MessageHeader, VersionedMessage,
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::{pubkey,Pubkey};
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::versioned::VersionedTransaction;
use std::{str::FromStr, time::Duration};
use surfpool_types::{
    types::{BlockProductionMode, RpcConfig, SimnetConfig},
    SimnetEvent, SurfpoolConfig,
};
use tokio::task;

use crate::{
    rpc::{full::FullClient, minimal::MinimalClient},
    runloops::start_local_surfnet_runloop,
    surfnet::SurfnetSvm,
    tests::helpers::get_free_port,
};

#[tokio::test]
async fn test_simnet_ready() {
    let config = SurfpoolConfig {
        simnets: vec![SimnetConfig {
            block_production_mode: BlockProductionMode::Manual, // Prevent ticks
            ..SimnetConfig::default()
        }],
        ..SurfpoolConfig::default()
    };

    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            surfnet_svm,
            config,
            subgraph_commands_tx,
            simnet_commands_tx,
            simnet_commands_rx,
            geyser_events_rx,
        );
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    match simnet_events_rx.recv() {
        Ok(SimnetEvent::Ready) | Ok(SimnetEvent::Connected(_)) => (),
        e => panic!("Expected Ready event: {e:?}"),
    }
}

#[tokio::test]
async fn test_simnet_ticks() {
    let bind_host = "127.0.0.1";
    let bind_port = get_free_port().unwrap();
    let config = SurfpoolConfig {
        simnets: vec![SimnetConfig {
            slot_time: 1,
            ..SimnetConfig::default()
        }],
        rpc: RpcConfig {
            bind_host: bind_host.to_string(),
            bind_port,
            ..Default::default()
        },
        ..SurfpoolConfig::default()
    };

    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = unbounded();
    let (test_tx, test_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            surfnet_svm,
            config,
            subgraph_commands_tx,
            simnet_commands_tx,
            simnet_commands_rx,
            geyser_events_rx,
        );
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    let _ = hiro_system_kit::thread_named("ticks").spawn(move || {
        let mut ticks = 0;
        loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::ClockUpdate(_)) => ticks += 1,
                _ => (),
            }

            if ticks > 100 {
                let _ = test_tx.send(true);
            }
        }
    });

    match test_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(_) => (),
        Err(_) => panic!("not enough ticks"),
    }
}

#[tokio::test]
async fn test_simnet_some_sol_transfers() {
    let n_addresses = 10;
    let airdrop_keypairs = (0..n_addresses).map(|_| Keypair::new()).collect::<Vec<_>>();
    let airdrop_addresses: Vec<Pubkey> = airdrop_keypairs.iter().map(|kp| kp.pubkey()).collect();
    let airdrop_token_amount = LAMPORTS_PER_SOL;
    let bind_host = "127.0.0.1";
    let bind_port = get_free_port().unwrap();
    let config = SurfpoolConfig {
        simnets: vec![SimnetConfig {
            slot_time: 1,
            airdrop_addresses: airdrop_addresses.clone(),
            airdrop_token_amount,
            ..SimnetConfig::default()
        }],
        rpc: RpcConfig {
            bind_host: bind_host.to_string(),
            bind_port,
            ..Default::default()
        },
        ..SurfpoolConfig::default()
    };

    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            surfnet_svm,
            config,
            subgraph_commands_tx,
            simnet_commands_tx,
            simnet_commands_rx,
            geyser_events_rx,
        );
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Ready) | Ok(SimnetEvent::Connected(_)) => break,
            _ => (),
        }
    }

    let minimal_client =
        http::connect::<MinimalClient>(format!("http://{bind_host}:{bind_port}").as_str())
            .await
            .expect("Failed to connect to Surfpool");
    let full_client =
        http::connect::<FullClient>(format!("http://{bind_host}:{bind_port}").as_str())
            .await
            .expect("Failed to connect to Surfpool");

    let recent_blockhash = full_client
        .get_latest_blockhash(None)
        .await
        .map(|r| {
            Hash::from_str(r.value.blockhash.as_str()).expect("Failed to deserialize blockhash")
        })
        .expect("Failed to get blockhash");

    let balances = join_all(
        airdrop_addresses
            .iter()
            .map(|pk| minimal_client.get_balance(pk.to_string(), None)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .expect("Failed to fetch balances");

    assert!(
        balances.iter().all(|b| b.value == airdrop_token_amount),
        "All addresses did not receive the airdrop"
    );

    let _transfers = join_all(airdrop_keypairs.iter().map(|kp| {
        let msg = Message::new_with_blockhash(
            &[system_instruction::transfer(
                &kp.pubkey(),
                &airdrop_addresses[0],
                airdrop_token_amount / 2,
            )],
            Some(&kp.pubkey()),
            &recent_blockhash,
        );

        let Ok(tx) = VersionedTransaction::try_new(
            VersionedMessage::Legacy(msg),
            &vec![kp.insecure_clone()],
        ) else {
            return Box::pin(future::err(Error::invalid_params("tx")));
        };

        let Ok(encoded) = bincode::serialize(&tx) else {
            return Box::pin(future::err(Error::invalid_params("encoded")));
        };
        let data = bs58::encode(encoded).into_string();

        Box::pin(future::ready(Ok(full_client.send_transaction(data, None))))
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .expect("Transfers failed");

    // Wait for all transactions to be received
    let expected = airdrop_addresses.len();
    let _ = task::spawn_blocking(move || {
        let mut processed = 0;
        loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::TransactionProcessed(..)) => processed += 1,
                _ => (),
            }

            if processed == expected {
                break;
            }
        }
    })
    .await;

    let final_balances = join_all(
        airdrop_addresses
            .iter()
            .map(|pk| minimal_client.get_balance(pk.to_string(), None)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .expect("Failed to fetch final balances");

    assert!(
        final_balances.iter().enumerate().all(|(i, b)| {
            if i == 0 {
                b.value > airdrop_token_amount
            } else {
                b.value < airdrop_token_amount / 2
            }
        }), // TODO: compute fee
        "Some transfers failed"
    );
}

//Sample jupiter CPI swap txn that uses ALTs, the purpose is to test whether the ALT accounts
//are getting fetched as part of the accounts that surfpool fetches to store locally
#[tokio::test]
async fn test_add_alt_fetching() {
    let payer = Keypair::new();
    let pk = payer.pubkey();
    // let recent_blockhash = full_client
    // .get_latest_blockhash(None)
    // .await 
    // .map(|r| {
    //     Hash::from_str(r.value.blockhash.as_str()).expect("Failed to deserialize blockhash")
    // }).expect("Failed to get blockhash");
    let mock_msg = v0::Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 11,
        },
        account_keys: vec![
            pk, // this is the payer address
            pubkey!("g7dD1FHSemkUQrX1Eak37wzvDjscgBW2pFCENwjLdMX"),
            pubkey!("7svh6D6s1eqT9WNSZ3zad9LN5STRBYP7jJnyY3xeE9Fr"),
            pubkey!("B4rJzJ4N7EbXvxn9KHpbDudZGDHBU94Dv9CACN3PKctq"),
            pubkey!("CTDshrXhSQwiVdavX1ZKHtXMevrRa8JVgShdNgvcU37H"),
            pubkey!("DVCeozFGbe6ew3eWTnZByjHeYqTq1cvbrB7JJhkLxaRJ"),
            pubkey!("HY8dzcqGXeRo1pb9wmboJME87k3pYmnPhMjdHcfmM4be"),
            pubkey!("HkphEpUqnFBxBuCPEq5j1HA9L8EwmsmRT6UcFKziptM1"),
            pubkey!("Hua38wowX7fRVUzj4i8meQAyoqVRuFRaLkhTHCnqZ81x"),
            pubkey!("JANiiyQqkoH5RqQAhZzQTE4aDWbWobaXv9V9pnxqSGjy"),
            pubkey!("11111111111111111111111111111111"),
            pubkey!("ComputeBudget111111111111111111111111111111"),
            pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"),
            pubkey!("LMMGrBSX84ZC519PSBkppyVdT4XfM3VP3hw4XLXqhrf"),
            pubkey!("ZJpqR6zydcs7bw2YHzCVyDf3A2PajQHctAPhfZ1UDj5"),
            pubkey!("6YawcNeZ74tRyCv4UfGydYMr7eho7vbUR6ScVffxKAb3"),
            pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
            pubkey!("Cd8JNmh6iBHJR2RXKJMLe5NRqYmpkYco7anoar1DWFyy"),
            pubkey!("D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf"),
            pubkey!("GGztQqQ6pCPaJQnNpXBgELr5cs3WwDakRbh1iEMzjgSJ"),
            pubkey!("GZsNmWKbqhMYtdSkkvMdEyQF9k5mLmP7tTKYWZjcHVPE"),
        ],
        recent_blockhash:Hash::from_str("FhJTgaF1K8M8o6fVvGEaJp5SGwwZv9azWRCsiWiqsX8c").expect("Failed to deserialize blockhash"),
        instructions: vec![
            CompiledInstruction {
                program_id_index: 11,
                accounts: vec![],
                data: vec![2, 192, 92, 21, 0],
            },
            CompiledInstruction {
                program_id_index: 11,
                accounts: vec![],
                data: vec![3, 64, 13, 3, 0, 0, 0, 0, 0],
            },
            CompiledInstruction {
                program_id_index: 16,
                accounts: vec![0, 8, 4, 38, 10, 30],
                data: vec![1],
            },
            CompiledInstruction {
                program_id_index: 16,
                accounts: vec![0, 3, 14, 38, 10, 30],
                data: vec![1],
            },
            CompiledInstruction {
                program_id_index: 13,
                accounts: vec![
                    29, 30, 38, 30, 4, 6, 8, 3, 14, 16, 12, 30, 19, 17, 9, 1, 5, 2, 29, 38, 12, 12,
                    18, 12, 37, 27, 20, 15, 25, 26, 1, 7, 28, 28, 36, 19, 30, 32, 19, 7, 5, 21, 22,
                    23, 24, 35, 31, 34, 33, 30,
                ],
                data: vec![
                    248, 198, 158, 145, 225, 117, 135, 200, 41, 0, 0, 0, 193, 32, 155, 51, 65, 214,
                    156, 129, 0, 2, 0, 0, 0, 58, 1, 100, 0, 1, 56, 100, 1, 2, 128, 132, 30, 0, 0,
                    0, 0, 0, 145, 159, 4, 0, 0, 0, 0, 0, 50, 0, 0, 232, 3, 0, 0, 0, 0, 0, 0,
                ],
            },
        ],
        address_table_lookups: vec![
            MessageAddressTableLookup {
                account_key: pubkey!("5KcPJehcpBLcPde2UhmY4dE9zCrv2r9AKFmW5CGtY1io"),
                writable_indexes: vec![110, 185, 112, 117],
                readonly_indexes: vec![13, 0, 118, 113, 115, 116, 120],
            },
            MessageAddressTableLookup {
                account_key: pubkey!("E1iuryq4fcxyqfQZginfahXTyBC3XmDNsVjSDukkyUrc"),
                writable_indexes: vec![119, 118, 120, 121],
                readonly_indexes: vec![122, 117, 83],
            },
        ],
    };
    let bind_host = "127.0.0.1";
    let bind_port = get_free_port().unwrap();
    let airdrop_token_amount = LAMPORTS_PER_SOL;
    let config = SurfpoolConfig {
        simnets: vec![SimnetConfig {
            slot_time: 1,
            airdrop_addresses: vec![pk], // just one
            airdrop_token_amount,
            ..SimnetConfig::default()
        }],
        rpc: RpcConfig {
            bind_host: bind_host.to_string(),
            bind_port,
            ..Default::default()
        },
        ..SurfpoolConfig::default()
    };

    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
    let (simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            surfnet_svm,
            config,
            subgraph_commands_tx,
            simnet_commands_tx,
            simnet_commands_rx,
            geyser_events_rx,
        );
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Ready) | Ok(SimnetEvent::Connected(_)) => break,
            _ => (),
        }
    }

    let full_client =
        http::connect::<FullClient>(format!("http://{bind_host}:{bind_port}").as_str())
            .await
            .expect("Failed to connect to Surfpool");

    let Ok(tx) = VersionedTransaction::try_new(VersionedMessage::V0(mock_msg.clone()), &[&payer])
    else {
        // return Box::pin(future::err(Error::invalid_params("tx")));
        panic!("Invalid transaction parameters");
    };

    let Ok(encoded) = bincode::serialize(&tx) else {
        // return Box::pin(future::err(Error::invalid_params("encoded")));
        panic!("Failed to serialize transaction");
    };
    let data = bs58::encode(encoded).into_string();

    // Box::pin(future::ready(Ok(full_client.send_transaction(data, None))));
    let _ = match full_client.send_transaction(data, None).await {
        Ok(res) => println!("result {}", res),
        Err(err) => println!("err {}", err),
    };

     // Wait for all transactions to be received
     let _ = task::spawn_blocking(move || {
        let mut processed = 0;
        let expected = 1;
        loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::TransactionProcessed(..)) => processed += 1,
                _ => (),
            }

            if processed == expected {
                break;
            }
        }
    })
    .await;

    // //////////////////////////////////////////////////////
    // let inner = LiteSVM::new()
    // .with_blockhash_check(false); // we might need this 

    // //initialize a testSetup with a liteSVM instance 
    // let svm_setup = new_with_svm(NetworkType::Mainnet, inner);

    // //access the liteSVM instance 
    // let state_reader = svm_setup.context.surfnet_svm.blocking_read().inner;

    // //process the mock versioned tx we have 
    // process_txs(vec![mock_tx]);

    //get all the account keys + the address lookup tables from the txn
    let alts = mock_msg.address_table_lookups.clone();
    let mut acc_keys = mock_msg.account_keys.clone();
    let mut alt_pubkeys = alts.iter().map(|msg| msg.account_key).collect::<Vec<_>>();
    acc_keys.append(&mut alt_pubkeys);

    //inner.get_account returns an Option<Account> assert that none of them are None
    assert!( 
        acc_keys.iter().all(|key| {
           surfnet_svm.inner.get_account(key).is_some()
         }),  
        "account not found"
    );
}
