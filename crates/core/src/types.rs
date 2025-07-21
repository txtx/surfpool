use base64::{Engine, prelude::BASE64_STANDARD};
use litesvm::types::TransactionMetadata;
use solana_account::Account;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_message::{
    AccountKeys, VersionedMessage,
    v0::{LoadedAddresses, LoadedMessage},
};
use solana_pubkey::Pubkey;
use solana_sdk::{
    program_option::COption,
    program_pack::Pack,
    reserved_account_keys::ReservedAccountKeys,
    transaction::{SanitizedTransaction, TransactionVersion, VersionedTransaction},
};
use solana_transaction_status::{
    Encodable, EncodableWithMeta, EncodeError, EncodedTransaction,
    EncodedTransactionWithStatusMeta, InnerInstruction, InnerInstructions,
    TransactionBinaryEncoding, TransactionConfirmationStatus, TransactionStatus,
    TransactionStatusMeta, TransactionTokenBalance, UiAccountsList, UiLoadedAddresses,
    UiTransaction, UiTransactionEncoding, UiTransactionStatusMeta,
    option_serializer::OptionSerializer,
    parse_accounts::{parse_legacy_message_accounts, parse_v0_message_accounts},
    parse_ui_inner_instructions,
};
use spl_token_2022::extension::StateWithExtensions;

use crate::{
    error::{SurfpoolError, SurfpoolResult},
    surfnet::locker::{format_ui_amount, format_ui_amount_string},
};

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

#[derive(Debug, Clone, Default)]
pub struct TransactionWithStatusMeta {
    pub slot: u64,
    pub transaction: VersionedTransaction,
    pub meta: TransactionStatusMeta,
}

impl TransactionWithStatusMeta {
    pub fn into_status(&self, current_slot: u64) -> TransactionStatus {
        TransactionStatus {
            slot: self.slot,
            confirmations: Some((current_slot - self.slot) as usize),
            status: self.meta.status.clone(),
            err: match &self.meta.status {
                Ok(_) => None,
                Err(e) => Some(e.clone()),
            },
            confirmation_status: Some(TransactionConfirmationStatus::Finalized),
        }
    }
    pub fn new(
        slot: u64,
        transaction: VersionedTransaction,
        transaction_meta: TransactionMetadata,
        accounts_before: Vec<Option<Account>>,
        accounts_after: Vec<Option<Account>>,
        pre_token_accounts_with_indexes: Vec<(usize, TokenAccount)>,
        post_token_accounts_with_indexes: Vec<(usize, TokenAccount)>,
        token_mints: Vec<MintAccount>,
        token_program_ids: Vec<Pubkey>,
        loaded_addresses: LoadedAddresses,
    ) -> Self {
        let signatures_len = transaction.signatures.len();
        Self {
            slot,
            transaction,
            meta: TransactionStatusMeta {
                status: Ok(()),
                fee: 5000 * signatures_len as u64,
                pre_balances: accounts_before
                    .iter()
                    .map(|a| a.clone().map(|a| a.lamports).unwrap_or(0))
                    .collect(),
                post_balances: accounts_after
                    .iter()
                    .map(|a| a.clone().map(|a| a.lamports).unwrap_or(0))
                    .collect(),
                inner_instructions: Some(
                    transaction_meta
                        .inner_instructions
                        .iter()
                        .enumerate()
                        .map(|(i, ixs)| InnerInstructions {
                            index: i as u8,
                            instructions: ixs
                                .iter()
                                .map(|ix| InnerInstruction {
                                    instruction: ix.instruction.clone(),
                                    stack_height: Some(ix.stack_height as u32),
                                })
                                .collect(),
                        })
                        .collect(),
                ),
                log_messages: Some(transaction_meta.logs),
                pre_token_balances: Some(
                    pre_token_accounts_with_indexes
                        .iter()
                        .zip(token_mints.clone())
                        .zip(token_program_ids.clone())
                        .map(|(((i, a), mint), token_program)| TransactionTokenBalance {
                            account_index: *i as u8,
                            mint: a.mint().to_string(),
                            ui_token_amount: UiTokenAmount {
                                ui_amount: Some(format_ui_amount(a.amount(), mint.decimals())),
                                decimals: mint.decimals(),
                                amount: a.amount().to_string(),
                                ui_amount_string: format_ui_amount_string(
                                    a.amount(),
                                    mint.decimals(),
                                ),
                            },
                            owner: a.owner().to_string(),
                            program_id: token_program.to_string(),
                        })
                        .collect(),
                ),
                post_token_balances: Some(
                    post_token_accounts_with_indexes
                        .iter()
                        .zip(token_mints)
                        .zip(token_program_ids)
                        .map(|(((i, a), mint), token_program)| TransactionTokenBalance {
                            account_index: *i as u8,
                            mint: a.mint().to_string(),
                            ui_token_amount: UiTokenAmount {
                                ui_amount: Some(format_ui_amount(a.amount(), mint.decimals())),
                                decimals: mint.decimals(),
                                amount: a.amount().to_string(),
                                ui_amount_string: format_ui_amount_string(
                                    a.amount(),
                                    mint.decimals(),
                                ),
                            },
                            owner: a.owner().to_string(),
                            program_id: token_program.to_string(),
                        })
                        .collect(),
                ),
                rewards: Some(vec![]),
                loaded_addresses,
                return_data: Some(transaction_meta.return_data),
                compute_units_consumed: Some(transaction_meta.compute_units_consumed),
            },
        }
    }

