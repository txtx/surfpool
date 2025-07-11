use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, RwLock, atomic},
};

use jsonrpc_core::{Error, ErrorCode, Result};
use jsonrpc_derive::rpc;
use jsonrpc_pubsub::{
    SubscriptionId,
    typed::{Sink, Subscriber},
};
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use solana_client::{
    rpc_config::RpcSignatureSubscribeConfig,
    rpc_response::{
        ProcessedSignatureResult, ReceivedSignatureResult, RpcResponseContext, RpcSignatureResult,
    },
};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::{Response as RpcResponse, SlotInfo};
use solana_signature::Signature;
use solana_transaction_status::TransactionConfirmationStatus;

use super::{State, SurfnetRpcContext, SurfpoolWebsocketMeta};
use crate::surfnet::{GetTransactionResult, SignatureSubscriptionType, locker::SvmAccessContext};

/// Configuration for account subscription requests.
///
/// This struct defines the parameters that clients can specify when subscribing
/// to account change notifications through WebSocket connections. It allows customization
/// of both the commitment level for updates and the encoding format for account data.
///
/// ## Fields
/// - `commitment`: Optional commitment configuration specifying when to send notifications
///   (processed, confirmed, or finalized). Defaults to the node's default commitment level.
/// - `encoding`: Optional encoding format for account data serialization (base58, base64, jsonParsed, etc.).
///   Defaults to base58 encoding if not specified.
///
/// ## Usage
/// Clients can provide this configuration to customize their subscription behavior:
/// - Set commitment level to control notification timing based on confirmation status
/// - Set encoding to specify the preferred format for receiving account data
///
/// ## Example Usage
/// ```json
/// {
///   "commitment": "confirmed",
///   "encoding": "base64"
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcAccountSubscribeConfig {
    #[serde(flatten)]
    pub commitment: Option<CommitmentConfig>,
    pub encoding: Option<UiAccountEncoding>,
}

#[rpc]
pub trait Rpc {
    type Metadata;

    /// Subscribe to signature status notifications via WebSocket.
    ///
    /// This method allows clients to subscribe to status updates for a specific transaction signature.
    /// The subscriber will receive notifications when the transaction reaches the desired confirmation level
    /// or when it's initially received by the network (if configured).
    ///
    /// ## Parameters
    /// - `meta`: WebSocket metadata containing RPC context and connection information.
    /// - `subscriber`: The subscription sink for sending signature status notifications to the client.
    /// - `signature_str`: The transaction signature to monitor, as a base-58 encoded string.
    /// - `config`: Optional configuration specifying commitment level and notification preferences.
    ///
    /// ## Returns
    /// This method does not return a value directly. Instead, it establishes a WebSocket subscription
    /// that will send `RpcResponse<RpcSignatureResult>` notifications to the subscriber when the
    /// transaction status changes.
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "signatureSubscribe",
    ///   "params": [
    ///     "2id3YC2jK9G5Wo2phDx4gJVAew8DcY5NAojnVuao8rkxwPYPe8cSwE5GzhEgJA2y8fVjDEo6iR6ykBvDxrTQrtpb",
    ///     {
    ///       "commitment": "finalized",
    ///       "enableReceivedNotification": false
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response (Subscription Confirmation)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": 0,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Example WebSocket Notification
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "method": "signatureNotification",
    ///   "params": {
    ///     "result": {
    ///       "context": {
    ///         "slot": 5207624
    ///       },
    ///       "value": {
    ///         "err": null
    ///       }
    ///     },
    ///     "subscription": 0
    ///   }
    /// }
    /// ```
    ///
    /// ## Notes
    /// - If the transaction already exists with the desired confirmation status, the subscriber
    ///   will be notified immediately and the subscription will complete.
    /// - The subscription automatically terminates after sending the first matching notification.
    /// - Invalid signature formats will cause the subscription to be rejected with an error.
    /// - Each subscription runs in its own async task for optimal performance.
    ///
    /// ## See Also
    /// - `signatureUnsubscribe`: Remove an active signature subscription
    /// - `getSignatureStatuses`: Get current status of multiple signatures
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

