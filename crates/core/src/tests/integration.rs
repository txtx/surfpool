use std::{str::FromStr, sync::Arc, time::Duration};

use base64::Engine;
use crossbeam_channel::{unbounded, unbounded as crossbeam_unbounded};
use jsonrpc_core::{
    Error, Result as JsonRpcResult,
    futures::future::{self, join_all},
};
use jsonrpc_core_client::transports::http;
use solana_account::Account;
use solana_account_decoder::{UiAccountData, UiAccountEncoding, parse_account_data::ParsedAccount};
use solana_address_lookup_table_interface::state::{AddressLookupTable, LookupTableMeta};
use solana_client::rpc_response::RpcLogsResponse;
use solana_clock::{Clock, Slot};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_epoch_info::EpochInfo;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::{
    AddressLookupTableAccount, Message, VersionedMessage,
    v0::{self},
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_signer::Signer;
use solana_system_interface::{
    instruction as system_instruction, instruction::transfer, program as system_program,
};
use solana_transaction::{Transaction, versioned::VersionedTransaction};
use surfpool_types::{
    DEFAULT_SLOT_TIME_MS, Idl, ResetAccountConfig, RpcProfileDepth, RpcProfileResultConfig,
    SimnetCommand, SimnetEvent, SurfpoolConfig, UiAccountChange, UiAccountProfileState,
    UiKeyedProfileResult,
    types::{
        BlockProductionMode, RpcConfig, SimnetConfig, TransactionStatusEvent, UuidOrSignature,
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
        surfnet_cheatcodes::{SurfnetCheatcodes, SurfnetCheatcodesRpc},
    },
    runloops::start_local_surfnet_runloop,
    surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm},
    tests::helpers::get_free_port,
    types::{TimeTravelConfig, TokenAccount, TransactionLoadedAddresses},
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
                Ok(SimnetEvent::SystemClockUpdated(_)) => ticks += 1,
                _ => (),
            }

            if ticks > 100 {
                let _ = test_tx.send(true);
            }
        }
    });

    match test_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(_) => (),
        Err(e) => panic!("not enough ticks: {e:?}"),
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

    let random_address = Pubkey::from_str_const("7zdYkYf7yD83j3TLXmkhxn6LjQP9y9bQ4pjfpquP8Hqw");

    let instruction = transfer(&pk, &random_address, 100);

    let alt_address = Pubkey::from_str_const("5KcPJehcpBLcPde2UhmY4dE9zCrv2r9AKFmW5CGtY1io"); // a mainnet lookup table

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
        let mut loaded_addresses = TransactionLoadedAddresses::new();
        svm_locker
            .get_lookup_table_addresses(&None, msg, &mut loaded_addresses)
            .await?;
        Ok::<_, SurfpoolError>(
            loaded_addresses
                .all_loaded_addresses()
                .into_iter()
                .map(|p| *p)
                .collect::<Vec<Pubkey>>(),
        )
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<Vec<Pubkey>>, SurfpoolError>>()
    .unwrap()
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

    let random_address = Pubkey::from_str_const("7zdYkYf7yD83j3TLXmkhxn6LjQP9y9bQ4pjfpquP8Hqw");

    let instruction = transfer(&pk, &random_address, 100);
    let recent_blockhash = svm_locker.with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

    let alt_address = Pubkey::from_str_const("5KcPJehcpBLcPde2UhmY4dE9zCrv2r9AKFmW5CGtY1io"); // a mainnet lookup table

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
    let response_no_tag_initial: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
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
            .log_messages
            .is_some(),
        "Log messages should be present for None tag"
    );

    // Test 1: Estimate with a tag and retrieve
    let tag1 = "test_tag_1".to_string();
    println!("\nTesting with tag: {}", tag1);
    let response_tagged_1: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
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

    println!("Retrieving profile results for tag: {}", tag1);
    let results_vec_tag1 = rpc_server
        .get_profile_results_by_tag(Some(runloop_context.clone()), tag1.clone(), None)
        .unwrap()
        .value
        .unwrap_or_default();
    assert_eq!(results_vec_tag1.len(), 1, "Expected 1 result for tag1");

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

    // Test 2: Retrieve with a non-existent tag
    let tag_non_existent = "non_existent_tag".to_string();
    println!(
        "\nTesting retrieval with non-existent tag: {}",
        tag_non_existent
    );
    let results_non_existent_vec = rpc_server
        .get_profile_results_by_tag(
            Some(runloop_context.clone()),
            tag_non_existent.clone(),
            None,
        )
        .unwrap()
        .value
        .unwrap_or_default();

    assert!(
        results_non_existent_vec.is_empty(),
        "Expected empty vec for non-existent tag"
    );

    // Test 3: Estimate multiple times with the same tag
    let tag2 = "test_tag_2".to_string();
    println!("\nTesting multiple estimations with tag: {}", tag2);
    let response_tagged_2a: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
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

    let response_tagged_2b: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
        .profile_transaction(
            Some(runloop_context.clone()),
            tx_b64.clone(),
            Some(tag2.clone()),
            None,
        )
        .await;
    assert!(response_tagged_2b.is_ok(), "Second call with tag2 failed");
    let cu_2b_profile_result = response_tagged_2b.unwrap().value;

    println!("Retrieving profile results for tag: {}", tag2);
    let results_response_tag2 =
        rpc_server.get_profile_results_by_tag(Some(runloop_context.clone()), tag2.clone(), None);

    assert!(
        results_response_tag2.is_ok(),
        "get_profile_results for tag2 failed"
    );
    let results_vec_tag2 = results_response_tag2.unwrap().value.unwrap_or_default();
    assert_eq!(results_vec_tag2.len(), 2, "Expected 2 results for tag2");

    assert_eq!(
        results_vec_tag2[0]
            .transaction_profile
            .compute_units_consumed,
        cu_2a_profile_result
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

    // Test 4: Estimate with another None tag, ensure it doesn't affect tagged results for tag1
    println!(
        "\nTesting None tag again to ensure no interference with tag: {}",
        tag1
    );
    let response_no_tag_again: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
        .profile_transaction(Some(runloop_context.clone()), tx_b64.clone(), None, None)
        .await;
    assert!(
        response_no_tag_again.is_ok(),
        "RPC call with None tag (again) failed"
    );
    let rpc_response_no_tag_again_value = response_no_tag_again.unwrap().value;

    println!("Retrieving profile results for tag: {} again", tag1);
    let results_response_tag1_again =
        rpc_server.get_profile_results_by_tag(Some(runloop_context), tag1.clone(), None);
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
    assert_eq!(
        results_vec_tag1_again[0]
            .transaction_profile
            .compute_units_consumed,
        rpc_response_tagged_1_value
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

    let profile_response: JsonRpcResult<RpcResponse<UiKeyedProfileResult>> = rpc_server
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
    let uuid_response: JsonRpcResult<RpcResponse<Option<UiKeyedProfileResult>>> = rpc_server
        .get_transaction_profile(
            Some(runloop_context.clone()),
            UuidOrSignature::Uuid(uuid),
            None,
        );

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
    let signature_response: JsonRpcResult<RpcResponse<Option<UiKeyedProfileResult>>> = rpc_server
        .get_transaction_profile(
            Some(runloop_context.clone()),
            UuidOrSignature::Signature(signature),
            None,
        );

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
    let non_existent_uuid_response: JsonRpcResult<RpcResponse<Option<UiKeyedProfileResult>>> =
        rpc_server.get_transaction_profile(
            Some(runloop_context.clone()),
            UuidOrSignature::Uuid(non_existent_uuid),
            None,
        );

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
    let non_existent_signature = solana_signature::Signature::new_unique();
    let non_existent_sig_response: JsonRpcResult<RpcResponse<Option<UiKeyedProfileResult>>> =
        rpc_server.get_transaction_profile(
            Some(runloop_context.clone()),
            UuidOrSignature::Signature(non_existent_signature),
            None,
        );

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
    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify UUID generation
    let UuidOrSignature::Uuid(uuid) = profile_result.key else {
        panic!(
            "Expected a UUID from the profile result, got: {:?}",
            profile_result.key
        );
    };
    println!("Generated UUID: {}", uuid);

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Transaction profiling should succeed"
    );
    assert!(
        profile_result.transaction_profile.compute_units_consumed > 0,
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
        .get_profile_result(
            UuidOrSignature::Uuid(uuid),
            &RpcProfileResultConfig::default(),
        )
        .unwrap();
    assert!(stored_profile.is_some(), "Profile should be stored in SVM");

    let stored = stored_profile.unwrap();
    assert_eq!(
        stored.transaction_profile.compute_units_consumed,
        profile_result.transaction_profile.compute_units_consumed,
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
    println!("Sender: {}", payer.pubkey());
    println!("Recipient 1: {}", recipient);
    println!("Recipient 2: {}", recipient2);
    println!("Recipient 3: {}", recipient3);

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

    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);

    // Check profile result with Transaction depth
    {
        let rpc_profile_config = RpcProfileResultConfig {
            encoding: Some(UiAccountEncoding::JsonParsed),
            depth: Some(RpcProfileDepth::Transaction),
        };

        let profile_result = svm_locker
            .get_profile_result(key, &rpc_profile_config)
            .unwrap()
            .expect("Profile result should exist");

        // Verify UUID generation
        let UuidOrSignature::Uuid(uuid) = profile_result.key else {
            panic!(
                "Expected a UUID from the profile result, got: {:?}",
                profile_result.key
            );
        };
        println!("Generated UUID: {}", uuid);

        assert!(profile_result.transaction_profile.error_message.is_none(),);
        assert_eq!(
            profile_result.transaction_profile.compute_units_consumed,
            450
        );
        assert_eq!(
            profile_result.instruction_profiles, None,
            "Instruction profiles should be None for Transaction depth config"
        );

        let account_states = profile_result.transaction_profile.account_states;

        let _ = profile_result
            .readonly_account_states
            .get(&system_program::id())
            .expect("System program should be present in readonly account states");
        let UiAccountProfileState::Readonly = account_states
            .get(&system_program::id())
            .expect("System program state should be present")
        else {
            panic!("Expected system program state to be Readonly");
        };

        // assert payer states
        {
            let UiAccountProfileState::Writable(sender_account_change) = account_states
                .get(&payer.pubkey())
                .expect("Payer account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match sender_account_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(
                        after.lamports,
                        before.lamports - (lamports_to_send * 3) - 5000,
                        "Payer account should be original balance minus three transfers and fees"
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }
        }

        // assert recipient 1 states
        {
            let UiAccountProfileState::Writable(recipient_1_change) = account_states
                .get(&recipient)
                .expect("Recipient 1 account state should be present")
            else {
                panic!("Expected recipient 1 account state to be Writable");
            };

            match recipient_1_change {
                UiAccountChange::Create(new) => {
                    assert_eq!(
                        new.lamports, lamports_to_send,
                        "Recipient 1 account should have received the transfer amount"
                    );
                }
                other => {
                    panic!("Expected account state to be an Create, got: {:?}", other);
                }
            }
        }

        // assert recipient 2 states
        {
            let UiAccountProfileState::Writable(recipient_2_change) = account_states
                .get(&recipient2)
                .expect("Recipient 2 account state should be present")
            else {
                panic!("Expected recipient 2 account state to be Writable");
            };

            match recipient_2_change {
                UiAccountChange::Create(new) => {
                    assert_eq!(
                        new.lamports, lamports_to_send,
                        "Recipient 2 account should have received the transfer amount"
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }
        }

        // assert recipient 3 states
        {
            let UiAccountProfileState::Writable(recipient_3_change) = account_states
                .get(&recipient3)
                .expect("Recipient 3 account state should be present")
            else {
                panic!("Expected recipient 3 account state to be Writable");
            };

            match recipient_3_change {
                UiAccountChange::Create(new) => {
                    assert_eq!(
                        new.lamports, lamports_to_send,
                        "Recipient 3 account should have received the transfer amount"
                    );
                }
                other => {
                    panic!("Expected account state to be Create, got: {:?}", other);
                }
            }
        }
    }

    // Check profile result with Instruction depth
    {
        let rpc_profile_config = RpcProfileResultConfig {
            encoding: Some(UiAccountEncoding::JsonParsed),
            depth: Some(RpcProfileDepth::Instruction),
        };
        let profile_result = svm_locker
            .get_profile_result(key, &rpc_profile_config)
            .unwrap()
            .expect("Profile result should exist");

        println!(
            "Profile result with Instruction depth: {}",
            serde_json::to_string_pretty(&profile_result).unwrap()
        );

        let instruction_profiles = profile_result
            .instruction_profiles
            .expect("Instruction profiles should be present for Instruction depth config");

        // assert ix 1 data
        {
            let ix_profile = instruction_profiles
                .get(0)
                .expect("Instruction profile should exist");
            assert_eq!(
                ix_profile.compute_units_consumed, 150,
                "Instruction should consume 150 CU"
            );
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;
            // assert account sender states
            {
                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(
                            after.lamports,
                            before.lamports - lamports_to_send - 5000,
                            "Payer account should be original balance minus transfer amount"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            // assert recipient 1 states
            {
                let UiAccountProfileState::Writable(recipient_1_change) = account_states
                    .get(&recipient)
                    .expect("Recipient 1 account state should be present")
                else {
                    panic!("Expected recipient 1 account state to be Writable");
                };

                match recipient_1_change {
                    UiAccountChange::Create(new) => {
                        assert_eq!(
                            new.lamports, lamports_to_send,
                            "Recipient 1 account should have received the transfer amount"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            assert!(
                account_states.get(&recipient2).is_none(),
                "Recipient 2 should not be affected by first instruction"
            );
            assert!(
                account_states.get(&recipient3).is_none(),
                "Recipient 3 should not be affected by first instruction"
            );
        }

        // assert ix 2 data
        {
            let ix_profile = instruction_profiles
                .get(1)
                .expect("Instruction profile should exist");
            assert_eq!(
                ix_profile.compute_units_consumed, 150,
                "Instruction should consume 150 CU"
            );
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;
            // assert account sender states
            {
                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(after.lamports, before.lamports - lamports_to_send);
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            println!(
                "Recipient 1 account state: {:?}",
                account_states.get(&recipient)
            );
            assert!(
                account_states.get(&recipient).is_none(),
                "Recipient 1 should not be affected by second instruction"
            );
            // assert recipient 2 states
            {
                let UiAccountProfileState::Writable(recipient_2_change) = account_states
                    .get(&recipient2)
                    .expect("Recipient 2 account state should be present")
                else {
                    panic!("Expected recipient 2 account state to be Writable");
                };

                match recipient_2_change {
                    UiAccountChange::Create(new) => {
                        assert_eq!(
                            new.lamports, lamports_to_send,
                            "Recipient 2 account should have received the transfer amount"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            assert!(
                account_states.get(&recipient3).is_none(),
                "Recipient 3 should not be affected by first instruction"
            );
        }

        // assert ix 3 data
        {
            let ix_profile = instruction_profiles
                .get(2)
                .expect("Instruction profile should exist");
            assert_eq!(
                ix_profile.compute_units_consumed, 150,
                "Instruction should consume 150 CU"
            );
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;
            // assert account sender states
            {
                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(after.lamports, before.lamports - lamports_to_send,);
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            assert!(
                account_states.get(&recipient).is_none(),
                "Recipient 1 should not be affected by second instruction"
            );

            assert!(
                account_states.get(&recipient2).is_none(),
                "Recipient 2 should not be affected by first instruction"
            );
            // assert recipient 3 states
            {
                let UiAccountProfileState::Writable(recipient_3_change) = account_states
                    .get(&recipient3)
                    .expect("Recipient 3 account state should be present")
                else {
                    panic!("Expected recipient 3 account state to be Writable");
                };

                match recipient_3_change {
                    UiAccountChange::Create(new) => {
                        assert_eq!(
                            new.lamports, lamports_to_send,
                            "Recipient 3 account should have received the transfer amount"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }
        }
    }
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

    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), Some(tag.clone()))
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Transaction profiling should succeed"
    );

    // Verify tag-based retrieval
    let tagged_results = svm_locker
        .get_profile_results_by_tag(tag.clone(), &RpcProfileResultConfig::default())
        .unwrap();
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
        UuidOrSignature::Uuid(profile_uuid),
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

    let uuid2 = svm_locker
        .profile_transaction(&None, transaction2.clone(), Some(tag.clone()))
        .await
        .unwrap()
        .inner;

    // Verify both profiles are now associated with the tag
    let tagged_results_updated = svm_locker
        .get_profile_results_by_tag(tag.clone(), &RpcProfileResultConfig::default())
        .unwrap();
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
        uuids.contains(&profile_uuid),
        "First UUID should be in tagged results"
    );
    assert!(
        uuids.contains(&uuid2),
        "Second UUID should be in tagged results"
    );

    // Test retrieval with non-existent tag
    let non_existent_tag = "non_existent_tag".to_string();
    let non_existent_results = svm_locker
        .get_profile_results_by_tag(non_existent_tag, &RpcProfileResultConfig::default())
        .unwrap();
    assert!(
        non_existent_results.is_none(),
        "Non-existent tag should return None"
    );

    // Test retrieval by individual UUIDs
    let stored_profile1 = svm_locker
        .get_profile_result(
            UuidOrSignature::Uuid(profile_uuid),
            &RpcProfileResultConfig::default(),
        )
        .unwrap();
    assert!(
        stored_profile1.is_some(),
        "First profile should be retrievable by UUID"
    );

    let stored_profile2 = svm_locker
        .get_profile_result(
            UuidOrSignature::Uuid(uuid2),
            &RpcProfileResultConfig::default(),
        )
        .unwrap();
    assert!(
        stored_profile2.is_some(),
        "Second profile should be retrievable by UUID"
    );

    println!("Tag-based transaction profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_token_transfer() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let mint = Keypair::new();
    let lamports_to_send = 2 * LAMPORTS_PER_SOL;
    println!("Sender: {}", payer.pubkey());
    println!("Recipient: {}", recipient);
    println!("Mint: {}", mint.pubkey());

    // Airdrop SOL to payer
    svm_locker
        .airdrop(&payer.pubkey(), lamports_to_send)
        .unwrap();

    let recent_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    println!("Recent blockhash after airdrop: {}", recent_blockhash);

    // Create account for mint
    let mint_rent = 1461600;
    let create_account_ix = system_instruction::create_account(
        &payer.pubkey(),
        &mint.pubkey(),
        mint_rent,
        82,
        &spl_token_2022::id(),
    );

    // Initialize mint
    let initialize_mint_ix = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &mint.pubkey(),
        &payer.pubkey(),
        Some(&payer.pubkey()),
        2, // decimals
    )
    .unwrap();

    // Create associated token accounts
    let source_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &payer.pubkey(),
        &mint.pubkey(),
        &spl_token_2022::id(),
    );
    println!("Source ATA: {}", source_ata);
    let dest_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &recipient,
        &mint.pubkey(),
        &spl_token_2022::id(),
    );

    let create_source_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account(
            &payer.pubkey(),
            &payer.pubkey(),
            &mint.pubkey(),
            &spl_token_2022::id(),
        );

    let create_dest_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account(
            &payer.pubkey(),
            &recipient,
            &mint.pubkey(),
            &spl_token_2022::id(),
        );

    // Mint tokens
    let mint_amount = 100_00; // 100 tokens with 2 decimals
    let mint_to_ix = spl_token_2022::instruction::mint_to(
        &spl_token_2022::id(),
        &mint.pubkey(),
        &source_ata,
        &payer.pubkey(),
        &[&payer.pubkey()],
        mint_amount,
    )
    .unwrap();

    // Create setup transaction
    let setup_message = Message::new_with_blockhash(
        &[
            create_account_ix,
            initialize_mint_ix,
            create_source_ata_ix,
            // create_dest_ata_ix,
            // mint_to_ix,
        ],
        Some(&payer.pubkey()),
        &recent_blockhash,
    );
    let setup_tx =
        VersionedTransaction::try_new(VersionedMessage::Legacy(setup_message), &[&payer, &mint])
            .unwrap();

    {
        let profile_result = svm_locker
            .profile_transaction(&None, setup_tx.clone(), None)
            .await
            .unwrap();

        let rpc_profile_config = RpcProfileResultConfig {
            encoding: Some(UiAccountEncoding::JsonParsed),
            depth: Some(RpcProfileDepth::Instruction),
        };
        let key = UuidOrSignature::Uuid(profile_result.inner);
        let ui_profile_result = svm_locker
            .get_profile_result(key, &rpc_profile_config)
            .unwrap()
            .expect("Profile result should exist");

        assert!(
            ui_profile_result
                .transaction_profile
                .error_message
                .is_none(),
            "Setup transaction should succeed, found error: {}",
            ui_profile_result.transaction_profile.error_message.unwrap()
        );

        // instruction 1: create_account
        {
            let ix_profile = ui_profile_result
                .instruction_profiles
                .as_ref()
                .unwrap()
                .get(0)
                .expect("instruction profile should exist");
            assert!(
                ix_profile.error_message.is_none(),
                "Profile should succeed, found error: {}",
                ix_profile.error_message.as_ref().unwrap()
            );
            assert_eq!(ix_profile.compute_units_consumed, 150);
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;

            let UiAccountProfileState::Writable(sender_account_change) = account_states
                .get(&payer.pubkey())
                .expect("Payer account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match sender_account_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(
                        after.lamports,
                        before.lamports - mint_rent - (2 * 5000), // two signers, so 2 * 5000 for fees
                        "Payer account should be original balance minus rent"
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }

            let UiAccountProfileState::Writable(mint_account_change) = account_states
                .get(&mint.pubkey())
                .expect("Mint account state should be present")
            else {
                panic!("Expected mint account state to be Writable");
            };
            match mint_account_change {
                UiAccountChange::Create(mint_account) => {
                    assert_eq!(
                        mint_account.lamports, mint_rent,
                        "Mint account should have the correct rent amount"
                    );
                    assert_eq!(
                        mint_account.owner,
                        spl_token_2022::id().to_string(),
                        "Mint account should be owned by the SPL Token program"
                    );
                    // initialized account data should be empty bytes
                    assert_eq!(
                        mint_account.data,
                        UiAccountData::Binary(
                            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==".into(),
                            UiAccountEncoding::Base64
                        ),
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }
        }

        // instruction 2: initialize mint
        {
            let ix_profile = ui_profile_result
                .instruction_profiles
                .as_ref()
                .unwrap()
                .get(1)
                .expect("instruction profile should exist");
            assert!(
                ix_profile.error_message.is_none(),
                "Profile should succeed, found error: {}",
                ix_profile.error_message.as_ref().unwrap()
            );
            assert_eq!(ix_profile.compute_units_consumed, 1031);
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;

            assert!(account_states.get(&payer.pubkey()).is_none());

            let UiAccountProfileState::Writable(mint_account_change) = account_states
                .get(&mint.pubkey())
                .expect("Mint account state should be present")
            else {
                panic!("Expected mint account state to be Writable");
            };
            match mint_account_change {
                UiAccountChange::Update(_before, after) => {
                    assert_eq!(
                        after.lamports, mint_rent,
                        "Mint account should have the correct rent amount"
                    );
                    assert_eq!(
                        after.owner,
                        spl_token_2022::id().to_string(),
                        "Mint account should be owned by the SPL Token program"
                    );
                    // initialized account data should be empty bytes
                    assert_eq!(
                        after.data,
                        UiAccountData::Json(ParsedAccount {
                            program: "spl-token-2022".to_string(),
                            parsed: json!({
                                "info": {
                                    "decimals": 2,
                                    "freezeAuthority": payer.pubkey().to_string(),
                                    "mintAuthority": payer.pubkey().to_string(),
                                    "isInitialized": true,
                                    "supply": "0",
                                },
                                "type": "mint"
                            }),
                            space: 82,
                        }),
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }
        }

        // instruction 3: create token account
        {
            let ix_profile = ui_profile_result
                .instruction_profiles
                .as_ref()
                .unwrap()
                .get(2)
                .expect("instruction profile should exist");
            assert!(
                ix_profile.error_message.is_none(),
                "Profile should succeed, found error: {}",
                ix_profile.error_message.as_ref().unwrap()
            );
            // assert_eq!(ix_profile.compute_units_consumed, 15758);
            assert!(ix_profile.error_message.is_none());
            let account_states = &ix_profile.account_states;

            let UiAccountProfileState::Writable(sender_account_change) = account_states
                .get(&payer.pubkey())
                .expect("Payer account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match sender_account_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(
                        after.lamports,
                        before.lamports - 2074080,
                        "Payer account should be original balance minus rent"
                    );
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }

            let UiAccountProfileState::Writable(mint_account_change) = account_states
                .get(&mint.pubkey())
                .expect("Mint account state should be present")
            else {
                panic!("Expected mint account state to be Writable");
            };
            match mint_account_change {
                UiAccountChange::Unchanged(mint_account) => {
                    assert!(mint_account.is_some())
                }
                other => {
                    panic!("Expected account state to be Unchanged, got: {:?}", other);
                }
            }

            let UiAccountProfileState::Writable(source_ata_change) = account_states
                .get(&source_ata)
                .expect("account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match source_ata_change {
                UiAccountChange::Create(new) => {
                    assert_eq!(
                        new.lamports, 2074080,
                        "Source ATA should have the correct lamports after creation"
                    );
                    assert_eq!(
                        new.owner,
                        spl_token_2022::id().to_string(),
                        "Source ATA should be owned by the SPL Token program"
                    );
                    // since we're profiling, the "additional data" needed to json parse token 2022 accounts isn't available,
                    // so the result is base64 encoded
                    let UiAccountData::Binary(new_data, UiAccountEncoding::Base64) = &new.data
                    else {
                        panic!("Expected account data to be Base64 encoded");
                    };

                    let mut mint_bs64 =
                        base64::engine::general_purpose::STANDARD.encode(&mint.pubkey().to_bytes());
                    mint_bs64.truncate(42);
                    assert!(new_data.starts_with(mint_bs64.as_str()));
                    assert!(
                        new_data.ends_with(
                            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgcAAAA="
                        )
                    );
                }
                other => {
                    panic!("Expected account state to be Create, got: {:?}", other);
                }
            }
        }

        assert_eq!(
            ui_profile_result.transaction_profile.compute_units_consumed,
            ui_profile_result
                .instruction_profiles
                .as_ref()
                .unwrap()
                .iter()
                .map(|ix| ix.compute_units_consumed)
                .sum::<u64>(),
        );
    }

    // Process setup transaction
    // let (status_tx, status_rx) = crossbeam_channel::unbounded();
    // svm_locker
    //     .process_transaction(&None, setup_tx, status_tx, false, true)
    //     .await
    //     .unwrap();

    // match status_rx.recv() {
    //     Ok(TransactionStatusEvent::Success(_)) => println!("Setup transaction successful"),
    //     other => panic!("Setup transaction failed: {:?}", other),
    // }

    // // Now create a token transfer transaction to profile
    // let transfer_amount = 50; // 0.5 tokens
    // let transfer_ix = spl_token_2022::instruction::transfer_checked(
    //     &spl_token_2022::id(),
    //     &source_ata,
    //     &mint.pubkey(),
    //     &dest_ata,
    //     &payer.pubkey(),
    //     &[&payer.pubkey()],
    //     transfer_amount,
    //     2, // decimals
    // )
    // .unwrap();

    // let transfer_message =
    //     Message::new_with_blockhash(&[transfer_ix], Some(&payer.pubkey()), &recent_blockhash);
    // let transfer_tx =
    //     VersionedTransaction::try_new(VersionedMessage::Legacy(transfer_message), &[&payer])
    //         .unwrap();

    // // Profile the token transfer transaction
    // let profile_result = svm_locker
    //     .profile_transaction(&None, transfer_tx.clone(), None)
    //     .await
    //     .unwrap();

    // let rpc_profile_config = RpcProfileResultConfig {
    //     encoding: Some(UiAccountEncoding::JsonParsed),
    //     depth: Some(RpcProfileDepth::Instruction),
    // };
    // let ui_profile_result = svm_locker
    //     .encode_ui_keyed_profile_result(profile_result.inner.clone(), &rpc_profile_config);

    // println!(
    //     "UI Profile Result: {}",
    //     serde_json::to_string_pretty(&ui_profile_result).unwrap()
    // );

    // // Verify UUID generation
    // let UuidOrSignature::Uuid(_uuid) = profile_result.inner.key else {
    //     panic!("Expected a UUID from the profile result");
    // };

    // // Verify transaction profile
    // assert!(
    //     profile_result
    //         .inner
    //         .transaction_profile
    //         .error_message
    //         .is_none(),
    //     "Token transfer profiling should succeed"
    // );
    // assert!(
    //     profile_result
    //         .inner
    //         .transaction_profile
    //         .compute_units_consumed
    //         > 0,
    //     "Token transfer should consume compute units"
    // );

    // println!("Token transfer profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_insufficient_funds() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts with insufficient funds
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let insufficient_funds = 10000;
    let large_transfer = LAMPORTS_PER_SOL * 10;

    svm_locker
        .airdrop(&payer.pubkey(), insufficient_funds)
        .unwrap();

    // Create a transfer transaction that will fail due to insufficient funds
    let instruction = transfer(&payer.pubkey(), &recipient, large_transfer);
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    // Profile the failing transaction
    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let ui_profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile shows failure
    assert!(
        ui_profile_result
            .transaction_profile
            .error_message
            .is_some(),
        "Transaction should fail due to insufficient funds"
    );

    let error_msg = ui_profile_result
        .transaction_profile
        .error_message
        .as_ref()
        .unwrap();
    assert!(
        error_msg.eq("Error processing Instruction 0: custom program error: 0x1"),
        "Error message should indicate insufficient funds, got: {}",
        error_msg
    );

    // Verify compute units were still consumed
    assert!(
        ui_profile_result.transaction_profile.compute_units_consumed > 0,
        "Failed transaction should still consume compute units"
    );

    println!("Insufficient funds profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_multi_instruction_failure() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let payer2 = Keypair::new();
    let recipient1 = Pubkey::new_unique();
    let lamports_to_send = LAMPORTS_PER_SOL;

    // Airdrop SOL to payer
    svm_locker
        .airdrop(&payer.pubkey(), lamports_to_send * 3)
        .unwrap();

    // Create a multi-instruction transaction where the second instruction will fail
    let valid_instruction = transfer(&payer.pubkey(), &recipient1, lamports_to_send);
    let invalid_instruction = transfer(&payer2.pubkey(), &recipient1, lamports_to_send * 2); // payer2 has no funds

    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message = Message::new_with_blockhash(
        &[valid_instruction, invalid_instruction],
        Some(&payer.pubkey()),
        &latest_blockhash,
    );
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer, &payer2])
            .unwrap();

    // Profile the multi-instruction transaction

    let uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile shows failure
    assert!(
        profile_result.transaction_profile.error_message.is_some(),
        "Multi-instruction transaction should fail"
    );

    // Verify instruction profiles exist
    assert!(
        profile_result.instruction_profiles.is_some(),
        "Instruction profiles should be generated for multi-instruction transaction"
    );

    let instruction_profiles = profile_result.instruction_profiles.as_ref().unwrap();
    assert_eq!(instruction_profiles.len(), 2,);

    // Verify the first instruction profile succeeded
    let first_instruction_profile = &instruction_profiles[0];
    assert!(
        first_instruction_profile.error_message.is_none(),
        "First instruction should succeed"
    );
    assert!(
        first_instruction_profile.compute_units_consumed > 0,
        "First instruction should consume compute units"
    );

    let second_profile = &instruction_profiles[1];
    assert!(
        second_profile.error_message.is_some(),
        "Second instruction should fail due to insufficient funds"
    );

    println!("Multi-instruction failure profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_with_encoding() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = LAMPORTS_PER_SOL;

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

    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Transaction profiling should succeed with base64 encoding"
    );

    // // Verify state snapshots use the specified encoding
    // let pre_execution_accounts = &profile_result.inner.transaction_profile.state.pre_execution;
    // let post_execution_accounts = &profile_result
    //     .inner
    //     .transaction_profile
    //     .state
    //     .post_execution;

    // for (_, account_opt) in pre_execution_accounts
    //     .iter()
    //     .chain(post_execution_accounts.iter())
    // {
    //     if let Some(account) = account_opt {
    //         // Verify the account data is base64 encoded
    //         if let UiAccountData::Binary(_, encoding) = &account.data {
    //             assert!(
    //                 *encoding == UiAccountEncoding::Base64,
    //                 "Account data should be base64 encoded"
    //             );
    //         }
    //     }
    // }

    println!("Encoding profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_with_tag_and_retrieval() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = LAMPORTS_PER_SOL;

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

    // Profile with a tag
    let tag = "test_tag_retrieval".to_string();

    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), Some(tag.clone()))
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Transaction profiling should succeed"
    );

    // Test retrieval by UUID
    let retrieved_by_uuid = svm_locker
        .get_profile_result(
            UuidOrSignature::Uuid(profile_uuid),
            &RpcProfileResultConfig::default(),
        )
        .unwrap();
    assert!(
        retrieved_by_uuid.is_some(),
        "Profile should be retrievable by UUID"
    );

    let retrieved = retrieved_by_uuid.unwrap();
    assert_eq!(
        retrieved.transaction_profile.compute_units_consumed,
        profile_result.transaction_profile.compute_units_consumed,
        "Retrieved profile should match original profile"
    );

    // Test retrieval by tag
    let retrieved_by_tag = svm_locker
        .get_profile_results_by_tag(tag.clone(), &RpcProfileResultConfig::default())
        .unwrap();
    assert!(
        retrieved_by_tag.is_some(),
        "Profile should be retrievable by tag"
    );

    let tagged_profiles = retrieved_by_tag.unwrap();
    assert_eq!(
        tagged_profiles.len(),
        1,
        "Should have exactly one profile for this tag"
    );

    let tagged_profile = &tagged_profiles[0];
    assert_eq!(
        tagged_profile.key,
        UuidOrSignature::Uuid(profile_uuid),
        "Tagged profile should have the same UUID"
    );

    // Test retrieval with non-existent tag
    let non_existent_tag = "non_existent_tag".to_string();
    let non_existent_result = svm_locker
        .get_profile_results_by_tag(non_existent_tag, &RpcProfileResultConfig::default())
        .unwrap();
    assert!(
        non_existent_result.is_none(),
        "Non-existent tag should return None"
    );

    println!("Tag and retrieval profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_empty_instruction() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let lamports_to_send = LAMPORTS_PER_SOL;

    // Airdrop SOL to payer
    svm_locker
        .airdrop(&payer.pubkey(), lamports_to_send)
        .unwrap();

    // Create a transaction with no instructions
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message = Message::new_with_blockhash(
        &[], // No instructions
        Some(&payer.pubkey()),
        &latest_blockhash,
    );
    let transaction =
        VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    // Profile the empty transaction
    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    println!("profile result: {:#?}", profile_result);

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Empty transaction profiling should succeed"
    );

    // Verify compute units consumed (should be minimal)
    assert_eq!(profile_result.transaction_profile.compute_units_consumed, 0);

    // Verify no instruction profiles for empty transaction
    assert!(
        profile_result.instruction_profiles.is_none(),
        "Empty transaction should not have instruction profiles"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_profile_transaction_versioned_message() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Set up test accounts
    let payer = Keypair::new();
    let recipient = Pubkey::new_unique();
    let lamports_to_send = LAMPORTS_PER_SOL;

    // Airdrop SOL to payer
    svm_locker
        .airdrop(&payer.pubkey(), 2 * lamports_to_send)
        .unwrap();

    svm_locker.confirm_current_block().unwrap();

    // Create a transfer instruction
    let instruction = transfer(&payer.pubkey(), &recipient, lamports_to_send);
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());

    // Create a v0 message
    let v0_message = v0::Message::try_compile(
        &payer.pubkey(),
        &[instruction],
        &[], // No address table lookups
        latest_blockhash,
    )
    .expect("Failed to compile v0 message");

    let transaction =
        VersionedTransaction::try_new(VersionedMessage::V0(v0_message), &[&payer]).unwrap();

    // Profile the versioned transaction
    let profile_uuid = svm_locker
        .profile_transaction(&None, transaction.clone(), None)
        .await
        .unwrap()
        .inner;

    let key = UuidOrSignature::Uuid(profile_uuid);
    let profile_result = svm_locker
        .get_profile_result(key, &RpcProfileResultConfig::default())
        .unwrap()
        .expect("Profile result should exist");

    // Verify transaction profile
    assert!(
        profile_result.transaction_profile.error_message.is_none(),
        "Versioned transaction profiling should succeed, found: {:?}",
        profile_result.transaction_profile.error_message.unwrap()
    );
    assert!(
        profile_result.transaction_profile.compute_units_consumed > 0,
        "Versioned transaction should consume compute units"
    );

    println!("Versioned message profiling test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_local_signatures_without_limit() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance.clone());

    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    let payer = Keypair::new();
    let recipient = Keypair::new();
    let lamports_to_send = 1_000_000;

    svm_locker_for_context
        .airdrop(&payer.pubkey(), lamports_to_send * 2)
        .unwrap();

    svm_locker_for_context.confirm_current_block().unwrap();

    let create_account_instruction = system_instruction::create_account(
        &payer.pubkey(),
        &recipient.pubkey(),
        lamports_to_send / 2,
        0,
        &solana_sdk_ids::system_program::id(),
    );

    let create_account_tx = Transaction::new_signed_with_payer(
        &[create_account_instruction],
        Some(&payer.pubkey()),
        &[&payer, &recipient],
        svm_locker_for_context.with_svm_reader(|svm| svm.latest_blockhash()),
    );

    let (create_status_tx, _create_status_rx) = crossbeam_channel::bounded(1);
    svm_locker_for_context
        .process_transaction(
            &None,
            create_account_tx.into(),
            create_status_tx,
            false,
            true,
        )
        .await
        .unwrap();
    // Confirm the block after creating the account
    svm_locker_for_context.confirm_current_block().unwrap();

    // Now create the transfer transaction
    let instruction = transfer(&payer.pubkey(), &recipient.pubkey(), lamports_to_send);
    let latest_blockhash = svm_locker_for_context.with_svm_reader(|svm| svm.latest_blockhash());
    let message =
        Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message.clone()), &[&payer])
        .unwrap();

    let (status_tx, _status_rx) = crossbeam_channel::bounded(1);

    svm_locker_for_context
        .process_transaction(&None, tx.clone(), status_tx, false, true)
        .await
        .unwrap();

    // Confirm the current block to create a block with the transaction signature
    svm_locker_for_context.confirm_current_block().unwrap();

    let get_local_signatures_response: JsonRpcResult<RpcResponse<Vec<RpcLogsResponse>>> =
        rpc_server
            .get_local_signatures(Some(runloop_context.clone()), None)
            .await;

    assert!(
        get_local_signatures_response.is_ok(),
        "Get local signatures failed: {:?}",
        get_local_signatures_response.err()
    );

    let local_signatures = get_local_signatures_response.unwrap().value;
    assert!(local_signatures.len() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_local_signatures_with_limit() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();

    let svm_locker_for_context = SurfnetSvmLocker::new(svm_instance.clone());

    let (simnet_cmd_tx, _simnet_cmd_rx) = crossbeam_unbounded::<SimnetCommand>();
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker_for_context.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    let payer = Keypair::new();
    let _recipient = Keypair::new();
    let lamports_to_send = 1_000_000;

    svm_locker_for_context
        .airdrop(&payer.pubkey(), lamports_to_send * 10)
        .unwrap();

    svm_locker_for_context.confirm_current_block().unwrap();

    // Get the initial number of signatures to establish a baseline
    let initial_signatures_response: JsonRpcResult<RpcResponse<Vec<RpcLogsResponse>>> = rpc_server
        .get_local_signatures(Some(runloop_context.clone()), None)
        .await;

    let initial_count = initial_signatures_response.unwrap().value.len();

    // Create multiple transactions to test limit functionality
    let num_transactions = 10;
    let mut transaction_signatures = Vec::new();

    for _i in 0..num_transactions {
        // Create a unique recipient for each transaction
        let unique_recipient = Keypair::new();

        let instruction = transfer(
            &payer.pubkey(),
            &unique_recipient.pubkey(),
            lamports_to_send / num_transactions as u64,
        );
        let latest_blockhash = svm_locker_for_context.with_svm_reader(|svm| svm.latest_blockhash());
        let message =
            Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
        let tx =
            VersionedTransaction::try_new(VersionedMessage::Legacy(message.clone()), &[&payer])
                .unwrap();

        let (status_tx, _status_rx) = crossbeam_channel::bounded(1);

        svm_locker_for_context
            .process_transaction(&None, tx.clone(), status_tx, false, true)
            .await
            .unwrap();

        // Store the signature for verification
        transaction_signatures.push(tx.signatures[0]);

        // Confirm the current block to create a new block with this transaction
        svm_locker_for_context.confirm_current_block().unwrap();
    }

    // Test with different limit values
    let test_limits = vec![1, 3, 5, 10, 15];

    for limit in test_limits {
        let get_local_signatures_response: JsonRpcResult<RpcResponse<Vec<RpcLogsResponse>>> =
            rpc_server
                .get_local_signatures(Some(runloop_context.clone()), Some(limit))
                .await;

        assert!(
            get_local_signatures_response.is_ok(),
            "Get local signatures with limit {} failed: {:?}",
            limit,
            get_local_signatures_response.err()
        );

        let local_signatures = get_local_signatures_response.unwrap().value;

        // Verify that the number of returned signatures respects the limit
        assert!(
            local_signatures.len() <= limit as usize,
            "Expected at most {} signatures, but got {}",
            limit,
            local_signatures.len()
        );

        // Verify that we get the expected number of signatures
        // The total expected count should be min(limit, initial_count + num_transactions)
        let total_expected_signatures = initial_count + num_transactions;
        let expected_count = std::cmp::min(limit as usize, total_expected_signatures);

        assert!(
            local_signatures.len() == expected_count,
            "Expected {} signatures with limit {}, but got {} (initial: {}, new: {})",
            expected_count,
            limit,
            local_signatures.len(),
            initial_count,
            num_transactions
        );
    }

    // Test with limit = 0 (should return empty list)
    let get_local_signatures_response: JsonRpcResult<RpcResponse<Vec<RpcLogsResponse>>> =
        rpc_server
            .get_local_signatures(Some(runloop_context.clone()), Some(0))
            .await;

    assert!(
        get_local_signatures_response.is_ok(),
        "Get local signatures with limit 0 failed: {:?}",
        get_local_signatures_response.err()
    );

    let local_signatures = get_local_signatures_response.unwrap().value;
    assert!(
        local_signatures.is_empty(),
        "Expected empty list with limit 0, but got {} signatures",
        local_signatures.len()
    );

    println!("All local signatures tests passed successfully!");
}

// ============================================================================
// Clock Control Tests (pauseClock, resumeClock, timeTravel)
// ============================================================================

fn boot_simnet(
    block_production_mode: BlockProductionMode,
    slot_time: Option<u64>,
) -> (
    SurfnetSvmLocker,
    crossbeam_channel::Sender<SimnetCommand>,
    crossbeam_channel::Receiver<SimnetEvent>,
) {
    let bind_host = "127.0.0.1";
    let bind_port = get_free_port().unwrap();
    let config = SurfpoolConfig {
        simnets: vec![SimnetConfig {
            slot_time: slot_time.unwrap_or(DEFAULT_SLOT_TIME_MS),
            block_production_mode,
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

    let svm_locker_cc: SurfnetSvmLocker = svm_locker.clone();
    let simnet_commands_tx_cc = simnet_commands_tx.clone();
    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start_local_surfnet_runloop(
            svm_locker_cc,
            config,
            subgraph_commands_tx,
            simnet_commands_tx_cc,
            simnet_commands_rx,
            geyser_events_rx,
        );
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    loop {
        if let Ok(SimnetEvent::Ready) = simnet_events_rx.recv_timeout(Duration::from_millis(1000)) {
            break;
        }
    }

    (svm_locker, simnet_commands_tx, simnet_events_rx)
}

#[test]
fn test_time_travel_resume_paused_clock() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_locker, simnet_cmd_tx, _) = boot_simnet(BlockProductionMode::Clock, Some(20));
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker.clone(),
        simnet_commands_tx: simnet_cmd_tx,
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    // Get initial epoch info
    let initial_slot = svm_locker.get_latest_absolute_slot();

    // Ensure the clock has advanced
    std::thread::sleep(Duration::from_millis(500));
    let new_slot = svm_locker.get_latest_absolute_slot();
    assert!(
        new_slot > initial_slot,
        "Slot should change when clock is not paused"
    );

    // Test pause clock
    let _ = rpc_server.pause_clock(Some(runloop_context.clone()));

    // Buffer to ensure the clock is paused
    std::thread::sleep(Duration::from_millis(100));

    // Get latest slot
    let slot_after_pause = svm_locker.get_latest_absolute_slot();

    // Ensure the clock is paused after 500ms
    std::thread::sleep(Duration::from_millis(500));
    let slot_after_pause_and_500ms = svm_locker.get_latest_absolute_slot();
    assert!(
        slot_after_pause == slot_after_pause_and_500ms,
        "Slot should change when clock is not paused"
    );

    // Ensure the clock is still paused after 2s
    std::thread::sleep(Duration::from_millis(2000));
    let slot_after_pause_and_2500ms = svm_locker.get_latest_absolute_slot();
    assert!(
        slot_after_pause == slot_after_pause_and_2500ms,
        "Slot should change when clock is not paused"
    );

    // Now let's resume the clock
    let resume_response: JsonRpcResult<EpochInfo> =
        rpc_server.resume_clock(Some(runloop_context.clone()));
    let slot_after_resume = svm_locker.get_latest_absolute_slot();

    assert!(
        resume_response.is_ok(),
        "Resume clock failed: {:?}",
        resume_response.err()
    );

    // Ensure the clock is paused after 500ms
    std::thread::sleep(Duration::from_millis(500));
    let slot_after_resume_and_500ms = svm_locker.get_latest_absolute_slot();
    assert!(
        slot_after_resume < slot_after_resume_and_500ms,
        "Slot should change when clock is not paused"
    );

    println!("Resume clock test passed successfully!");
}

#[test]
fn test_time_travel_absolute_timestamp() {
    let rpc_server = SurfnetCheatcodesRpc;
    let slot_time = 100;
    let (svm_locker, simnet_cmd_tx, simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(slot_time.clone()));
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker.clone(),
        simnet_commands_tx: simnet_cmd_tx.clone(),
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    let clock = Clock {
        slot: 1,
        epoch_start_timestamp: 1,
        epoch: 1,
        leader_schedule_epoch: 100,
        unix_timestamp: 100,
    };

    simnet_cmd_tx
        .send(SimnetCommand::UpdateInternalClock(None, clock))
        .unwrap();
    let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
        simnet_events_rx.recv_timeout(Duration::from_millis(5000))
    else {
        panic!("failed to update internal clock")
    };

    let initial_epoch_info = svm_locker.get_epoch_info();

    println!("Initial epoch info: {:?}", initial_epoch_info);

    let seven_days = 7 * 24 * 60 * 60 * 1000;
    let target_timestamp = svm_locker.0.blocking_read().updated_at + seven_days;

    // Test time travel to absolute timestamp
    let time_travel_response: JsonRpcResult<EpochInfo> = rpc_server.time_travel(
        Some(runloop_context.clone()),
        Some(TimeTravelConfig::AbsoluteTimestamp(target_timestamp)),
    );
    loop {
        if let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
            simnet_events_rx.recv_timeout(Duration::from_millis(5000))
        {
            break;
        }
    }

    assert!(
        time_travel_response.is_ok(),
        "Time travel to absolute timestamp failed: {:?}",
        time_travel_response.err()
    );

    let new_epoch_info = time_travel_response.unwrap();
    println!("Response: {:?}", new_epoch_info);

    // Verify the epoch info reflects the time travel
    assert_ne!(
        new_epoch_info.epoch, initial_epoch_info.epoch,
        "Epoch should change after time travel"
    );
    assert_ne!(
        new_epoch_info.absolute_slot, initial_epoch_info.absolute_slot,
        "Slot should change after time travel"
    );

    // Verify the current epoch info in SVM matches
    let current_epoch_info = svm_locker.get_epoch_info();
    println!("Updated epoch info: {:?}", current_epoch_info);

    assert_eq!(current_epoch_info.epoch, new_epoch_info.epoch);
    assert_eq!(
        current_epoch_info.absolute_slot,
        new_epoch_info.absolute_slot
    );

    println!("Time travel to absolute timestamp test passed successfully!");
}

#[test]
fn test_time_travel_absolute_slot() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_locker, simnet_cmd_tx, simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(400));
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker.clone(),
        simnet_commands_tx: simnet_cmd_tx.clone(),
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    let clock = Clock {
        slot: 1,
        epoch_start_timestamp: 1,
        epoch: 1,
        leader_schedule_epoch: 100,
        unix_timestamp: 100,
    };

    simnet_cmd_tx
        .send(SimnetCommand::UpdateInternalClock(None, clock))
        .unwrap();
    let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
        simnet_events_rx.recv_timeout(Duration::from_millis(5000))
    else {
        panic!("failed to update internal clock")
    };

    let initial_epoch_info = svm_locker.get_epoch_info();
    let target_slot = initial_epoch_info.absolute_slot + 1000000; // A future slot number

    // Test time travel to absolute slot
    let time_travel_response: JsonRpcResult<EpochInfo> = rpc_server.time_travel(
        Some(runloop_context.clone()),
        Some(TimeTravelConfig::AbsoluteSlot(target_slot)),
    );
    loop {
        if let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
            simnet_events_rx.recv_timeout(Duration::from_millis(5000))
        {
            break;
        }
    }

    assert!(
        time_travel_response.is_ok(),
        "Time travel to absolute slot failed: {:?}",
        time_travel_response.err()
    );

    let new_epoch_info = time_travel_response.unwrap();

    // Verify the epoch info reflects the time travel
    assert_eq!(
        new_epoch_info.absolute_slot, target_slot,
        "Slot should match target slot"
    );
    assert!(
        new_epoch_info.epoch > initial_epoch_info.epoch,
        "Epoch should change after time travel"
    );
    assert!(
        new_epoch_info.absolute_slot > initial_epoch_info.absolute_slot,
        "Epoch should change after time travel"
    );

    // Verify the current epoch info in SVM matches
    let current_epoch_info = svm_locker.get_epoch_info();
    assert_eq!(current_epoch_info.absolute_slot, target_slot);
    assert_eq!(current_epoch_info.epoch, new_epoch_info.epoch);

    println!("Time travel to absolute slot test passed successfully!");
}

#[test]
fn test_time_travel_absolute_epoch() {
    let rpc_server = SurfnetCheatcodesRpc;
    let (svm_locker, simnet_cmd_tx, simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(400));
    let (plugin_cmd_tx, _plugin_cmd_rx) = crossbeam_unbounded::<PluginManagerCommand>();

    let runloop_context = RunloopContext {
        id: None,
        svm_locker: svm_locker.clone(),
        simnet_commands_tx: simnet_cmd_tx.clone(),
        plugin_manager_commands_tx: plugin_cmd_tx,
        remote_rpc_client: None,
    };

    let clock = Clock {
        slot: 1,
        epoch_start_timestamp: 1,
        epoch: 1,
        leader_schedule_epoch: 100,
        unix_timestamp: 100,
    };

    simnet_cmd_tx
        .send(SimnetCommand::UpdateInternalClock(None, clock))
        .unwrap();
    let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
        simnet_events_rx.recv_timeout(Duration::from_millis(5000))
    else {
        panic!("failed to update internal clock")
    };

    let initial_epoch_info = svm_locker.get_epoch_info();
    let target_epoch = initial_epoch_info.epoch + 100; // A future epoch number

    // Test time travel to absolute epoch
    let time_travel_response: JsonRpcResult<EpochInfo> = rpc_server.time_travel(
        Some(runloop_context.clone()),
        Some(TimeTravelConfig::AbsoluteEpoch(target_epoch)),
    );
    loop {
        if let Ok(SimnetEvent::SystemClockUpdated(_clock_updated)) =
            simnet_events_rx.recv_timeout(Duration::from_millis(5000))
        {
            break;
        }
    }

    assert!(
        time_travel_response.is_ok(),
        "Time travel to absolute epoch failed: {:?}",
        time_travel_response.err()
    );

    let new_epoch_info = time_travel_response.unwrap();

    // Verify the epoch info reflects the time travel
    assert_eq!(
        new_epoch_info.epoch, target_epoch,
        "Epoch should match target epoch"
    );
    assert_ne!(
        new_epoch_info.epoch, initial_epoch_info.epoch,
        "Epoch should change after time travel"
    );
    assert_ne!(
        new_epoch_info.absolute_slot, initial_epoch_info.absolute_slot,
        "Slot should change after time travel"
    );

    // Verify the current epoch info in SVM matches
    let current_epoch_info = svm_locker.get_epoch_info();
    assert_eq!(current_epoch_info.epoch, target_epoch);
    assert_eq!(
        current_epoch_info.absolute_slot,
        new_epoch_info.absolute_slot
    );

    println!("Time travel to absolute epoch test passed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ix_profiling_with_alt_tx() {
    let (svm_locker, _simnet_cmd_tx, _simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(400));

    let p1 = Keypair::new();
    let p2 = Keypair::new();

    svm_locker.airdrop(&p1.pubkey(), LAMPORTS_PER_SOL).unwrap();
    svm_locker.airdrop(&p2.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let recent_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());

    let mint = Keypair::new();
    let mint_rent = 1461600;
    let create_mint_ix = system_instruction::create_account(
        &p1.pubkey(),
        &mint.pubkey(),
        mint_rent,
        82,
        &spl_token_2022::id(),
    );

    let initialize_mint_ix = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &mint.pubkey(),
        &p1.pubkey(),
        Some(&p1.pubkey()),
        2,
    )
    .unwrap();

    let at1 = spl_associated_token_account::get_associated_token_address_with_program_id(
        &p1.pubkey(),
        &mint.pubkey(),
        &spl_token_2022::id(),
    );
    let at2 = spl_associated_token_account::get_associated_token_address_with_program_id(
        &p2.pubkey(),
        &mint.pubkey(),
        &spl_token_2022::id(),
    );

    let create_at1_ix = spl_associated_token_account::instruction::create_associated_token_account(
        &p1.pubkey(),
        &p1.pubkey(),
        &mint.pubkey(),
        &spl_token_2022::id(),
    );

    let create_at2_ix = spl_associated_token_account::instruction::create_associated_token_account(
        &p1.pubkey(),
        &p2.pubkey(),
        &mint.pubkey(),
        &spl_token_2022::id(),
    );

    let mint_amount = 100_00;
    let mint_to_ix = spl_token_2022::instruction::mint_to(
        &spl_token_2022::id(),
        &mint.pubkey(),
        &at1,
        &p1.pubkey(),
        &[&p1.pubkey()],
        mint_amount,
    )
    .unwrap();

    let setup_instructions = vec![
        create_mint_ix,
        initialize_mint_ix,
        create_at1_ix,
        create_at2_ix,
        mint_to_ix,
    ];

    let setup_message =
        Message::new_with_blockhash(&setup_instructions, Some(&p1.pubkey()), &recent_blockhash);

    let setup_tx =
        VersionedTransaction::try_new(VersionedMessage::Legacy(setup_message), &[&p1, &mint])
            .unwrap();

    let status_tx = crossbeam_unbounded::<TransactionStatusEvent>().0;
    svm_locker
        .process_transaction(&None, setup_tx, status_tx, false, true)
        .await
        .unwrap();

    let alt_key = Pubkey::new_unique();

    let address_lookup_table_account = AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![at1, at2, spl_token_2022::id(), mint.pubkey()],
    };

    println!(
        "addresses present on the ALT: {:?}",
        address_lookup_table_account.addresses
    );

    let alt_account_data = AddressLookupTable {
        meta: LookupTableMeta {
            authority: Some(p1.pubkey()),
            ..Default::default()
        },
        addresses: address_lookup_table_account.addresses.clone().into(),
    };

    svm_locker.with_svm_writer(|svm| {
        let alt_data = alt_account_data.serialize_for_tests().unwrap();
        let alt_account = solana_account::Account {
            lamports: 1000000,
            data: alt_data,
            owner: solana_address_lookup_table_interface::program::id(),
            executable: false,
            rent_epoch: 0,
        };

        svm.set_account(&alt_key, alt_account).unwrap();
    });

    let transfer_amount = 50_00;
    let transfer_ix = spl_token_2022::instruction::transfer_checked(
        &spl_token_2022::id(),
        &at1,
        &mint.pubkey(),
        &at2,
        &p1.pubkey(),
        &[&p1.pubkey()],
        transfer_amount,
        2,
    )
    .unwrap();

    let alt_message = v0::Message::try_compile(
        &p1.pubkey(),
        &[transfer_ix],
        &[address_lookup_table_account.clone()],
        recent_blockhash,
    )
    .expect("Failed to compile ALT message");

    let alt_tx = VersionedTransaction::try_new(VersionedMessage::V0(alt_message.clone()), &[&p1])
        .expect("Failed to create ALT transaction");

    let binding = svm_locker
        .profile_transaction(&None, alt_tx.clone(), None)
        .await
        .unwrap();
    let profile_result_uuid = binding.inner();

    let profile_result = svm_locker
        .get_profile_result(
            UuidOrSignature::Uuid(*profile_result_uuid),
            &RpcProfileResultConfig::default(),
        )
        .unwrap()
        .unwrap();
    let ix_profiles = profile_result
        .instruction_profiles
        .as_ref()
        .expect("instruction profiles should exist");
    assert_eq!(
        ix_profiles.len(),
        1,
        "ALT transfer should have one instruction"
    );
    let ix_profile = ix_profiles.get(0).unwrap();
    assert!(
        ix_profile.error_message.is_none(),
        "Profile should succeed, found error: {:?}",
        ix_profile.error_message.as_ref().unwrap()
    );
    assert!(
        ix_profile.account_states.get(&alt_key).is_none(),
        "ALT account should not be in instruction account states"
    );
    assert!(
        profile_result
            .readonly_account_states
            .get(&alt_key)
            .is_none(),
        "ALT account should not be in readonly account states"
    );

    let table = alt_message
        .address_table_lookups
        .first()
        .expect("ALT lookups should exist");
    let expected_loaded_writable: Vec<Pubkey> = table
        .writable_indexes
        .iter()
        .map(|&i| address_lookup_table_account.addresses[i as usize])
        .collect();
    let expected_loaded_readonly: Vec<Pubkey> = table
        .readonly_indexes
        .iter()
        .map(|&i| address_lookup_table_account.addresses[i as usize])
        .collect();

    for pk in &expected_loaded_writable {
        match ix_profile
            .account_states
            .get(pk)
            .expect("loaded writable address must be present")
        {
            UiAccountProfileState::Writable(_) => {}
            other => panic!(
                "expected Writable for loaded writable address {}, got {:?}",
                pk, other
            ),
        }
    }
    for pk in &expected_loaded_readonly {
        match ix_profile
            .account_states
            .get(pk)
            .expect("loaded readonly address must be present")
        {
            UiAccountProfileState::Readonly => {}
            other => panic!(
                "expected Readonly for loaded readonly address {}, got {:?}",
                pk, other
            ),
        }
    }

    let account_states = ix_profile.account_states.clone();

    let UiAccountProfileState::Readonly = account_states
        .get(&spl_token_2022::id())
        .expect("token-2022 program should be present")
    else {
        panic!("expected token-2022 program to be Readonly");
    };

    let UiAccountProfileState::Readonly = account_states
        .get(&mint.pubkey())
        .expect("mint should be present")
    else {
        panic!("expected mint to be Readonly");
    };

    let UiAccountProfileState::Writable(src_change) = account_states
        .get(&at1)
        .expect("source ATA should be present")
    else {
        panic!("expected source ATA to be Writable");
    };
    match src_change {
        UiAccountChange::Update(before, after) => {
            assert_ne!(
                before.data, after.data,
                "token data should change on source"
            );
        }
        _ => panic!("expected Update for source ATA"),
    }

    let UiAccountProfileState::Writable(dst_change) = account_states
        .get(&at2)
        .expect("destination ATA should be present")
    else {
        panic!("expected destination ATA to be Writable");
    };
    match dst_change {
        UiAccountChange::Update(before, after) => {
            assert_ne!(
                before.data, after.data,
                "token data should change on destination"
            );
        }
        _ => panic!("expected Update for destination ATA"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn it_should_delete_accounts_with_no_lamports() {
    let (svm_locker, _simnet_cmd_tx, _simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(400));

    let p1 = Keypair::new();
    let p2 = Keypair::new();

    svm_locker.airdrop(&p1.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let recent_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());

    let message = Message::new_with_blockhash(
        &[system_instruction::transfer(
            &p1.pubkey(),
            &p2.pubkey(),
            LAMPORTS_PER_SOL - 5000,
        )],
        Some(&p1.pubkey()),
        &recent_blockhash,
    );
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&p1]).unwrap();

    let (status_tx, rx) = unbounded();
    let _ = svm_locker
        .process_transaction(&None, tx, status_tx, true, false)
        .await
        .unwrap();

    #[allow(clippy::never_loop)]
    loop {
        match rx.recv() {
            Ok(status) => {
                println!("Transaction status: {:?}", status);
                break;
            }
            Err(_) => panic!("status channel closed unexpectedly"),
        }
    }

    assert!(
        svm_locker.get_account_local(&p1.pubkey()).inner.is_none(),
        "Account should be deleted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_compute_budget_profiling() {
    let (svm_locker, _simnet_cmd_tx, _simnet_events_rx) =
        boot_simnet(BlockProductionMode::Clock, Some(400));

    let p1 = Keypair::new();
    let p2 = Keypair::new();

    svm_locker.airdrop(&p1.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let recent_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());

    let message = Message::new_with_blockhash(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
            ComputeBudgetInstruction::set_compute_unit_price(1),
            system_instruction::transfer(&p1.pubkey(), &p2.pubkey(), LAMPORTS_PER_SOL),
        ],
        Some(&p1.pubkey()),
        &recent_blockhash,
    );
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&p1]).unwrap();

    let uuid = svm_locker
        .profile_transaction(&None, tx, None)
        .await
        .unwrap()
        .inner;
    let profile_result = svm_locker
        .get_profile_result(
            UuidOrSignature::Uuid(uuid),
            &RpcProfileResultConfig::default(),
        )
        .unwrap()
        .unwrap();

    let ix_profile = profile_result.instruction_profiles.unwrap();
    assert_eq!(ix_profile.len(), 3, "Should have 3 instruction profiles");

    let ix = &ix_profile[0];
    assert!(
        ix.error_message.is_none(),
        "Expected no error for instruction, found {}",
        ix.error_message.as_ref().unwrap()
    );
    assert_eq!(ix.compute_units_consumed, 150);

    let ix = &ix_profile[1];
    assert!(
        ix.error_message.is_none(),
        "Expected no error for instruction, found {}",
        ix.error_message.as_ref().unwrap()
    );
    assert_eq!(ix.compute_units_consumed, 150);

    let ix = &ix_profile[1];
    assert!(
        ix.error_message.is_none(),
        "Expected no error for instruction, found {}",
        ix.error_message.as_ref().unwrap()
    );
    assert_eq!(ix.compute_units_consumed, 150);
}

#[test]
fn test_reset_account() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);
    let p1 = Keypair::new();
    println!("P1 pubkey: {}", p1.pubkey());
    svm_locker.airdrop(&p1.pubkey(), LAMPORTS_PER_SOL).unwrap(); // account is created in the SVM
    println!("Airdropped SOL to p1");

    println!(
        "Account before reset: {:?}",
        svm_locker.get_account_local(&p1.pubkey()).inner
    );

    svm_locker
        .reset_account(
            p1.pubkey(),
            ResetAccountConfig {
                recursive: Some(false),
            },
        )
        .unwrap();

    println!("Reset account");

    println!(
        "Account deleted: {:?}",
        svm_locker.get_account_local(&p1.pubkey()).inner
    );

    assert!(
        svm_locker.get_account_local(&p1.pubkey()).inner.is_none(),
        "Account should be deleted"
    );
}

#[test]
fn test_reset_account_cascade() {
    let (svm_instance, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::new();
    let svm_locker = SurfnetSvmLocker::new(svm_instance);

    // Create owner account and owned account
    let owner = Pubkey::new_unique();
    let owned = Pubkey::new_unique();

    let owner_account = Account {
        lamports: 10 * LAMPORTS_PER_SOL,
        data: vec![0x01, 0x02],
        owner: solana_sdk_ids::system_program::id(),
        executable: false,
        rent_epoch: 0,
    };

    let owned_account = Account {
        lamports: 5 * LAMPORTS_PER_SOL,
        data: vec![0x03, 0x04],
        owner, // Owned by the first account
        executable: false,
        rent_epoch: 0,
    };

    // Insert accounts
    svm_locker
        .with_svm_writer(|svm_writer| {
            svm_writer.set_account(&owner, owner_account).unwrap();
            svm_writer.set_account(&owned, owned_account).unwrap();
            Ok::<(), SurfpoolError>(())
        })
        .unwrap();

    // Verify accounts exist
    assert!(!svm_locker.get_account_local(&owner).inner.is_none());
    assert!(!svm_locker.get_account_local(&owned).inner.is_none());

    // Reset with cascade=true (for regular accounts, doesn't cascade but tests the code path)
    svm_locker
        .reset_account(
            owner,
            ResetAccountConfig {
                recursive: Some(true),
            },
        )
        .unwrap();

    // Owner is deleted, owned account is deleted
    assert!(svm_locker.get_account_local(&owner).inner.is_none());
    assert!(svm_locker.get_account_local(&owned).inner.is_none());

    // Clean up
    svm_locker
        .reset_account(
            owned,
            ResetAccountConfig {
                recursive: Some(false),
            },
        )
        .unwrap();
}
