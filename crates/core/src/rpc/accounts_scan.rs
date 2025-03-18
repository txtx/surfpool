use super::not_implemented_err_async;
use super::RunloopContext;
use crate::rpc::State;
use jsonrpc_core::futures::future::{self};
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcLargestAccountsConfig, RpcProgramAccountsConfig, RpcSupplyConfig,
    RpcTokenAccountsFilter,
};
use solana_client::rpc_response::{
    OptionalContext, RpcAccountBalance, RpcKeyedAccount, RpcSupply, RpcTokenAccountBalance,
};
use solana_client::rpc_response::{RpcApiVersion, RpcResponseContext};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};

#[rpc]
pub trait AccountsScan {
    type Metadata;

    #[rpc(meta, name = "getProgramAccounts")]
    fn get_program_accounts(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>>;

    #[rpc(meta, name = "getLargestAccounts")]
    fn get_largest_accounts(
        &self,
        meta: Self::Metadata,
        config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>>;

    #[rpc(meta, name = "getSupply")]
    fn get_supply(
        &self,
        meta: Self::Metadata,
        config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>>;

    // SPL Token-specific RPC endpoints
    // See https://github.com/solana-labs/solana-program-library/releases/tag/token-v2.0.0 for
    // program details

    #[rpc(meta, name = "getTokenLargestAccounts")]
    fn get_token_largest_accounts(
        &self,
        meta: Self::Metadata,
        mint_str: String,
        commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>>;

    #[rpc(meta, name = "getTokenAccountsByOwner")]
    fn get_token_accounts_by_owner(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>>;

    #[rpc(meta, name = "getTokenAccountsByDelegate")]
    fn get_token_accounts_by_delegate(
        &self,
        meta: Self::Metadata,
        delegate_str: String,
        token_account_filter: RpcTokenAccountsFilter,
        config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>>;
}

pub struct SurfpoolAccountsScanRpc;
impl AccountsScan for SurfpoolAccountsScanRpc {
    type Metadata = Option<RunloopContext>;

    fn get_program_accounts(
        &self,
        _meta: Self::Metadata,
        _program_id_str: String,
        _config: Option<RpcProgramAccountsConfig>,
    ) -> BoxFuture<Result<OptionalContext<Vec<RpcKeyedAccount>>>> {
        not_implemented_err_async()
    }

    fn get_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcLargestAccountsConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcAccountBalance>>>> {
        not_implemented_err_async()
    }

    fn get_supply(
        &self,
        meta: Self::Metadata,
        config: Option<RpcSupplyConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSupply>>> {
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(err) => return Box::pin(future::err(err.into())),
        };

        let rpc_client = state_reader.rpc_client.clone();

        Box::pin(async move {
            if let Some(commitment) = config.and_then(|c| c.commitment) {
                let response = rpc_client
                    .supply_with_commitment(commitment)
                    .await
                    .map_err(|err| {
                        Error::invalid_params(format!(
                            "failed to get supply with commitment : {err:?}"
                        ))
                    });

                response
            } else {
                let response = rpc_client
                    .supply()
                    .await
                    .map_err(|err| Error::invalid_params(format!("failed to get supply: {err:?}")));

                response
            }
        })
    }

    fn get_token_largest_accounts(
        &self,
        _meta: Self::Metadata,
        _mint_str: String,
        _commitment: Option<CommitmentConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcTokenAccountBalance>>>> {
        not_implemented_err_async()
    }

    fn get_token_accounts_by_owner(
        &self,
        _meta: Self::Metadata,
        _owner_str: String,
        _token_account_filter: RpcTokenAccountsFilter,
        _config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        not_implemented_err_async()
    }

    fn get_token_accounts_by_delegate(
        &self,
        _meta: Self::Metadata,
        _delegate_str: String,
        _token_account_filter: RpcTokenAccountsFilter,
        _config: Option<RpcAccountInfoConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcKeyedAccount>>>> {
        not_implemented_err_async()
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::helpers::TestSetup;

    use super::*;
    use base64::{prelude::BASE64_STANDARD, Engine};
    use solana_account_decoder::{UiAccount, UiAccountData};
    use solana_client::rpc_config::RpcSimulateTransactionAccountsConfig;
    use solana_sdk::{
        commitment_config::CommitmentConfig,
        hash::Hash,
        message::{Message, MessageHeader},
        native_token::LAMPORTS_PER_SOL,
        signature::Keypair,
        signer::Signer,
        system_instruction, system_program,
        transaction::{Legacy, TransactionVersion},
    };
    use solana_transaction_status::{
        EncodedTransaction, EncodedTransactionWithStatusMeta, UiCompiledInstruction, UiMessage,
        UiRawMessage, UiTransaction,
    };
    use test_case::test_case;

    #[tokio::test]
    async fn test_get_supply() {
        let setup = TestSetup::new(SurfpoolAccountsScanRpc);
        let res = setup
            .rpc
            .get_supply(
                Some(setup.context),
                Some(RpcSupplyConfig {
                    commitment: Some(CommitmentConfig {
                        commitment: CommitmentLevel::Finalized,
                    }),
                    exclude_non_circulating_accounts_list: true,
                }),
            )
            .await
            .unwrap();

        let expected_response = RpcResponse {
            context: RpcResponseContext {
                slot: 2000,
                api_version: Some(RpcApiVersion::default()), // Some(RpcApiVersion( // we can't initialize a tuple struct which contains private fields
                                                             //     Version {
                                                             //         major: 1,
                                                             //         minor: 18,
                                                             //         patch: 18
                                                             //     }
                                                             // ))
            },
            value: RpcSupply {
                total: 503000508421036093,
                circulating: 503000508421036093,
                non_circulating: 0,
                non_circulating_accounts: vec![
                    "CakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S",
                    "CzAHrrrHKx9Lxf6wdCMrsZkLvk74c7J2vGv8VYPUmY6v",
                    "GdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ",
                    "8rT45mqpuDBR1vcnDc9kwP9DrZAXDR4ZeuKWw3u1gTGa",
                    "H1rt8KvXkNhQExTRfkY8r9wjZbZ8yCih6J4wQ5Fz9HGP",
                    "6nN69B4uZuESZYxr9nrLDjmKRtjDZQXrehwkfQTKw62U",
                    "Br3aeVGapRb2xTq17RU2pYZCoJpWA7bq6TKBCcYtMSmt",
                    "GmyW1nqYcrw7P7JqrcyP9ivU9hYNbrgZ1r5SYJJH41Fs",
                    "5XdtyEDREHJXXW1CTtCsVjJRjBapAwK78ZquzvnNVRrV",
                    "HKJgYGTTYYR2ZkfJKHbn58w676fKueQXmvbtpyvrSM3N",
                    "AzHQ8Bia1grVVbcGyci7wzueSWkgvu7YZVZ4B9rkL5P6",
                    "9xbcBZoGYFnfJZe81EDuDYKUm8xGkjzW8z4EgnVhNvsv",
                    "2WWb1gRzuXDd5viZLQF7pNRR6Y7UiyeaPpaL35X6j3ve",
                    "Eyr9P5XsjK2NUKNCnfu39eqpGoiLFgVAv1LSQgMZCwiQ",
                    "3itU5ME8L6FDqtMiRoUiT1F7PwbkTtHBbW51YWD5jtjm",
                    "FiWYY85b58zEEcPtxe3PuqzWPjqBJXqdwgZeqSBmT9Cn",
                    "Fgyh8EeYGZtbW8sS33YmNQnzx54WXPrJ5KWNPkCfWPot",
                    "CUageMFi49kzoDqtdU8NvQ4Bq3sbtJygjKDAXJ45nmAi",
                    "9huDUZfxoJ7wGMTffUE7vh1xePqef7gyrLJu9NApncqA",
                    "CuatS6njAcfkFHnvai7zXCs7syA9bykXWsDCJEWfhjHG",
                    "3iPvAS4xdhYr6SkhVDHCLr7tJjMAFK4wvvHWJxFQVg15",
                    "8pNBEppa1VcFAsx4Hzq9CpdXUXZjUXbvQwLX2K7QsCwb",
                    "4sxwau4mdqZ8zEJsfryXq4QFYnMJSCp3HWuZQod8WU5k",
                    "7Y8smnoUrYKGGuDq2uaFKVxJYhojgg7DVixHyAtGTYEV",
                    "CWeRmXme7LmbaUWTZWFLt6FMnpzLCHaQLuR2TdgFn4Lq",
                    "Hm9JW7of5i9dnrboS8pCUCSeoQUPh7JsP1rkbJnW7An4",
                    "4pV47TiPzZ7SSBPHmgUvSLmH9mMSe8tjyPhQZGbi1zPC",
                    "EAJJD6nDqtXcZ4DnQb19F9XEz8y8bRDHxbWbahatZNbL",
                    "GNiz4Mq886bTNDT3pijGsu2gbw6it7sqrwncro45USeB",
                    "F9MWFw8cnYVwsRq8Am1PGfFL3cQUZV37mbGoxZftzLjN",
                    "3jnknRabs7G2V9dKhxd2KP85pNWXKXiedYnYxtySnQMs",
                    "CND6ZjRTzaCFVdX7pSSWgjTfHZuhxqFDoUBqWBJguNoA",
                    "7xJ9CLtEAcEShw9kW2gSoZkRWL566Dg12cvgzANJwbTr",
                    "Ab1UcdsFXZVnkSt1Z3vcYU65GQk5MvCbs54SviaiaqHb",
                    "DQQGPtj7pphPHCLzzBuEyDDQByUcKGrsJdsH7SP3hAug",
                    "DUS1KxwUhUyDKB4A81E8vdnTe3hSahd92Abtn9CXsEcj",
                    "EMAY24PrS6rWfvpqffFCsTsFJypeeYYmtUc26wdh3Wup",
                    "14FUT96s9swbmH7ZjpDvfEDywnAYy9zaNhv4xvezySGu",
                    "63DtkW7zuARcd185EmHAkfF44bDcC2SiTSEj2spLP3iA",
                    "nGME7HgBT6tAJN1f6YuCCngpqT5cvSTndZUVLjQ4jwA",
                    "GK8R4uUmrawcREZ5xJy5dAzVV5V7aFvYg77id37pVTK",
                    "EMhn1U3TMimW3bvWYbPUvN2eZnCfsuBN4LGWhzzYhiWR",
                    "GEWSkfWgHkpiLbeKaAnwvqnECGdRNf49at5nFccVey7c",
                    "9hknftBZAQL4f48tWfk3bUEV5YSLcYYtDRqNmpNnhCWG",
                    "E8jcgWvrvV7rwYHJThwfiBeQ8VAH4FgNEEMG9aAuCMAq",
                    "BUjkdqUuH5Lz9XzcMcR4DdEMnFG6r8QzUMBm16Rfau96",
                    "AzVV9ZZDxTgW4wWfJmsG6ytaHpQGSe1yz76Nyy84VbQF",
                    "CY7X5o3Wi2eQhTocLmUS6JSWyx1NinBfW7AXRrkRCpi8",
                    "HQJtLqvEGGxgNYfRXUurfxV8E1swvCnsbC3456ik27HY",
                    "Fg12tB1tz8w6zJSQ4ZAGotWoCztdMJF9hqK8R11pakog",
                    "BUnRE27mYXN9p8H1Ay24GXhJC88q2CuwLoNU2v2CrW4W",
                    "3bTGcGB9F98XxnrBNftmmm48JGfPgi5sYxDEKiCjQYk3",
                    "BuCEvc9ze8UoAQwwsQLy8d447C8sA4zeVtVpc6m5wQeS",
                    "CVgyXrbEd1ctEuvq11QdpnCQVnPit8NLdhyqXQHLprM2",
                    "6o5v1HC7WhBnLfRHp8mQTtCP2khdXXjhuyGyYEoy2Suy",
                ]
                .iter()
                .map(|str| str.to_string())
                .collect::<Vec<String>>(),
            },
        };

        println!("this is the supply {:?}", res); //works
        assert_eq!(res, expected_response);
    }
}