    /// Unsubscribe from signature status notifications.
    ///
    /// This method removes an active signature subscription, stopping further notifications
    /// for the specified subscription ID.
    ///
    /// ## Parameters
    /// - `meta`: Optional WebSocket metadata containing connection information.
    /// - `subscription`: The subscription ID to remove, as returned by `signatureSubscribe`.
    ///
    /// ## Returns
    /// A `Result<bool>` indicating whether the unsubscription was successful:
    /// - `Ok(true)` if the subscription was successfully removed
    /// - `Err(Error)` with `InvalidParams` if the subscription ID doesn't exist
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "signatureUnsubscribe",
    ///   "params": [0]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": true,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Notes
    /// - Attempting to unsubscribe from a non-existent subscription will return an error.
    /// - Successfully unsubscribed connections will no longer receive notifications.
    /// - This method is thread-safe and can be called concurrently.
    ///
    /// ## See Also
    /// - `signatureSubscribe`: Create a signature status subscription
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

    /// Subscribe to account change notifications via WebSocket.
    ///
    /// This method allows clients to subscribe to updates for a specific account.
    /// The subscriber will receive notifications whenever the account's data, lamports balance,
    /// ownership, or other properties change.
    ///
    /// ## Parameters
    /// - `meta`: WebSocket metadata containing RPC context and connection information.
    /// - `subscriber`: The subscription sink for sending account update notifications to the client.
    /// - `pubkey_str`: The account public key to monitor, as a base-58 encoded string.
    /// - `config`: Optional configuration specifying commitment level and encoding format for account data.
    ///
    /// ## Returns
    /// This method does not return a value directly. Instead, it establishes a continuous WebSocket
    /// subscription that will send `RpcResponse<UiAccount>` notifications to the subscriber whenever
    /// the account state changes.
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "accountSubscribe",
    ///   "params": [
    ///     "CM78CPUeXjn8o3yroDHxUtKsZZgoy4GPkPPXfouKNH12",
    ///     {
    ///       "commitment": "finalized",
    ///       "encoding": "base64"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response (Subscription Confirmation)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": 23784,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Example WebSocket Notification
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "method": "accountNotification",
    ///   "params": {
    ///     "result": {
    ///       "context": {
    ///         "slot": 5208469
    ///       },
    ///       "value": {
    ///         "data": ["base64EncodedAccountData", "base64"],
    ///         "executable": false,
    ///         "lamports": 33594,
    ///         "owner": "11111111111111111111111111111112",
    ///         "rentEpoch": 636
    ///       }
    ///     },
    ///     "subscription": 23784
    ///   }
    /// }
    /// ```
    ///
    /// ## Notes
    /// - The subscription remains active until explicitly unsubscribed or the connection is closed.
    /// - Account notifications are sent whenever any aspect of the account changes.
    /// - The encoding format specified in the config determines how account data is serialized.
    /// - Invalid public key formats will cause the subscription to be rejected with an error.
    /// - Each subscription runs in its own async task to ensure optimal performance.
    ///
    /// ## See Also
    /// - `accountUnsubscribe`: Remove an active account subscription
    /// - `getAccountInfo`: Get current account information
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

    /// Unsubscribe from account change notifications.
    ///
    /// This method removes an active account subscription, stopping further notifications
    /// for the specified subscription ID. The monitoring task will automatically terminate
    /// when the subscription is removed.
    ///
    /// ## Parameters
    /// - `meta`: Optional WebSocket metadata containing connection information.
    /// - `subscription`: The subscription ID to remove, as returned by `accountSubscribe`.
    ///
    /// ## Returns
    /// A `Result<bool>` indicating whether the unsubscription was successful:
    /// - `Ok(true)` if the subscription was successfully removed
    /// - `Err(Error)` with `InvalidParams` if the subscription ID doesn't exist
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "accountUnsubscribe",
    ///   "params": [23784]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": true,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Notes
    /// - Attempting to unsubscribe from a non-existent subscription will return an error.
    /// - Successfully unsubscribed connections will no longer receive account notifications.
    /// - The monitoring task automatically detects subscription removal and terminates gracefully.
    /// - This method is thread-safe and can be called concurrently.
    ///
    /// ## See Also
    /// - `accountSubscribe`: Create an account change subscription
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

