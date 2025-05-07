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
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_signature::Signature;

use crate::surfnet::SignatureSubscriptionType;

use super::{State, SurfpoolWebsocketMeta};

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

        let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
        let sub_id = SubscriptionId::Number(id as u64);
        let sink = subscriber
            .assign_id(sub_id.clone())
            .expect("Failed to assign subscription ID");

        let active = Arc::clone(&self.active);
        let meta = meta.clone();

        self.tokio_handle.spawn(async move {
            active.write().unwrap().insert(sub_id.clone(), sink);

            let svm_locker = match meta.get_svm_locker() {
                Ok(res) => res,
                Err(_) => panic!(),
            };

            let mut writer = svm_locker.write().await;

            let res = match writer.get_transaction(&signature, None).await {
                Ok(res) => res,
                Err(e) => {
                    let error = Error {
                        code: ErrorCode::InvalidParams,
                        message: format!("Failed to get transaction from remote: {}", e),
                        data: None,
                    };
                    if let Some(sink) = active.write().unwrap().get(&sub_id) {
                        let _ = sink.notify(Err(error));
                    }

                    return;
                }
            };

            if let Some((_, tx)) = res {
                if let Some(sink) = active.write().unwrap().get(&sub_id) {
                    let _ = sink.notify(Ok(RpcResponse {
                        context: RpcResponseContext::new(tx.slot),
                        value: RpcSignatureResult::ProcessedSignature(ProcessedSignatureResult {
                            err: None,
                        }),
                    }));
                }
            } else {
                let subscription_type = if config.enable_received_notification.unwrap_or(false) {
                    SignatureSubscriptionType::Received
                } else {
                    SignatureSubscriptionType::Commitment(
                        config.commitment.unwrap_or_default().commitment,
                    )
                };
                let rx =
                    writer.subscribe_for_signature_updates(&signature, subscription_type.clone());

                loop {
                    if active.read().unwrap().get(&sub_id).is_none() {
                        return;
                    } else if let Ok((slot, some_err)) = rx.try_recv() {
                        match subscription_type {
                            SignatureSubscriptionType::Received => {
                                if let Some(sink) = active.write().unwrap().get(&sub_id) {
                                    let _ = sink.notify(Ok(RpcResponse {
                                        context: RpcResponseContext::new(slot),
                                        value: RpcSignatureResult::ReceivedSignature(
                                            ReceivedSignatureResult::ReceivedSignature,
                                        ),
                                    }));
                                    return;
                                }
                            }
                            SignatureSubscriptionType::Commitment(_) => {
                                if let Some(sink) = active.write().unwrap().get(&sub_id) {
                                    let _ = sink.notify(Ok(RpcResponse {
                                        context: RpcResponseContext::new(slot),
                                        value: RpcSignatureResult::ProcessedSignature(
                                            ProcessedSignatureResult { err: some_err },
                                        ),
                                    }));
                                    return;
                                }
                            }
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
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
