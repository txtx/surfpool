use base64::{engine::general_purpose::STANDARD, Engine as _};
use jsonrpc_core::{futures::future, BoxFuture, Error, Result};
use jsonrpc_derive::rpc;
use solana_account::Account;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_response::RpcResponseContext;
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::{
    program_option::COption, program_pack::Pack, system_program, transaction::VersionedTransaction,
};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token::state::{Account as TokenAccount, AccountState};
use surfpool_types::{
    types::{AccountUpdate, ProfileResult, SetSomeAccount, SupplyUpdate, TokenAccountUpdate},
    SimnetEvent,
};

use super::{RunloopContext, SurfnetRpcContext};
use crate::{
    error::SurfpoolError,
    rpc::{
        utils::{verify_pubkey, verify_pubkeys},
        State,
    },
    surfnet::{locker::SvmAccessContext, GetAccountResult},
};

pub trait AccountUpdateExt {
    fn is_full_account_data_ext(&self) -> bool;
    fn to_account_ext(&self) -> Result<Option<Account>>;
    fn apply_ext(self, account: &mut GetAccountResult) -> Result<()>;
    fn expect_hex_data_ext(&self) -> Result<Vec<u8>>;
}

impl AccountUpdateExt for AccountUpdate {
    fn is_full_account_data_ext(&self) -> bool {
        self.lamports.is_some()
            && self.owner.is_some()
            && self.executable.is_some()
            && self.rent_epoch.is_some()
            && self.data.is_some()
    }

    fn to_account_ext(&self) -> Result<Option<Account>> {
        if self.is_full_account_data_ext() {
            Ok(Some(Account {
                lamports: self.lamports.unwrap(),
                owner: verify_pubkey(&self.owner.clone().unwrap())?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.expect_hex_data_ext()?,
            }))
        } else {
            Ok(None)
        }
    }

