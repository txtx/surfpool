use crossbeam_channel::unbounded;
use helpers::get_free_port;
use jsonrpc_core::{
    futures::future::{self, join_all},
    Error,
};
use jsonrpc_core_client::transports::http;
use solana_sdk::{
    hash::Hash,
    message::{Message, VersionedMessage},
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::VersionedTransaction,
};
use std::{str::FromStr, time::Duration};
use surfpool_core::{
    rpc::{full::FullClient, minimal::MinimalClient},
    simnet::{start, SimnetEvent},
    types::{RpcConfig, RunloopTriggerMode, SimnetConfig, SurfpoolConfig},
};
use tokio::{task, time::sleep};

mod helpers;

#[tokio::test]
async fn test_simnet_ready() {
    let config = SurfpoolConfig {
        simnet: SimnetConfig {
            runloop_trigger_mode: RunloopTriggerMode::Manual, // Prevent ticks
            ..SimnetConfig::default()
        },
        ..SurfpoolConfig::default()
    };

    let (simnet_events_tx, simnet_events_rx) = unbounded();
    let (_simnet_commands_tx, simnet_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start(config, simnet_events_tx, simnet_commands_rx);
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    match simnet_events_rx.recv() {
        Ok(SimnetEvent::Ready) => println!("Simnet is ready"),
        e => panic!("Expected Ready event: {e:?}"),
    }
}

#[tokio::test]
async fn test_simnet_ticks() {
    let config = SurfpoolConfig {
        simnet: SimnetConfig {
            slot_time: 1,
            ..SimnetConfig::default()
        },
        ..SurfpoolConfig::default()
    };

    let (simnet_events_tx, simnet_events_rx) = unbounded();
    let (_simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (test_tx, test_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start(config, simnet_events_tx, simnet_commands_rx);
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
        simnet: SimnetConfig {
            slot_time: 1,
            airdrop_addresses: airdrop_addresses.clone(),
            airdrop_token_amount,
            ..SimnetConfig::default()
        },
        rpc: RpcConfig {
            bind_host: bind_host.to_string(),
            bind_port,
            ..Default::default()
        },
        ..SurfpoolConfig::default()
    };

    let (simnet_events_tx, simnet_events_rx) = unbounded();
    let (_simnet_commands_tx, simnet_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start(config, simnet_events_tx, simnet_commands_rx);
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Ready) => break,
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
            println!("{}", b.value);
            if i == 0 {
                b.value > airdrop_token_amount
            } else {
                b.value < airdrop_token_amount / 2
            }
        }), // TODO: compute fee
        "Some transfers failed"
    );
}
