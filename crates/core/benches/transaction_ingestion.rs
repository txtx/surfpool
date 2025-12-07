use criterion::{black_box, criterion_group, criterion_main, Criterion};
use crossbeam_channel::unbounded;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_types::{SurfpoolConfig, types::{BlockProductionMode, RpcConfig, SimnetConfig}};
use surfpool_core::{
    rpc::{full::{Full, SurfpoolFullRpc}, RunloopContext},
    runloops::start_local_surfnet_runloop,
    surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm},
};

struct BenchmarkFixture {
    context: RunloopContext,
    rpc: SurfpoolFullRpc,
}

impl BenchmarkFixture {
    fn new() -> Self {
        let (surfnet_svm, _simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();
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
                ..SimnetConfig::default()
            }],
            rpc: RpcConfig::default(),
            ..SurfpoolConfig::default()
        };

        let (subgraph_commands_tx, _subgraph_commands_rx) = unbounded();
        
        std::thread::spawn(move || {
            let future = start_local_surfnet_runloop(
                svm_locker,
                config,
                subgraph_commands_tx,
                simnet_commands_tx,
                simnet_commands_rx,
                geyser_events_rx,
            );
            hiro_system_kit::nestable_block_on(future).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(100));

        Self {
            context,
            rpc: SurfpoolFullRpc,
        }
    }

    fn create_transfer_tx(&self, num_instructions: usize, airdrop_amount: u64) -> String {
        let payer = Keypair::new();
        let recipients: Vec<Pubkey> = (0..num_instructions).map(|_| Pubkey::new_unique()).collect();
        
        self.context.svm_locker
            .with_svm_writer(|svm| {
                svm.airdrop(&payer.pubkey(), airdrop_amount).unwrap();
            });

        let latest_blockhash = self.context.svm_locker
            .with_svm_reader(|svm| svm.latest_blockhash());
        
        let instructions: Vec<_> = recipients
            .iter()
            .map(|recipient| transfer(&payer.pubkey(), recipient, 1_000_000))
            .collect();
        
        let message = Message::new_with_blockhash(
            &instructions,
            Some(&payer.pubkey()),
            &latest_blockhash,
        );
        let tx = VersionedTransaction::try_new(
            VersionedMessage::Legacy(message),
            &[&payer],
        ).unwrap();

        bs58::encode(bincode::serialize(&tx).unwrap()).into_string()
    }

    fn create_simple_transfer(&self) -> String {
        self.create_transfer_tx(1, 10_000_000_000)
    }

    fn create_multi_instruction_transfer(&self) -> String {
        self.create_transfer_tx(5, 50_000_000_000)
    }
}

fn benchmark_send_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_ingestion");
    group.sample_size(100);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    let fixture = BenchmarkFixture::new();
    let context = fixture.context.clone();

    group.bench_function("simple_transfer", |b| {
        b.iter(|| {
            let encoded_tx = fixture.create_simple_transfer();
            let result = fixture.rpc.send_transaction(
                Some(context.clone()),
                black_box(encoded_tx),
                None,
            );
            assert!(result.is_ok());
        });
    });

    group.bench_function("multi_instruction_transfer", |b| {
        b.iter(|| {
            let encoded_tx = fixture.create_multi_instruction_transfer();
            let result = fixture.rpc.send_transaction(
                Some(context.clone()),
                black_box(encoded_tx),
                None,
            );
            assert!(result.is_ok());
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_send_transaction);
criterion_main!(benches);

