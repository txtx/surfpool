use crate::error::SurfpoolError;
use crossbeam_channel::unbounded;
use jsonrpc_core::{
    futures::future::{self, join_all},
    Error,
};
use jsonrpc_core_client::transports::http;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{
    v0::{self},
    AddressLookupTableAccount, Message, VersionedMessage,
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::{pubkey, Pubkey};
use solana_sdk::system_instruction::transfer;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::versioned::VersionedTransaction;
use std::{str::FromStr, sync::Arc, time::Duration};
use surfpool_types::{
    types::{BlockProductionMode, RpcConfig, SimnetConfig},
    SimnetEvent, SurfpoolConfig,
};
use tokio::{sync::RwLock, task};
use crate::rpc::surfnet_cheatcodes::{SvmTricksRpc, ComputeUnitsEstimationResult};
use jsonrpc_core::Result as JsonRpcResult;
use solana_rpc_client_api::response::Response as RpcResponse;
use crossbeam_channel::unbounded as crossbeam_unbounded;
use crate::rpc::RunloopContext;
use surfpool_types::SimnetCommand;
use crate::PluginManagerCommand;

use crate::{
    rpc::{full::FullClient, minimal::MinimalClient},
    runloops::start_local_surfnet_runloop,
    surfnet::SurfnetSvm,
    tests::helpers::get_free_port,
};

fn wait_for_ready_and_connected(simnet_events_rx: &crossbeam_channel::Receiver<SimnetEvent>) {
    let mut ready = false;
    let mut connected = false;
    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Ready) => {
                ready = true;
            }
            Ok(SimnetEvent::Connected(_)) => {
                connected = true;
            }
            _ => (),
        }
        if ready && connected {
            break;
        }
    }
}

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
    let svm_locker = Arc::new(RwLock::new(surfnet_svm));

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            svm_locker,
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
    let svm_locker = Arc::new(RwLock::new(surfnet_svm));

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            svm_locker,
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
    let svm_locker = Arc::new(RwLock::new(surfnet_svm));

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            svm_locker,
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

    wait_for_ready_and_connected(&simnet_events_rx);

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