    /// Subscribe to slot notifications.
    ///
    /// This method allows clients to subscribe to updates for a specific slot.
    /// The subscriber will receive notifications whenever the slot changes.
    ///
    /// ## Parameters
    /// - `meta`: WebSocket metadata containing RPC context and connection information.
    /// - `subscriber`: The subscription sink for sending slot update notifications to the client.
    ///
    /// ## Returns
    /// This method does not return a value directly. Instead, it establishes a continuous WebSocket
    /// subscription that will send `SlotInfo` notifications to the subscriber whenever
    /// the slot changes.
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "slotSubscribe",
    ///   "params": [
    ///     {
    ///       "commitment": "finalized"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response (Subscription Confirmation)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": 5207624,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Example WebSocket Notification
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "method": "slotNotification",
    ///   "params": {
    ///     "result": {
    ///       "slot": 5207624
    ///     },
    ///     "subscription": 5207624
    ///   }
    /// }
    /// ```
    ///
    /// ## Notes
    /// - The subscription remains active until explicitly unsubscribed or the connection is closed.
    /// - Slot notifications are sent whenever the slot changes.
    /// - The subscription automatically terminates when the slot changes.
    /// - Each subscription runs in its own async task for optimal performance.
    ///
    /// ## See Also
    /// - `slotUnsubscribe`: Remove an active slot subscription
    #[pubsub(subscription = "slotNotification", subscribe, name = "slotSubscribe")]
    fn slot_subscribe(&self, meta: Self::Metadata, subscriber: Subscriber<SlotInfo>);

    /// Unsubscribe from slot notifications.
    ///
    /// This method removes an active slot subscription, stopping further notifications
    /// for the specified subscription ID.
    ///
    /// ## Parameters
    /// - `meta`: Optional WebSocket metadata containing connection information.
    /// - `subscription`: The subscription ID to remove, as returned by `slotSubscribe`.
    ///
    /// ## Returns
    /// A `Result<bool>` indicating whether the unsubscription was successful:
    /// - `Ok(true)` if the subscription was successfully removed
    /// - `Err(Error)` with `InvalidParams` if the subscription ID doesn't exist
    ///
    /// ## Example WebSocket Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "slotUnsubscribe",
    ///   "params": [0]
    /// }
    /// ```
    ///
    /// ## Example WebSocket Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": true,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Notes
    /// - Attempting to unsubscribe from a non-existent subscription will return an error.
    /// - Successfully unsubscribed connections will no longer receive notifications.
    /// - This method is thread-safe and can be called concurrently.
    ///
    /// ## See Also
    /// - `slotSubscribe`: Create a slot subscription
    #[pubsub(
        subscription = "slotNotification",
        unsubscribe,
        name = "slotUnsubscribe"
    )]
    fn slot_unsubscribe(
        &self,
        meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool>;
}

