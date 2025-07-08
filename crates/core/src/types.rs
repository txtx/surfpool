use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::{BASE64_STANDARD, Engine};
use solana_client::rpc_client::SerializableTransaction;
use solana_message::VersionedMessage;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction,
    EncodedTransactionWithStatusMeta, TransactionConfirmationStatus, TransactionStatus,
    UiAddressTableLookup, UiCompiledInstruction, UiMessage, UiRawMessage, UiReturnDataEncoding,
    UiTransaction, UiTransactionReturnData, UiTransactionStatusMeta,
    option_serializer::OptionSerializer,
};
use surfpool_types::TransactionMetadata;

use crate::rpc::utils::transform_tx_metadata_to_ui_accounts;

#[derive(Debug, Clone)]
pub enum SurfnetTransactionStatus {
    Received,
    Processed(Box<TransactionWithStatusMeta>),
}

impl SurfnetTransactionStatus {
    pub fn expect_processed(&self) -> &TransactionWithStatusMeta {
        match &self {
            SurfnetTransactionStatus::Received => unreachable!(),
            SurfnetTransactionStatus::Processed(status) => status,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionWithStatusMeta(
    pub u64,
    pub VersionedTransaction,
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

impl From<TransactionWithStatusMeta> for EncodedConfirmedTransactionWithStatusMeta {
    fn from(val: TransactionWithStatusMeta) -> Self {
        let TransactionWithStatusMeta(slot, tx, meta, err) = val;

        let (header, account_keys, instructions) = match &tx.message {
            VersionedMessage::Legacy(message) => (
                message.header,
                message.account_keys.iter().map(|k| k.to_string()).collect(),
                message
                    .instructions
                    .iter()
                    // TODO: use stack height
                    .map(|ix| UiCompiledInstruction::from(ix, None))
                    .collect(),
            ),
            VersionedMessage::V0(message) => (
                message.header,
                message.account_keys.iter().map(|k| k.to_string()).collect(),
                message
                    .instructions
                    .iter()
                    // TODO: use stack height
                    .map(|ix| UiCompiledInstruction::from(ix, None))
                    .collect(),
            ),
        };

        EncodedConfirmedTransactionWithStatusMeta {
            slot,
            transaction: EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: tx.signatures.iter().map(|s| s.to_string()).collect(),
                    message: UiMessage::Raw(UiRawMessage {
                        header,
                        account_keys,
                        recent_blockhash: tx.get_recent_blockhash().to_string(),
                        instructions,
                        address_table_lookups: match tx.message {
                            VersionedMessage::Legacy(_) => None,
                            VersionedMessage::V0(ref msg) => Some(
                                msg.address_table_lookups
                                    .iter()
                                    .map(UiAddressTableLookup::from)
                                    .collect::<Vec<UiAddressTableLookup>>(),
                            ),
                        },
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
                        transform_tx_metadata_to_ui_accounts(
                            &surfpool_tx_metadata_to_litesvm_tx_metadata(&meta),
                        ),
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
                version: Some(tx.version()),
            },
            block_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .ok(),
        }
    }
}

pub fn surfpool_tx_metadata_to_litesvm_tx_metadata(
    metadata: &surfpool_types::TransactionMetadata,
) -> litesvm::types::TransactionMetadata {
    litesvm::types::TransactionMetadata {
        compute_units_consumed: metadata.compute_units_consumed,
        logs: metadata.logs.clone(),
        return_data: metadata.return_data.clone(),
        inner_instructions: metadata.inner_instructions.clone(),
        signature: metadata.signature,
    }
}
