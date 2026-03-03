mod fixtures;

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use fixtures::{
    MULTI_TRANSFER_LAMPORTS, SIMPLE_TRANSFER_LAMPORTS, TRANSFER_AMOUNT_PER_RECIPIENT,
    create_complex_transaction_with_recipients, create_protocol_like_transaction_with_recipients,
    create_transfer_transaction, create_transfer_transaction_with_recipients,
};
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::surfnet::{locker::SurfnetSvmLocker, svm::SurfnetSvm};

const BENCHMARK_SAMPLE_SIZE: usize = 10;
const BENCHMARK_WARM_UP_SECS: u64 = 1;
const BENCHMARK_MEASUREMENT_MILLIS: u64 = 50;

static SEND_TRANSACTION_FIXTURE: OnceLock<Arc<BenchmarkFixture>> = OnceLock::new();
static COMPONENTS_FIXTURE: OnceLock<Arc<BenchmarkFixture>> = OnceLock::new();

fn get_send_transaction_fixture() -> Arc<BenchmarkFixture> {
    SEND_TRANSACTION_FIXTURE
        .get_or_init(|| Arc::new(BenchmarkFixture::new()))
        .clone()
}

fn get_components_fixture() -> Arc<BenchmarkFixture> {
    COMPONENTS_FIXTURE
        .get_or_init(|| Arc::new(BenchmarkFixture::new()))
        .clone()
}

struct BenchmarkFixture {
    svm_locker: SurfnetSvmLocker,
}

impl BenchmarkFixture {
    fn new() -> Self {
        let (surfnet_svm, _simnet_events_rx, _geyser_events_rx) = SurfnetSvm::default();
        let svm_locker = SurfnetSvmLocker::new(surfnet_svm);

        Self { svm_locker }
    }
}

fn benchmark_send_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_ingestion");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(Duration::from_millis(BENCHMARK_MEASUREMENT_MILLIS));

    let fixture = get_send_transaction_fixture();

    // Setup transaction types: (name, num_recipients, airdrop_amount, is_complex, is_protocol_like)
    let tx_types = [
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

    for (tx_type_name, num_recipients, airdrop_amount, is_complex, is_protocol_like) in tx_types {
        group.bench_function(BenchmarkId::new(tx_type_name, num_recipients), |b| {
            b.iter_with_setup(
                || {
                    let payer = Keypair::new();
                    let recipients: Vec<Pubkey> =
                        (0..num_recipients).map(|_| Pubkey::new_unique()).collect();

                    let encoded_tx = if is_protocol_like {
                        let intermediate_keypairs: Vec<Keypair> =
                            (0..num_recipients).map(|_| Keypair::new()).collect();

                        fixture.svm_locker.with_svm_writer(|svm| {
                            let payer_account = Account {
                                lamports: airdrop_amount,
                                data: vec![],
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                            };
                            svm.set_account(&payer.pubkey(), payer_account).unwrap();
                            for kp in &intermediate_keypairs {
                                let kp_account = Account {
                                    lamports: TRANSFER_AMOUNT_PER_RECIPIENT * 2,
                                    data: vec![],
                                    owner: system_program::id(),
                                    executable: false,
                                    rent_epoch: 0,
                                };
                                svm.set_account(&kp.pubkey(), kp_account).unwrap();
                            }
                        });

                        create_protocol_like_transaction_with_recipients(
                            &fixture.svm_locker,
                            &payer,
                            &intermediate_keypairs,
                            &recipients,
                            TRANSFER_AMOUNT_PER_RECIPIENT,
                        )
                    } else if is_complex {
                        fixture.svm_locker.with_svm_writer(|svm| {
                            let payer_account = Account {
                                lamports: airdrop_amount,
                                data: vec![],
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                            };
                            svm.set_account(&payer.pubkey(), payer_account).unwrap();
                        });

                        create_complex_transaction_with_recipients(
                            &fixture.svm_locker,
                            &payer,
                            &recipients,
                            TRANSFER_AMOUNT_PER_RECIPIENT,
                        )
                    } else {
                        fixture.svm_locker.with_svm_writer(|svm| {
                            let payer_account = Account {
                                lamports: airdrop_amount,
                                data: vec![],
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                            };
                            svm.set_account(&payer.pubkey(), payer_account).unwrap();
                        });

                        create_transfer_transaction_with_recipients(
                            &fixture.svm_locker,
                            &payer,
                            &recipients,
                            TRANSFER_AMOUNT_PER_RECIPIENT,
                        )
                    };

                    (encoded_tx, payer)
                },
                |(encoded_tx, _payer)| {
                    let decoded = bs58::decode(&encoded_tx).into_vec().expect("Valid base58");
                    let tx: VersionedTransaction =
                        bincode::deserialize(&decoded).expect("Valid transaction");

                    let result = fixture
                        .svm_locker
                        .with_svm_writer(|svm| svm.send_transaction(tx, false, false));
                    black_box(result.unwrap())
                },
            );
        });
    }

    group.finish();
}

fn benchmark_transaction_components(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_components");
    group.sample_size(BENCHMARK_SAMPLE_SIZE);
    group.warm_up_time(std::time::Duration::from_secs(BENCHMARK_WARM_UP_SECS));
    group.measurement_time(std::time::Duration::from_millis(
        BENCHMARK_MEASUREMENT_MILLIS,
    ));

    let fixture = get_components_fixture();
    let payer = Keypair::new();

    fixture.svm_locker.with_svm_writer(|svm| {
        let payer_account = Account {
            lamports: SIMPLE_TRANSFER_LAMPORTS,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        };
        svm.set_account(&payer.pubkey(), payer_account).unwrap();
    });

    group.bench_function("transaction_deserialization", |b| {
        b.iter(|| {
            let encoded_tx = create_transfer_transaction(
                &fixture.svm_locker,
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

        fixture.svm_locker.with_svm_writer(|svm| {
            let payer_account = Account {
                lamports: SIMPLE_TRANSFER_LAMPORTS,
                data: vec![],
                owner: system_program::id(),
                executable: false,
                rent_epoch: 0,
            };
            svm.set_account(&payer.pubkey(), payer_account).unwrap();
        });

        let latest_blockhash = fixture
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
            &fixture.svm_locker,
            &payer,
            1,
            TRANSFER_AMOUNT_PER_RECIPIENT,
        );
        b.iter(|| black_box(sample_tx.clone()));
    });

    group.bench_function("clone_overhead_svm_locker", |b| {
        let locker = fixture.svm_locker.clone();
        b.iter(|| black_box(locker.clone()));
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_send_transaction,
    benchmark_transaction_components
);
criterion_main!(benches);