/// WebSocket RPC server implementation for Surfpool.
///
/// This struct manages WebSocket subscriptions for both signature status updates
/// and account change notifications in the Surfpool environment. It provides a complete
/// WebSocket RPC interface that allows clients to subscribe to real-time updates
/// from the Solana Virtual Machine (SVM) and handles the lifecycle of WebSocket connections.
///
/// ## Fields
/// - `uid`: Atomic counter for generating unique subscription IDs across all subscription types.
/// - `signature_subscription_map`: Thread-safe HashMap containing active signature subscriptions, mapping subscription IDs to their notification sinks.
/// - `account_subscription_map`: Thread-safe HashMap containing active account subscriptions, mapping subscription IDs to their notification sinks.
/// - `slot_subscription_map`: Thread-safe HashMap containing active slot subscriptions, mapping subscription IDs to their notification sinks.
/// - `tokio_handle`: Runtime handle for spawning asynchronous subscription monitoring tasks.
///
/// ## Features
/// - **Concurrent Subscriptions**: Supports multiple simultaneous subscriptions without blocking.
/// - **Thread Safety**: All subscription management operations are thread-safe using RwLock.
/// - **Automatic Cleanup**: Subscriptions are automatically cleaned up when completed or unsubscribed.
/// - **Efficient Monitoring**: Each subscription runs in its own async task for optimal performance.
/// - **Real-time Updates**: Provides immediate notifications when monitored conditions are met.
///
/// ## Usage
/// This struct implements the `Rpc` trait and is typically used as part of a larger
/// WebSocket server infrastructure to provide real-time blockchain data to clients.
///
/// ## Notes
/// - Each subscription is assigned a unique numeric ID for tracking and management.
/// - The struct maintains separate maps for different subscription types to optimize performance.
/// - All async operations are managed through the provided Tokio runtime handle.
///
/// ## See Also
/// - `Rpc`: The trait interface this struct implements
/// - `RpcAccountSubscribeConfig`: Configuration options for account subscriptions
pub struct SurfpoolWsRpc {
    pub uid: atomic::AtomicUsize,
    pub signature_subscription_map:
        Arc<RwLock<HashMap<SubscriptionId, Sink<RpcResponse<RpcSignatureResult>>>>>,
    pub account_subscription_map:
        Arc<RwLock<HashMap<SubscriptionId, Sink<RpcResponse<UiAccount>>>>>,
    pub slot_subscription_map: Arc<RwLock<HashMap<SubscriptionId, Sink<SlotInfo>>>>,
    pub tokio_handle: tokio::runtime::Handle,
}

impl Rpc for SurfpoolWsRpc {
    type Metadata = Option<SurfpoolWebsocketMeta>;

    /// Implementation of signature subscription for WebSocket clients.
    ///
    /// This method handles the complete lifecycle of signature subscriptions:
    /// 1. Validates the provided signature string format
    /// 2. Determines the subscription type (received vs commitment-based)
    /// 3. Checks if the transaction already exists in the desired state
    /// 4. If found and confirmed, immediately notifies the subscriber
    /// 5. Otherwise, sets up a continuous monitoring loop
    /// 6. Spawns an async task to handle ongoing subscription management
    ///
    /// # Error Handling
    /// - Rejects subscription with `InvalidParams` for malformed signatures
    /// - Handles RPC context retrieval failures
    /// - Manages subscription cleanup on completion or failure
    ///
    /// # Concurrency
    /// Each subscription runs in its own async task, allowing multiple
    /// concurrent subscriptions without blocking each other.
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
                if let Err(e) = subscriber.reject(error.clone()) {
                    log::error!("Failed to reject subscriber: {:?}", e);
                }
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
        let sink = match subscriber.assign_id(sub_id.clone()) {
            Ok(sink) => sink,
            Err(e) => {
                log::error!("Failed to assign subscription ID: {:?}", e);
                return;
            }
        };
        let active = Arc::clone(&self.signature_subscription_map);
        let meta = meta.clone();

