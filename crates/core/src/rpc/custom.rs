use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use solana_account::Account;
use solana_client::rpc_response::RpcResponseContext;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::clock::Epoch;
use solana_sdk::pubkey::Pubkey;

use super::RunloopContext;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    /// providing this value sets the lamports in the account
    pub lamports: Option<u64>,
    /// providing this value sets the data held in this account
    #[cfg_attr(serde, with = "serde_bytes")]
    pub data: Option<Vec<u8>>,
    ///  providing this value sets the program that owns this account. If executable, the program that loads this account.
    pub owner: Option<Pubkey>,
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
    pub fn to_account(&self) -> Option<Account> {
        if self.is_full_account_data() {
            Some(Account {
                lamports: self.lamports.unwrap(),
                owner: self.owner.unwrap(),
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.data.clone().unwrap(),
            })
        } else {
            None
        }
    }
    /// Apply the update to the account
    pub fn apply(self, account: &mut Account) {
        if let Some(lamports) = self.lamports {
            account.lamports = lamports;
        }
        if let Some(owner) = self.owner {
            account.owner = owner;
        }
        if let Some(executable) = self.executable {
            account.executable = executable;
        }
        if let Some(rent_epoch) = self.rent_epoch {
            account.rent_epoch = rent_epoch;
        }
        if let Some(data) = &self.data {
            account.data = data.clone();
        }
    }
}

#[rpc]
pub trait CustomRpc {
    type Metadata;

    #[rpc(meta, name = "surfpool_setAccount")]
    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;
}

pub fn write_account(
    meta: Option<RunloopContext>,
    pubkey: Pubkey,
    account: Account,
) -> std::result::Result<(), String> {
    println!("writing pubkey {pubkey:?} with account {account:?}");
    let mut state_reader = meta.get_state_mut().map_err(|e| e.to_string())?;
    state_reader
        .svm
        .set_account(pubkey, account.clone())
        .map_err(|err| format!("failed to save fetched account {pubkey:?}: {err:?}"))?;
    Ok(())
}

pub struct SurfpoolCustomRpc;
impl CustomRpc for SurfpoolCustomRpc {
    type Metadata = Option<RunloopContext>;

    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        println!("set_account called with pubkey {pubkey_str:?} and update {update:?}");
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let absolute_slot = state_reader.epoch_info.absolute_slot;
        let account = state_reader.svm.get_account(&pubkey);
        let rpc_client = state_reader.rpc_client.clone();
        drop(state_reader);

        if let Some(account) = update.to_account() {
            return match write_account(meta, pubkey, account).map_err(|e| Error::invalid_params(e))
            {
                Ok(_) => Box::pin(future::ok(RpcResponse {
                    context: RpcResponseContext::new(absolute_slot),
                    value: (),
                })),
                Err(e) => Box::pin(future::err(e)),
            };
        } else {
            return Box::pin(async move {
                let mut account = match account {
                    Some(account) => account,
                    None => {
                        if let Some(fetched_account) = rpc_client.get_account(&pubkey).await.ok() {
                            fetched_account
                        } else {
                            return Err(Error::invalid_params(format!(
                                "cannot mutate account that does not exist unless all account fields are provided"
                            )));
                        }
                    }
                };
                update.apply(&mut account);
                return match write_account(meta, pubkey, account)
                    .map_err(|e| Error::invalid_params(e))
                {
                    Ok(_) => Ok(RpcResponse {
                        context: RpcResponseContext::new(absolute_slot),
                        value: (),
                    }),
                    Err(e) => Err(e),
                };
            });
        }

        // not_implemented_err_async()
    }
}