// This test is pretty minimal for lookup tables at this point.
// We are creating a v0 transaction with a lookup table that does exist on mainnet,
// and sending that tx to surfpool. We are verifying that the transaction is processed
// and that the lookup table and its entries are fetched from mainnet and added to the accounts in the SVM.
// However, we are not actually setting up a tx that will use the lookup table internally,
// we are kind of just trusting that LiteSVM will do its job here.
#[tokio::test]
async fn test_add_alt_entries_fetching() {
    let payer = Keypair::new();
    let pk = payer.pubkey();

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
    let svm_locker = Arc::new(RwLock::new(surfnet_svm));

    let moved_svm_locker = svm_locker.clone();
    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            moved_svm_locker,
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

    wait_for_ready_and_connected(&simnet_events_rx);

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

    let random_address = pubkey!("7zdYkYf7yD83j3TLXmkhxn6LjQP9y9bQ4pjfpquP8Hqw");

    let instruction = transfer(&pk, &random_address, 100);

    let alt_address = pubkey!("5KcPJehcpBLcPde2UhmY4dE9zCrv2r9AKFmW5CGtY1io"); // a mainnet lookup table

    let address_lookup_table_account = AddressLookupTableAccount {
        key: alt_address,
        addresses: vec![random_address],
    };

    let tx = VersionedTransaction::try_new(
        VersionedMessage::V0(
            v0::Message::try_compile(
                &payer.pubkey(),
                &[instruction],
                &[address_lookup_table_account],
                recent_blockhash,
            )
            .expect("Failed to compile message"),
        ),
        &[payer],
    )
    .expect("Failed to create transaction");

    let Ok(encoded) = bincode::serialize(&tx) else {
        panic!("Failed to serialize transaction");
    };
    let data = bs58::encode(encoded).into_string();

    let _ = match full_client.send_transaction(data, None).await {
        Ok(res) => println!("Send transaction result: {}", res),
        Err(err) => println!("Send transaction error result: {}", err),
    };

    // Wait for all transactions to be received
    let _ = task::spawn_blocking(move || {
        let mut processed = 0;
        let expected = 1;
        let mut alt_updated = false;
        loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::TransactionProcessed(..)) => processed += 1,
                Ok(SimnetEvent::AccountUpdate(_, account)) => {
                    if account == alt_address {
                        alt_updated = true;
                    }
                }
                _ => (),
            }

            if processed == expected && alt_updated {
                break;
            }
        }
    })
    .await;

    let surfnet_svm = svm_locker.read().await;

    // get all the account keys + the address lookup tables + table_entries from the txn
    let alts = tx.message.address_table_lookups().clone().unwrap();
    let mut acc_keys = tx.message.static_account_keys().to_vec();
    let mut alt_pubkeys = alts.iter().map(|msg| msg.account_key).collect::<Vec<_>>();
    let mut table_entries = join_all(alts.iter().map(|msg| async {
        let loaded_addresses = surfnet_svm.load_lookup_table_addresses(msg).await?;
        let mut combined = loaded_addresses.writable;
        combined.extend(loaded_addresses.readonly);
        Ok::<_, SurfpoolError>(combined)
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<Vec<Pubkey>>, SurfpoolError>>()
    .unwrap() // Result<Vec<Vec<Pubkey>>, _>
    .into_iter()
    .flatten()
    .collect();

    acc_keys.append(&mut alt_pubkeys);
    acc_keys.append(&mut table_entries);

    // inner.get_account returns an Option<Account>; assert that none of them are None
    assert!(
        acc_keys
            .iter()
            .all(|key| { surfnet_svm.inner.get_account(key).is_some() }),
        "account not found"
    );
}

#[tokio::test]
async fn test_surfnet_estimate_compute_units() {
    let (mut svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let rpc_server = crate::rpc::surfnet_cheatcodes::SurfnetCheatcodesRpc;

    let payer = Keypair::new(); 
    let recipient = Pubkey::new_unique();
    let lamports_to_send = 1_000_000; 

    svm_instance.airdrop(&payer.pubkey(), lamports_to_send * 2).unwrap();

    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_instance.latest_blockhash();
    let message = Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(
        VersionedMessage::Legacy(message.clone()), 
        &[&payer],
    )
    .unwrap();

    let tx_bytes = bincode::serialize(&tx).unwrap();
    let tx_b64 = base64::encode(&tx_bytes);

    // Manually construct RunloopContext
    let svm_locker_for_context = Arc::new(RwLock::new(svm_instance));
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        surfnet_svm: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
    };

    let response: JsonRpcResult<RpcResponse<ComputeUnitsEstimationResult>> = rpc_server
        .estimate_compute_units(Some(runloop_context), tx_b64)
        .await;

    assert!(response.is_ok(), "RPC call failed: {:?}", response.err());
    let rpc_response_value = response.unwrap().value;

    assert!(rpc_response_value.success, "CU estimation failed");
    assert!(rpc_response_value.compute_units_consumed > 0, "Invalid compute units consumed");
    assert!(rpc_response_value.error_message.is_none(), "Error message should be None. Got: {:?}", rpc_response_value.error_message);
    assert!(rpc_response_value.log_messages.is_some(), "Log messages should be present");

    // Test send_transaction with cu_analysis_enabled = true
    // Create a new SVM instance for this part to avoid RwLock issues with the previous instance if it's still in runloop_context
    let (mut svm_for_send, simnet_rx_for_send, _geyser_rx_for_send) = SurfnetSvm::new();
    svm_for_send.airdrop(&payer.pubkey(), lamports_to_send * 2).unwrap();
    
    let latest_blockhash_for_send = svm_for_send.latest_blockhash();
    let message_for_send = Message::new_with_blockhash(
        &[transfer(&payer.pubkey(), &recipient, lamports_to_send)], 
        Some(&payer.pubkey()),
        &latest_blockhash_for_send
    );
    let tx_for_send = VersionedTransaction::try_new(
        VersionedMessage::Legacy(message_for_send),
        &[&payer],
    ).unwrap();

    let _send_result = svm_for_send.send_transaction(tx_for_send, true);

    let mut found_cu_event = false;
    for _ in 0..10 { 
        if let Ok(event) = simnet_rx_for_send.try_recv() { 
            if let surfpool_types::SimnetEvent::InfoLog(_, msg) = event { 
                if msg.starts_with("CU Estimation for tx") {
                    println!("Found CU estimation event: {}", msg);
                    found_cu_event = true;
                    break;
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await; 
    }
    assert!(found_cu_event, "Did not find CU estimation SimnetEvent");
}
