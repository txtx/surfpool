use std::{any::type_name, io::Write, sync::Arc};

use base64::prelude::*;
use bincode::Options;
use jsonrpc_core::{Error, Result};
use solana_account::Account;
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{
    rpc_config::{RpcAccountInfoConfig, RpcTokenAccountsFilter},
    rpc_custom_error::RpcCustomError,
    rpc_filter::RpcFilterType,
    rpc_request::{TokenAccountsFilter, MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT},
};
use solana_runtime::verify_precompiles::verify_precompiles;
use solana_sdk::{
    hash::Hash, packet::PACKET_DATA_SIZE, pubkey::Pubkey, signature::Signature,
    transaction::SanitizedTransaction,
};
use solana_transaction_status::TransactionBinaryEncoding;

fn optimize_filters(filters: &mut [RpcFilterType]) {
    filters.iter_mut().for_each(|filter_type| {
        if let RpcFilterType::Memcmp(compare) = filter_type {
            if let Err(err) = compare.convert_to_raw_bytes() {
                // All filters should have been previously verified
                warn!("Invalid filter: bytes could not be decoded, {err}");
            }
        }
    })
}

fn verify_transaction(
    transaction: &SanitizedTransaction,
    feature_set: &Arc<solana_feature_set::FeatureSet>,
) -> Result<()> {
    #[allow(clippy::question_mark)]
    if transaction.verify().is_err() {
        return Err(RpcCustomError::TransactionSignatureVerificationFailure.into());
    }

    let move_precompile_verification_to_svm =
        feature_set.is_active(&solana_feature_set::move_precompile_verification_to_svm::id());
    if !move_precompile_verification_to_svm {
        if let Err(e) = verify_precompiles(transaction, feature_set) {
            return Err(RpcCustomError::TransactionPrecompileVerificationFailure(e).into());
        }
    }

    Ok(())
}

