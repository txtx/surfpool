use std::{str::FromStr, sync::Arc, time::Duration};

use base64::Engine;
use crossbeam_channel::{unbounded, unbounded as crossbeam_unbounded};
use jsonrpc_core::{
    Error, Result as JsonRpcResult,
    futures::future::{self, join_all},
};
use jsonrpc_core_client::transports::http;
use solana_clock::Slot;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{
    AddressLookupTableAccount, Message, VersionedMessage,
    v0::{self},
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::{Pubkey, pubkey};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::system_instruction::transfer;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_types::{
    Idl, SimnetCommand, SimnetEvent, SurfpoolConfig,
    types::{
        BlockProductionMode, KeyedProfileResult as SurfpoolProfileResult, RpcConfig, SimnetConfig,
        TransactionStatusEvent, UuidOrSignature,
    },
};
use tokio::{sync::RwLock, task};
use uuid::Uuid;

use crate::{
    PluginManagerCommand,
    error::SurfpoolError,
    rpc::{
        RunloopContext,
        full::FullClient,
        minimal::MinimalClient,
        surfnet_cheatcodes::{SurfnetCheatcodesRpc, SvmTricksRpc},
    },
    runloops::start_local_surfnet_runloop,
    surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm},
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

#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
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
    let svm_locker = SurfnetSvmLocker::new(surfnet_svm);

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

#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
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
    let svm_locker = SurfnetSvmLocker::new(surfnet_svm);

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

#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
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
    let svm_locker = SurfnetSvmLocker::new(surfnet_svm);

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
#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
#[tokio::test(flavor = "multi_thread")]
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
            SurfnetSvmLocker(moved_svm_locker),
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
    let svm_locker = SurfnetSvmLocker(svm_locker);

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

    // Wait for all transactions to be received
    let _ = match full_client.send_transaction(data, None).await {
        Ok(res) => println!("Send transaction result: {}", res),
        Err(err) => println!("Send transaction error result: {}", err),
    };

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
            Ok(SimnetEvent::ClockUpdate(_)) => {
                // do nothing
            }
            other => println!("Unexpected event: {:?}", other),
        }

        if processed == expected && alt_updated {
            break;
        }
    }

    // get all the account keys + the address lookup tables + table_entries from the txn
    let alts = tx.message.address_table_lookups().clone().unwrap();
    let mut acc_keys = tx.message.static_account_keys().to_vec();
    let mut alt_pubkeys = alts.iter().map(|msg| msg.account_key).collect::<Vec<_>>();
    let mut table_entries = join_all(alts.iter().map(|msg| async {
        let loaded_addresses = svm_locker
            .get_lookup_table_addresses(&None, msg)
            .await?
            .inner;
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

    assert!(
        acc_keys.iter().all(|key| {
            svm_locker
                .get_account_local(key)
                .inner
                .map_account()
                .is_ok()
        }),
        "account not found"
    );
}

// This test is pretty minimal for lookup tables at this point.
// We are creating a v0 transaction with a lookup table that does exist on mainnet,
// and sending that tx to surfpool. We are verifying that the transaction is processed
// and that the lookup table and its entries are fetched from mainnet and added to the accounts in the SVM.
// However, we are not actually setting up a tx that will use the lookup table internally,
// we are kind of just trusting that LiteSVM will do its job here.
#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_add_alt_entries_fetching() {
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
            SurfnetSvmLocker(moved_svm_locker),
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
    let svm_locker = SurfnetSvmLocker(svm_locker);

    wait_for_ready_and_connected(&simnet_events_rx);

    let full_client =
        http::connect::<FullClient>(format!("http://{bind_host}:{bind_port}").as_str())
            .await
            .expect("Failed to connect to Surfpool");

    let random_address = pubkey!("7zdYkYf7yD83j3TLXmkhxn6LjQP9y9bQ4pjfpquP8Hqw");

    let instruction = transfer(&pk, &random_address, 100);
    let recent_blockhash = svm_locker.with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

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

    let simulation_res = full_client.simulate_transaction(data, None).await.unwrap();
    assert_eq!(
        simulation_res.value.err, None,
        "Unexpected simulation error"
    );
}

#[cfg_attr(feature = "ignore_tests_ci", ignore = "flaky CI tests")]
#[tokio::test(flavor = "multi_thread")]
async fn test_surfnet_estimate_compute_units() {
    let (mut svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let rpc_server = crate::rpc::surfnet_cheatcodes::SurfnetCheatcodesRpc;

    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = 1_000_000;

    svm_instance
        .airdrop(&payer.pubkey(), lamports_to_send * 2)
        .unwrap();

    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_instance.latest_blockhash();
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message.clone()), &[&payer])
        .unwrap();

    let tx_bytes = bincode::serialize(&tx).unwrap();
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    // Manually construct RunloopContext
    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance);
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Test with None tag
    let response_no_tag_initial: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(Some(runloop_context.clone()), tx_b64.clone(), None, None)
        .await;

    assert!(
        response_no_tag_initial.is_ok(),
        "RPC call with None tag failed: {:?}",
        response_no_tag_initial.err()
    );
    let rpc_response_value_no_tag = response_no_tag_initial.unwrap().value;

    assert!(
        rpc_response_value_no_tag
            .transaction_profile
            .error_message
            .is_none(),
        "CU estimation with None tag failed"
    );
    println!(
        "Initial CU estimation (no tag): consumed = {}, success = {}",
        rpc_response_value_no_tag
            .transaction_profile
            .compute_units_consumed,
        rpc_response_value_no_tag
            .transaction_profile
            .error_message
            .is_none()
    );
    assert!(
        rpc_response_value_no_tag
            .transaction_profile
            .compute_units_consumed
            > 0,
        "Invalid compute units consumed for None tag"
    );
    assert!(
        rpc_response_value_no_tag
            .transaction_profile
            .error_message
            .is_none(),
        "Error message should be None for None tag. Got: {:?}",
        rpc_response_value_no_tag.transaction_profile.error_message
    );
    assert!(
        rpc_response_value_no_tag
            .transaction_profile
            .log_messages
            .is_some(),
        "Log messages should be present for None tag"
    );

    // Test 1: Estimate with a tag and retrieve
    let tag1 = "test_tag_1".to_string();
    println!("\nTesting with tag: {}", tag1);
    let response_tagged_1: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(
            Some(runloop_context.clone()),
            tx_b64.clone(),
            Some(tag1.clone()),
            None,
        )
        .await;
    assert!(
        response_tagged_1.is_ok(),
        "RPC call with tag1 failed: {:?}",
        response_tagged_1.err()
    );
    let rpc_response_tagged_1_value = response_tagged_1.unwrap().value;
    assert!(
        rpc_response_tagged_1_value
            .transaction_profile
            .error_message
            .is_none(),
        "CU estimation with tag1 failed"
    );
    println!(
        "CU estimation (tag: {}): consumed = {}, success = {}",
        tag1,
        rpc_response_tagged_1_value
            .transaction_profile
            .compute_units_consumed,
        rpc_response_tagged_1_value
            .transaction_profile
            .error_message
            .is_none()
    );

    println!("Retrieving profile results for tag: {}", tag1);
    let results_response_tag1 = rpc_server
        .get_profile_results_by_tag(Some(runloop_context.clone()), tag1.clone())
        .await;
    assert!(
        results_response_tag1.is_ok(),
        "get_profile_results for tag1 failed: {:?}",
        results_response_tag1.err()
    );
    let results_vec_tag1 = results_response_tag1.unwrap().value.unwrap_or_default();
    assert_eq!(results_vec_tag1.len(), 1, "Expected 1 result for tag1");
    println!(
        "Found {} result(s) for tag: {}",
        results_vec_tag1.len(),
        tag1
    );
    assert_eq!(
        results_vec_tag1[0]
            .transaction_profile
            .compute_units_consumed,
        rpc_response_tagged_1_value
            .transaction_profile
            .compute_units_consumed
    );
    assert_eq!(
        results_vec_tag1[0]
            .transaction_profile
            .error_message
            .is_none(),
        rpc_response_tagged_1_value
            .transaction_profile
            .error_message
            .is_none()
    );
    println!(
        "Verified retrieved result for tag {}: CU = {}, success = {}",
        tag1,
        results_vec_tag1[0]
            .transaction_profile
            .compute_units_consumed,
        results_vec_tag1[0]
            .transaction_profile
            .error_message
            .is_none()
    );

    // Test 2: Retrieve with a non-existent tag
    let tag_non_existent = "non_existent_tag".to_string();
    println!(
        "\nTesting retrieval with non-existent tag: {}",
        tag_non_existent
    );
    let results_non_existent_response = rpc_server
        .get_profile_results_by_tag(Some(runloop_context.clone()), tag_non_existent.clone())
        .await;
    assert!(
        results_non_existent_response.is_ok(),
        "get_profile_results for non-existent tag failed"
    );
    let results_non_existent_vec = results_non_existent_response
        .unwrap()
        .value
        .unwrap_or_default();
    assert!(
        results_non_existent_vec.is_empty(),
        "Expected empty vec for non-existent tag"
    );
    println!(
        "Verified empty results for non-existent tag: {}",
        tag_non_existent
    );

    // Test 3: Estimate multiple times with the same tag
    let tag2 = "test_tag_2".to_string();
    println!("\nTesting multiple estimations with tag: {}", tag2);
    let response_tagged_2a: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(
            Some(runloop_context.clone()),
            tx_b64.clone(),
            Some(tag2.clone()),
            None,
        )
        .await;
    assert!(response_tagged_2a.is_ok(), "First call with tag2 failed");
    let cu_2a_profile_result = response_tagged_2a.unwrap().value;
    println!(
        "CU estimation 1 (tag: {}): consumed = {}, success = {}",
        tag2,
        cu_2a_profile_result
            .transaction_profile
            .compute_units_consumed,
        cu_2a_profile_result
            .transaction_profile
            .error_message
            .is_none()
    );

    let response_tagged_2b: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(
            Some(runloop_context.clone()),
            tx_b64.clone(),
            Some(tag2.clone()),
            None,
        )
        .await;
    assert!(response_tagged_2b.is_ok(), "Second call with tag2 failed");
    let cu_2b_profile_result = response_tagged_2b.unwrap().value;
    println!(
        "CU estimation 2 (tag: {}): consumed = {}, success = {}",
        tag2,
        cu_2b_profile_result
            .transaction_profile
            .compute_units_consumed,
        cu_2b_profile_result
            .transaction_profile
            .error_message
            .is_none()
    );

    println!("Retrieving profile results for tag: {}", tag2);
    let results_response_tag2 = rpc_server
        .get_profile_results_by_tag(Some(runloop_context.clone()), tag2.clone())
        .await;
    assert!(
        results_response_tag2.is_ok(),
        "get_profile_results for tag2 failed"
    );
    let results_vec_tag2 = results_response_tag2.unwrap().value.unwrap_or_default();
    assert_eq!(results_vec_tag2.len(), 2, "Expected 2 results for tag2");
    println!(
        "Found {} result(s) for tag: {}",
        results_vec_tag2.len(),
        tag2
    );
    assert_eq!(
        results_vec_tag2[0]
            .transaction_profile
            .compute_units_consumed,
        cu_2a_profile_result
            .transaction_profile
            .compute_units_consumed
    );
    println!(
        "Verified retrieved result 1 for tag {}: CU = {}",
        tag2,
        results_vec_tag2[0]
            .transaction_profile
            .compute_units_consumed
    );
    assert_eq!(
        results_vec_tag2[1]
            .transaction_profile
            .compute_units_consumed,
        cu_2b_profile_result
            .transaction_profile
            .compute_units_consumed
    );
    println!(
        "Verified retrieved result 2 for tag {}: CU = {}",
        tag2,
        results_vec_tag2[1]
            .transaction_profile
            .compute_units_consumed
    );

    // Test 4: Estimate with another None tag, ensure it doesn't affect tagged results for tag1
    println!(
        "\nTesting None tag again to ensure no interference with tag: {}",
        tag1
    );
    let response_no_tag_again: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(Some(runloop_context.clone()), tx_b64.clone(), None, None)
        .await;
    assert!(
        response_no_tag_again.is_ok(),
        "RPC call with None tag (again) failed"
    );
    let rpc_response_no_tag_again_value = response_no_tag_again.unwrap().value; // consume it
    println!(
        "CU estimation (None tag again): consumed = {}, success = {}",
        rpc_response_no_tag_again_value
            .transaction_profile
            .compute_units_consumed,
        rpc_response_no_tag_again_value
            .transaction_profile
            .error_message
            .is_none()
    );

    println!("Retrieving profile results for tag: {} again", tag1);
    let results_response_tag1_again = rpc_server
        .get_profile_results_by_tag(Some(runloop_context), tag1.clone())
        .await;
    assert!(
        results_response_tag1_again.is_ok(),
        "get_profile_results for tag1 (again) failed"
    );
    let results_vec_tag1_again = results_response_tag1_again
        .unwrap()
        .value
        .unwrap_or_default();
    assert_eq!(
        results_vec_tag1_again.len(),
        1,
        "Expected 1 result for tag1 after another None tag call, was {}",
        results_vec_tag1_again.len()
    );
    println!(
        "Found {} result(s) for tag: {} again",
        results_vec_tag1_again.len(),
        tag1
    );
    assert_eq!(
        results_vec_tag1_again[0]
            .transaction_profile
            .compute_units_consumed,
        rpc_response_tagged_1_value
            .transaction_profile
            .compute_units_consumed
    );
    println!(
        "Verified retrieved result for tag {}: CU = {} (after None tag call)",
        tag1,
        results_vec_tag1_again[0]
            .transaction_profile
            .compute_units_consumed
    );

    // Test send_transaction with cu_analysis_enabled = true
    // Create a new SVM instance
    let (mut svm_for_send, simnet_rx_for_send, _geyser_rx_for_send) = SurfnetSvm::new();
    svm_for_send
        .airdrop(&payer.pubkey(), lamports_to_send * 2)
        .unwrap();

    let latest_blockhash_for_send = svm_for_send.latest_blockhash();
    let message_for_send = Message::new_with_blockhash(
        &[transfer(&payer.pubkey(), &recipient, lamports_to_send)],
        Some(&payer.pubkey()),
        &latest_blockhash_for_send,
    );
    let tx_for_send =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message_for_send), &[&payer])
            .unwrap();

    let _send_result = svm_for_send.send_transaction(tx_for_send, true, true);

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

