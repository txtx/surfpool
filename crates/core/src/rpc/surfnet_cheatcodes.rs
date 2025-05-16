use std::fmt;

use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;
use crate::surfnet::GetAccountStrategy;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use serde::de::Visitor;
use serde::Serialize;
use serde::{Deserialize, Deserializer, Serializer};
use serde_with::{serde_as, BytesOrString};
use solana_account::Account;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::Epoch;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::program_option::COption;
use solana_sdk::program_pack::Pack;
use solana_sdk::system_program;
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token::state::{Account as TokenAccount, AccountState};
use surfpool_types::SimnetEvent;
use surfpool_types::types::{ProfileResult, ComputeUnitsEstimationResult};

use super::RunloopContext;

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    /// providing this value sets the lamports in the account
    pub lamports: Option<u64>,
    /// providing this value sets the data held in this account
    #[serde_as(as = "Option<BytesOrString>")]
    pub data: Option<Vec<u8>>,
    ///  providing this value sets the program that owns this account. If executable, the program that loads this account.
    pub owner: Option<String>,
    /// providing this value sets whether this account's data contains a loaded program (and is now read-only)
    pub executable: Option<bool>,
    /// providing this value sets the epoch at which this account will next owe rent
    pub rent_epoch: Option<Epoch>,
}

impl AccountUpdate {
    fn is_full_account_data(&self) -> bool {
        self.lamports.is_some()
            && self.owner.is_some()
            && self.executable.is_some()
            && self.rent_epoch.is_some()
            && self.data.is_some()
    }
    /// Convert the update to an account if all fields are provided
    pub fn to_account(&self) -> Result<Option<Account>> {
        if self.is_full_account_data() {
            Ok(Some(Account {
                lamports: self.lamports.unwrap(),
                owner: verify_pubkey(&self.owner.clone().unwrap())?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.expect_hex_data()?,
            }))
        } else {
            Ok(None)
        }
    }
    /// Apply the update to the account
    pub fn apply(self, account: &mut Account) -> Result<()> {
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
            account.data = self.expect_hex_data()?;
        }
        Ok(())
    }

    pub fn expect_hex_data(&self) -> Result<Vec<u8>> {
        let data = self.data.as_ref().expect("missing expected data field");
        hex::decode(data)
            .map_err(|e| Error::invalid_params(format!("Invalid hex data provided: {}", e)))
    }
}

#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    /// providing this value sets the amount of the token in the account data
    pub amount: Option<u64>,
    /// providing this value sets the delegate of the token account
    pub delegate: Option<SetSomeAccount>,
    /// providing this value sets the state of the token account
    pub state: Option<String>,
    /// providing this value sets the amount authorized to the delegate
    pub delegated_amount: Option<u64>,
    /// providing this value sets the close authority of the token account
    pub close_authority: Option<SetSomeAccount>,
}

#[derive(Debug, Clone)]
pub enum SetSomeAccount {
    Account(String),
    NoAccount,
}

impl<'de> Deserialize<'de> for SetSomeAccount {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SetSomeAccountVisitor;

        impl<'de> Visitor<'de> for SetSomeAccountVisitor {
            type Value = SetSomeAccount;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Pubkey String or the String 'null'")
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer).map(|v: String| match v.as_str() {
                    "null" => SetSomeAccount::NoAccount,
                    _ => SetSomeAccount::Account(v.to_string()),
                })
            }
        }

        deserializer.deserialize_option(SetSomeAccountVisitor)
    }
}

impl Serialize for SetSomeAccount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            SetSomeAccount::Account(val) => serializer.serialize_str(val),
            SetSomeAccount::NoAccount => serializer.serialize_str("null"),
        }
    }
}

impl TokenAccountUpdate {
    /// Apply the update to the account
    pub fn apply(self, token_account: &mut TokenAccount) -> Result<()> {
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
    ///
    /// ## Returns
    /// A `RpcResponse<ProfileResult>` containing the estimation details.
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
    fn estimate_compute_units(
        &self,
        meta: Self::Metadata,
        transaction_data: String, // Base64 encoded VersionedTransaction
        tag: Option<String>,      // Optional tag for the transaction
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
    ) -> BoxFuture<Result<RpcResponse<Vec<ProfileResult>>>>;
}

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
        let account_update = match update.to_account() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;