fn verify_filter(input: &RpcFilterType) -> Result<()> {
    input
        .verify()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

pub fn verify_pubkey(input: &str) -> Result<Pubkey> {
    input
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

fn verify_hash(input: &str) -> Result<Hash> {
    input
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

fn verify_signature(input: &str) -> Result<Signature> {
    input
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

fn verify_token_account_filter(
    token_account_filter: RpcTokenAccountsFilter,
) -> Result<TokenAccountsFilter> {
    match token_account_filter {
        RpcTokenAccountsFilter::Mint(mint_str) => {
            let mint = verify_pubkey(&mint_str)?;
            Ok(TokenAccountsFilter::Mint(mint))
        }
        RpcTokenAccountsFilter::ProgramId(program_id_str) => {
            let program_id = verify_pubkey(&program_id_str)?;
            Ok(TokenAccountsFilter::ProgramId(program_id))
        }
    }
}

fn verify_and_parse_signatures_for_address_params(
    address: String,
    before: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
) -> Result<(Pubkey, Option<Signature>, Option<Signature>, usize)> {
    let address = verify_pubkey(&address)?;
    let before = before
        .map(|ref before| verify_signature(before))
        .transpose()?;
    let until = until.map(|ref until| verify_signature(until)).transpose()?;
    let limit = limit.unwrap_or(MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT);

    if limit == 0 || limit > MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT {
        return Err(Error::invalid_params(format!(
            "Invalid limit; max {MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT}"
        )));
    }
    Ok((address, before, until, limit))
}

const MAX_BASE58_SIZE: usize = 1683; // Golden, bump if PACKET_DATA_SIZE changes
const MAX_BASE64_SIZE: usize = 1644; // Golden, bump if PACKET_DATA_SIZE changes
pub fn decode_and_deserialize<T>(
    encoded: String,
    encoding: TransactionBinaryEncoding,
) -> Result<(Vec<u8>, T)>
where
    T: serde::de::DeserializeOwned,
{
    let wire_output = match encoding {
        TransactionBinaryEncoding::Base58 => {
            if encoded.len() > MAX_BASE58_SIZE {
                return Err(Error::invalid_params(format!(
                    "base58 encoded {} too large: {} bytes (max: encoded/raw {}/{})",
                    type_name::<T>(),
                    encoded.len(),
                    MAX_BASE58_SIZE,
                    PACKET_DATA_SIZE,
                )));
            }
            bs58::decode(encoded)
                .into_vec()
                .map_err(|e| Error::invalid_params(format!("invalid base58 encoding: {e:?}")))?
        }
        TransactionBinaryEncoding::Base64 => {
            if encoded.len() > MAX_BASE64_SIZE {
                return Err(Error::invalid_params(format!(
                    "base64 encoded {} too large: {} bytes (max: encoded/raw {}/{})",
                    type_name::<T>(),
                    encoded.len(),
                    MAX_BASE64_SIZE,
                    PACKET_DATA_SIZE,
                )));
            }
            BASE64_STANDARD
                .decode(encoded)
                .map_err(|e| Error::invalid_params(format!("invalid base64 encoding: {e:?}")))?
        }
    };
    if wire_output.len() > PACKET_DATA_SIZE {
        return Err(Error::invalid_params(format!(
            "decoded {} too large: {} bytes (max: {} bytes)",
            type_name::<T>(),
            wire_output.len(),
            PACKET_DATA_SIZE
        )));
    }
    bincode::options()
        .with_limit(PACKET_DATA_SIZE as u64)
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .deserialize_from(&wire_output[..])
        .map_err(|err| {
            Error::invalid_params(format!(
                "failed to deserialize {}: {}",
                type_name::<T>(),
                &err.to_string()
            ))
        })
        .map(|output| (wire_output, output))
}

pub fn transform_account_to_ui_account(
    account: &Option<Account>,
    config: &RpcAccountInfoConfig,
) -> Result<Option<UiAccount>> {
    if let Some(account) = account {
        Ok(Some(UiAccount {
            lamports: account.lamports,
            owner: account.owner.to_string(),
            data: {
                let account_data = if let Some(data_slice) = config.data_slice {
                    let end =
                        std::cmp::min(account.data.len(), data_slice.offset + data_slice.length);
                    account.data.clone()[data_slice.offset..end].to_vec()
                } else {
                    account.data.clone()
                };

                match config.encoding {
                    Some(UiAccountEncoding::Base58) => UiAccountData::Binary(
                        bs58::encode(account_data).into_string(),
                        UiAccountEncoding::Base58,
                    ),
                    Some(UiAccountEncoding::Base64) => UiAccountData::Binary(
                        BASE64_STANDARD.encode(account_data),
                        UiAccountEncoding::Base64,
                    ),
                    Some(UiAccountEncoding::Base64Zstd) => {
                        let mut data = Vec::with_capacity(account_data.len());

                        // Default compression level
                        match zstd::Encoder::new(&mut data, 0).and_then(|mut encoder| {
                            encoder
                                .write_all(&account_data)
                                .and_then(|_| encoder.finish())
                        }) {
                            Ok(_) => UiAccountData::Binary(
                                BASE64_STANDARD.encode(&data),
                                UiAccountEncoding::Base64Zstd,
                            ),
                            // Falling back on standard base64 encoding if compression failed
                            Err(err) => {
                                eprintln!("Zstd compression failed: {err}");
                                UiAccountData::Binary(
                                    BASE64_STANDARD.encode(&account_data),
                                    UiAccountEncoding::Base64,
                                )
                            }
                        }
                    }
                    None => UiAccountData::Binary(
                        bs58::encode(account_data.clone()).into_string(),
                        UiAccountEncoding::Base58,
                    ),
                    encoding => Err(jsonrpc_core::Error::invalid_params(format!(
                        "Encoding {encoding:?} is not supported yet."
                    )))?,
                }
            },
            executable: account.executable,
            rent_epoch: account.rent_epoch,
            space: Some(account.data.len() as u64),
        }))
    } else {
        Ok(None)
    }
}