    pub fn encode(
        &self,
        encoding: UiTransactionEncoding,
        max_supported_transaction_version: Option<u8>,
        show_rewards: bool,
    ) -> Result<EncodedTransactionWithStatusMeta, EncodeError> {
        let version = self.validate_version(max_supported_transaction_version)?;
        Ok(EncodedTransactionWithStatusMeta {
            transaction: match encoding {
                UiTransactionEncoding::Binary => EncodedTransaction::LegacyBinary(
                    bs58::encode(bincode::serialize(&self.transaction).unwrap()).into_string(),
                ),
                UiTransactionEncoding::Base58 => EncodedTransaction::Binary(
                    bs58::encode(bincode::serialize(&self.transaction).unwrap()).into_string(),
                    TransactionBinaryEncoding::Base58,
                ),
                UiTransactionEncoding::Base64 => EncodedTransaction::Binary(
                    BASE64_STANDARD.encode(bincode::serialize(&self.transaction).unwrap()),
                    TransactionBinaryEncoding::Base64,
                ),
                UiTransactionEncoding::Json => EncodedTransaction::Json(UiTransaction {
                    signatures: self
                        .transaction
                        .signatures
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    message: match &self.transaction.message {
                        VersionedMessage::Legacy(message) => {
                            message.encode(UiTransactionEncoding::Json)
                        }
                        VersionedMessage::V0(message) => message.json_encode(),
                    },
                }),
                UiTransactionEncoding::JsonParsed => EncodedTransaction::Json(UiTransaction {
                    signatures: self
                        .transaction
                        .signatures
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    message: match &self.transaction.message {
                        VersionedMessage::Legacy(message) => {
                            message.encode(UiTransactionEncoding::JsonParsed)
                        }
                        VersionedMessage::V0(message) => {
                            message.encode_with_meta(UiTransactionEncoding::JsonParsed, &self.meta)
                        }
                    },
                }),
            },
            meta: Some(match encoding {
                UiTransactionEncoding::JsonParsed => {
                    parse_ui_transaction_status_meta_with_account_keys(
                        self.meta.clone(),
                        self.transaction.message.static_account_keys(),
                        show_rewards,
                    )
                }
                _ => {
                    let mut meta = parse_ui_transaction_status_meta(self.meta.clone());
                    if !show_rewards {
                        meta.rewards = OptionSerializer::None;
                    }
                    meta
                }
            }),
            version,
        })
    }

    pub fn to_json_accounts(
        &self,
        max_supported_transaction_version: Option<u8>,
        show_rewards: bool,
    ) -> Result<EncodedTransactionWithStatusMeta, EncodeError> {
        let version = self.validate_version(max_supported_transaction_version)?;
        let reserved_account_keys = ReservedAccountKeys::new_all_activated();

        let account_keys = match &self.transaction.message {
            VersionedMessage::Legacy(message) => parse_legacy_message_accounts(message),
            VersionedMessage::V0(message) => {
                let loaded_message = LoadedMessage::new_borrowed(
                    message,
                    &self.meta.loaded_addresses,
                    &reserved_account_keys.active,
                );
                parse_v0_message_accounts(&loaded_message)
            }
        };

        Ok(EncodedTransactionWithStatusMeta {
            transaction: EncodedTransaction::Accounts(UiAccountsList {
                signatures: self
                    .transaction
                    .signatures
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                account_keys,
            }),
            meta: Some(build_simple_ui_transaction_status_meta(
                self.meta.clone(),
                show_rewards,
            )),
            version,
        })
    }

