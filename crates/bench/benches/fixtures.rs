use crossbeam_channel::Receiver;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::surfnet::locker::SurfnetSvmLocker;
use surfpool_types::SimnetEvent;

pub const SIMPLE_TRANSFER_LAMPORTS: u64 = 10_000_000_000;
pub const MULTI_TRANSFER_LAMPORTS: u64 = 50_000_000_000;
pub const TRANSFER_AMOUNT_PER_RECIPIENT: u64 = 1_000_000;

pub fn wait_for_ready(simnet_events_rx: &Receiver<SimnetEvent>) {
    let mut ready = false;
    let mut connected = false;
    let mut ready_time: Option<std::time::Instant> = None;
    const MAX_WAIT_FOR_CONNECTED_SECS: u64 = 3;

    loop {
        let timeout = if ready {
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_secs(10)
        };

        match simnet_events_rx.recv_timeout(timeout) {
            Ok(SimnetEvent::Ready) => {
                ready = true;
                ready_time = Some(std::time::Instant::now());
            }
            Ok(SimnetEvent::Connected(_)) => {
                connected = true;
            }
            Ok(SimnetEvent::Aborted(msg)) => panic!("Runloop aborted: {msg}"),
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if !ready {
                    panic!(
                        "Timeout waiting for Ready event after {} seconds",
                        timeout.as_secs()
                    );
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                panic!("Channel disconnected while waiting for runloop");
            }
        }

        if ready && connected {
            return;
        }

        if ready_time
            .map(|ready_instant| ready_instant.elapsed().as_secs() >= MAX_WAIT_FOR_CONNECTED_SECS)
            .unwrap_or(false)
        {
            return;
        }
    }
}

pub fn create_transfer_transaction(
    svm_locker: &SurfnetSvmLocker,
    payer: &Keypair,
    num_instructions: usize,
    transfer_amount: u64,
) -> String {
    let recipients: Vec<Pubkey> = (0..num_instructions)
        .map(|_| Pubkey::new_unique())
        .collect();
    create_transfer_transaction_with_recipients(svm_locker, payer, &recipients, transfer_amount)
}

pub fn create_transfer_transaction_with_recipients(
    svm_locker: &SurfnetSvmLocker,
    payer: &Keypair,
    recipients: &[Pubkey],
    transfer_amount: u64,
) -> String {
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let instructions: Vec<_> = recipients
        .iter()
        .map(|recipient| transfer(&payer.pubkey(), recipient, transfer_amount))
        .collect();
    let message =
        Message::new_with_blockhash(&instructions, Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[payer]).unwrap();
    bs58::encode(bincode::serialize(&tx).unwrap()).into_string()
}

pub fn create_complex_transaction_with_recipients(
    svm_locker: &SurfnetSvmLocker,
    payer: &Keypair,
    recipients: &[Pubkey],
    transfer_amount: u64,
) -> String {
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let mut instructions = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(200_000),
        ComputeBudgetInstruction::set_compute_unit_price(1),
    ];
    instructions.extend(
        recipients
            .iter()
            .map(|recipient| transfer(&payer.pubkey(), recipient, transfer_amount)),
    );
    let message =
        Message::new_with_blockhash(&instructions, Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[payer]).unwrap();
    bs58::encode(bincode::serialize(&tx).unwrap()).into_string()
}

pub fn create_protocol_like_transaction_with_recipients(
    svm_locker: &SurfnetSvmLocker,
    payer: &Keypair,
    intermediate_keypairs: &[Keypair],
    final_recipients: &[Pubkey],
    transfer_amount: u64,
) -> String {
    let intermediate_accounts: Vec<Pubkey> =
        intermediate_keypairs.iter().map(|kp| kp.pubkey()).collect();

    let mut instructions = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(1000),
    ];
    for i in 0..final_recipients.len() {
        instructions.push(transfer(
            &payer.pubkey(),
            &intermediate_accounts[i],
            transfer_amount,
        ));
        instructions.push(transfer(
            &intermediate_accounts[i],
            &final_recipients[i],
            transfer_amount,
        ));
    }

    let mut signers: Vec<&Keypair> = vec![payer];
    signers.extend(intermediate_keypairs.iter());
    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());
    let message =
        Message::new_with_blockhash(&instructions, Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &signers).unwrap();
    bs58::encode(bincode::serialize(&tx).unwrap()).into_string()
}
