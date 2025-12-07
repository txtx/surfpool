mod fixtures;

use std::thread::JoinHandle;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use crossbeam_channel::{Receiver, unbounded};
use fixtures::{
    MULTI_TRANSFER_LAMPORTS, SIMPLE_TRANSFER_LAMPORTS, TRANSFER_AMOUNT_PER_RECIPIENT,
    create_complex_transaction_with_recipients, create_protocol_like_transaction_with_recipients,
    create_transfer_transaction, create_transfer_transaction_with_recipients,
    load_protocol_transactions, protocol_fixtures_available, wait_for_ready,
};
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::{
    rpc::{
        RunloopContext,
        full::{Full, SurfpoolFullRpc},
    },
    runloops::start_local_surfnet_runloop,
    surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm},
};
use surfpool_types::{
    SimnetEvent, SurfpoolConfig,
    types::{BlockProductionMode, RpcConfig, SimnetConfig},
};

const BENCHMARK_SAMPLE_SIZE: usize = 10;
const BENCHMARK_WARM_UP_SECS: u64 = 1;
const BENCHMARK_MEASUREMENT_SECS: u64 = 2;

struct BenchmarkFixture {
    context: RunloopContext,
    rpc: SurfpoolFullRpc,
    runloop_handle: Option<JoinHandle<()>>,
    simnet_events_rx: Receiver<SimnetEvent>,
}

impl BenchmarkFixture {
    fn new() -> Self {
        let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
        let svm_locker = SurfnetSvmLocker::new(surfnet_svm);
        let (simnet_commands_tx, simnet_commands_rx) = unbounded();
        let (plugin_manager_commands_tx, _) = unbounded();

        let context = RunloopContext {
            simnet_commands_tx: simnet_commands_tx.clone(),
            plugin_manager_commands_tx,
            id: None,
            svm_locker: svm_locker.clone(),
            remote_rpc_client: None,
        };

        let config = SurfpoolConfig {
            simnets: vec![SimnetConfig {
                block_production_mode: BlockProductionMode::Manual,
                offline_mode: true,
                remote_rpc_url: None,
                ..SimnetConfig::default()
            }],
            rpc: RpcConfig {
                bind_port: 0,
                ws_port: 0,
                ..RpcConfig::default()
            },
            ..SurfpoolConfig::default()
        };

        let (subgraph_commands_tx, _) = unbounded();

        let runloop_handle = hiro_system_kit::thread_named("benchmark_runloop")
            .spawn(move || {
                let future = start_local_surfnet_runloop(
                    svm_locker,
                    config,
                    subgraph_commands_tx,
                    simnet_commands_tx,
                    simnet_commands_rx,
                    geyser_events_rx,
                );
                if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                    eprintln!("Benchmark runloop error: {e:?}");
                }
            })
            .expect("Failed to spawn benchmark runloop thread");

        let fixture = Self {
            context,
            rpc: SurfpoolFullRpc,
            runloop_handle: Some(runloop_handle),
            simnet_events_rx,
        };

        wait_for_ready(&fixture.simnet_events_rx);
        fixture
    }
}

impl Drop for BenchmarkFixture {
    fn drop(&mut self) {
        if let Some(handle) = self.runloop_handle.take() {
            drop(handle);
        }
    }
}

fn benchmark_send_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_ingestion");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(std::time::Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(std::time::Duration::from_secs(BENCHMARK_MEASUREMENT_SECS));

    let fixture = BenchmarkFixture::new();
    let tx_pools = [
        ("simple_transfer", 1, SIMPLE_TRANSFER_LAMPORTS, false, false),
        (
            "multi_instruction_transfer",
            5,
            MULTI_TRANSFER_LAMPORTS,
            false,
            false,
        ),
        (
            "large_transfer",
            10,
            MULTI_TRANSFER_LAMPORTS * 2,
            false,
            false,
        ),
        (
            "complex_with_compute_budget",
            5,
            MULTI_TRANSFER_LAMPORTS,
            true,
            false,
        ),
        (
            "kamino_strategy",
            5,
            MULTI_TRANSFER_LAMPORTS * 3,
            false,
            true,
        ),
    ];

    for (name, num_instructions, airdrop_amount, is_complex, is_protocol_like) in tx_pools {
        let pool_size = BENCHMARK_SAMPLE_SIZE * 10;
        let payers: Vec<Keypair> = (0..pool_size).map(|_| Keypair::new()).collect();
        let intermediate_keypairs_pool: Vec<Vec<Keypair>> = if is_protocol_like {
            (0..pool_size)
                .map(|_| (0..num_instructions).map(|_| Keypair::new()).collect())
                .collect()
        } else {
            vec![]
        };

        fixture.context.svm_locker.with_svm_writer(|svm| {
            for payer in &payers {
                svm.airdrop(&payer.pubkey(), airdrop_amount).unwrap();
            }
            if is_protocol_like {
                for keypairs in &intermediate_keypairs_pool {
                    for kp in keypairs {
                        svm.airdrop(&kp.pubkey(), TRANSFER_AMOUNT_PER_RECIPIENT * 2)
                            .unwrap();
                    }
                }
            }
        });

        let tx_pool: Vec<String> = (0..pool_size)
            .map(|i| {
                let payer = &payers[i];
                let recipients: Vec<Pubkey> = (0..num_instructions)
                    .map(|_| Pubkey::new_unique())
                    .collect();
                if is_protocol_like {
                    let intermediate_keypairs = &intermediate_keypairs_pool[i];
                    create_protocol_like_transaction_with_recipients(
                        &fixture.context.svm_locker,
                        payer,
                        intermediate_keypairs,
                        &recipients,
                        TRANSFER_AMOUNT_PER_RECIPIENT,
                    )
                } else if is_complex {
                    create_complex_transaction_with_recipients(
                        &fixture.context.svm_locker,
                        payer,
                        &recipients,
                        TRANSFER_AMOUNT_PER_RECIPIENT,
                    )
                } else {
                    create_transfer_transaction_with_recipients(
                        &fixture.context.svm_locker,
                        payer,
                        &recipients,
                        TRANSFER_AMOUNT_PER_RECIPIENT,
                    )
                }
            })
            .collect();

        group.bench_function(BenchmarkId::new(name, num_instructions), |b| {
            let mut idx = 0;
            b.iter(|| {
                let ctx = Some(fixture.context.clone());
                let encoded_tx = &tx_pool[idx % tx_pool.len()];
                idx = (idx + 1) % tx_pool.len();
                let result = fixture.rpc.send_transaction(ctx, encoded_tx.clone(), None);
                black_box(result.ok())
            });
        });
    }

    group.finish();
}