    fn validate_version(
        &self,
        max_supported_transaction_version: Option<u8>,
    ) -> Result<Option<TransactionVersion>, EncodeError> {
        match (
            max_supported_transaction_version,
            self.transaction.version(),
        ) {
            // Set to none because old clients can't handle this field
            (None, TransactionVersion::LEGACY) => Ok(None),
            (None, TransactionVersion::Number(version)) => {
                Err(EncodeError::UnsupportedTransactionVersion(version))
            }
            (Some(_), TransactionVersion::LEGACY) => Ok(Some(TransactionVersion::LEGACY)),
            (Some(max_version), TransactionVersion::Number(version)) => {
                if version <= max_version {
                    Ok(Some(TransactionVersion::Number(version)))
                } else {
                    Err(EncodeError::UnsupportedTransactionVersion(version))
                }
            }
        }
    }
}

fn parse_ui_transaction_status_meta_with_account_keys(
    meta: TransactionStatusMeta,
    static_keys: &[Pubkey],
    show_rewards: bool,
) -> UiTransactionStatusMeta {
    let account_keys = AccountKeys::new(static_keys, Some(&meta.loaded_addresses));
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: meta
            .inner_instructions
            .map(|ixs| {
                ixs.into_iter()
                    .map(|ix| parse_ui_inner_instructions(ix, &account_keys))
                    .collect()
            })
            .into(),
        log_messages: meta.log_messages.into(),
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: if show_rewards { meta.rewards } else { None }.into(),
        loaded_addresses: OptionSerializer::Skip,
        return_data: OptionSerializer::or_skip(
            meta.return_data.map(|return_data| return_data.into()),
        ),
        compute_units_consumed: OptionSerializer::or_skip(meta.compute_units_consumed),
    }
}

fn parse_ui_transaction_status_meta(meta: TransactionStatusMeta) -> UiTransactionStatusMeta {
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: meta
            .inner_instructions
            .map(|ixs| ixs.into_iter().map(Into::into).collect())
            .into(),
        log_messages: meta.log_messages.into(),
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: meta.rewards.into(),
        loaded_addresses: Some(UiLoadedAddresses::from(&meta.loaded_addresses)).into(),
        return_data: OptionSerializer::or_skip(
            meta.return_data.map(|return_data| return_data.into()),
        ),
        compute_units_consumed: OptionSerializer::or_skip(meta.compute_units_consumed),
    }
}