            // if the account update is contains all fields, we can directly set the account
            let account_to_set = if let Some(account) = account_update {
                account
            } else {
                // otherwise, we need to fetch the account and apply the update
                let mut account_to_update = svm_writer
                    .get_account(
                        &pubkey,
                        GetAccountStrategy::LocalThenConnectionOrDefault(Some(Box::new(move |surfnet_svm| {
                            // if the account does not exist locally or in the remote, create a new account with default values
                            let _ = surfnet_svm.simnet_events_tx.send(SimnetEvent::info(format!(
                                "Account {pubkey} not found, creating a new account from default values"
                            )));

                            Account {
                                lamports: 0,
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                                data: vec![],
                            }
                        }))),
                    )
                    .await?.unwrap();

                update.apply(&mut account_to_update)?;
                account_to_update
            };
            svm_writer.set_account(&pubkey, account_to_set)?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
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

        let svm_locker = match meta.get_svm_locker() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            let mut token_account = svm_writer
                .get_account(
                    &associated_token_account,
                    GetAccountStrategy::LocalThenConnectionOrDefault(Some(Box::new(move |surfnet_svm| {
                        let _ = surfnet_svm.simnet_events_tx.send(SimnetEvent::info(
                            format!("Associated token account {associated_token_account} not found, creating a new account from default values"),
                        ));

                        let minimum_rent = surfnet_svm
                            .inner
                            .minimum_balance_for_rent_exemption(TokenAccount::LEN);

                        let mut data = [0; TokenAccount::LEN];
                        let default = TokenAccount {
                            mint,
                            owner,
                            state: AccountState::Initialized,
                            ..Default::default()
                        };
                        default.pack_into_slice(&mut data);
                        Account {
                            lamports: minimum_rent,
                            owner: token_program_id,
                            executable: false,
                            rent_epoch: 0,
                            data: data.to_vec(),
                        }
                    }))),
                )
                .await?
                .unwrap();
            let mut token_account_data =
                TokenAccount::unpack(&token_account.data).map_err(|e| {
                    Error::invalid_params(format!("Failed to unpack token account data: {}", e))
                })?;

            update.apply(&mut token_account_data)?;

            let mut final_account_bytes = [0; TokenAccount::LEN];
            token_account_data.pack_into_slice(&mut final_account_bytes);
            token_account.data = final_account_bytes.to_vec();
            svm_writer.set_account(&associated_token_account, token_account)?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
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

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            svm_writer
                .clone_program_account(&source_program_id, &destination_program_id)
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: (),
            })
        })
    }

    fn estimate_compute_units(
        &self,
        meta: Self::Metadata,
        transaction_data_b64: String,
        tag: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<ProfileResult>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        let transaction_bytes = match STANDARD.decode(&transaction_data_b64) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Base64 decoding failed: {}", e);
                let error_cu_result = ComputeUnitsEstimationResult {
                    success: false,
                    compute_units_consumed: 0,
                    log_messages: None,
                    error_message: Some(format!("Invalid base64 for transaction data: {}", e)),
                };
                return Box::pin(future::ok(RpcResponse {
                    context: RpcResponseContext::new(0),
                    value: ProfileResult { compute_units: error_cu_result },
                }));
            }
        };

        let transaction: VersionedTransaction = match bincode::deserialize(&transaction_bytes) {
            Ok(tx) => tx,
            Err(e) => {
                let error_cu_result = ComputeUnitsEstimationResult {
                    success: false,
                    compute_units_consumed: 0,
                    log_messages: None,
                    error_message: Some(format!("Failed to deserialize transaction: {}", e)),
                };
                return Box::pin(future::ok(RpcResponse {
                    context: RpcResponseContext::new(0),
                    value: ProfileResult { compute_units: error_cu_result },
                }));
            }
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;

            let estimation_result = svm_writer.estimate_compute_units(&transaction);

            if let Some(tag_str) = tag {
                if estimation_result.success {
                    let profile_result_to_store = ProfileResult {
                        compute_units: estimation_result.clone(),
                    };
                    svm_writer
                        .tagged_profiling_results
                        .entry(tag_str.clone())
                        .or_default()
                        .push(profile_result_to_store.clone());
                    let _ = svm_writer
                        .simnet_events_tx
                        .try_send(SimnetEvent::tagged_profile(
                            profile_result_to_store,
                            tag_str.clone(),
                        ));
                }
            }

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: ProfileResult { compute_units: estimation_result },
            })
        })
    }

    fn get_profile_results(
        &self,
        meta: Self::Metadata,
        tag: String,
    ) -> BoxFuture<Result<RpcResponse<Vec<ProfileResult>>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let svm_reader = svm_locker.read().await;
            let results = svm_reader
                .tagged_profiling_results
                .get(&tag)
                .cloned()
                .unwrap_or_default();

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_reader.get_latest_absolute_slot()),
                value: results,
            })
        })
    }
}