    fn apply_ext(self, account_result: &mut GetAccountResult) -> Result<()> {
        account_result.apply_update(|account| {
            if let Some(lamports) = self.lamports {
                account.lamports = lamports;
            }
            if let Some(owner) = &self.owner {
                account.owner = verify_pubkey(owner)?;
            }
            if let Some(executable) = self.executable {
                account.executable = executable;
            }
            if let Some(rent_epoch) = self.rent_epoch {
                account.rent_epoch = rent_epoch;
            }
            if self.data.is_some() {
                account.data = self.expect_hex_data_ext()?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn expect_hex_data_ext(&self) -> Result<Vec<u8>> {
        let data = self.data.as_ref().expect("missing expected data field");
        hex::decode(data)
            .map_err(|e| Error::invalid_params(format!("Invalid hex data provided: {}", e)))
    }
}

pub trait TokenAccountUpdateExt {
    fn apply(self, token_account: &mut TokenAccount) -> Result<()>;
}

impl TokenAccountUpdateExt for TokenAccountUpdate {
    /// Apply the update to the account
    fn apply(self, token_account: &mut TokenAccount) -> Result<()> {
        if let Some(amount) = self.amount {
            token_account.amount = amount;
        }
        if let Some(delegate) = self.delegate {
            match delegate {
                SetSomeAccount::Account(pubkey) => {
                    token_account.delegate = COption::Some(verify_pubkey(&pubkey)?);
                }
                SetSomeAccount::NoAccount => {
                    token_account.delegate = COption::None;
                }
            }
        }
        if let Some(state) = self.state {
            token_account.state = match state.as_str() {
                "uninitialized" => AccountState::Uninitialized,
                "frozen" => AccountState::Frozen,
                "initialized" => AccountState::Initialized,
                _ => {
                    return Err(Error::invalid_params(format!(
                        "Invalid token account state: {}",
                        state
                    )))
                }
            };
        }
        if let Some(delegated_amount) = self.delegated_amount {
            token_account.delegated_amount = delegated_amount;
        }
        if let Some(close_authority) = self.close_authority {
            match close_authority {
                SetSomeAccount::Account(pubkey) => {
                    token_account.close_authority = COption::Some(verify_pubkey(&pubkey)?);
                }
                SetSomeAccount::NoAccount => {
                    token_account.close_authority = COption::None;
                }
            }
        }
        Ok(())
    }
}

#[rpc]
pub trait SvmTricksRpc {
    type Metadata;

    /// A "cheat code" method for developers to set or update an account in Surfpool.
    ///
    /// This method allows developers to set or update the lamports, data, owner, executable status,
    /// and rent epoch of a given account.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `pubkey`: The public key of the account to be updated, as a base-58 encoded string.
    /// - `update`: The `AccountUpdate` struct containing the fields to update the account.
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the account update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setAccount",
    ///   "params": ["account_pubkey", {"lamports": 1000, "data": "base58string", "owner": "program_pubkey"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers set or modify account properties within Surfpool.
    /// Developers can quickly test or update account attributes, such as lamports, program ownership, and executable status.
    ///
    /// # See Also
    /// - `getAccount`, `getAccountInfo`, `getAccountBalance`
    #[rpc(meta, name = "surfnet_setAccount")]
    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// A "cheat code" method for developers to set or update a token account in Surfpool.
    ///
    /// This method allows developers to set or update various properties of a token account,
    /// including the token amount, delegate, state, delegated amount, and close authority.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `owner`: The base-58 encoded public key of the token account's owner.
    /// - `mint`: The base-58 encoded public key of the token mint (e.g., the token type).
    /// - `update`: The `TokenAccountUpdate` struct containing the fields to update the token account.
    /// - `token_program`: The optional base-58 encoded address of the token program (defaults to the system token program).
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the token account update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setTokenAccount",
    ///   "params": ["owner_pubkey", "mint_pubkey", {"amount": 1000, "state": "initialized"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers quickly test or modify token account properties in Surfpool.
    /// Developers can update attributes such as token amounts, delegates, and authorities for specific token accounts.
    ///
    /// # See Also
    /// - `getTokenAccountInfo`, `getTokenAccountBalance`, `getTokenAccountDelegate`
    #[rpc(meta, name = "surfnet_setTokenAccount")]
    fn set_token_account(
        &self,
        meta: Self::Metadata,
        owner: String,
        mint: String,
        update: TokenAccountUpdate,
        token_program: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    #[rpc(meta, name = "surfnet_cloneProgramAccount")]
    fn clone_program_account(
        &self,
        meta: Self::Metadata,
        source_program_id: String,
        destination_program_id: String,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// Estimates the compute units that a given transaction will consume.
    ///
    /// This method simulates the transaction without committing its state changes
    /// and returns an estimation of the compute units used, along with logs and
    /// potential errors.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request.
    /// - `transaction_data`: A base64 encoded string of the `VersionedTransaction`.
    /// - `tag`: An optional tag for the transaction.
    /// - `encoding`: An optional encoding for returned account data.
    ///
    /// ## Returns
    /// A `RpcResponse<ProfileResult>` containing the estimation details and a snapshot of the accounts before and after execution.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_profileTransaction",
    ///   "params": ["base64_encoded_transaction_string", "optional_tag"]
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_profileTransaction")]
    fn profile_transaction(
        &self,
        meta: Self::Metadata,
        transaction_data: String, // Base64 encoded VersionedTransaction
        tag: Option<String>,      // Optional tag for the transaction
        encoding: Option<UiAccountEncoding>,
    ) -> BoxFuture<Result<RpcResponse<ProfileResult>>>;

    /// Retrieves all profiling results for a given tag.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request.
    /// - `tag`: The tag to retrieve profiling results for.
    ///
    /// ## Returns
    /// A `RpcResponse<Vec<ProfileResult>>` containing the profiling results.
    #[rpc(meta, name = "surfnet_getProfileResults")]
    fn get_profile_results(
        &self,
        meta: Self::Metadata,
        tag: String,
    ) -> Result<RpcResponse<Vec<ProfileResult>>>;

    /// A "cheat code" method for developers to set or update the network supply information in Surfpool.
    ///
    /// This method allows developers to configure the total supply, circulating supply,
    /// non-circulating supply, and non-circulating accounts list that will be returned
    /// by the `getSupply` RPC method.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `update`: The `SupplyUpdate` struct containing the optional fields to update:
    ///   - `total`: Optional total supply in lamports
    ///   - `circulating`: Optional circulating supply in lamports  
    ///   - `non_circulating`: Optional non-circulating supply in lamports
    ///   - `non_circulating_accounts`: Optional list of non-circulating account addresses
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the supply update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setSupply",
    ///   "params": [{
    ///     "total": 1000000000000000,
    ///     "circulating": 800000000000000,
    ///     "non_circulating": 200000000000000,
    ///     "non_circulating_accounts": ["Account1...", "Account2..."]
    ///   }]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers test supply-related functionality by
    /// allowing them to configure the values returned by `getSupply` without needing
    /// to connect to a real network or manipulate actual token supplies.
    ///
    /// # See Also
    /// - `getSupply`
    #[rpc(meta, name = "surfnet_setSupply")]
    fn set_supply(
        &self,
        meta: Self::Metadata,
        update: SupplyUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;
}

#[derive(Clone)]
pub struct SurfnetCheatcodesRpc;
impl SvmTricksRpc for SurfnetCheatcodesRpc {
    type Metadata = Option<RunloopContext>;

    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        let account_update_opt = match update.to_account_ext() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let (account_to_set, latest_absolute_slot) = if let Some(account) = account_update_opt {
                (
                    GetAccountResult::FoundAccount(pubkey, account, true),
                    svm_locker.get_latest_absolute_slot(),
                )
            } else {
                // otherwise, we need to fetch the account and apply the update
                let SvmAccessContext {
                    slot, inner: mut account_result_to_update,
                    ..
                } = svm_locker.get_account(&remote_ctx, &pubkey, Some(Box::new(move |svm_locker| {

                            // if the account does not exist locally or in the remote, create a new account with default values
                            let _ = svm_locker.simnet_events_tx().send(SimnetEvent::info(format!(
                                "Account {pubkey} not found, creating a new account from default values"
                            )));
                            GetAccountResult::FoundAccount(
                                pubkey,
                                solana_account::Account {
                                    lamports: 0,
                                    owner: system_program::id(),
                                    executable: false,
                                    rent_epoch: 0,
                                    data: vec![],
                                },
                                true, // indicate that the account should be updated in the SVM, since it's new
                            )
                }))).await?;

                update.apply_ext(&mut account_result_to_update)?;
                (account_result_to_update, slot)
            };

            svm_locker.write_account_update(account_to_set);

            Ok(RpcResponse {
                context: RpcResponseContext::new(latest_absolute_slot),
                value: (),
            })
        })
    }

    fn set_token_account(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        mint_str: String,
        update: TokenAccountUpdate,
        some_token_program_str: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let owner = match verify_pubkey(&owner_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let mint = match verify_pubkey(&mint_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let token_program_id = match some_token_program_str {
            Some(token_program_str) => match verify_pubkey(&token_program_str) {
                Ok(res) => res,
                Err(e) => return e.into(),
            },
            None => spl_token::id(),
        };

        let associated_token_account =
            get_associated_token_address_with_program_id(&owner, &mint, &token_program_id);

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: mut token_account,
                ..
            } = svm_locker
                .get_account(
                    &remote_ctx,
                    &associated_token_account,
                    Some(Box::new(move |svm_locker| {
                        let minimum_rent = svm_locker.with_svm_reader(|svm_reader| {
                            svm_reader
                                .inner
                                .minimum_balance_for_rent_exemption(TokenAccount::LEN)
                        });

                        let mut data = [0; TokenAccount::LEN];
                        let default = TokenAccount {
                            mint,
                            owner,
                            state: AccountState::Initialized,
                            ..Default::default()
                        };
                        default.pack_into_slice(&mut data);
                        GetAccountResult::FoundAccount(
                            associated_token_account,
                            Account {
                                lamports: minimum_rent,
                                owner: token_program_id,
                                executable: false,
                                rent_epoch: 0,
                                data: data.to_vec(),
                            },
                            true, // indicate that the account should be updated in the SVM, since it's new
                        )
                    })),
                )
                .await?;

            let mut token_account_data = TokenAccount::unpack(token_account.expected_data())
                .map_err(|e| {
                    Error::invalid_params(format!("Failed to unpack token account data: {}", e))
                })?;

            update.apply(&mut token_account_data)?;

            let mut final_account_bytes = [0; TokenAccount::LEN];
            token_account_data.pack_into_slice(&mut final_account_bytes);
            token_account.apply_update(|account| {
                account.data = final_account_bytes.to_vec();
                Ok(())
            })?;
            svm_locker.write_account_update(token_account);

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: (),
            })
        })
    }

    /// Clones a program account from one program ID to another.
    /// A program account contains a pointer to a program data account, which is a PDA derived from the program ID.
    /// So, when cloning a program account, we need to clone the program data account as well.
    ///
    /// This method will:
    ///  1. Get the program account for the source program ID.
    ///  2. Get the program data account for the source program ID.
    ///  3. Calculate the program data address for the destination program ID.
    ///  4. Set the destination program account's data to point to the calculated destination program address.
    ///  5. Copy the source program data account to the destination program data account.
    fn clone_program_account(
        &self,
        meta: Self::Metadata,
        source_program_id: String,
        destination_program_id: String,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let source_program_id = match verify_pubkey(&source_program_id) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        let destination_program_id = match verify_pubkey(&destination_program_id) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext { slot, .. } = svm_locker
                .clone_program_account(&remote_ctx, &source_program_id, &destination_program_id)
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: (),
            })
        })
    }

    fn profile_transaction(
        &self,
        meta: Self::Metadata,
        transaction_data_b64: String,
        tag: Option<String>,
        encoding: Option<UiAccountEncoding>,
    ) -> BoxFuture<Result<RpcResponse<ProfileResult>>> {
        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(()) {
            Ok(ctx) => ctx,
            Err(e) => return e.into(),
        };
        let remote_ctx = remote_ctx.map(|(client, _)| client);

        let transaction_bytes = match STANDARD.decode(&transaction_data_b64) {
            Ok(bytes) => bytes,
            Err(e) => return SurfpoolError::invalid_base64_data("transaction", e).into(),
        };

        let transaction: VersionedTransaction = match bincode::deserialize(&transaction_bytes) {
            Ok(tx) => tx,
            Err(e) => return SurfpoolError::deserialize_error("transaction", e).into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: profile_result,
                ..
            } = svm_locker
                .profile_transaction(&remote_ctx, &transaction, encoding)
                .await?;

            if let Some(tag_str) = tag {
                if profile_result.compute_units.success {
                    svm_locker.write_profiling_results(tag_str, profile_result.clone());
                }
            }

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: profile_result,
            })
        })
    }

    fn get_profile_results(
        &self,
        meta: Self::Metadata,
        tag: String,
    ) -> Result<RpcResponse<Vec<ProfileResult>>> {
        meta.with_svm_reader(|svm_reader| {
            let results = svm_reader
                .tagged_profiling_results
                .get(&tag)
                .cloned()
                .unwrap_or_default();

            RpcResponse {
                context: RpcResponseContext::new(svm_reader.get_latest_absolute_slot()),
                value: results,
            }
        })
        .map_err(Into::into)
    }

    fn set_supply(
        &self,
        meta: Self::Metadata,
        update: SupplyUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        // validate non-circulating accounts are valid pubkeys
        if let Some(ref non_circulating_accounts) = update.non_circulating_accounts {
            if let Err(e) = verify_pubkeys(non_circulating_accounts) {
                return e.into();
            }
        }

        Box::pin(async move {
            let latest_absolute_slot = svm_locker.with_svm_writer(|svm_writer| {
                // update the supply fields if provided
                if let Some(total) = update.total {
                    svm_writer.total_supply = total;
                }

                if let Some(circulating) = update.circulating {
                    svm_writer.circulating_supply = circulating;
                }

                if let Some(non_circulating) = update.non_circulating {
                    svm_writer.non_circulating_supply = non_circulating;
                }

                if let Some(ref accounts) = update.non_circulating_accounts {
                    svm_writer.non_circulating_accounts = accounts.clone();
                }

                svm_writer.updated_at = chrono::Utc::now().timestamp_millis() as u64;
                svm_writer.get_latest_absolute_slot()
            });

            Ok(RpcResponse {
                context: RpcResponseContext::new(latest_absolute_slot),
                value: (),
            })
        })
    }
}
