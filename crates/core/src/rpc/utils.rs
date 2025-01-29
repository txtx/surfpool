use std::sync::Arc;

use solana_client::{rpc_config::RpcTokenAccountsFilter, rpc_custom_error::RpcCustomError, rpc_filter::RpcFilterType, rpc_request::{TokenAccountsFilter, MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT}};
use solana_runtime::verify_precompiles::verify_precompiles;
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature, transaction::SanitizedTransaction};
use jsonrpc_core::{Error, Result};

use super::CommitmentConfig;



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