        self.tokio_handle.spawn(async move {
            if let Ok(mut guard) = active.write() {
                guard.insert(sub_id.clone(), sink);
            } else {
                log::error!("Failed to acquire write lock on signature_subscription_map");
                return;
            }

            let SurfnetRpcContext {
                svm_locker,
                remote_ctx,
            } = match meta.get_rpc_context(None) {
                Ok(res) => res,
                Err(e) => {
                    log::error!("Failed to get RPC context: {:?}", e);
                    if let Ok(mut guard) = active.write() {
                        if let Some(sink) = guard.remove(&sub_id) {
                            if let Err(e) = sink.notify(Err(e.into())) {
                                log::error!("Failed to notify client about RPC context error: {e}");
                            }
                        }
                    }
                    return;
                }
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
                        if let Ok(mut guard) = active.write() {
                            if let Some(sink) = guard.remove(&sub_id) {
                                let _ = sink.notify(Ok(RpcResponse {
                                    context: RpcResponseContext::new(tx.slot),
                                    value: RpcSignatureResult::ProcessedSignature(
                                        ProcessedSignatureResult { err: None },
                                    ),
                                }));
                            }
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
                    if let Ok(mut guard) = active.write() {
                        if let Some(sink) = guard.remove(&sub_id) {
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
                    }
                    return;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        });
    }

    /// Implementation of signature unsubscription for WebSocket clients.
    ///
    /// This method removes an active signature subscription from the internal
    /// tracking maps, effectively stopping further notifications for that subscription.
    ///
    /// # Implementation Details
    /// - Attempts to remove the subscription from the active subscriptions map
    /// - Returns success if the subscription existed and was removed
    /// - Returns an error if the subscription ID was not found
    ///
    /// # Thread Safety
    /// Uses write locks to ensure thread-safe removal from the subscription map.
    fn signature_unsubscribe(
        &self,
        _meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool> {
        let removed = if let Ok(mut guard) = self.signature_subscription_map.write() {
            guard.remove(&subscription)
        } else {
            log::error!("Failed to acquire write lock on signature_subscription_map");
            None
        };
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

    /// Implementation of account subscription for WebSocket clients.
    ///
    /// This method handles the complete lifecycle of account subscriptions:
    /// 1. Validates the provided public key string format
    /// 2. Parses the subscription configuration (commitment and encoding)
    /// 3. Generates a unique subscription ID and assigns it to the subscriber
    /// 4. Spawns an async task to continuously monitor account changes
    /// 5. Sends notifications whenever the account state changes
    ///
    /// # Monitoring Loop
    /// The spawned task runs a continuous loop that:
    /// - Checks if the subscription is still active (not unsubscribed)
    /// - Polls for account updates from the SVM
    /// - Sends notifications to the subscriber when changes occur
    /// - Automatically terminates when the subscription is removed
    ///
    /// # Error Handling
    /// - Rejects subscription with `InvalidParams` for malformed public keys
    /// - Handles encoding configuration for account data serialization
    /// - Manages subscription cleanup through the monitoring loop
    ///
    /// # Performance
    /// Uses efficient polling with minimal CPU overhead and automatic
    /// cleanup when subscriptions are no longer needed.
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
                if subscriber.reject(error.clone()).is_err() {
                    log::error!("Failed to reject subscriber for invalid pubkey format.");
                }
                return;
            }
        };

        let config = config.unwrap_or(RpcAccountSubscribeConfig {
            commitment: None,
            encoding: None,
        });

        let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
        let sub_id = SubscriptionId::Number(id as u64);
        let sink = match subscriber.assign_id(sub_id.clone()) {
            Ok(sink) => sink,
            Err(e) => {
                log::error!("Failed to assign subscription ID: {:?}", e);
                return;
            }
        };

        let account_active = Arc::clone(&self.account_subscription_map);
        let meta = meta.clone();
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => {
                log::error!("Failed to get SVM locker for account subscription: {e}");
                if let Err(e) = sink.notify(Err(e.into())) {
                    log::error!(
                        "Failed to send error notification to client for SVM locker failure: {e}"
                    );
                }
                return;
            }
        };
        let slot = svm_locker.with_svm_reader(|svm| svm.get_latest_absolute_slot());

        self.tokio_handle.spawn(async move {
            if let Ok(mut guard) = account_active.write() {
                guard.insert(sub_id.clone(), sink);
            } else {
                log::error!("Failed to acquire write lock on account_subscription_map");
                return;
            }

            // subscribe to account updates
            let rx = svm_locker.subscribe_for_account_updates(&pubkey, config.encoding);

            loop {
                // if the subscription has been removed, break the loop
                if let Ok(guard) = account_active.read() {
                    if guard.get(&sub_id).is_none() {
                        break;
                    }
                } else {
                    log::error!("Failed to acquire read lock on account_subscription_map");
                    break;
                }

                if let Ok(ui_account) = rx.try_recv() {
                    if let Ok(guard) = account_active.read() {
                        if let Some(sink) = guard.get(&sub_id) {
                            let _ = sink.notify(Ok(RpcResponse {
                                context: RpcResponseContext::new(slot),
                                value: ui_account,
                            }));
                        }
                    }
                }
            }
        });
    }

