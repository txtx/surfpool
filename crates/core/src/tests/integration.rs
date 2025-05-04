use crossbeam_channel::unbounded;
use jsonrpc_core::{
    futures::future::{self, join_all},
    Error,
};
use jsonrpc_core_client::transports::http;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{
    Message,
    MessageHeader,
    VersionedMessage,
    v0::MessageAddressTableLookup,
    compiled_instruction::CompiledInstruction,
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
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



#[tokio::test]
async fn test_add_alt_fetching() {
    let mock_tx = VersionedTransaction {
        signatures: [
            "3xmBatMuC9oxysE5FVgevtXdQynSD8PrVrttZ9adZjGMALbG4Lf4iBWnTbxpf1UB2myvDc9CSsVAY7pCv8cHpes6",
        ],
        message: V0(
            Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 11,
                },
                account_keys: [
                    "8DxHEgxBxaea6mNv5yrcu8RGccKCUSZQPH9mzaEffzBR",
                    "g7dD1FHSemkUQrX1Eak37wzvDjscgBW2pFCENwjLdMX",
                    "7svh6D6s1eqT9WNSZ3zad9LN5STRBYP7jJnyY3xeE9Fr",
                    "B4rJzJ4N7EbXvxn9KHpbDudZGDHBU94Dv9CACN3PKctq",
                    "CTDshrXhSQwiVdavX1ZKHtXMevrRa8JVgShdNgvcU37H",
                    "DVCeozFGbe6ew3eWTnZByjHeYqTq1cvbrB7JJhkLxaRJ",
                    "HY8dzcqGXeRo1pb9wmboJME87k3pYmnPhMjdHcfmM4be",
                    "HkphEpUqnFBxBuCPEq5j1HA9L8EwmsmRT6UcFKziptM1",
                    "Hua38wowX7fRVUzj4i8meQAyoqVRuFRaLkhTHCnqZ81x",
                    "JANiiyQqkoH5RqQAhZzQTE4aDWbWobaXv9V9pnxqSGjy",
                    "11111111111111111111111111111111",
                    "ComputeBudget111111111111111111111111111111",
                    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                    "LMMGrBSX84ZC519PSBkppyVdT4XfM3VP3hw4XLXqhrf",
                    "ZJpqR6zydcs7bw2YHzCVyDf3A2PajQHctAPhfZ1UDj5",
                    "6YawcNeZ74tRyCv4UfGydYMr7eho7vbUR6ScVffxKAb3",
                    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
                    "Cd8JNmh6iBHJR2RXKJMLe5NRqYmpkYco7anoar1DWFyy",
                    "D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf",
                    "GGztQqQ6pCPaJQnNpXBgELr5cs3WwDakRbh1iEMzjgSJ",
                    "GZsNmWKbqhMYtdSkkvMdEyQF9k5mLmP7tTKYWZjcHVPE",
                ],
                recent_blockhash: "FhJTgaF1K8M8o6fVvGEaJp5SGwwZv9azWRCsiWiqsX8c",
                instructions: [
                    CompiledInstruction {
                        program_id_index: 11,
                        accounts: [],
                        data: [2, 192, 92, 21, 0],
                    },
                    CompiledInstruction {
                        program_id_index: 11,
                        accounts: [],
                        data: [3, 64, 13, 3, 0, 0, 0, 0, 0],
                    },
                    CompiledInstruction {
                        program_id_index: 16,
                        accounts: [0, 8, 4, 38, 10, 30],
                        data: [1],
                    },
                    CompiledInstruction {
                        program_id_index: 16,
                        accounts: [0, 3, 14, 38, 10, 30],
                        data: [1],
                    },
                    CompiledInstruction {
                        program_id_index: 13,
                        accounts: [
                            29, 30, 38, 30, 4, 6, 8, 3, 14, 16, 12, 30, 19, 17, 9, 1, 5, 2, 29,
                            38, 12, 12, 18, 12, 37, 27, 20, 15, 25, 26, 1, 7, 28, 28, 36, 19, 30,
                            32, 19, 7, 5, 21, 22, 23, 24, 35, 31, 34, 33, 30
                        ],
                        data: [
                            248, 198, 158, 145, 225, 117, 135, 200, 41, 0, 0, 0, 193, 32, 155, 51,
                            65, 214, 156, 129, 0, 2, 0, 0, 0, 58, 1, 100, 0, 1, 56, 100, 1, 2, 128,
                            132, 30, 0, 0, 0, 0, 0, 145, 159, 4, 0, 0, 0, 0, 0, 50, 0, 0, 232, 3,
                            0, 0, 0, 0, 0, 0,
                        ],
                    },
                ],
                address_table_lookups: [
                    MessageAddressTableLookup {
                        account_key: "5KcPJehcpBLcPde2UhmY4dE9zCrv2r9AKFmW5CGtY1io",
                        writable_indexes: [110, 185, 112, 117],
                        readonly_indexes: [13, 0, 118, 113, 115, 116, 120],
                    },
                    MessageAddressTableLookup {
                        account_key: "E1iuryq4fcxyqfQZginfahXTyBC3XmDNsVjSDukkyUrc",
                        writable_indexes: [119, 118, 120, 121],
                        readonly_indexes: [122, 117, 83],
                    },
                ],
            }
        )
    }
    
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

        let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer]).unwrap();
     
        if let Ok(tx_hash) = full_client.send_transaction(&tx).await {
            println!(
                "Transaction confirmed: https://explorer.solana.com/tx/{}",
                tx_hash
            );
        } else {
            println!(
                "Transaction failed: https://explorer.solana.com/tx/{}",
                tx_hash
            );
            return;
        };
    

 
    // let _transfers = join_all(airdrop_keypairs.iter().map(|kp| {
    //     let msg = Message::new_with_blockhash(
    //         &[system_instruction::transfer(
    //             &kp.pubkey(),
    //             &airdrop_addresses[0],
    //             airdrop_token_amount / 2,
    //         )],
    //         Some(&kp.pubkey()),
    //         &recent_blockhash,
    //     );

    //     let Ok(tx) = VersionedTransaction::try_new(
    //         VersionedMessage::Legacy(msg),
    //         &vec![kp.insecure_clone()],
    //     ) else {
    //         return Box::pin(future::err(Error::invalid_params("tx")));
    //     };

    //     let Ok(encoded) = bincode::serialize(&tx) else {
    //         return Box::pin(future::err(Error::invalid_params("encoded")));
    //     };
    //     let data = bs58::encode(encoded).into_string();

    //     Box::pin(future::ready(Ok(full_client.send_transaction(data, None))))
    // }))
    // .await
    // .into_iter()
    // .collect::<Result<Vec<_>, _>>()
    // .expect("Transfers failed");

    // Wait for all transactions to be received
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

    //assert that list of final account keys internally == [all pubkeys + alts]
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