#[tokio::test(flavor = "multi_thread")]
async fn test_surfnet_estimate_compute_units_with_state_snapshots() {
    let (mut svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let rpc_server = crate::rpc::surfnet_cheatcodes::SurfnetCheatcodesRpc;

    let payer_keypair = Keypair::new();
    let payer_pubkey = payer_keypair.pubkey();
    let recipient_pubkey = Pubkey::new_unique();
    let initial_payer_lamports = 2 * LAMPORTS_PER_SOL;
    let lamports_to_send = 1 * LAMPORTS_PER_SOL;

    // Airdrop to payer
    svm_instance
        .airdrop(&payer_pubkey, initial_payer_lamports)
        .unwrap();

    // Store initial recipient balance (should be 0 or account non-existent)
    let initial_recipient_lamports_pre = svm_instance
        .inner
        .get_account(&recipient_pubkey)
        .map_or(0, |acc| acc.lamports);

    // Create a transfer transaction
    let instruction = transfer(&payer_pubkey, &recipient_pubkey, lamports_to_send);
    let latest_blockhash = svm_instance.latest_blockhash();
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer_pubkey), &latest_blockhash);
    let tx =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message.clone()), &[&payer_keypair])
            .unwrap();

    let tx_bytes = bincode::serialize(&tx).unwrap();
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    // Manually construct RunloopContext
    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance.clone()); // Clone for the locker
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Call profile_transaction
    let response: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(Some(runloop_context.clone()), tx_b64.clone(), None, None)
        .await;

    assert!(response.is_ok(), "RPC call failed: {:?}", response.err());
    let profile_result = response.unwrap().value;

    // Verify compute units part
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "CU estimation failed"
    );
    assert!(
        profile_result.transaction_profile.compute_units_consumed > 0,
        "Invalid CU consumption"
    );

    // Verify pre_execution state
    assert!(
        profile_result
            .transaction_profile
            .state
            .pre_execution
            .contains_key(&payer_pubkey)
    );

    assert!(
        profile_result
            .transaction_profile
            .state
            .pre_execution
            .contains_key(&recipient_pubkey)
    );

    let payer_pre_account = profile_result
        .transaction_profile
        .state
        .pre_execution
        .get(&payer_pubkey)
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(
        payer_pre_account.lamports, initial_payer_lamports,
        "Payer pre-execution lamports mismatch"
    );

    let recipient_pre_account = profile_result
        .transaction_profile
        .state
        .pre_execution
        .get(&recipient_pubkey)
        .unwrap();

    assert!(
        recipient_pre_account.is_none(),
        "Recipient pre-execution account should be None (not created yet)"
    );

    // Verify post_execution state
    assert!(
        profile_result
            .transaction_profile
            .state
            .post_execution
            .contains_key(&payer_pubkey)
    );
    assert!(
        profile_result
            .transaction_profile
            .state
            .post_execution
            .contains_key(&recipient_pubkey)
    );

    let payer_post_data = profile_result
        .transaction_profile
        .state
        .post_execution
        .get(&payer_pubkey)
        .unwrap()
        .as_ref()
        .unwrap();

    // Payer's balance should decrease by lamports_to_send + fees. For simplicity, just check that it's less than initial.
    // A more precise check would involve calculating exact fees, which LiteSVM not expose easily here.
    assert!(
        payer_post_data.lamports < initial_payer_lamports,
        "Payer post-execution lamports did not decrease as expected"
    );
    assert!(
        payer_post_data.lamports <= initial_payer_lamports - lamports_to_send,
        "Payer post-execution lamports mismatch after send"
    );

    let recipient_post_data = profile_result
        .transaction_profile
        .state
        .post_execution
        .get(&recipient_pubkey)
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(
        recipient_post_data.lamports,
        initial_recipient_lamports_pre + lamports_to_send,
        "Recipient post-execution lamports mismatch"
    );

    println!("Profiled transaction successfully with state snapshots.");
    println!(
        "  Payer pre-lamports: {}, post-lamports: {}",
        payer_pre_account.lamports, payer_post_data.lamports
    );
    println!(
        "  Recipient pre-lamports: {}, post-lamports: {}",
        initial_recipient_lamports_pre, recipient_post_data.lamports
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transaction_profile() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (mut svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = 1_000_000;

    svm_instance
        .airdrop(&payer.pubkey(), lamports_to_send * 2)
        .unwrap();

    // Create a transaction to profile
    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_instance.latest_blockhash();
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message.clone()), &[&payer])
        .unwrap();

    let tx_bytes = bincode::serialize(&tx).unwrap();
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    // Manually construct RunloopContext
    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance);
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Test 1: Profile a transaction with a tag and retrieve by UUID
    let tag = "test_get_transaction_profile_tag".to_string();
    println!("Testing transaction profiling with tag: {}", tag);

    let profile_response: JsonRpcResult<RpcResponse<SurfpoolProfileResult>> = rpc_server
        .profile_transaction(
            Some(runloop_context.clone()),
            tx_b64.clone(),
            Some(tag.clone()),
            None,
        )
        .await;

    assert!(
        profile_response.is_ok(),
        "Profile transaction failed: {:?}",
        profile_response.err()
    );

    let profile_result = profile_response.unwrap().value;
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Transaction profiling failed"
    );

    let UuidOrSignature::Uuid(uuid) = profile_result.key else {
        panic!(
            "Expected a UUID from the profile result, got: {:?}",
            profile_result.key
        );
    };
    println!("Generated UUID: {}", uuid);

    // Test 2: Retrieve profile by UUID
    println!("Testing retrieval by UUID: {}", uuid);
    let uuid_response: JsonRpcResult<RpcResponse<Option<SurfpoolProfileResult>>> = rpc_server
        .get_transaction_profile(Some(runloop_context.clone()), UuidOrSignature::Uuid(uuid))
        .await;

    assert!(
        uuid_response.is_ok(),
        "Get transaction profile by UUID failed: {:?}",
        uuid_response.err()
    );

    let retrieved_profile = uuid_response.unwrap().value;
    assert!(
        retrieved_profile.is_some(),
        "Profile should be found by UUID"
    );

    let retrieved = retrieved_profile.unwrap();
    assert_eq!(
        retrieved.transaction_profile.compute_units_consumed,
        profile_result.transaction_profile.compute_units_consumed,
        "Retrieved profile should match original profile"
    );
    assert_eq!(
        retrieved.transaction_profile.error_message.is_none(),
        profile_result.transaction_profile.error_message.is_none(),
        "Retrieved profile success should match original"
    );
    assert_eq!(
        retrieved.key,
        UuidOrSignature::Uuid(uuid),
        "Retrieved profile should have the same UUID"
    );

    // Test 3: Process the transaction to get a signature and retrieve by signature
    println!("Processing transaction to get signature");
    let (status_tx, status_rx) = crossbeam_unbounded();

    svm_locker_for_context
        .process_transaction(&None, tx.clone(), status_tx, false, true)
        .await
        .unwrap();

    // Wait for transaction processing
    match status_rx.recv() {
        Ok(TransactionStatusEvent::Success(_)) => {
            println!("Transaction processed successfully");
        }
        Ok(TransactionStatusEvent::SimulationFailure((error, _))) => {
            panic!("Transaction simulation failed: {:?}", error);
        }
        Ok(TransactionStatusEvent::ExecutionFailure((error, _))) => {
            panic!("Transaction execution failed: {:?}", error);
        }
        Ok(TransactionStatusEvent::VerificationFailure(error)) => {
            panic!("Transaction verification failed: {}", error);
        }
        Err(e) => {
            panic!("Failed to receive transaction status: {:?}", e);
        }
    }

    let signature = tx.signatures[0];
    println!("Transaction signature: {}", signature);

    // Test 4: Retrieve profile by signature
    println!("Testing retrieval by signature: {}", signature);
    let signature_response: JsonRpcResult<RpcResponse<Option<SurfpoolProfileResult>>> = rpc_server
        .get_transaction_profile(
            Some(runloop_context.clone()),
            UuidOrSignature::Signature(signature),
        )
        .await;

    assert!(
        signature_response.is_ok(),
        "Get transaction profile by signature failed: {:?}",
        signature_response.err()
    );

    let retrieved_by_signature = signature_response.unwrap().value;
    assert!(
        retrieved_by_signature.is_some(),
        "Profile should be found by signature"
    );

    let retrieved_sig = retrieved_by_signature.unwrap();
    assert!(
        retrieved_sig.transaction_profile.error_message.is_none(),
        "Retrieved profile by signature should be successful"
    );
    assert!(
        retrieved_sig.transaction_profile.compute_units_consumed > 0,
        "Retrieved profile should have consumed compute units"
    );

    // Test 5: Test retrieval with non-existent UUID
    println!("Testing retrieval with non-existent UUID");
    let non_existent_uuid = uuid::Uuid::new_v4();
    let non_existent_uuid_response: JsonRpcResult<RpcResponse<Option<SurfpoolProfileResult>>> =
        rpc_server
            .get_transaction_profile(
                Some(runloop_context.clone()),
                UuidOrSignature::Uuid(non_existent_uuid),
            )
            .await;

    assert!(
        non_existent_uuid_response.is_ok(),
        "Get transaction profile with non-existent UUID should not fail"
    );

    let non_existent_result = non_existent_uuid_response.unwrap().value;
    assert!(
        non_existent_result.is_none(),
        "Non-existent UUID should return None"
    );

    // Test 6: Test retrieval with non-existent signature
    println!("Testing retrieval with non-existent signature");
    let non_existent_signature = solana_sdk::signature::Signature::new_unique();
    let non_existent_sig_response: JsonRpcResult<RpcResponse<Option<SurfpoolProfileResult>>> =
        rpc_server
            .get_transaction_profile(
                Some(runloop_context.clone()),
                UuidOrSignature::Signature(non_existent_signature),
            )
            .await;

    assert!(
        non_existent_sig_response.is_ok(),
        "Get transaction profile with non-existent signature should not fail"
    );

    let non_existent_sig_result = non_existent_sig_response.unwrap().value;
    assert!(
        non_existent_sig_result.is_none(),
        "Non-existent signature should return None"
    );
    println!("All get_transaction_profile tests passed successfully!");
}

