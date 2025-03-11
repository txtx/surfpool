use base64::prelude::{Engine, BASE64_STANDARD};
use litesvm::LiteSVM;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_sdk::{
    epoch_info::EpochInfo,
    signature::Signature,
    transaction::{Transaction, TransactionError, TransactionVersion},
};
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedConfirmedTransactionWithStatusMeta,
    EncodedTransaction, EncodedTransactionWithStatusMeta, TransactionConfirmationStatus,
    TransactionStatus, UiCompiledInstruction, UiInnerInstructions, UiInstruction, UiMessage,
    UiRawMessage, UiReturnDataEncoding, UiTransaction, UiTransactionReturnData,
    UiTransactionStatusMeta,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use surfpool_types::TransactionMetadata;

pub struct GlobalState {
    pub svm: LiteSVM,
    pub transactions: HashMap<Signature, EntryStatus>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub epoch_info: EpochInfo,
    pub rpc_client: Arc<RpcClient>,
}

impl GlobalState {
    pub fn new(svm: LiteSVM, epoch_info: &EpochInfo, rpc_client: Arc<RpcClient>) -> Self {
        Self {
            svm,
            transactions: HashMap::new(),
            perf_samples: VecDeque::new(),
            transactions_processed: 0,
            epoch_info: epoch_info.clone(),
            rpc_client,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EntryStatus {
    Received,
    Processed(TransactionWithStatusMeta),
}

impl EntryStatus {
    pub fn expect_processed(&self) -> &TransactionWithStatusMeta {
        match &self {
            EntryStatus::Received => unreachable!(),
            EntryStatus::Processed(status) => status,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionWithStatusMeta(
    pub u64,
    pub Transaction,
    pub TransactionMetadata,
    pub Option<TransactionError>,
);

impl TransactionWithStatusMeta {
    pub fn into_status(&self, current_slot: u64) -> TransactionStatus {
        TransactionStatus {
            slot: self.0,
            confirmations: Some((current_slot - self.0) as usize),
            status: match self.3.clone() {
                Some(err) => Err(err),
                None => Ok(()),
            },
            err: self.3.clone(),
            confirmation_status: Some(TransactionConfirmationStatus::Finalized),
        }
    }
}

impl Into<EncodedConfirmedTransactionWithStatusMeta> for TransactionWithStatusMeta {
    fn into(self) -> EncodedConfirmedTransactionWithStatusMeta {
        let slot = self.0;
        let tx = self.1;
        let meta = self.2;
        let err = self.3;
        EncodedConfirmedTransactionWithStatusMeta {
            slot,
            transaction: EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: tx.signatures.iter().map(|s| s.to_string()).collect(),
                    message: UiMessage::Raw(UiRawMessage {
                        header: tx.message.header,
                        account_keys: tx
                            .message
                            .account_keys
                            .iter()
                            .map(|pk| pk.to_string())
                            .collect(),
                        recent_blockhash: tx.message.recent_blockhash.to_string(),
                        instructions: tx
                            .message
                            .instructions
                            .iter()
                            // TODO: use stack height
                            .map(|ix| UiCompiledInstruction::from(ix, None))
                            .collect(),
                        address_table_lookups: None, // TODO: use lookup table
                    }),
                }),
                meta: Some(UiTransactionStatusMeta {
                    err: err.clone(),
                    status: match err {
                        Some(e) => Err(e),
                        None => Ok(()),
                    },
                    fee: 5000 * (tx.signatures.len() as u64), // TODO: fix calculation
                    pre_balances: vec![],
                    post_balances: vec![],
                    inner_instructions: OptionSerializer::Some(
                        meta.inner_instructions
                            .iter()
                            .enumerate()
                            .map(|(i, ixs)| UiInnerInstructions {
                                index: i as u8,
                                instructions: ixs
                                    .iter()
                                    .map(|ix| {
                                        UiInstruction::Compiled(UiCompiledInstruction {
                                            program_id_index: ix.instruction.program_id_index,
                                            accounts: ix.instruction.accounts.clone(),
                                            data: String::from_utf8(ix.instruction.data.clone())
                                                .unwrap(),
                                            stack_height: Some(ix.stack_height as u32),
                                        })
                                    })
                                    .collect(),
                            })
                            .collect(),
                    ),
                    log_messages: OptionSerializer::Some(meta.logs),
                    pre_token_balances: OptionSerializer::None,
                    post_token_balances: OptionSerializer::None,
                    rewards: OptionSerializer::None,
                    loaded_addresses: OptionSerializer::None,
                    return_data: OptionSerializer::Some(UiTransactionReturnData {
                        program_id: meta.return_data.program_id.to_string(),
                        data: (
                            BASE64_STANDARD.encode(meta.return_data.data),
                            UiReturnDataEncoding::Base64,
                        ),
                    }),
                    compute_units_consumed: OptionSerializer::Some(meta.compute_units_consumed),
                }),
                version: Some(TransactionVersion::Legacy(
                    solana_sdk::transaction::Legacy::Legacy,
                )),
            },
            block_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .ok(),
        }
    }
}
