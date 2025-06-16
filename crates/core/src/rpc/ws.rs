use std::{
    collections::HashMap,
    str::FromStr,
    sync::{atomic, Arc, RwLock},
};

use jsonrpc_core::{Error, ErrorCode, Result};
use jsonrpc_derive::rpc;
use jsonrpc_pubsub::{
    typed::{Sink, Subscriber},
    SubscriptionId,
};
use solana_account_decoder::{encode_ui_account, UiAccount, UiAccountEncoding};
use solana_client::{
    rpc_config::RpcSignatureSubscribeConfig,
    rpc_response::{
        ProcessedSignatureResult, ReceivedSignatureResult, RpcResponseContext, RpcSignatureResult,
    },
};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_signature::Signature;
use solana_transaction_status::TransactionConfirmationStatus;

use super::{State, SurfnetRpcContext, SurfpoolWebsocketMeta};
use crate::surfnet::{
    locker::SvmAccessContext, GetAccountResult, GetTransactionResult, SignatureSubscriptionType,
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RpcAccountSubscribeConfig {
    pub commitment: Option<CommitmentConfig>,
    pub encoding: Option<UiAccountEncoding>,
}

#[rpc]
pub trait Rpc {
    type Metadata;

    #[pubsub(
        subscription = "signatureNotification",
        subscribe,
        name = "signatureSubscribe"
    )]
    fn signature_subscribe(
        &self,
        meta: Self::Metadata,
        subscriber: Subscriber<RpcResponse<RpcSignatureResult>>,
        signature_str: String,
        config: Option<RpcSignatureSubscribeConfig>,
    );

    #[pubsub(
        subscription = "signatureNotification",
        unsubscribe,
        name = "signatureUnsubscribe"
    )]
    fn signature_unsubscribe(
        &self,
        meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool>;

    #[pubsub(
        subscription = "accountNotification",
        subscribe,
        name = "accountSubscribe"
    )]
    fn account_subscribe(
        &self,
        meta: Self::Metadata,
        subscriber: Subscriber<RpcResponse<UiAccount>>,
        pubkey_str: String,
        config: Option<RpcAccountSubscribeConfig>,
    );

    #[pubsub(
        subscription = "accountNotification",
        unsubscribe,
        name = "accountUnsubscribe"
    )]
    fn account_unsubscribe(
        &self,
        meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool>;
}

pub struct SurfpoolWsRpc {
    pub uid: atomic::AtomicUsize,
    pub active: Arc<RwLock<HashMap<SubscriptionId, Sink<RpcResponse<RpcSignatureResult>>>>>,
    pub account_active: Arc<RwLock<HashMap<SubscriptionId, Sink<RpcResponse<UiAccount>>>>>,
    pub tokio_handle: tokio::runtime::Handle,
}
impl Rpc for SurfpoolWsRpc {
    type Metadata = Option<SurfpoolWebsocketMeta>;