#[test]
fn test_register_and_get_idl_without_slot() {
    let idl: Idl = serde_json::from_slice(include_bytes!("./assets/idl_v1.json")).unwrap();
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance);
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Test 1: Register IDL without slot

    let register_response: JsonRpcResult<RpcResponse<()>> =
        rpc_server.register_idl(Some(runloop_context.clone()), idl.clone(), None);

    assert!(
        register_response.is_ok(),
        "Register IDL failed: {:?}",
        register_response.err()
    );

    // Test 2: Get IDL without slot

    let get_idl_response: JsonRpcResult<RpcResponse<Option<Idl>>> =
        rpc_server.get_idl(Some(runloop_context.clone()), idl.address.to_string(), None);

    assert!(
        get_idl_response.is_ok(),
        "Get IDL failed: {:?}",
        get_idl_response.err()
    );

    let retrieved_idl = get_idl_response.unwrap().value;
    assert!(retrieved_idl.is_some(), "IDL should be found");
    assert_eq!(
        retrieved_idl.unwrap(),
        idl,
        "Retrieved IDL should match registered IDL"
    );

    println!("All IDL registration and retrieval tests passed successfully!");
}

#[test]
fn test_register_and_get_idl_with_slot() {
    let idl: Idl = serde_json::from_slice(include_bytes!("./assets/idl_v1.json")).unwrap();
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance);
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Test 1: Register IDL with slot

    let register_response: JsonRpcResult<RpcResponse<()>> = rpc_server.register_idl(
        Some(runloop_context.clone()),
        idl.clone(),
        Some(Slot::from(
            svm_locker_for_context.get_latest_absolute_slot(),
        )),
    );

    assert!(
        register_response.is_ok(),
        "Register IDL failed: {:?}",
        register_response.err()
    );

    // Test 2: Get IDL with slot

    let get_idl_response: JsonRpcResult<RpcResponse<Option<Idl>>> = rpc_server.get_idl(
        Some(runloop_context.clone()),
        idl.address.to_string(),
        Some(Slot::from(
            svm_locker_for_context.get_latest_absolute_slot(),
        )),
    );

    assert!(
        get_idl_response.is_ok(),
        "Get IDL failed: {:?}",
        get_idl_response.err()
    );

    let retrieved_idl = get_idl_response.unwrap().value;
    assert!(retrieved_idl.is_some(), "IDL should be found");
    assert_eq!(
        retrieved_idl.unwrap(),
        idl,
        "Retrieved IDL should match registered IDL"
    );

    println!("All IDL registration and retrieval tests passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_register_and_get_same_idl_with_different_slots() {
    let idl_v1: Idl = serde_json::from_slice(include_bytes!("./assets/idl_v1.json")).unwrap();
    let idl_v2: Idl = serde_json::from_slice(include_bytes!("./assets/idl_v2.json")).unwrap();
    let idl_v3: Idl = serde_json::from_slice(include_bytes!("./assets/idl_v3.json")).unwrap();
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance);

    // Prepare slots for registering different IDLs
    let current_slot = svm_locker_for_context.get_latest_absolute_slot();
    let slot_1 = current_slot.saturating_add(10); // First IDL registration
    let slot_2 = current_slot.saturating_add(50); // Second IDL registration  
    let slot_3 = current_slot.saturating_add(100); // Third IDL registration

    println!("Current slot: {}", current_slot);
    println!("Slot 1 (IDL v1): {}", slot_1);
    println!("Slot 2 (IDL v2): {}", slot_2);
    println!("Slot 3 (IDL v3): {}", slot_3);

    // Setup runloop context
    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Step 1: Register IDL v1 at slot_1
    println!("  [1] Registering IDL v1 at slot: {}", slot_1);
    let register_response: JsonRpcResult<RpcResponse<()>> = rpc_server.register_idl(
        Some(runloop_context.clone()),
        idl_v1.clone(),
        Some(Slot::from(slot_1)),
    );

    assert!(
        register_response.is_ok(),
        "Register IDL v1 failed at slot {}: {:?}",
        slot_1,
        register_response.err()
    );

    // Step 2: Register IDL v2 at slot_2
    println!("  [2] Registering IDL v2 at slot: {}", slot_2);
    let register_response: JsonRpcResult<RpcResponse<()>> = rpc_server.register_idl(
        Some(runloop_context.clone()),
        idl_v2.clone(),
        Some(Slot::from(slot_2)),
    );

    assert!(
        register_response.is_ok(),
        "Register IDL v2 failed at slot {}: {:?}",
        slot_2,
        register_response.err()
    );

    // Step 3: Register IDL v3 at slot_3
    println!("  [3] Registering IDL v3 at slot: {}", slot_3);
    let register_response: JsonRpcResult<RpcResponse<()>> = rpc_server.register_idl(
        Some(runloop_context.clone()),
        idl_v3.clone(),
        Some(Slot::from(slot_3)),
    );

    assert!(
        register_response.is_ok(),
        "Register IDL v3 failed at slot {}: {:?}",
        slot_3,
        register_response.err()
    );

    // Step 4: Test retrieval at different points in time
    let test_cases = vec![
        (current_slot + 5, "before any registration", None), // Before slot_1
        (slot_1 + 5, "after v1 registration", Some(&idl_v1)), // After slot_1, before slot_2
        (slot_2 + 5, "after v2 registration", Some(&idl_v2)), // After slot_2, before slot_3
        (slot_3 + 5, "after v3 registration", Some(&idl_v3)), // After slot_3
    ];

    for (i, (query_slot, description, expected_idl)) in test_cases.iter().enumerate() {
        println!(
            "  [{}] Querying IDL at slot {} ({})",
            i + 4,
            query_slot,
            description
        );

        let get_idl_response: JsonRpcResult<RpcResponse<Option<Idl>>> = rpc_server.get_idl(
            Some(runloop_context.clone()),
            idl_v1.address.to_string(),
            Some(Slot::from(*query_slot)),
        );

        assert!(
            get_idl_response.is_ok(),
            "Get IDL failed at slot {}: {:?}",
            query_slot,
            get_idl_response.err()
        );

        let retrieved_idl = get_idl_response.unwrap().value;

        match expected_idl {
            None => {
                // Should not have any IDL before first registration
                assert!(
                    retrieved_idl.is_none(),
                    "IDL should not be available when querying at slot {} (before first registration)",
                    query_slot
                );
                println!(
                    "  [{}] Correctly: No IDL available at slot {} (before first registration)",
                    i + 4,
                    query_slot
                );
            }
            Some(expected) => {
                // Should have the appropriate IDL
                assert!(
                    retrieved_idl.is_some(),
                    "IDL should be available when querying at slot {}",
                    query_slot
                );

                let retrieved = retrieved_idl.unwrap();
                assert_eq!(
                    retrieved, **expected,
                    "Retrieved IDL should match expected IDL at slot {}",
                    query_slot
                );
                println!(
                    "  [{}] Correctly: Expected IDL retrieved at slot {}",
                    i + 4,
                    query_slot
                );
            }
        }
    }

    println!("All IDL registration and retrieval tests at different slots passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_basic() {
    // Set up test environment
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = 1_000_000;

    // Airdrop SOL to payer
    svm_locker
        .with_svm_writer(|svm| svm.airdrop(&payer.pubkey(), lamports_to_send * 2))
        .unwrap();

    // Create a simple transfer transaction
    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    // Test basic profiling without tag
    println!("Testing basic transaction profiling");
    let profile_result = svm_locker
        .profile_transaction(&None, transaction.clone(), None, None)
        .await
        .unwrap();

    // Verify UUID generation
    let UuidOrSignature::Uuid(uuid) = profile_result.inner.key else {
        panic!(
            "Expected a UUID from the profile result, got: {:?}",
            profile_result.inner.key
        );
    };
    println!("Generated UUID: {}", uuid);

    // Verify transaction profile
    assert!(
        profile_result
            .inner
            .transaction_profile
            .error_message
            .is_none(),
        "Transaction profiling should succeed"
    );
    assert!(
        profile_result
            .inner
            .transaction_profile
            .compute_units_consumed
            > 0,
        "Transaction should consume compute units"
    );

    // Verify slot and context
    assert_eq!(
        profile_result.slot,
        svm_locker.get_latest_absolute_slot(),
        "Profile slot should match current slot"
    );

    // Verify storage in SVM
    let stored_profile = svm_locker
        .get_profile_result(UuidOrSignature::Uuid(uuid))
        .unwrap();
    assert!(stored_profile.is_some(), "Profile should be stored in SVM");

    let stored = stored_profile.unwrap();
    assert_eq!(
        stored.transaction_profile.compute_units_consumed,
        profile_result
            .inner
            .transaction_profile
            .compute_units_consumed,
        "Stored profile should match returned profile"
    );

    println!("Basic transaction profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_multi_instruction_basic() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    let payer = Keypair::new();
    let lamports_to_send = 1_000_000;

    svm_locker
        .with_svm_writer(|svm| svm.airdrop(&payer.pubkey(), lamports_to_send * 4))
        .unwrap();

    // Create a multi-instruction transaction: 3 transfers to different recipients
    let recipient = Pubkey::new_unique();
    let recipient2 = Pubkey::new_unique();
    let recipient3 = Pubkey::new_unique();

    let transfer_ix = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let transfer_ix2 = transfer(&payer.pubkey(), &recipient2, lamports_to_send);
    let transfer_ix3 = transfer(&payer.pubkey(), &recipient3, lamports_to_send);

    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message = Message::new_with_blockhash(
        &[transfer_ix, transfer_ix2, transfer_ix3],
        Some(&payer.pubkey()),
        &latest_blockhash,
    );
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    let profile_result = svm_locker
        .profile_transaction(&None, transaction.clone(), None, None)
        .await
        .unwrap();

    // Verify UUID generation
    let UuidOrSignature::Uuid(uuid) = profile_result.inner.key else {
        panic!(
            "Expected a UUID from the profile result, got: {:?}",
            profile_result.inner.key
        );
    };
    println!("Generated UUID: {}", uuid);

    println!(
        "Profile result: {}",
        serde_json::to_string_pretty(&profile_result.inner).unwrap()
    );

    // Verify transaction profile
    assert!(
        profile_result
            .inner
            .transaction_profile
            .error_message
            .is_none(),
        "Transaction profiling should succeed"
    );
    assert!(
        profile_result
            .inner
            .transaction_profile
            .compute_units_consumed
            > 0,
        "Transaction should consume compute units"
    );

    // Verify that the transaction consumes compute units
    assert!(
        profile_result
            .inner
            .transaction_profile
            .compute_units_consumed
            > 0,
        "Transaction should consume compute units"
    );

    // Verify state snapshots exist
    assert!(
        !profile_result
            .inner
            .transaction_profile
            .state
            .pre_execution
            .is_empty(),
        "Pre-execution state should be captured"
    );
    assert!(
        !profile_result
            .inner
            .transaction_profile
            .state
            .post_execution
            .is_empty(),
        "Post-execution state should be captured"
    );

    // Verify storage in SVM
    let stored_profile = svm_locker
        .get_profile_result(UuidOrSignature::Uuid(uuid))
        .unwrap();
    assert!(stored_profile.is_some(), "Profile should be stored in SVM");

    let stored = stored_profile.unwrap();
    assert_eq!(
        stored.transaction_profile.compute_units_consumed,
        profile_result
            .inner
            .transaction_profile
            .compute_units_consumed,
        "Stored profile should match returned profile"
    );

    println!("Multi-instruction transaction basic profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_with_tag() {
    // Set up test environment
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = 1_000_000;

    // Airdrop SOL to payer
    svm_locker
        .with_svm_writer(|svm| svm.airdrop(&payer.pubkey(), lamports_to_send * 3))
        .unwrap();

    // Create a simple transfer transaction
    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    // Test profiling with a tag
    let tag = "test_profile_transaction_tag".to_string();
    println!("Testing transaction profiling with tag: {}", tag);

    let profile_result = svm_locker
        .profile_transaction(&None, transaction.clone(), None, Some(tag.clone()))
        .await
        .unwrap();
    println!("Profile result: {:?}", profile_result.inner);

    // Verify UUID generation
    let UuidOrSignature::Uuid(uuid) = profile_result.inner.key else {
        panic!(
            "Expected a UUID from the profile result, got: {:?}",
            profile_result.inner.key
        );
    };
    println!("Generated UUID: {}", uuid);

    // Verify transaction profile
    assert!(
        profile_result
            .inner
            .transaction_profile
            .error_message
            .is_none(),
        "Transaction profiling should succeed"
    );

    // Verify tag-based retrieval
    let tagged_results = svm_locker.get_profile_results_by_tag(tag.clone()).unwrap();
    assert!(tagged_results.is_some(), "Tagged results should be found");

    let tagged_profiles = tagged_results.unwrap();
    assert_eq!(
        tagged_profiles.len(),
        1,
        "Should have exactly one profile for this tag"
    );

    let tagged_profile = &tagged_profiles[0];
    assert_eq!(
        tagged_profile.key,
        UuidOrSignature::Uuid(uuid),
        "Tagged profile should have the same UUID"
    );

    // Test multiple profiles with the same tag
    println!("Testing multiple profiles with the same tag");

    // Create another transaction
    let recipient2 = Pubkey::new_unique();
    let instruction2 = transfer(&payer.pubkey(), &recipient2, lamports_to_send);
    let message2 =
        Message::new_with_blockhash(&[instruction2], Some(&payer.pubkey()), &latest_blockhash);
    let transaction2 =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message2), &[&payer]).unwrap();

    let profile_result2 = svm_locker
        .profile_transaction(&None, transaction2, None, Some(tag.clone()))
        .await
        .unwrap();

    let UuidOrSignature::Uuid(uuid2) = profile_result2.inner.key else {
        panic!("Expected a UUID from the second profile result");
    };
    println!("Generated second UUID: {}", uuid2);

    // Verify both profiles are now associated with the tag
    let tagged_results_updated = svm_locker.get_profile_results_by_tag(tag.clone()).unwrap();
    assert!(
        tagged_results_updated.is_some(),
        "Tagged results should still be found after adding second profile"
    );

    let tagged_profiles_updated = tagged_results_updated.unwrap();
    assert_eq!(
        tagged_profiles_updated.len(),
        2,
        "Should have exactly two profiles for this tag"
    );

    // Verify both UUIDs are present
    let uuids: Vec<Uuid> = tagged_profiles_updated
        .iter()
        .filter_map(|profile| {
            if let UuidOrSignature::Uuid(uuid) = profile.key {
                Some(uuid)
            } else {
                None
            }
        })
        .collect();

    assert!(
        uuids.contains(&uuid),
        "First UUID should be in tagged results"
    );
    assert!(
        uuids.contains(&uuid2),
        "Second UUID should be in tagged results"
    );

    // Test retrieval with non-existent tag
    let non_existent_tag = "non_existent_tag".to_string();
    let non_existent_results = svm_locker
        .get_profile_results_by_tag(non_existent_tag)
        .unwrap();
    assert!(
        non_existent_results.is_none(),
        "Non-existent tag should return None"
    );

    // Test retrieval by individual UUIDs
    let stored_profile1 = svm_locker
        .get_profile_result(UuidOrSignature::Uuid(uuid))
        .unwrap();
    assert!(
        stored_profile1.is_some(),
        "First profile should be retrievable by UUID"
    );

    let stored_profile2 = svm_locker
        .get_profile_result(UuidOrSignature::Uuid(uuid2))
        .unwrap();
    assert!(
        stored_profile2.is_some(),
        "Second profile should be retrievable by UUID"
    );

    println!("Tag-based transaction profiling test passed successfully!");
}
