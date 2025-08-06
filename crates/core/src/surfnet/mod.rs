use std::collections::HashMap;

use crossbeam_channel::Sender;
use jsonrpc_core::Result as RpcError;
use locker::SurfnetSvmLocker;
use solana_account::Account;
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use solana_client::{rpc_config::RpcTransactionLogsFilter, rpc_response::RpcLogsResponse};
use solana_clock::Slot;
use solana_commitment_config::CommitmentLevel;
use solana_epoch_info::EpochInfo;
use solana_pubkey::Pubkey;
use solana_sdk::transaction::SanitizedTransaction;
use solana_signature::Signature;
use solana_transaction_error::TransactionError;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, TransactionStatus};
use svm::SurfnetSvm;

use crate::{
    error::{SurfpoolError, SurfpoolResult},
    types::{GeyserAccountUpdate, TransactionWithStatusMeta},
};

pub mod locker;
pub mod remote;
pub mod svm;

pub const SURFPOOL_IDENTITY_PUBKEY: Pubkey =
    Pubkey::from_str_const("SUrFPooLSUrFPooLSUrFPooLSUrFPooLSUrFPooLSUr");
pub const FINALIZATION_SLOT_THRESHOLD: u64 = 31;
pub const SLOTS_PER_EPOCH: u64 = 432000;

pub type AccountFactory = Box<dyn Fn(SurfnetSvmLocker) -> GetAccountResult + Send + Sync>;

pub enum GeyserEvent {
    NotifyTransaction(TransactionWithStatusMeta, Option<SanitizedTransaction>),
    UpdateAccount(GeyserAccountUpdate),
    // todo: add more events
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct BlockIdentifier {
    pub index: u64,
    pub hash: String,
}

impl BlockIdentifier {
    pub fn zero() -> Self {
        Self::new(0, "")
    }

    pub fn new(index: u64, hash: &str) -> Self {
        Self {
            index,
            hash: hash.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub hash: String,
    pub previous_blockhash: String,
    pub parent_slot: Slot,
    pub block_time: i64,
    pub block_height: u64,
    pub signatures: Vec<Signature>,
}

#[derive(PartialEq, Eq, Clone)]
pub enum SurfnetDataConnection {
    Offline,
    Connected(String, EpochInfo),
}

pub type SignatureSubscriptionData = (
    SignatureSubscriptionType,
    Sender<(Slot, Option<TransactionError>)>,
);

pub type AccountSubscriptionData =
    HashMap<Pubkey, Vec<(Option<UiAccountEncoding>, Sender<UiAccount>)>>;

pub type LogsSubscriptionData = (
    CommitmentLevel,
    RpcTransactionLogsFilter,
    Sender<(Slot, RpcLogsResponse)>,
);

#[derive(Debug, Clone, PartialEq)]
pub enum SignatureSubscriptionType {
    Received,
    Commitment(CommitmentLevel),
}

type DoUpdateSvm = bool;

#[derive(Clone, Debug)]
/// Represents the result of a get_account operation.
pub enum GetAccountResult {
    /// Represents that the account was not found.
    None(Pubkey),
    /// Represents that the account was found.
    /// The `DoUpdateSvm` flag indicates whether the SVM should be updated after this account is found.
    /// This is useful for cases where the account was fetched from a remote source and needs to be
    /// updated in the SVM to reflect the latest state. However, when the account is found locally,
    /// it likely does not need to be updated in the SVM.
    FoundAccount(Pubkey, Account, DoUpdateSvm),
    FoundProgramAccount((Pubkey, Account), (Pubkey, Option<Account>)),
    FoundTokenAccount((Pubkey, Account), (Pubkey, Option<Account>)),
}

impl GetAccountResult {
    pub fn expected_data(&self) -> &Vec<u8> {
        match &self {
            Self::None(_) => unreachable!(),
            Self::FoundAccount(_, account, _)
            | Self::FoundProgramAccount((_, account), _)
            | Self::FoundTokenAccount((_, account), _) => &account.data,
        }
    }

    pub fn apply_update<T>(&mut self, update: T) -> RpcError<()>
    where
        T: Fn(&mut Account) -> RpcError<()>,
    {
        match self {
            Self::None(_) => unreachable!(),
            Self::FoundAccount(_, account, do_update_account) => {
                update(account)?;
                *do_update_account = true;
            }
            Self::FoundProgramAccount((_, account), _) => {
                update(account)?;
            }
            Self::FoundTokenAccount((_, account), _) => {
                update(account)?;
            }
        }
        Ok(())
    }

    pub fn map_account(self) -> SurfpoolResult<Account> {
        match self {
            Self::None(pubkey) => Err(SurfpoolError::account_not_found(pubkey)),
            Self::FoundAccount(_, account, _)
            | Self::FoundProgramAccount((_, account), _)
            | Self::FoundTokenAccount((_, account), _) => Ok(account),
        }
    }

    pub fn map_account_with_token_data(
        self,
    ) -> Option<((Pubkey, Account), Option<(Pubkey, Option<Account>)>)> {
        match self {
            Self::None(_) => None,
            Self::FoundAccount(pubkey, account, _) => Some(((pubkey, account), None)),
            Self::FoundProgramAccount((pubkey, account), _) => Some(((pubkey, account), None)),
            Self::FoundTokenAccount((pubkey, account), token_data) => {
                Some(((pubkey, account), Some(token_data)))
            }
        }
    }

    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None(_))
    }