fn build_simple_ui_transaction_status_meta(
    meta: TransactionStatusMeta,
    show_rewards: bool,
) -> UiTransactionStatusMeta {
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: OptionSerializer::Skip,
        log_messages: OptionSerializer::Skip,
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: if show_rewards {
            meta.rewards.into()
        } else {
            OptionSerializer::Skip
        },
        loaded_addresses: OptionSerializer::Skip,
        return_data: OptionSerializer::Skip,
        compute_units_consumed: OptionSerializer::Skip,
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

#[derive(Debug, Clone)]
pub enum RemoteRpcResult<T> {
    Ok(T),
    MethodNotSupported,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum TokenAccount {
    SplToken2022(spl_token_2022::state::Account),
    SplToken(spl_token::state::Account),
}

impl TokenAccount {
    pub fn unpack(bytes: &[u8]) -> SurfpoolResult<Self> {
        if let Ok(account) = spl_token_2022::state::Account::unpack(bytes) {
            Ok(Self::SplToken2022(account))
        } else if let Ok(account) = spl_token::state::Account::unpack(bytes) {
            Ok(Self::SplToken(account))
        } else if let Ok(account) =
            StateWithExtensions::<spl_token_2022::state::Account>::unpack(bytes)
        {
            Ok(Self::SplToken2022(account.base))
        } else {
            Err(SurfpoolError::unpack_token_account())
        }
    }

    pub fn new(token_program_id: &Pubkey, owner: Pubkey, mint: Pubkey) -> Self {
        if token_program_id == &spl_token_2022::id() {
            Self::SplToken2022(spl_token_2022::state::Account {
                mint,
                owner,
                state: spl_token_2022::state::AccountState::Initialized,
                ..Default::default()
            })
        } else {
            Self::SplToken(spl_token::state::Account {
                mint,
                owner,
                state: spl_token::state::AccountState::Initialized,
                ..Default::default()
            })
        }
    }

    pub fn pack_into_vec(&self) -> Vec<u8> {
        match self {
            Self::SplToken2022(account) => {
                let mut dst = [0u8; spl_token_2022::state::Account::LEN];
                account.pack_into_slice(&mut dst);
                dst.to_vec()
            }
            Self::SplToken(account) => {
                let mut dst = [0u8; spl_token::state::Account::LEN];
                account.pack_into_slice(&mut dst);
                dst.to_vec()
            }
        }
    }

    pub fn owner(&self) -> Pubkey {
        match self {
            Self::SplToken2022(account) => account.owner,
            Self::SplToken(account) => account.owner,
        }
    }

    pub fn mint(&self) -> Pubkey {
        match self {
            Self::SplToken2022(account) => account.mint,
            Self::SplToken(account) => account.mint,
        }
    }

    pub fn delegate(&self) -> COption<Pubkey> {
        match self {
            Self::SplToken2022(account) => account.delegate,
            Self::SplToken(account) => account.delegate,
        }
    }

    pub fn set_delegate(&mut self, delegate: COption<Pubkey>) {
        match self {
            Self::SplToken2022(account) => account.delegate = delegate,
            Self::SplToken(account) => account.delegate = delegate,
        }
    }

    pub fn set_delegated_amount(&mut self, delegated_amount: u64) {
        match self {
            Self::SplToken2022(account) => account.delegated_amount = delegated_amount,
            Self::SplToken(account) => account.delegated_amount = delegated_amount,
        }
    }

    pub fn set_close_authority(&mut self, close_authority: COption<Pubkey>) {
        match self {
            Self::SplToken2022(account) => account.close_authority = close_authority,
            Self::SplToken(account) => account.close_authority = close_authority,
        }
    }

    pub fn amount(&self) -> u64 {
        match self {
            Self::SplToken2022(account) => account.amount,
            Self::SplToken(account) => account.amount,
        }
    }

    pub fn set_amount(&mut self, amount: u64) {
        match self {
            Self::SplToken2022(account) => account.amount = amount,
            Self::SplToken(account) => account.amount = amount,
        }
    }

    pub fn get_packed_len_for_token_program_id(token_program_id: &Pubkey) -> usize {
        if *token_program_id == spl_token::id() {
            spl_token::state::Account::get_packed_len()
        } else {
            spl_token_2022::state::Account::get_packed_len()
        }
    }

    pub fn set_state_from_str(&mut self, state: &str) -> SurfpoolResult<()> {
        match self {
            Self::SplToken2022(account) => {
                account.state = match state {
                    "uninitialized" => spl_token_2022::state::AccountState::Uninitialized,
                    "frozen" => spl_token_2022::state::AccountState::Frozen,
                    "initialized" => spl_token_2022::state::AccountState::Initialized,
                    _ => {
                        return Err(SurfpoolError::invalid_token_account_state(
                            &state.to_string(),
                        ));
                    }
                }
            }
            Self::SplToken(account) => {
                account.state = match state {
                    "uninitialized" => spl_token::state::AccountState::Uninitialized,
                    "frozen" => spl_token::state::AccountState::Frozen,
                    "initialized" => spl_token::state::AccountState::Initialized,
                    _ => {
                        return Err(SurfpoolError::invalid_token_account_state(
                            &state.to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum MintAccount {
    SplToken2022(spl_token_2022::state::Mint),
    SplToken(spl_token::state::Mint),
}

impl MintAccount {
    pub fn unpack(bytes: &[u8]) -> SurfpoolResult<Self> {
        if let Ok(mint) = spl_token_2022::state::Mint::unpack(bytes) {
            Ok(Self::SplToken2022(mint))
        } else if let Ok(mint) = spl_token::state::Mint::unpack(bytes) {
            Ok(Self::SplToken(mint))
        } else {
            Err(SurfpoolError::unpack_mint_account())
        }
    }

    pub fn decimals(&self) -> u8 {
        match self {
            Self::SplToken2022(mint) => mint.decimals,
            Self::SplToken(mint) => mint.decimals,
        }
    }
}

pub struct GeyserAccountUpdate {
    pub pubkey: Pubkey,
    pub account: Account,
    pub slot: u64,
    pub sanitized_transaction: SanitizedTransaction,
    pub write_version: u64,
}
impl GeyserAccountUpdate {
    pub fn new(
        pubkey: Pubkey,
        account: Account,
        slot: u64,
        sanitized_transaction: SanitizedTransaction,
        write_version: u64,
    ) -> Self {
        Self {
            pubkey,
            account,
            slot,
            sanitized_transaction,
            write_version,
        }
    }
}
