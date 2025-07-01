use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::response::{
    RpcResponse, RpcResponseContext, RpcSignatureResult, SlotInfo, UiAccountSchema,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Complete mapping of all Surfpool WebSocket RPC endpoints")]
pub struct SurfpoolWebSocketEndpoints {
    pub signature_subscriptions: SignatureSubscriptionEndpoints,
    pub account_subscriptions: AccountSubscriptionEndpoints,
    pub slot_subscriptions: SlotSubscriptionEndpoints,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Signature subscription WebSocket endpoints")]
pub struct SignatureSubscriptionEndpoints {
    #[schemars(description = "signatureSubscribe - Subscribe to signature status notifications")]
    pub signature_subscribe: SignatureSubscribeEndpoint,

    #[schemars(description = "signatureUnsubscribe - Unsubscribe from signature notifications")]
    pub signature_unsubscribe: SignatureUnsubscribeEndpoint,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Signature subscribe endpoint specification")]
pub struct SignatureSubscribeEndpoint {
    #[schemars(description = "Request parameters for signature subscription")]
    pub request: SignatureSubscribeRequest,

    #[schemars(description = "Response type for subscription confirmation")]
    pub response: SubscriptionResponse,

    #[schemars(description = "Notification type sent to subscribers")]
    pub notification: SignatureNotification,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Signature unsubscribe endpoint specification")]
pub struct SignatureUnsubscribeEndpoint {
    #[schemars(description = "Request parameters for signature unsubscription")]
    pub request: UnsubscribeRequest,

    #[schemars(description = "Response indicating unsubscription success")]
    pub response: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Request parameters for signature subscription")]
pub struct SignatureSubscribeRequest {
    #[schemars(description = "Transaction signature to monitor (base-58 encoded)")]
    pub signature: String,

    #[schemars(description = "Optional subscription configuration")]
    pub config: Option<RpcSignatureSubscribeConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Configuration for signature subscription")]
pub struct RpcSignatureSubscribeConfig {
    #[serde(flatten)]
    #[schemars(flatten, description = "Commitment level for notifications")]
    pub commitment: Option<CommitmentConfig>,

    #[schemars(description = "Whether to enable received notification")]
    pub enable_received_notification: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Commitment configuration")]
pub struct CommitmentConfig {
    #[schemars(description = "Commitment level")]
    pub commitment: CommitmentLevel,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[schemars(description = "Commitment levels for transaction confirmation")]
pub enum CommitmentLevel {
    #[schemars(description = "Transaction is processed but not yet confirmed")]
    Processed,
    #[schemars(description = "Transaction is confirmed")]
    Confirmed,
    #[schemars(description = "Transaction is finalized")]
    Finalized,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Signature status notification")]
pub struct SignatureNotification {
    #[schemars(description = "Notification result")]
    pub result: RpcResponse<RpcSignatureResult>,

    #[schemars(description = "Subscription ID")]
    pub subscription: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account subscription WebSocket endpoints")]
pub struct AccountSubscriptionEndpoints {
    #[schemars(description = "accountSubscribe - Subscribe to account change notifications")]
    pub account_subscribe: AccountSubscribeEndpoint,

    #[schemars(description = "accountUnsubscribe - Unsubscribe from account notifications")]
    pub account_unsubscribe: AccountUnsubscribeEndpoint,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account subscribe endpoint specification")]
pub struct AccountSubscribeEndpoint {
    #[schemars(description = "Request parameters for account subscription")]
    pub request: AccountSubscribeRequest,

    #[schemars(description = "Response type for subscription confirmation")]
    pub response: SubscriptionResponse,

    #[schemars(description = "Notification type sent to subscribers")]
    pub notification: AccountNotification,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account unsubscribe endpoint specification")]
pub struct AccountUnsubscribeEndpoint {
    #[schemars(description = "Request parameters for account unsubscription")]
    pub request: UnsubscribeRequest,

    #[schemars(description = "Response indicating unsubscription success")]
    pub response: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Request parameters for account subscription")]
pub struct AccountSubscribeRequest {
    #[schemars(description = "Account public key to monitor (base-58 encoded)")]
    pub pubkey: String,

    #[schemars(description = "Optional subscription configuration")]
    pub config: Option<RpcAccountSubscribeConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Configuration for account subscription")]
pub struct RpcAccountSubscribeConfig {
    #[serde(flatten)]
    #[schemars(flatten, description = "Commitment level for notifications")]
    pub commitment: Option<CommitmentConfig>,

    #[schemars(description = "Encoding format for account data")]
    pub encoding: Option<UiAccountEncoding>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[schemars(description = "Account data encoding formats")]
pub enum UiAccountEncoding {
    #[schemars(description = "Base58 encoding")]
    Base58,
    #[schemars(description = "Base64 encoding")]
    Base64,
    #[schemars(description = "Base64+zstd encoding")]
    #[serde(rename = "base64+zstd")]
    Base64Zstd,
    #[schemars(description = "JSON parsed data")]
    JsonParsed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Account change notification")]
pub struct AccountNotification {
    #[schemars(description = "Notification result")]
    pub result: RpcResponse<UiAccountSchema>,

    #[schemars(description = "Subscription ID")]
    pub subscription: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Slot subscription WebSocket endpoints")]
pub struct SlotSubscriptionEndpoints {
    #[schemars(description = "slotSubscribe - Subscribe to slot change notifications")]
    pub slot_subscribe: SlotSubscribeEndpoint,

    #[schemars(description = "slotUnsubscribe - Unsubscribe from slot notifications")]
    pub slot_unsubscribe: SlotUnsubscribeEndpoint,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Slot subscribe endpoint specification")]
pub struct SlotSubscribeEndpoint {
    #[schemars(description = "Request parameters for slot subscription")]
    pub request: SlotSubscribeRequest,

    #[schemars(description = "Response type for subscription confirmation")]
    pub response: SubscriptionResponse,

    #[schemars(description = "Notification type sent to subscribers")]
    pub notification: SlotNotification,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Slot unsubscribe endpoint specification")]
pub struct SlotUnsubscribeEndpoint {
    #[schemars(description = "Request parameters for slot unsubscription")]
    pub request: UnsubscribeRequest,

    #[schemars(description = "Response indicating unsubscription success")]
    pub response: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Request parameters for slot subscription")]
pub struct SlotSubscribeRequest {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Slot change notification")]
pub struct SlotNotification {
    #[schemars(description = "Notification result")]
    pub result: SlotInfo,

    #[schemars(description = "Subscription ID")]
    pub subscription: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Subscription response containing subscription ID")]
pub struct SubscriptionResponse {
    #[schemars(description = "Unique subscription identifier")]
    pub subscription_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Unsubscribe request parameters")]
pub struct UnsubscribeRequest {
    #[schemars(description = "Subscription ID to remove")]
    pub subscription_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket JSON-RPC request")]
pub struct WebSocketRequest {
    #[schemars(description = "JSON-RPC version")]
    pub jsonrpc: String,

    #[schemars(description = "Request ID")]
    pub id: serde_json::Value,

    #[schemars(description = "Method name")]
    pub method: String,

    #[schemars(description = "Method parameters")]
    pub params: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket JSON-RPC response")]
pub struct WebSocketResponse {
    #[schemars(description = "JSON-RPC version")]
    pub jsonrpc: String,

    #[schemars(description = "Request ID")]
    pub id: serde_json::Value,

    #[schemars(description = "Successful response result")]
    pub result: Option<serde_json::Value>,

    #[schemars(description = "Error response")]
    pub error: Option<WebSocketError>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket JSON-RPC notification")]
pub struct WebSocketNotification {
    #[schemars(description = "JSON-RPC version")]
    pub jsonrpc: String,

    #[schemars(description = "Notification method")]
    pub method: String,

    #[schemars(description = "Notification parameters")]
    pub params: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket JSON-RPC error")]
pub struct WebSocketError {
    #[schemars(description = "Error code")]
    pub code: i32,

    #[schemars(description = "Error message")]
    pub message: String,

    #[schemars(description = "Additional error data")]
    pub data: Option<serde_json::Value>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Complete WebSocket RPC API documentation with examples")]
pub struct SurfpoolWebSocketApiDocumentation {
    #[schemars(description = "All available WebSocket endpoints")]
    pub endpoints: SurfpoolWebSocketEndpoints,

    #[schemars(description = "Protocol-level message formats")]
    pub protocol: WebSocketProtocol,

    #[schemars(description = "Usage examples for each endpoint")]
    pub examples: WebSocketExamples,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "WebSocket protocol message formats")]
pub struct WebSocketProtocol {
    #[schemars(description = "Request message format")]
    pub request: WebSocketRequest,

    #[schemars(description = "Response message format")]
    pub response: WebSocketResponse,

    #[schemars(description = "Notification message format")]
    pub notification: WebSocketNotification,

    #[schemars(description = "Error message format")]
    pub error: WebSocketError,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Example WebSocket interactions")]
pub struct WebSocketExamples {
    #[schemars(description = "Signature subscription example")]
    pub signature_subscription: SignatureSubscriptionExample,

    #[schemars(description = "Account subscription example")]
    pub account_subscription: AccountSubscriptionExample,

    #[schemars(description = "Slot subscription example")]
    pub slot_subscription: SlotSubscriptionExample,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Example signature subscription workflow")]
pub struct SignatureSubscriptionExample {
    #[schemars(description = "Subscribe request")]
    pub subscribe_request: String,

    #[schemars(description = "Subscribe response")]
    pub subscribe_response: String,

    #[schemars(description = "Notification")]
    pub notification: String,

    #[schemars(description = "Unsubscribe request")]
    pub unsubscribe_request: String,

    #[schemars(description = "Unsubscribe response")]
    pub unsubscribe_response: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Example account subscription workflow")]
pub struct AccountSubscriptionExample {
    #[schemars(description = "Subscribe request")]
    pub subscribe_request: String,

    #[schemars(description = "Subscribe response")]
    pub subscribe_response: String,

    #[schemars(description = "Notification")]
    pub notification: String,

    #[schemars(description = "Unsubscribe request")]
    pub unsubscribe_request: String,

    #[schemars(description = "Unsubscribe response")]
    pub unsubscribe_response: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Example slot subscription workflow")]
pub struct SlotSubscriptionExample {
    #[schemars(description = "Subscribe request")]
    pub subscribe_request: String,

    #[schemars(description = "Subscribe response")]
    pub subscribe_response: String,

    #[schemars(description = "Notification")]
    pub notification: String,

    #[schemars(description = "Unsubscribe request")]
    pub unsubscribe_request: String,

    #[schemars(description = "Unsubscribe response")]
    pub unsubscribe_response: String,
}