fn benchmark_transaction_components(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_components");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(std::time::Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(std::time::Duration::from_secs(BENCHMARK_MEASUREMENT_SECS));

    let fixture = BenchmarkFixture::new();
    let payer = Keypair::new();

    fixture.context.svm_locker.with_svm_writer(|svm| {
        svm.airdrop(&payer.pubkey(), SIMPLE_TRANSFER_LAMPORTS)
            .unwrap();
    });

    group.bench_function("transaction_deserialization", |b| {
        b.iter(|| {
            let encoded_tx = create_transfer_transaction(
                &fixture.context.svm_locker,
                &payer,
                1,
                TRANSFER_AMOUNT_PER_RECIPIENT,
            );
            let decoded = bs58::decode(&encoded_tx).into_vec().expect("Valid base58");
            black_box(
                bincode::deserialize::<VersionedTransaction>(&decoded).expect("Valid transaction"),
            )
        });
    });

    group.bench_function("transaction_serialization", |b| {
        let payer = Keypair::new();
        let recipient = Pubkey::new_unique();

        fixture.context.svm_locker.with_svm_writer(|svm| {
            svm.airdrop(&payer.pubkey(), SIMPLE_TRANSFER_LAMPORTS)
                .unwrap();
        });

        let latest_blockhash = fixture
            .context
            .svm_locker
            .with_svm_reader(|svm| svm.latest_blockhash());
        let instruction = transfer(&payer.pubkey(), &recipient, TRANSFER_AMOUNT_PER_RECIPIENT);
        let message =
            Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &latest_blockhash);
        let tx =
            VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

        b.iter(|| {
            let serialized = bincode::serialize(&tx).expect("Serialization should succeed");
            black_box(bs58::encode(&serialized).into_string())
        });
    });

    group.bench_function("clone_overhead_string", |b| {
        let sample_tx = create_transfer_transaction(
            &fixture.context.svm_locker,
            &payer,
            1,
            TRANSFER_AMOUNT_PER_RECIPIENT,
        );
        b.iter(|| black_box(sample_tx.clone()));
    });

    group.bench_function("clone_overhead_context", |b| {
        let ctx = Some(fixture.context.clone());
        b.iter(|| black_box(ctx.clone()));
    });

    group.finish();
}

fn benchmark_protocol_transactions(c: &mut Criterion) {
    let mut group = c.benchmark_group("protocol_transactions");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(std::time::Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(std::time::Duration::from_secs(BENCHMARK_MEASUREMENT_SECS));

    let fixture = BenchmarkFixture::new();
    let protocols = ["kamino", "jupiter"];

    for protocol in protocols {
        if !protocol_fixtures_available(protocol) {
            continue;
        }

        let Some(tx_pool) = load_protocol_transactions(protocol) else {
            continue;
        };

        if tx_pool.is_empty() {
            continue;
        }

        group.bench_function(BenchmarkId::new(protocol, tx_pool.len()), |b| {
            let mut idx = 0;
            b.iter(|| {
                let ctx = Some(fixture.context.clone());
                let encoded_tx = &tx_pool[idx % tx_pool.len()];
                idx = (idx + 1) % tx_pool.len();
                let result = fixture.rpc.send_transaction(ctx, encoded_tx.clone(), None);
                black_box(result.expect("Transaction should succeed"))
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_send_transaction,
    benchmark_transaction_components,
    benchmark_protocol_transactions
);
criterion_main!(benches);
