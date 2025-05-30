use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{atomic, Arc, RwLock};

use jsonrpc_core::Result;
use jsonrpc_core::{Error, ErrorCode};
use jsonrpc_derive::rpc;
use jsonrpc_pubsub::typed::{Sink, Subscriber};
use jsonrpc_pubsub::SubscriptionId;
use solana_client::rpc_config::RpcSignatureSubscribeConfig;
use solana_client::rpc_response::{
    ProcessedSignatureResult, ReceivedSignatureResult, RpcResponseContext, RpcSignatureResult,
};
use solana_commitment_config::CommitmentLevel;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_signature::Signature;
use solana_transaction_status::TransactionConfirmationStatus;

use crate::surfnet::locker::SvmAccessContext;
use crate::surfnet::{GetTransactionResult, SignatureSubscriptionType};

use super::{State, SurfnetRpcContext, SurfpoolWebsocketMeta};

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
}

pub struct SurfpoolWsRpc {
    pub uid: atomic::AtomicUsize,
    pub active: Arc<RwLock<HashMap<SubscriptionId, Sink<RpcResponse<RpcSignatureResult>>>>>,
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
}
