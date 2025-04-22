use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use serde_with::{serde_as, BytesOrString};
use solana_account::Account;
use solana_client::rpc_response::RpcResponseContext;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::clock::Epoch;
use solana_sdk::pubkey::Pubkey;

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
                owner: verify_pubkey(&self.owner.clone().unwrap())
                    .map_err(|e| Error::invalid_params(format!("Invalid owner: {}", e.message)))?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.data.clone().unwrap(),
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
        if let Some(owner) = self.owner {
            account.owner = verify_pubkey(&owner)
                .map_err(|e| Error::invalid_params(format!("Invalid owner: {}", e.message)))?;
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
        Ok(())
    }
}

#[rpc]
pub trait SvmTricksRpc {
    type Metadata;

    #[rpc(meta, name = "svm_setAccount")]
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
    let mut state_reader = meta.get_state_mut().map_err(|e| e.to_string())?;
    state_reader
        .svm
        .set_account(pubkey, account.clone())
        .map_err(|err| format!("failed to save fetched account '{pubkey:?}': {err:?}"))?;
    Ok(())
}

pub struct SurfpoolSvmTricksRpc;
impl SvmTricksRpc for SurfpoolSvmTricksRpc {
    type Metadata = Option<RunloopContext>;

    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
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

        let full_account_update = match update.to_account() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };
        if let Some(account) = full_account_update {
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
                            let Some(ctx) = &meta else {
                                return Err(RpcCustomError::NodeUnhealthy {
                                    num_slots_behind: None,
                                }
                                .into());
                            };
                            let _ = ctx.simnet_events_tx.send(SimnetEvent::info(
                                format!("Account {pubkey} not found, creating a new account from default values"),
                            ));
                            Account {
                                lamports: 0,
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                                data: vec![],
                            }
                        }
                    }
                };
                if let Err(e) = update.apply(&mut account) {
                    return Err(e);
                };
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
    }
}