    /// Implementation of account unsubscription for WebSocket clients.
    ///
    /// This method removes an active account subscription from the internal
    /// tracking maps, effectively stopping further notifications for that subscription.
    /// The monitoring loop in the corresponding subscription task will detect this
    /// removal and automatically terminate.
    ///
    /// # Implementation Details
    /// - Attempts to remove the subscription from the account subscriptions map
    /// - Returns success if the subscription existed and was removed
    /// - Returns an error if the subscription ID was not found
    /// - The removal triggers automatic cleanup of the monitoring task
    ///
    /// # Thread Safety
    /// Uses write locks to ensure thread-safe removal from the subscription map.
    /// The monitoring task uses read locks to check subscription status, creating
    /// a clean synchronization pattern.
    fn account_unsubscribe(
        &self,
        _meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool> {
        let removed = if let Ok(mut guard) = self.account_subscription_map.write() {
            guard.remove(&subscription)
        } else {
            log::error!("Failed to acquire write lock on account_subscription_map");
            None
        };
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

    fn slot_subscribe(&self, meta: Self::Metadata, subscriber: Subscriber<SlotInfo>) {
        let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
        let sub_id = SubscriptionId::Number(id as u64);
        let sink = match subscriber.assign_id(sub_id.clone()) {
            Ok(sink) => sink,
            Err(e) => {
                log::error!("Failed to assign subscription ID: {:?}", e);
                return;
            }
        };

        let slot_active = Arc::clone(&self.slot_subscription_map);
        let meta = meta.clone();

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => {
                log::error!("Failed to get SVM locker for slot subscription: {e}");
                if let Err(e) = sink.notify(Err(e.into())) {
                    log::error!(
                        "Failed to send error notification to client for SVM locker failure: {e}"
                    );
                }
                return;
            }
        };

        self.tokio_handle.spawn(async move {
            if let Ok(mut guard) = slot_active.write() {
                guard.insert(sub_id.clone(), sink);
            } else {
                log::error!("Failed to acquire write lock on slot_subscription_map");
                return;
            }

            let rx = svm_locker.subscribe_for_slot_updates();

            loop {
                // if the subscription has been removed, break the loop
                if let Ok(guard) = slot_active.read() {
                    if guard.get(&sub_id).is_none() {
                        break;
                    }
                } else {
                    log::error!("Failed to acquire read lock on slot_subscription_map");
                    break;
                }

                if let Ok(slot_info) = rx.try_recv() {
                    if let Ok(guard) = slot_active.read() {
                        if let Some(sink) = guard.get(&sub_id) {
                            let _ = sink.notify(Ok(slot_info));
                        }
                    }
                }
            }
        });
    }

    fn slot_unsubscribe(
        &self,
        _meta: Option<Self::Metadata>,
        subscription: SubscriptionId,
    ) -> Result<bool> {
        let removed = if let Ok(mut guard) = self.slot_subscription_map.write() {
            guard.remove(&subscription)
        } else {
            log::error!("Failed to acquire write lock on slot_subscription_map");
            None
        };
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