    pub const fn requires_update(&self) -> bool {
        match self {
            Self::None(_) => false,
            Self::FoundAccount(_, _, do_update) => *do_update,
            Self::FoundProgramAccount(_, _) => true,
            Self::FoundTokenAccount(_, _) => true,
        }
    }
}

impl From<GetAccountResult> for Result<Account, SurfpoolError> {
    fn from(value: GetAccountResult) -> Self {
        value.map_account()
    }
}

impl SignatureSubscriptionType {
    pub const fn received() -> Self {
        SignatureSubscriptionType::Received
    }

    pub const fn processed() -> Self {
        SignatureSubscriptionType::Commitment(CommitmentLevel::Processed)
    }

    pub const fn confirmed() -> Self {
        SignatureSubscriptionType::Commitment(CommitmentLevel::Confirmed)
    }

    pub const fn finalized() -> Self {
        SignatureSubscriptionType::Commitment(CommitmentLevel::Finalized)
    }
}

pub enum GetTransactionResult {
    None(Signature),
    FoundTransaction(
        Signature,
        EncodedConfirmedTransactionWithStatusMeta,
        TransactionStatus,
    ),
}

impl GetTransactionResult {
    pub fn found_transaction(
        signature: Signature,
        tx: EncodedConfirmedTransactionWithStatusMeta,
        latest_absolute_slot: u64,
    ) -> Self {
        let is_finalized = latest_absolute_slot >= tx.slot + FINALIZATION_SLOT_THRESHOLD;
        let (confirmation_status, confirmations) = if is_finalized {
            (
                Some(solana_transaction_status::TransactionConfirmationStatus::Finalized),
                None,
            )
        } else {
            (
                Some(solana_transaction_status::TransactionConfirmationStatus::Confirmed),
                Some((latest_absolute_slot - tx.slot) as usize),
            )
        };
        let status = TransactionStatus {
            slot: tx.slot,
            confirmations,
            status: tx.transaction.clone().meta.map_or(Ok(()), |m| m.status),
            err: tx.transaction.clone().meta.and_then(|m| m.err),
            confirmation_status,
        };

        Self::FoundTransaction(signature, tx, status)
    }

    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None(_))
    }

    pub fn map_found_transaction(&self) -> SurfpoolResult<TransactionStatus> {
        match self {
            Self::None(sig) => Err(SurfpoolError::transaction_not_found(sig)),
            Self::FoundTransaction(_, _, status) => Ok(status.clone()),
        }
    }

    pub fn map_some_transaction_status(&self) -> Option<TransactionStatus> {
        match self {
            Self::None(_) => None,
            Self::FoundTransaction(_, _, status) => Some(status.clone()),
        }
    }
}
