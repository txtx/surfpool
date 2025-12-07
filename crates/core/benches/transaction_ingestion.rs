mod fixtures;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use crossbeam_channel::{Receiver, unbounded};
use fixtures::{
    MULTI_TRANSFER_LAMPORTS, SIMPLE_TRANSFER_LAMPORTS, TRANSFER_AMOUNT_PER_RECIPIENT,
    create_transfer_transaction, wait_for_ready,
};
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use std::thread::JoinHandle;
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

const BENCHMARK_SAMPLE_SIZE: usize = 100;
const BENCHMARK_WARM_UP_SECS: u64 = 2;
const BENCHMARK_MEASUREMENT_SECS: u64 = 10;
const TRANSACTION_POOL_SIZE: usize = 1000;

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
                offline_mode: true, // Benchmarks don't need network connection
                remote_rpc_url: None,
                ..SimnetConfig::default()
            }],
            rpc: RpcConfig::default(),
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

    fn create_transaction_pool(
        &self,
        pool_size: usize,
        num_instructions: usize,
        airdrop_amount: u64,
    ) -> Vec<String> {
        (0..pool_size)
            .map(|_| {
                create_transfer_transaction(
                    &self.context.svm_locker,
                    num_instructions,
                    airdrop_amount,
                    TRANSFER_AMOUNT_PER_RECIPIENT,
                )
            })
            .collect()
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

    // Generate transaction pools at runtime
    let simple_tx_pool = fixture.create_transaction_pool(TRANSACTION_POOL_SIZE, 1, SIMPLE_TRANSFER_LAMPORTS);
    let multi_tx_pool = fixture.create_transaction_pool(TRANSACTION_POOL_SIZE, 5, MULTI_TRANSFER_LAMPORTS);
    let large_tx_pool = fixture.create_transaction_pool(TRANSACTION_POOL_SIZE, 10, MULTI_TRANSFER_LAMPORTS * 2);

    group.bench_function(BenchmarkId::new("simple_transfer", 1), |b| {
        let mut idx = 0;
        b.iter(|| {
            let ctx = Some(fixture.context.clone());
            let encoded_tx = &simple_tx_pool[idx % simple_tx_pool.len()];
            idx = (idx + 1) % simple_tx_pool.len();
            let result = fixture.rpc.send_transaction(ctx, encoded_tx.clone(), None);
            black_box(result.expect("Transaction should succeed"))
        });
    });

    group.bench_function(BenchmarkId::new("multi_instruction_transfer", 5), |b| {
        let mut idx = 0;
        b.iter(|| {
            let ctx = Some(fixture.context.clone());
            let encoded_tx = &multi_tx_pool[idx % multi_tx_pool.len()];
            idx = (idx + 1) % multi_tx_pool.len();
            let result = fixture.rpc.send_transaction(ctx, encoded_tx.clone(), None);
            black_box(result.expect("Transaction should succeed"))
        });
    });

    group.bench_function(BenchmarkId::new("large_transfer", 10), |b| {
        let mut idx = 0;
        b.iter(|| {
            let ctx = Some(fixture.context.clone());
            let encoded_tx = &large_tx_pool[idx % large_tx_pool.len()];
            idx = (idx + 1) % large_tx_pool.len();
            let result = fixture.rpc.send_transaction(ctx, encoded_tx.clone(), None);
            black_box(result.expect("Transaction should succeed"))
        });
    });

    group.finish();
}

fn benchmark_transaction_components(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_components");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(std::time::Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(std::time::Duration::from_secs(BENCHMARK_MEASUREMENT_SECS));

    let fixture = BenchmarkFixture::new();
    let serialized_pool =
        fixture.create_transaction_pool(TRANSACTION_POOL_SIZE, 1, SIMPLE_TRANSFER_LAMPORTS);

    group.bench_function("transaction_deserialization", |b| {
        let mut idx = 0;
        b.iter(|| {
            let encoded_tx = &serialized_pool[idx % serialized_pool.len()];
            idx = (idx + 1) % serialized_pool.len();
            let decoded = bs58::decode(encoded_tx).into_vec().expect("Valid base58");
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

    // Benchmark overhead from cloning to understand measurement contamination
    group.bench_function("clone_overhead_string", |b| {
        let sample_tx = &serialized_pool[0];
        b.iter(|| black_box(sample_tx.clone()));
    });

    group.bench_function("clone_overhead_context", |b| {
        let ctx = Some(fixture.context.clone());
        b.iter(|| black_box(ctx.clone()));
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_send_transaction,
    benchmark_transaction_components
);
criterion_main!(benches);