    fn signature_subscribe(
        &self,
        meta: Self::Metadata,
        subscriber: Subscriber<RpcResponse<RpcSignatureResult>>,
        signature_str: String,
        config: Option<RpcSignatureSubscribeConfig>,
    ) {
        let signature = match Signature::from_str(&signature_str) {
            Ok(sig) => sig,
            Err(_) => {
                let error = Error {
                    code: ErrorCode::InvalidParams,
                    message: "Invalid signature format.".into(),
                    data: None,
                };
                subscriber.reject(error.clone()).unwrap();
                return;
            }
        };
        let config = config.unwrap_or_default();
        let subscription_type = if config.enable_received_notification.unwrap_or(false) {
            SignatureSubscriptionType::Received
        } else {
            SignatureSubscriptionType::Commitment(config.commitment.unwrap_or_default().commitment)
        };

        let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
        let sub_id = SubscriptionId::Number(id as u64);
        let sink = subscriber
            .assign_id(sub_id.clone())
            .expect("Failed to assign subscription ID");

        let active = Arc::clone(&self.active);
        let meta = meta.clone();

        self.tokio_handle.spawn(async move {
            active.write().unwrap().insert(sub_id.clone(), sink);

            let SurfnetRpcContext {
                svm_locker,
                remote_ctx,
            } = match meta.get_rpc_context(None) {
                Ok(res) => res,
                Err(_) => panic!(),
            };

            // get the signature from the SVM to see if it's already been processed
            let SvmAccessContext {
                inner: tx_result, ..
            } = svm_locker.get_transaction(&remote_ctx, &signature).await;

            // if we already had the transaction, check if its confirmation status matches the desired status set by the subscription
            // if so, notify the user and complete the subscription
            // otherwise, subscribe to the transaction updates
            if let GetTransactionResult::FoundTransaction(_, _, tx) = tx_result {
                match (&subscription_type, tx.confirmation_status) {
                    (&SignatureSubscriptionType::Received, _)
                    | (
                        &SignatureSubscriptionType::Commitment(CommitmentLevel::Processed),
                        Some(TransactionConfirmationStatus::Processed),
                    )
                    | (
                        &SignatureSubscriptionType::Commitment(CommitmentLevel::Confirmed),
                        Some(TransactionConfirmationStatus::Confirmed),
                    )
                    | (
                        &SignatureSubscriptionType::Commitment(CommitmentLevel::Finalized),
                        Some(TransactionConfirmationStatus::Finalized),
                    ) => {
                        if let Some(sink) = active.write().unwrap().remove(&sub_id) {
                            let _ = sink.notify(Ok(RpcResponse {
                                context: RpcResponseContext::new(tx.slot),
                                value: RpcSignatureResult::ProcessedSignature(
                                    ProcessedSignatureResult { err: None },
                                ),
                            }));
                        }
                        return;
                    }
                    _ => {}
                }
            }

            // update our surfnet SVM to subscribe to the signature updates
            let rx =
                svm_locker.subscribe_for_signature_updates(&signature, subscription_type.clone());

            loop {
                if let Ok((slot, some_err)) = rx.try_recv() {
                    if let Some(sink) = active.write().unwrap().remove(&sub_id) {
                        match subscription_type {
                            SignatureSubscriptionType::Received => {
                                let _ = sink.notify(Ok(RpcResponse {
                                    context: RpcResponseContext::new(slot),
                                    value: RpcSignatureResult::ReceivedSignature(
                                        ReceivedSignatureResult::ReceivedSignature,
                                    ),
                                }));
                            }
                            SignatureSubscriptionType::Commitment(_) => {
                                let _ = sink.notify(Ok(RpcResponse {
                                    context: RpcResponseContext::new(slot),
                                    value: RpcSignatureResult::ProcessedSignature(
                                        ProcessedSignatureResult { err: some_err },
                                    ),
                                }));
                            }
                        }
                    }
                    return;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        });
    }

    fn signature_unsubscribe(
        &self,
        _meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool> {
        let removed = self.active.write().unwrap().remove(&subscription);
        if removed.is_some() {
            Ok(true)
        } else {
            Err(Error {
                code: ErrorCode::InvalidParams,
                message: "Invalid subscription.".into(),
                data: None,
            })
        }
    }

    fn account_subscribe(
        &self,
        meta: Self::Metadata,
        subscriber: Subscriber<RpcResponse<UiAccount>>,
        pubkey_str: String,
        config: Option<RpcAccountSubscribeConfig>,
    ) {
        let pubkey = match Pubkey::from_str(&pubkey_str) {
            Ok(pk) => pk,
            Err(_) => {
                let error = Error {
                    code: ErrorCode::InvalidParams,
                    message: "Invalid pubkey format.".into(),
                    data: None,
                };
                subscriber.reject(error.clone()).unwrap();
                return;
            }
        };

        let config = config.unwrap_or(RpcAccountSubscribeConfig {
            commitment: None,
            encoding: None,
        });

        let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
        let sub_id = SubscriptionId::Number(id as u64);
        let sink = subscriber
            .assign_id(sub_id.clone())
            .expect("Failed to assign subscription ID");

        let account_active = Arc::clone(&self.account_active);
        let meta = meta.clone();

        self.tokio_handle.spawn(async move {
            account_active.write().unwrap().insert(sub_id.clone(), sink);

            let SurfnetRpcContext { svm_locker, .. } =
                match meta.get_rpc_context(config.commitment.unwrap_or_default()) {
                    Ok(res) => res,
                    Err(_) => panic!(),
                };

            // subscribe to account updates
            let rx = svm_locker.subscribe_for_account_updates(&pubkey, config.encoding);

            loop {
                // if the subscription has been removed, break the loop
                if account_active.read().unwrap().get(&sub_id).is_none() {
                    break;
                }

                if let Ok(ui_account) = rx.try_recv() {
                    if let Some(sink) = account_active.read().unwrap().get(&sub_id) {
                        let _ = sink.notify(Ok(RpcResponse {
                            context: RpcResponseContext::new(
                                svm_locker.with_svm_reader(|svm| svm.get_latest_absolute_slot()),
                            ),
                            value: ui_account,
                        }));
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        });
    }

    fn account_unsubscribe(
        &self,
        _meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool> {
        let removed = self.account_active.write().unwrap().remove(&subscription);
        if removed.is_some() {
            Ok(true)
        } else {
            Err(Error {
                code: ErrorCode::InvalidParams,
                message: "Invalid subscription.".into(),
                data: None,
            })
        }
    }
}
