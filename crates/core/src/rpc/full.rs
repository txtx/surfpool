use std::str::FromStr;

use itertools::Itertools;
use jsonrpc_core::{BoxFuture, Error, Result};
use jsonrpc_derive::rpc;
use solana_account_decoder::{encode_ui_account, UiAccountEncoding};
use solana_client::{
    rpc_config::{
        RpcBlockConfig, RpcBlocksConfigWrapper, RpcContextConfig, RpcEncodingConfigWrapper,
        RpcEpochConfig, RpcRequestAirdropConfig, RpcSendTransactionConfig,
        RpcSignatureStatusConfig, RpcSignaturesForAddressConfig, RpcSimulateTransactionConfig,
        RpcTransactionConfig,
    },
    rpc_custom_error::RpcCustomError,
    rpc_response::{
        RpcApiVersion, RpcBlockhash, RpcConfirmedTransactionStatusWithSignature, RpcContactInfo,
        RpcInflationReward, RpcPerfSample, RpcPrioritizationFee, RpcResponseContext,
        RpcSimulateTransactionResult,
    },
};
use solana_clock::{Slot, UnixTimestamp};
use solana_commitment_config::CommitmentConfig;
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::{
    compute_budget::{self, ComputeBudgetInstruction},
    instruction::CompiledInstruction,
};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, TransactionBinaryEncoding, TransactionStatus,
    UiConfirmedBlock, UiTransactionEncoding,
};
use surfpool_types::{SimnetCommand, TransactionStatusEvent};

use super::{
    not_implemented_err, not_implemented_err_async,
    utils::{decode_and_deserialize, transform_tx_metadata_to_ui_accounts, verify_pubkey},
    RunloopContext, State, SurfnetRpcContext,
};
use crate::{
    error::{SurfpoolError, SurfpoolResult},
    surfnet::{locker::SvmAccessContext, GetTransactionResult},
    types::SurfnetTransactionStatus,
};

const MAX_PRIORITIZATION_FEE_BLOCKS_CACHE: usize = 150;

#[rpc]
pub trait Full {
    type Metadata;

    /// Retrieves inflation rewards for a list of addresses over a specified epoch or context.
    ///
    /// This RPC method allows you to query the inflation rewards credited to specific validator or voter addresses
    /// in a given epoch or range of slots. The rewards are provided as lamports, which are the smallest unit of SOL.
    ///
    /// ## Parameters
    /// - `address_strs`: A list of base-58 encoded public keys for which to query inflation rewards.
    /// - `config`: An optional configuration that allows you to specify:
    ///     - `epoch`: The epoch to query for inflation rewards. If `None`, the current epoch is used.
    ///     - `commitment`: The optional commitment level to use when querying for rewards.
    ///     - `min_context_slot`: The minimum slot to be considered when retrieving the rewards.
    ///
    /// ## Returns
    /// - `BoxFuture<Result<Vec<Option<RpcInflationReward>>>>`: A future that resolves to a vector of inflation reward information for each address provided.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getInflationReward",
    ///   "params": [
    ///     ["3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U", "BBh1FwXts8EZY6rPZ5kS2ygq99wYjFd5K5daRjc7eF9X"],
    ///     {
    ///       "epoch": 200,
    ///       "commitment": {"commitment": "finalized"}
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "epoch": 200,
    ///       "effectiveSlot": 123456,
    ///       "amount": 5000000,
    ///       "postBalance": 1000000000,
    ///       "commission": 10
    ///     },
    ///     null
    ///   ]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The `address_strs` parameter should contain the list of addresses for which to query rewards.
    /// - The response is a vector where each entry corresponds to an address in the `address_strs` input list.
    /// - If an address did not receive any reward during the query period, its corresponding entry in the result will be `null`.
    /// - The `amount` field represents the inflation reward (in lamports) that was credited to the address during the epoch.
    /// - The `post_balance` field represents the account balance after the reward was applied.
    /// - The `commission` field, if present, indicates the percentage commission (as an integer) for a vote account when the reward was credited.
    ///
    /// ## Example Response Interpretation
    /// - In the example response, the first address `3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U` received 5,000,000 lamports during epoch 200, with a post-reward balance of 1,000,000,000 lamports and a 10% commission.
    /// - The second address did not receive any inflation reward (represented as `null`).
    #[rpc(meta, name = "getInflationReward")]
    fn get_inflation_reward(
        &self,
        meta: Self::Metadata,
        address_strs: Vec<String>,
        config: Option<RpcEpochConfig>,
    ) -> BoxFuture<Result<Vec<Option<RpcInflationReward>>>>;

    /// Retrieves the list of cluster nodes and their contact information.
    ///
    /// This RPC method returns a list of nodes in the cluster, including their public keys and various
    /// communication ports, such as the gossip, Tpu, and RPC ports. This information is essential for
    /// understanding the connectivity and configuration of nodes in a Solana cluster.
    ///
    /// ## Returns
    /// - `Result<Vec<RpcContactInfo>>`: A result containing a vector of `RpcContactInfo` objects, each representing a node's contact information in the cluster.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getClusterNodes",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "pubkey": "3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U",
    ///       "gossip": "127.0.0.1:8001",
    ///       "tvu": "127.0.0.1:8002",
    ///       "tpu": "127.0.0.1:8003",
    ///       "tpu_quic": "127.0.0.1:8004",
    ///       "rpc": "127.0.0.1:8899",
    ///       "pubsub": "127.0.0.1:8900",
    ///       "version": "v1.9.0",
    ///       "feature_set": 1,
    ///       "shred_version": 3
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The response contains a list of nodes, each identified by its public key and with multiple optional ports for different services.
    /// - If a port is not configured, its value will be `null`.
    /// - The `version` field contains the software version of the node.
    #[rpc(meta, name = "getClusterNodes")]
    fn get_cluster_nodes(&self, meta: Self::Metadata) -> Result<Vec<RpcContactInfo>>;

    /// Retrieves recent performance samples of the Solana network.
    ///
    /// This RPC method provides performance metrics from the most recent samples, such as the number
    /// of transactions processed, slots, and the period over which these metrics were collected.
    ///
    /// ## Parameters
    /// - `limit`: An optional parameter that specifies the maximum number of performance samples to return. If not provided, all available samples will be returned.
    ///
    /// ## Returns
    /// - `Result<Vec<RpcPerfSample>>`: A result containing a vector of `RpcPerfSample` objects, each representing a snapshot of the network's performance for a particular slot.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getRecentPerformanceSamples",
    ///   "params": [10]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "slot": 12345,
    ///       "num_transactions": 1000,
    ///       "num_non_vote_transactions": 800,
    ///       "num_slots": 10,
    ///       "sample_period_secs": 60
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The `num_transactions` field represents the total number of transactions processed in the given slot.
    /// - The `num_non_vote_transactions` field is optional and represents the number of transactions that are not related to voting.
    /// - The `num_slots` field indicates the number of slots sampled for the given period.
    /// - The `sample_period_secs` represents the time period in seconds over which the performance sample was taken.
    #[rpc(meta, name = "getRecentPerformanceSamples")]
    fn get_recent_performance_samples(
        &self,
        meta: Self::Metadata,
        limit: Option<usize>,
    ) -> Result<Vec<RpcPerfSample>>;

    /// Retrieves the status of multiple transactions given their signatures.
    ///
    /// This RPC call returns the status of transactions, including details such as the transaction's
    /// slot, the number of confirmations it has, its success or failure status, and any errors that might have occurred.
    /// Optionally, it can also provide transaction history search results based on the provided configuration.
    ///
    /// ## Parameters
    /// - `signatureStrs`: A list of base-58 encoded transaction signatures for which the statuses are to be retrieved.
    /// - `config`: An optional configuration object to modify the query, such as enabling search for transaction history.
    ///   - If `None`, defaults to querying the current status of the provided transactions.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: A list of transaction statuses corresponding to the provided transaction signatures. Each entry in the list can be:
    ///   - A successful status (`status` field set to `"Ok"`)
    ///   - An error status (`status` field set to `"Err"`)
    ///   - A transaction's error information (e.g., `InsufficientFundsForFee`, `AccountNotFound`, etc.)
    ///   - The slot in which the transaction was processed.
    ///   - The number of confirmations the transaction has received (if applicable).
    ///   - The confirmation status (`"processed"`, `"confirmed"`, or `"finalized"`).
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getSignatureStatuses",
    ///   "params": [
    ///     [
    ///       "5FJkGv5JrMwWe6Eqn24Lz6vgsJ9y8g4rVZn3z9pKfqGhWR23Zef5GjS6SCN8h4J7rb42yYoA4m83d5V7A2KhQkm3",
    ///       "5eJZXh7FnSeFw5uJ5t9t5bjsKqS7khtjeFu6gAtfhsNj5fQYs5KZ5ZscknzFhfQj2rNJ4W2QqijKsyZk8tqbrT9m"
    ///     ],
    ///     {
    ///       "searchTransactionHistory": true
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "value": [
    ///       {
    ///         "slot": 1234567,
    ///         "confirmations": 5,
    ///         "status": {
    ///           "ok": {}
    ///         },
    ///         "err": null,
    ///         "confirmationStatus": "confirmed"
    ///       },
    ///       {
    ///         "slot": 1234568,
    ///         "confirmations": 3,
    ///         "status": {
    ///           "err": {
    ///             "insufficientFundsForFee": {}
    ///           }
    ///         },
    ///         "err": {
    ///           "insufficientFundsForFee": {}
    ///         },
    ///         "confirmationStatus": "processed"
    ///       }
    ///     ]
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if there was an issue processing the request, such as network failures or invalid signatures.
    ///
    /// # Notes
    /// - The `TransactionStatus` contains various error types (e.g., `TransactionError`) and confirmation statuses (e.g., `TransactionConfirmationStatus`), which can be used to determine the cause of failure or the progress of the transaction's confirmation.
    ///
    /// # See Also
    /// - [`TransactionStatus`](#TransactionStatus)
    /// - [`RpcSignatureStatusConfig`](#RpcSignatureStatusConfig)
    /// - [`TransactionError`](#TransactionError)
    /// - [`TransactionConfirmationStatus`](#TransactionConfirmationStatus)
    #[rpc(meta, name = "getSignatureStatuses")]
    fn get_signature_statuses(
        &self,
        meta: Self::Metadata,
        signature_strs: Vec<String>,
        config: Option<RpcSignatureStatusConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<TransactionStatus>>>>>;

    /// Retrieves the maximum slot number for which data may be retransmitted.
    ///
    /// This RPC call returns the highest slot that can be retransmitted in the cluster, typically
    /// representing the latest possible slot that may still be valid for network retransmissions.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: The maximum slot number available for retransmission. This is an integer value representing the highest slot
    ///   for which data can be retrieved or retransmitted from the network.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getMaxRetransmitSlot",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "value": 1234567
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if there was an issue processing the request, such as network failure.
    ///
    /// # Notes
    /// - The slot number returned by this RPC call can be used to identify the highest valid slot for retransmission,
    ///   which may be useful for managing data synchronization across nodes in the cluster.
    ///
    /// # See Also
    /// - `getSlot`
    #[rpc(meta, name = "getMaxRetransmitSlot")]
    fn get_max_retransmit_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    /// Retrieves the maximum slot number for which shreds may be inserted into the ledger.
    ///
    /// This RPC call returns the highest slot for which data can still be inserted (shredded) into the ledger,
    /// typically indicating the most recent slot that can be included in the block production process.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: The maximum slot number for which shreds can be inserted. This is an integer value that represents
    ///   the latest valid slot for including data in the ledger.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getMaxShredInsertSlot",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "value": 1234567
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if there was an issue processing the request, such as network failure.
    ///
    /// # Notes
    /// - This method is used to identify the highest slot where data can still be added to the ledger.
    ///   This is useful for managing the block insertion process and synchronizing data across the network.
    ///
    /// # See Also
    /// - `getSlot`
    #[rpc(meta, name = "getMaxShredInsertSlot")]
    fn get_max_shred_insert_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    /// Requests an airdrop of lamports to the specified public key.
    ///
    /// This RPC call triggers the network to send a specified amount of lamports to the given public key.
    /// It is commonly used for testing or initial setup of accounts.
    ///
    /// ## Parameters
    /// - `pubkeyStr`: The public key (as a base-58 encoded string) to which the airdrop will be sent.
    /// - `lamports`: The amount of lamports to be sent. This is the smallest unit of the native cryptocurrency.
    /// - `config`: Optional configuration for the airdrop request.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: A string representing the transaction signature for the airdrop request. This signature can be
    ///   used to track the status of the transaction.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "requestAirdrop",
    ///   "params": [
    ///     "PublicKeyHere",
    ///     1000000,
    ///     {}
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": "TransactionSignatureHere"
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if there is an issue with the airdrop request, such as invalid public key or insufficient funds.
    ///
    /// # Notes
    /// - Airdrop requests are commonly used for testing or initializing accounts in the development environment.
    ///   This is not typically used in a production environment where real funds are at stake.
    ///
    /// # See Also
    /// - `getBalance`
    #[rpc(meta, name = "requestAirdrop")]
    fn request_airdrop(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        lamports: u64,
        config: Option<RpcRequestAirdropConfig>,
    ) -> Result<String>;

    /// Sends a transaction to the network.
    ///
    /// This RPC method is used to submit a signed transaction to the network for processing.
    /// The transaction will be broadcast to the network, and the method returns a transaction signature
    /// that can be used to track the transaction's status.
    ///
    /// ## Parameters
    /// - `data`: The serialized transaction data in a specified encoding format.
    /// - `config`: Optional configuration for the transaction submission, including settings for retries, commitment level,
    ///   and encoding.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: A string representing the transaction signature for the submitted transaction.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "sendTransaction",
    ///   "params": [
    ///     "TransactionDataHere",
    ///     {
    ///       "skipPreflight": false,
    ///       "preflightCommitment": "processed",
    ///       "encoding": "base64",
    ///       "maxRetries": 3
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": "TransactionSignatureHere"
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the transaction fails to send, such as network issues or invalid transaction data.
    ///
    /// # Notes
    /// - This method is primarily used for submitting a signed transaction to the network and obtaining a signature
    ///   to track the transaction's status.
    /// - The `skipPreflight` option, if set to true, bypasses the preflight checks to speed up the transaction submission.
    ///
    /// # See Also
    /// - `getTransactionStatus`
    #[rpc(meta, name = "sendTransaction")]
    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String>;

    /// Simulates a transaction without sending it to the network.
    ///
    /// This RPC method simulates a transaction locally, allowing users to check how a transaction would
    /// behave on the blockchain without actually broadcasting it. It is useful for testing and debugging
    /// before sending a transaction to the network.
    ///
    /// ## Parameters
    /// - `data`: The serialized transaction data in a specified encoding format.
    /// - `config`: Optional configuration for simulating the transaction, including settings for signature verification,
    ///   blockhash replacement, and more.
    ///
    /// ## Returns
    /// A response containing:
    /// - `value`: An object with the result of the simulation, which includes information such as errors,
    ///   logs, accounts, units consumed, and return data.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "simulateTransaction",
    ///   "params": [
    ///     "TransactionDataHere",
    ///     {
    ///       "sigVerify": true,
    ///       "replaceRecentBlockhash": true,
    ///       "encoding": "base64",
    ///       "innerInstructions": true
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "err": null,
    ///     "logs": ["Log output"],
    ///     "accounts": [null, {}],
    ///     "unitsConsumed": 12345,
    ///     "returnData": {
    ///       "programId": "ProgramIDHere",
    ///       "data": ["returnDataHere", "base64"]
    ///     },
    ///     "innerInstructions": [{
    ///       "index": 0,
    ///       "instructions": [{ "parsed": { "programIdIndex": 0 } }]
    ///     }],
    ///     "replacementBlockhash": "BlockhashHere"
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the transaction simulation fails due to invalid data or other issues.
    ///
    /// # Notes
    /// - This method simulates the transaction locally and does not affect the actual blockchain state.
    /// - The `sigVerify` flag determines whether the transaction's signature should be verified during the simulation.
    /// - The `replaceRecentBlockhash` flag allows the simulation to use the most recent blockhash for the transaction.
    ///
    /// # See Also
    /// - `getTransactionStatus`
    #[rpc(meta, name = "simulateTransaction")]
    fn simulate_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSimulateTransactionResult>>>;

    /// Retrieves the minimum ledger slot.
    ///
    /// This RPC method returns the minimum ledger slot, which is the smallest slot number that
    /// contains some data or transaction. It is useful for understanding the earliest point in the
    /// blockchain's history where data is available.
    ///
    /// ## Parameters
    /// - None.
    ///
    /// ## Returns
    /// The minimum ledger slot as an integer representing the earliest slot where data is available.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "minimumLedgerSlot",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": 123456
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the ledger slot retrieval fails.
    ///
    /// # Notes
    /// - The returned slot is typically the earliest slot that contains useful data for the ledger.
    ///
    /// # See Also
    /// - `getSlot`
    #[rpc(meta, name = "minimumLedgerSlot")]
    fn minimum_ledger_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    /// Retrieves the details of a block in the blockchain.
    ///
    /// This RPC method fetches a block's details, including its transactions and associated metadata,
    /// given a specific slot number. The response includes information like the block's hash, previous
    /// block hash, rewards, transactions, and more.
    ///
    /// ## Parameters
    /// - `slot`: The slot number of the block you want to retrieve. This is the block's position in the
    ///   chain.
    /// - `config`: Optional configuration for the block retrieval. This allows you to customize the
    ///   encoding and details returned in the response (e.g., full transaction details, rewards, etc.).
    ///
    /// ## Returns
    /// A `UiConfirmedBlock` containing the block's information, such as the block's hash, previous block
    /// hash, and an optional list of transactions and rewards. If no block is found for the provided slot,
    /// the response will be `None`.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlock",
    ///   "params": [123456, {"encoding": "json", "transactionDetails": "full"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "previousBlockhash": "abc123",
    ///     "blockhash": "def456",
    ///     "parentSlot": 123455,
    ///     "transactions": [ ... ],
    ///     "rewards": [ ... ],
    ///     "blockTime": 1620000000,
    ///     "blockHeight": 1000
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the block cannot be found for the specified slot.
    /// - Returns an error if there is an issue with the configuration options provided.
    ///
    /// # Notes
    /// - The `transactionDetails` field in the configuration can be used to specify the level of detail
    ///   you want for transactions within the block (e.g., full transaction data, only signatures, etc.).
    /// - The block's `blockhash` and `previousBlockhash` are crucial for navigating through the blockchain's
    ///   history.
    ///
    /// # See Also
    /// - `getSlot`, `getBlockHeight`
    #[rpc(meta, name = "getBlock")]
    fn get_block(
        &self,
        meta: Self::Metadata,
        slot: Slot,
        config: Option<RpcEncodingConfigWrapper<RpcBlockConfig>>,
    ) -> BoxFuture<Result<Option<UiConfirmedBlock>>>;

    /// Retrieves the timestamp for a block, given its slot number.
    ///
    /// This RPC method fetches the timestamp of the block associated with a given slot. The timestamp
    /// represents the time at which the block was created.
    ///
    /// ## Parameters
    /// - `slot`: The slot number of the block you want to retrieve the timestamp for. This is the block's
    ///   position in the chain.
    ///
    /// ## Returns
    /// A `UnixTimestamp` containing the block's creation time in seconds since the Unix epoch. If no
    /// block exists for the provided slot, the response will be `None`.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlockTime",
    ///   "params": [123456]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": 1620000000
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if there is an issue with the provided slot or if the slot is invalid.
    ///
    /// # Notes
    /// - The returned `UnixTimestamp` represents the time in seconds since the Unix epoch (1970-01-01 00:00:00 UTC).
    /// - If the block for the given slot has not been processed or does not exist, the response will be `None`.
    ///
    /// # See Also
    /// - `getBlock`, `getSlot`, `getBlockHeight`
    #[rpc(meta, name = "getBlockTime")]
    fn get_block_time(
        &self,
        meta: Self::Metadata,
        slot: Slot,
    ) -> BoxFuture<Result<Option<UnixTimestamp>>>;

    /// Retrieves a list of slot numbers starting from a given `start_slot`.
    ///
    /// This RPC method fetches a sequence of block slots starting from the specified `start_slot`
    /// and continuing until a defined `end_slot` (if provided). If no `end_slot` is specified,
    /// it will return all blocks from the `start_slot` onward.
    ///
    /// ## Parameters
    /// - `start_slot`: The slot number from which to begin retrieving blocks.
    /// - `wrapper`: An optional parameter that can either specify an `end_slot` or contain a configuration
    ///   (`RpcContextConfig`) to define additional context settings such as commitment and minimum context slot.
    /// - `config`: An optional configuration for additional context parameters like commitment and minimum context slot.
    ///
    /// ## Returns
    /// A list of slot numbers, representing the sequence of blocks starting from `start_slot`.
    /// The returned slots are in ascending order. If no blocks are found, the response will be an empty list.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlocks",
    ///   "params": [123456, {"endSlotOnly": 123500}, {"commitment": "finalized"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": [123456, 123457, 123458, 123459]
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the provided `start_slot` is invalid or if there is an issue processing the request.
    ///
    /// # Notes
    /// - The response will return all blocks starting from the `start_slot` and up to the `end_slot` if specified.
    ///   If no `end_slot` is provided, the server will return all available blocks starting from `start_slot`.
    /// - The `commitment` setting determines the level of finality for the blocks returned (e.g., "finalized", "confirmed", etc.).
    ///
    /// # See Also
    /// - `getBlock`, `getSlot`, `getBlockTime`
    #[rpc(meta, name = "getBlocks")]
    fn get_blocks(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        wrapper: Option<RpcBlocksConfigWrapper>,
        config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>>;

    /// Retrieves a limited list of block slots starting from a given `start_slot`.
    ///
    /// This RPC method fetches a sequence of block slots starting from the specified `start_slot`,
    /// but limits the number of blocks returned to the specified `limit`. This is useful when you want
    /// to quickly retrieve a small number of blocks from a specific point in the blockchain.
    ///
    /// ## Parameters
    /// - `start_slot`: The slot number from which to begin retrieving blocks.
    /// - `limit`: The maximum number of block slots to return. This limits the size of the response.
    /// - `config`: An optional configuration for additional context parameters like commitment and minimum context slot.
    ///
    /// ## Returns
    /// A list of slot numbers, representing the sequence of blocks starting from `start_slot`, up to the specified `limit`.
    /// If fewer blocks are available, the response will contain only the available blocks.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlocksWithLimit",
    ///   "params": [123456, 5, {"commitment": "finalized"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": [123456, 123457, 123458, 123459, 123460]
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the provided `start_slot` is invalid, if the `limit` is zero, or if there is an issue processing the request.
    ///
    /// # Notes
    /// - The response will return up to the specified `limit` number of blocks starting from `start_slot`.
    /// - If the blockchain contains fewer than the requested number of blocks, the response will contain only the available blocks.
    /// - The `commitment` setting determines the level of finality for the blocks returned (e.g., "finalized", "confirmed", etc.).
    ///
    /// # See Also
    /// - `getBlocks`, `getBlock`, `getSlot`
    #[rpc(meta, name = "getBlocksWithLimit")]
    fn get_blocks_with_limit(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        limit: usize,
        config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>>;

    /// Retrieves the details of a specific transaction by its signature.
    ///
    /// This RPC method allows clients to fetch a previously confirmed transaction
    /// along with its metadata. It supports multiple encoding formats and lets you
    /// optionally limit which transaction versions are returned.
    ///
    /// ## Parameters
    /// - `signature`: The base-58 encoded signature of the transaction to fetch.
    /// - `config` (optional): Configuration for the encoding, commitment level, and supported transaction version.
    ///
    /// ## Returns
    /// If the transaction is found, returns an object containing:
    /// - `slot`: The slot in which the transaction was confirmed.
    /// - `blockTime`: The estimated production time of the block containing the transaction (in Unix timestamp).
    /// - `transaction`: The transaction itself, including all metadata such as status, logs, and account changes.
    ///
    /// Returns `null` if the transaction is not found.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getTransaction",
    ///   "params": [
    ///     "5YwKXNYCnbAednZcJ2Qu9swiyWLUWaKkTZb2tFCSM1uCEmFHe5zoHQaKzwX4e6RGXkPRqRpxwWBLTeYEGqZtA6nW",
    ///     {
    ///       "encoding": "jsonParsed",
    ///       "commitment": "finalized",
    ///       "maxSupportedTransactionVersion": 0
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": {
    ///     "slot": 175512345,
    ///     "blockTime": 1702345678,
    ///     "transaction": {
    ///       "version": 0,
    ///       "transaction": {
    ///         "message": { ... },
    ///         "signatures": [ ... ]
    ///       },
    ///       "meta": {
    ///         "err": null,
    ///         "status": { "Ok": null },
    ///         ...
    ///       }
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the signature is invalid or if there is a backend failure.
    /// - Returns `null` if the transaction is not found (e.g., dropped or not yet confirmed).
    ///
    /// # Notes
    /// - The `encoding` field supports formats like `base64`, `base58`, `json`, and `jsonParsed`.
    /// - If `maxSupportedTransactionVersion` is specified, transactions using a newer version will not be returned.
    /// - Depending on the commitment level, this method may or may not return the latest transactions.
    ///
    /// # See Also
    /// - `getSignatureStatuses`, `getConfirmedTransaction`, `getBlock`
    #[rpc(meta, name = "getTransaction")]
    fn get_transaction(
        &self,
        meta: Self::Metadata,
        signature_str: String,
        config: Option<RpcEncodingConfigWrapper<RpcTransactionConfig>>,
    ) -> BoxFuture<Result<Option<EncodedConfirmedTransactionWithStatusMeta>>>;

    /// Returns confirmed transaction signatures for transactions involving an address.
    ///
    /// This RPC method allows clients to look up historical transaction signatures
    /// that involved a given account address. The list is returned in reverse
    /// chronological order (most recent first) and can be paginated.
    ///
    /// ## Parameters
    /// - `address`: The base-58 encoded address to query.
    /// - `config` (optional): Configuration object with the following fields:
    ///   - `before`: Start search before this signature.
    ///   - `until`: Search until this signature (inclusive).
    ///   - `limit`: Maximum number of results to return (default: 1,000; max: 1,000).
    ///   - `commitment`: The level of commitment desired (e.g., finalized).
    ///   - `minContextSlot`: The minimum slot that the query should be evaluated at.
    ///
    /// ## Returns
    /// A list of confirmed transaction summaries, each including:
    /// - `signature`: Transaction signature (base-58).
    /// - `slot`: The slot in which the transaction was confirmed.
    /// - `err`: If the transaction failed, an error object; otherwise `null`.
    /// - `memo`: Optional memo attached to the transaction.
    /// - `blockTime`: Approximate production time of the block containing the transaction (Unix timestamp).
    /// - `confirmationStatus`: One of `processed`, `confirmed`, or `finalized`.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getSignaturesForAddress",
    ///   "params": [
    ///     "5ZJShu4hxq7gxcu1RUVUMhNeyPmnASvokhZ8QgxtzVzm",
    ///     {
    ///       "limit": 2,
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": [
    ///     {
    ///       "signature": "5VnFgjCwQoM2aBymRkdaV74ZKbbfUpR2zhfn9qN7shHPfLCXcfSBTfxhcuHsjVYz2UkAxw1cw6azS4qPGaKMyrjy",
    ///       "slot": 176012345,
    ///       "err": null,
    ///       "memo": null,
    ///       "blockTime": 1703456789,
    ///       "confirmationStatus": "finalized"
    ///     },
    ///     {
    ///       "signature": "3h1QfUHyjFdqLy5PSTLDmYqL2NhVLz9P9LtS43jJP3aNUv9yP1JWhnzMVg5crEXnEvhP6bLgRtbgi6Z1EGgdA1yF",
    ///       "slot": 176012344,
    ///       "err": null,
    ///       "memo": "example-memo",
    ///       "blockTime": 1703456770,
    ///       "confirmationStatus": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the address is invalid or if the request exceeds internal limits.
    /// - May return fewer results than requested if pagination is constrained by chain history.
    ///
    /// # Notes
    /// - For full transaction details, use the returned signatures with `getTransaction`.
    /// - The default `limit` is 1,000 and is capped at 1,000.
    ///
    /// # See Also
    /// - `getTransaction`, `getConfirmedSignaturesForAddress2` (legacy)
    #[rpc(meta, name = "getSignaturesForAddress")]
    fn get_signatures_for_address(
        &self,
        meta: Self::Metadata,
        address: String,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> BoxFuture<Result<Vec<RpcConfirmedTransactionStatusWithSignature>>>;

    /// Returns the slot of the lowest confirmed block that has not been purged from the ledger.
    ///
    /// This RPC method is useful for determining the oldest block that is still available
    /// from the node. Blocks before this slot have likely been purged and are no longer accessible
    /// for queries such as `getBlock`, `getTransaction`, etc.
    ///
    /// ## Parameters
    /// None.
    ///
    /// ## Returns
    /// A single integer representing the first available slot (block) that has not been purged.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getFirstAvailableBlock",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "result": 146392340
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the node is not fully initialized or if the ledger is inaccessible.
    ///
    /// # Notes
    /// - This value is typically useful for pagination or historical data indexing.
    /// - This slot may increase over time as the node prunes old ledger data.
    ///
    /// # See Also
    /// - `getBlock`, `getBlockTime`, `minimumLedgerSlot`
    #[rpc(meta, name = "getFirstAvailableBlock")]
    fn get_first_available_block(&self, meta: Self::Metadata) -> Result<Slot>;

    /// Returns the latest blockhash and associated metadata needed to sign and send a transaction.
    ///
    /// This method is essential for transaction construction. It provides the most recent
    /// blockhash that should be included in a transaction to be considered valid. It may
    /// also include metadata such as the last valid block height and the minimum context slot.
    ///
    /// ## Parameters
    /// - `config` *(optional)*: Optional context settings, such as commitment level and minimum slot.
    ///
    /// ## Returns
    /// A JSON object containing the recent blockhash, last valid block height,
    /// and the context slot of the response.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getLatestBlockhash",
    ///   "params": [
    ///     {
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 18123942
    ///     },
    ///     "value": {
    ///       "blockhash": "9Xc7XmXmpRmFAqMQUvn2utY5BJeXFY2ZHMxu2fbjZkfy",
    ///       "lastValidBlockHeight": 18123971
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the node is behind or if the blockhash cache is temporarily unavailable.
    ///
    /// # Notes
    /// - Transactions must include a recent blockhash to be accepted.
    /// - The blockhash will expire after a certain number of slots (around 150 slots typically).
    ///
    /// # See Also
    /// - `sendTransaction`, `simulateTransaction`, `requestAirdrop`
    #[rpc(meta, name = "getLatestBlockhash")]
    fn get_latest_blockhash(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<RpcBlockhash>>;

    /// Checks if a given blockhash is still valid for transaction inclusion.
    ///
    /// This method can be used to determine whether a specific blockhash can still
    /// be used in a transaction. Blockhashes expire after approximately 150 slots,
    /// and transactions that reference an expired blockhash will be rejected.
    ///
    /// ## Parameters
    /// - `blockhash`: A base-58 encoded string representing the blockhash to validate.
    /// - `config` *(optional)*: Optional context configuration such as commitment level or minimum context slot.
    ///
    /// ## Returns
    /// A boolean value wrapped in a `RpcResponse`:
    /// - `true` if the blockhash is valid and usable.
    /// - `false` if the blockhash has expired or is unknown to the node.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "isBlockhashValid",
    ///   "params": [
    ///     "9Xc7XmXmpRmFAqMQUvn2utY5BJeXFY2ZHMxu2fbjZkfy",
    ///     {
    ///       "commitment": "confirmed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 18123945
    ///     },
    ///     "value": true
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the node is unable to validate the blockhash (e.g., blockhash not found).
    ///
    /// # Notes
    /// - This endpoint is useful for transaction retries or for validating manually constructed transactions.
    ///
    /// # See Also
    /// - `getLatestBlockhash`, `sendTransaction`
    #[rpc(meta, name = "isBlockhashValid")]
    fn is_blockhash_valid(
        &self,
        meta: Self::Metadata,
        blockhash: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<bool>>;

    /// Returns the estimated fee required to submit a given transaction message.
    ///
    /// This method takes a base64-encoded `Message` (the serialized form of a transaction's message),
    /// and returns the fee in lamports that would be charged for processing that message,
    /// assuming it was submitted as a transaction.
    ///
    /// ## Parameters
    /// - `data`: A base64-encoded string of the binary-encoded `Message`.
    /// - `config` *(optional)*: Optional context configuration such as commitment level or minimum context slot.
    ///
    /// ## Returns
    /// A `RpcResponse` wrapping an `Option<u64>`:
    /// - `Some(fee)` if the fee could be calculated for the given message.
    /// - `None` if the fee could not be determined (e.g., due to invalid inputs or expired blockhash).
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getFeeForMessage",
    ///   "params": [
    ///     "Af4F...base64-encoded-message...==",
    ///     {
    ///       "commitment": "processed"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 19384722
    ///     },
    ///     "value": 5000
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// ## Errors
    /// - Returns an error if the input is not a valid message.
    /// - Returns `null` (i.e., `None`) if the fee cannot be determined.
    ///
    /// # Notes
    /// - This method is useful for estimating fees before submitting transactions.
    /// - It helps users decide whether to rebroadcast or update a transaction.
    ///
    /// # See Also
    /// - `sendTransaction`, `simulateTransaction`
    #[rpc(meta, name = "getFeeForMessage")]
    fn get_fee_for_message(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<Option<u64>>>;

    /// Returns the current minimum delegation amount required for a stake account.
    ///
    /// This method provides the minimum number of lamports that must be delegated
    /// in order to be considered active in the staking system. It helps users determine
    /// the minimum threshold to avoid their stake being considered inactive or rent-exempt only.
    ///
    /// ## Parameters
    /// - `config` *(optional)*: Optional context configuration including commitment level or minimum context slot.
    ///
    /// ## Returns
    /// A `RpcResponse` containing a `u64` value indicating the minimum required lamports for stake delegation.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getStakeMinimumDelegation",
    ///   "params": [
    ///     {
    ///       "commitment": "finalized"
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 21283712
    ///     },
    ///     "value": 10000000
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - This value may change over time due to protocol updates or inflation.
    /// - Stake accounts with a delegated amount below this value may not earn rewards.
    ///
    /// # See Also
    /// - `getStakeActivation`, `getInflationReward`, `getEpochInfo`
    #[rpc(meta, name = "getStakeMinimumDelegation")]
    fn get_stake_minimum_delegation(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>>;

    /// Returns recent prioritization fees for one or more accounts.
    ///
    /// This method is useful for estimating the prioritization fee required
    /// for a transaction to be included quickly in a block. It returns the
    /// most recent prioritization fee paid by each account provided.
    ///
    /// ## Parameters
    /// - `pubkey_strs` *(optional)*: A list of base-58 encoded account public keys (as strings).
    ///   If omitted, the node may return a default or empty set.
    ///
    /// ## Returns
    /// A list of `RpcPrioritizationFee` entries, each containing the slot and the fee paid
    /// to prioritize transactions.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getRecentPrioritizationFees",
    ///   "params": [
    ///     [
    ///       "9xz7uXmf3CjFWW5E8v9XJXuGzTZ2V7UtEG1epF2Tt6TL"
    ///     ]
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "slot": 21458900,
    ///       "prioritizationFee": 5000
    ///     }
    ///   ],
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - The prioritization fee helps validators prioritize transactions for inclusion in blocks.
    /// - These fees are dynamic and can vary significantly depending on network congestion.
    ///
    /// # See Also
    /// - `getFeeForMessage`, `simulateTransaction`
    #[rpc(meta, name = "getRecentPrioritizationFees")]
    fn get_recent_prioritization_fees(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Option<Vec<String>>,
    ) -> BoxFuture<Result<Vec<RpcPrioritizationFee>>>;
}

#[derive(Clone)]
pub struct SurfpoolFullRpc;
impl Full for SurfpoolFullRpc {
    type Metadata = Option<RunloopContext>;

    fn get_inflation_reward(
        &self,
        _meta: Self::Metadata,
        _address_strs: Vec<String>,
        _config: Option<RpcEpochConfig>,
    ) -> BoxFuture<Result<Vec<Option<RpcInflationReward>>>> {
        not_implemented_err_async("get_inflation_reward")
    }

    fn get_cluster_nodes(&self, _meta: Self::Metadata) -> Result<Vec<RpcContactInfo>> {
        not_implemented_err("get_cluster_nodes")
    }

    fn get_recent_performance_samples(
        &self,
        meta: Self::Metadata,
        limit: Option<usize>,
    ) -> Result<Vec<RpcPerfSample>> {
        let limit = limit.unwrap_or(720);
        if limit > 720 {
            return Err(Error::invalid_params("Invalid limit; max 720"));
        }

        meta.with_svm_reader(|svm_reader| {
            svm_reader
                .perf_samples
                .iter()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
        })
        .map_err(Into::into)
    }

    fn get_signature_statuses(
        &self,
        meta: Self::Metadata,
        signature_strs: Vec<String>,
        _config: Option<RpcSignatureStatusConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<TransactionStatus>>>>> {
        let signatures = match signature_strs
            .iter()
            .map(|s| {
                Signature::from_str(s)
                    .map_err(|e| SurfpoolError::invalid_signature(s, e.to_string()))
            })
            .collect::<std::result::Result<Vec<Signature>, SurfpoolError>>()
        {
            Ok(sigs) => sigs,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context::<Option<UiTransactionEncoding>>(None) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut responses = Vec::with_capacity(signatures.len());
            let mut last_latest_absolute_slot = 0;
            for signature in signatures.into_iter() {
                let res = svm_locker.get_transaction(&remote_ctx, &signature).await;

                last_latest_absolute_slot = res.slot;
                responses.push(res.inner.map_some_transaction_status());
            }
            Ok(RpcResponse {
                context: RpcResponseContext::new(last_latest_absolute_slot),
                value: responses,
            })
        })
    }

    fn get_max_retransmit_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err("get_max_retransmit_slot")
    }

    fn get_max_shred_insert_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err("get_max_shred_insert_slot")
    }

    fn request_airdrop(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        lamports: u64,
        _config: Option<RpcRequestAirdropConfig>,
    ) -> Result<String> {
        let pubkey = verify_pubkey(&pubkey_str)?;
        let svm_locker = meta.get_svm_locker()?;
        let res = svm_locker
            .airdrop(&pubkey, lamports)
            .map_err(|err| Error::invalid_params(format!("failed to send transaction: {err:?}")))?;
        Ok(res.signature.to_string())
    }

    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String> {
        let config = config.unwrap_or_default();
        let tx_encoding = config.encoding.unwrap_or(UiTransactionEncoding::Base58);
        let binary_encoding = tx_encoding.into_binary_encoding().ok_or_else(|| {
            Error::invalid_params(format!(
                "unsupported encoding: {tx_encoding}. Supported encodings: base58, base64"
            ))
        })?;
        let (_, unsanitized_tx) =
            decode_and_deserialize::<VersionedTransaction>(data, binary_encoding)?;
        let signatures = unsanitized_tx.signatures.clone();
        let signature = signatures[0];
        let Some(ctx) = meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        let (status_update_tx, status_update_rx) = crossbeam_channel::bounded(1);
        ctx.simnet_commands_tx
            .send(SimnetCommand::TransactionReceived(
                ctx.id,
                unsanitized_tx,
                status_update_tx,
                config.skip_preflight,
            ))
            .map_err(|_| RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            })?;
        match status_update_rx.recv() {
            Ok(TransactionStatusEvent::SimulationFailure(e)) => {
                return Err(Error {
                    data: None,
                    message: format!(
                        "Transaction simulation failed: {}: {} log messages:\n{}",
                        e.0,
                        e.1.logs.len(),
                        e.1.logs.iter().map(|l| l.to_string()).join("\n")
                    ),
                    code: jsonrpc_core::ErrorCode::ServerError(-32002),
                });
            }
            Ok(TransactionStatusEvent::ExecutionFailure(e)) => {
                return Err(Error {
                    data: None,
                    message: format!(
                        "Transaction execution failed: {}: {} log messages:\n{}",
                        e.0,
                        e.1.logs.len(),
                        e.1.logs.iter().map(|l| l.to_string()).join("\n")
                    ),
                    code: jsonrpc_core::ErrorCode::ServerError(-32002),
                });
            }
            Ok(TransactionStatusEvent::VerificationFailure(signature)) => {
                return Err(Error {
                    data: None,
                    message: format!("Transaction verification failed for transaction {signature}"),
                    code: jsonrpc_core::ErrorCode::ServerError(-32002),
                });
            }
            Err(e) => {
                return Err(Error {
                    data: None,
                    message: format!("Failed to process transaction: {e}"),
                    code: jsonrpc_core::ErrorCode::ServerError(-32002),
                });
            }
            Ok(TransactionStatusEvent::Success(_)) => {}
        }
        Ok(signature.to_string())
    }

    fn simulate_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSimulateTransactionResult>>> {
        let config = config.unwrap_or_default();
        let tx_encoding = config.encoding.unwrap_or(UiTransactionEncoding::Base58);
        let binary_encoding = tx_encoding
            .into_binary_encoding()
            .ok_or_else(|| {
                Error::invalid_params(format!(
                    "unsupported encoding: {tx_encoding}. Supported encodings: base58, base64"
                ))
            })
            .unwrap();
        let (_, unsanitized_tx) =
            decode_and_deserialize::<VersionedTransaction>(data, binary_encoding).unwrap();

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            svm_locker
                .with_svm_writer(|svm_writer| svm_writer.inner.set_sigverify(config.sig_verify));
            let pubkeys = svm_locker
                .get_pubkeys_from_message(&remote_ctx, &unsanitized_tx.message)
                .await?;

            let SvmAccessContext {
                slot,
                inner: account_updates,
                latest_blockhash,
                latest_epoch_info,
            } = svm_locker
                .get_multiple_accounts(&remote_ctx, &pubkeys, None)
                .await?;

            svm_locker.write_multiple_account_updates(&account_updates);

            let replacement_blockhash = Some(RpcBlockhash {
                blockhash: latest_blockhash.to_string(),
                last_valid_block_height: latest_epoch_info.block_height,
            });

            let value = match svm_locker.simulate_transaction(unsanitized_tx) {
                Ok(tx_info) => {
                    let mut accounts = None;
                    if let Some(observed_accounts) = config.accounts {
                        let mut ui_accounts = vec![];
                        for observed_pubkey in observed_accounts.addresses.iter() {
                            let mut ui_account = None;
                            for (updated_pubkey, account) in tx_info.post_accounts.iter() {
                                if observed_pubkey.eq(&updated_pubkey.to_string()) {
                                    ui_account = Some(encode_ui_account(
                                        updated_pubkey,
                                        account,
                                        UiAccountEncoding::Base64,
                                        None,
                                        None,
                                    ));
                                }
                            }
                            ui_accounts.push(ui_account);
                        }
                        accounts = Some(ui_accounts);
                    }

                    RpcSimulateTransactionResult {
                        err: None,
                        logs: Some(tx_info.meta.logs.clone()),
                        accounts,
                        units_consumed: Some(tx_info.meta.compute_units_consumed),
                        return_data: Some(tx_info.meta.return_data.clone().into()),
                        inner_instructions: if config.inner_instructions {
                            Some(transform_tx_metadata_to_ui_accounts(&tx_info.meta))
                        } else {
                            None
                        },
                        replacement_blockhash,
                    }
                }
                Err(tx_info) => RpcSimulateTransactionResult {
                    err: Some(tx_info.err),
                    logs: Some(tx_info.meta.logs.clone()),
                    accounts: None,
                    units_consumed: Some(tx_info.meta.compute_units_consumed),
                    return_data: Some(tx_info.meta.return_data.clone().into()),
                    inner_instructions: if config.inner_instructions {
                        Some(transform_tx_metadata_to_ui_accounts(&tx_info.meta))
                    } else {
                        None
                    },
                    replacement_blockhash,
                },
            };

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value,
            })
        })
    }

    fn minimum_ledger_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err("minimum_ledger_slot")
    }

    fn get_block(
        &self,
        meta: Self::Metadata,
        slot: Slot,
        _config: Option<RpcEncodingConfigWrapper<RpcBlockConfig>>,
    ) -> BoxFuture<Result<Option<UiConfirmedBlock>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            Ok(svm_locker.with_svm_reader(|svm_reader| svm_reader.get_block_at_slot(slot)))
        })
    }

    fn get_block_time(
        &self,
        _meta: Self::Metadata,
        _slot: Slot,
    ) -> BoxFuture<Result<Option<UnixTimestamp>>> {
        Box::pin(async { Ok(None) })
    }

    fn get_blocks(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _wrapper: Option<RpcBlocksConfigWrapper>,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        not_implemented_err_async("get_blocks")
    }

    fn get_blocks_with_limit(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _limit: usize,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        not_implemented_err_async("get_blocks_with_limit")
    }

    fn get_transaction(
        &self,
        meta: Self::Metadata,
        signature_str: String,
        config: Option<RpcEncodingConfigWrapper<RpcTransactionConfig>>,
    ) -> BoxFuture<Result<Option<EncodedConfirmedTransactionWithStatusMeta>>> {
        let config = config.map(|c| c.convert_to_current()).unwrap_or_default();

        let signature = match Signature::from_str(&signature_str)
            .map_err(|e| SurfpoolError::invalid_signature(&signature_str, e.to_string()))
        {
            Ok(s) => s,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(config.encoding) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            // TODO: implement new interfaces in LiteSVM to get all the relevant info
            // needed to return the actual tx, not just some metadata
            match svm_locker
                .get_transaction(&remote_ctx, &signature)
                .await
                .inner
            {
                GetTransactionResult::None(_) => Ok(None),
                GetTransactionResult::FoundTransaction(_, meta, _) => Ok(Some(meta)),
            }
        })
    }

    fn get_signatures_for_address(
        &self,
        _meta: Self::Metadata,
        _address: String,
        _config: Option<RpcSignaturesForAddressConfig>,
    ) -> BoxFuture<Result<Vec<RpcConfirmedTransactionStatusWithSignature>>> {
        not_implemented_err_async("get_signatures_for_address")
    }

    fn get_first_available_block(&self, meta: Self::Metadata) -> Result<Slot> {
        meta.with_svm_reader(|svm_reader| {
            svm_reader.blocks.keys().min().copied().unwrap_or_default()
        })
        .map_err(Into::into)
    }

    fn get_latest_blockhash(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<RpcBlockhash>> {
        meta.with_svm_reader(|svm_reader| {
            let last_valid_block_height = svm_reader.latest_epoch_info.block_height;
            let value = RpcBlockhash {
                blockhash: svm_reader.latest_blockhash().to_string(),
                last_valid_block_height,
            };
            RpcResponse {
                context: RpcResponseContext {
                    slot: svm_reader.get_latest_absolute_slot(),
                    api_version: Some(RpcApiVersion::default()),
                },
                value,
            }
        })
        .map_err(Into::into)
    }

    fn is_blockhash_valid(
        &self,
        meta: Self::Metadata,
        _blockhash: String,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<bool>> {
        meta.with_svm_reader(|svm_reader| RpcResponse {
            context: RpcResponseContext::new(svm_reader.get_latest_absolute_slot()),
            value: true,
        })
        .map_err(Into::into)
    }

    fn get_fee_for_message(
        &self,
        meta: Self::Metadata,
        encoded: String,
        _config: Option<RpcContextConfig>, // TODO: use config
    ) -> Result<RpcResponse<Option<u64>>> {
        let (_, message) =
            decode_and_deserialize::<VersionedMessage>(encoded, TransactionBinaryEncoding::Base64)?;

        meta.with_svm_reader(|svm_reader| RpcResponse {
            context: RpcResponseContext::new(svm_reader.get_latest_absolute_slot()),
            value: Some((message.header().num_required_signatures as u64) * 5000),
        })
        .map_err(Into::into)
    }

    fn get_stake_minimum_delegation(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>> {
        not_implemented_err("get_stake_minimum_delegation")
    }

    fn get_recent_prioritization_fees(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Option<Vec<String>>,
    ) -> BoxFuture<Result<Vec<RpcPrioritizationFee>>> {
        let pubkeys_filter = match pubkey_strs
            .map(|strs| {
                strs.iter()
                    .map(|s| verify_pubkey(s))
                    .collect::<SurfpoolResult<Vec<_>>>()
            })
            .transpose()
        {
            Ok(pubkeys) => pubkeys,
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
            let (blocks, transactions) = svm_locker.with_svm_reader(|svm_reader| {
                (svm_reader.blocks.clone(), svm_reader.transactions.clone())
            });

            // Get MAX_PRIORITIZATION_FEE_BLOCKS_CACHE most recent blocks
            let recent_headers = blocks
                .into_iter()
                .sorted_by_key(|(slot, _)| std::cmp::Reverse(*slot))
                .take(MAX_PRIORITIZATION_FEE_BLOCKS_CACHE)
                .map(|(slot, header)| (slot, header))
                .collect::<Vec<_>>();

            // Flatten the transactions map to get all transactions in the recent blocks
            let recent_transactions = recent_headers
                .into_iter()
                .flat_map(|(slot, header)| {
                    header
                        .signatures
                        .iter()
                        .filter_map(|signature| {
                            // Check if the signature exists in the transactions map
                            transactions.get(signature).map(|tx| (slot, tx))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            // Helper function to extract compute unit price from a CompiledInstruction
            fn get_compute_unit_price(
                ix: CompiledInstruction,
                accounts: &Vec<Pubkey>,
            ) -> Option<u64> {
                let program_account = accounts.get(ix.program_id_index as usize)?;
                if *program_account != compute_budget::id() {
                    return None;
                }

                if let Ok(parsed_instr) = borsh::from_slice::<ComputeBudgetInstruction>(&ix.data) {
                    if let ComputeBudgetInstruction::SetComputeUnitPrice(price) = parsed_instr {
                        return Some(price);
                    }
                }

                None
            }

            let mut prioritization_fees = vec![];
            for (slot, tx) in recent_transactions {
                match tx {
                    SurfnetTransactionStatus::Received => {}
                    SurfnetTransactionStatus::Processed(status_meta) => {
                        let tx = &status_meta.1;

                        // If the transaction has an ALT and includes a compute budget instruction,
                        // the ALT accounts are included in the recent prioritization fees,
                        // so we get _all_ the pubkeys from the message
                        let account_keys = svm_locker
                            .get_pubkeys_from_message(&remote_ctx, &tx.message)
                            .await?;

                        let instructions = match &tx.message {
                            VersionedMessage::V0(msg) => &msg.instructions,
                            VersionedMessage::Legacy(msg) => &msg.instructions,
                        };

                        // Find all compute unit prices in the transaction's instructions
                        let compute_unit_prices = instructions
                            .iter()
                            .filter_map(|ix| get_compute_unit_price(ix.clone(), &account_keys))
                            .collect::<Vec<_>>();

                        for compute_unit_price in compute_unit_prices {
                            if let Some(pubkeys_filter) = &pubkeys_filter {
                                // If none of the accounts involved in this transaction are in the filter,
                                // we don't include the prioritization fee, so we continue
                                if !pubkeys_filter
                                    .iter()
                                    .any(|pk| account_keys.iter().any(|a| a == pk))
                                {
                                    continue;
                                }
                            }
                            // if there's no filter, or if the filter matches an account in this transaction, we include the fee
                            prioritization_fees.push(RpcPrioritizationFee {
                                slot,
                                prioritization_fee: compute_unit_price,
                            });
                        }
                    }
                }
            }
            Ok(prioritization_fees)
        })
    }
}

#[cfg(test)]
mod tests {

    use std::thread::JoinHandle;

    use base64::{prelude::BASE64_STANDARD, Engine};
    use crossbeam_channel::Receiver;
    use solana_account_decoder::{UiAccount, UiAccountData};
    use solana_client::rpc_config::RpcSimulateTransactionAccountsConfig;
    use solana_commitment_config::CommitmentConfig;
    use solana_hash::Hash;
    use solana_keypair::Keypair;
    use solana_message::{
        legacy::Message as LegacyMessage, v0::Message as V0Message, MessageHeader,
    };
    use solana_native_token::LAMPORTS_PER_SOL;
    use solana_pubkey::Pubkey;
    use solana_sdk::{instruction::Instruction, system_instruction};
    use solana_signer::Signer;
    use solana_system_interface::program as system_program;
    use solana_transaction::{
        versioned::{Legacy, TransactionVersion},
        Transaction,
    };
    use solana_transaction_status::{
        EncodedTransaction, EncodedTransactionWithStatusMeta, UiCompiledInstruction, UiMessage,
        UiRawMessage, UiTransaction,
    };
    use surfpool_types::{SimnetCommand, TransactionConfirmationStatus, TransactionMetadata};
    use test_case::test_case;

    use super::*;
    use crate::{
        surfnet::{BlockHeader, BlockIdentifier},
        tests::helpers::TestSetup,
        types::TransactionWithStatusMeta,
    };

    fn build_v0_transaction(
        payer: &Pubkey,
        signers: &[&Keypair],
        instructions: &[Instruction],
        recent_blockhash: &Hash,
    ) -> VersionedTransaction {
        let msg = VersionedMessage::V0(
            V0Message::try_compile(&payer, instructions, &[], *recent_blockhash).unwrap(),
        );
        VersionedTransaction::try_new(msg, signers).unwrap()
    }

    fn build_legacy_transaction(
        payer: &Pubkey,
        signers: &[&Keypair],
        instructions: &[Instruction],
        recent_blockhash: &Hash,
    ) -> VersionedTransaction {
        let msg = VersionedMessage::Legacy(LegacyMessage::new_with_blockhash(
            instructions,
            Some(payer),
            recent_blockhash,
        ));
        VersionedTransaction::try_new(msg, signers).unwrap()
    }

    async fn send_and_await_transaction(
        tx: VersionedTransaction,
        setup: TestSetup<SurfpoolFullRpc>,
        mempool_rx: Receiver<SimnetCommand>,
    ) -> JoinHandle<String> {
        let setup_clone = setup.clone();
        let handle = hiro_system_kit::thread_named("send_tx")
            .spawn(move || {
                let res = setup_clone
                    .rpc
                    .send_transaction(
                        Some(setup_clone.context),
                        bs58::encode(bincode::serialize(&tx).unwrap()).into_string(),
                        None,
                    )
                    .unwrap();

                res
            })
            .unwrap();

        match mempool_rx.recv() {
            Ok(SimnetCommand::TransactionReceived(_, tx, status_tx, _)) => {
                let mut writer = setup.context.svm_locker.0.write().await;
                let slot = writer.get_latest_absolute_slot();
                writer
                    .transactions_queued_for_confirmation
                    .push_back((tx.clone(), status_tx.clone()));
                writer.transactions.insert(
                    tx.signatures[0],
                    SurfnetTransactionStatus::Processed(Box::new(TransactionWithStatusMeta(
                        slot,
                        tx,
                        TransactionMetadata::default(),
                        None,
                    ))),
                );
                status_tx
                    .send(TransactionStatusEvent::Success(
                        TransactionConfirmationStatus::Confirmed,
                    ))
                    .unwrap();
            }
            _ => panic!("failed to receive transaction from mempool"),
        }

        handle
    }

    #[test_case(None, false ; "when limit is None")]
    #[test_case(Some(1), false ; "when limit is ok")]
    #[test_case(Some(1000), true ; "when limit is above max spec")]
    fn test_get_recent_performance_samples(limit: Option<usize>, fails: bool) {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_recent_performance_samples(Some(setup.context), limit);

        if fails {
            assert!(res.is_err());
        } else {
            assert!(res.is_ok());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_signature_statuses() {
        let pks = (0..10).map(|_| Pubkey::new_unique());
        let valid_txs = pks.len();
        let invalid_txs = pks.len();
        let payer = Keypair::new();
        let mut setup = TestSetup::new(SurfpoolFullRpc).without_blockhash().await;
        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let valid = pks
            .clone()
            .map(|pk| {
                Transaction::new_signed_with_payer(
                    &[system_instruction::transfer(
                        &payer.pubkey(),
                        &pk,
                        LAMPORTS_PER_SOL,
                    )],
                    Some(&payer.pubkey()),
                    &[payer.insecure_clone()],
                    recent_blockhash,
                )
            })
            .collect::<Vec<_>>();
        let invalid = pks
            .map(|pk| {
                Transaction::new_unsigned(LegacyMessage::new(
                    &[system_instruction::transfer(
                        &pk,
                        &payer.pubkey(),
                        LAMPORTS_PER_SOL,
                    )],
                    Some(&payer.pubkey()),
                ))
            })
            .collect::<Vec<_>>();
        let txs = valid
            .into_iter()
            .chain(invalid.into_iter())
            .map(|tx| VersionedTransaction {
                signatures: tx.signatures,
                message: VersionedMessage::Legacy(tx.message),
            })
            .collect::<Vec<_>>();
        let _ = setup.context.svm_locker.0.write().await.airdrop(
            &payer.pubkey(),
            (valid_txs + invalid_txs) as u64 * 2 * LAMPORTS_PER_SOL,
        );
        setup.process_txs(txs.clone()).await;

        let res = setup
            .rpc
            .get_signature_statuses(
                Some(setup.context),
                txs.iter().map(|tx| tx.signatures[0].to_string()).collect(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            res.value
                .iter()
                .filter(|status| {
                    println!("status: {:?}", status);
                    if let Some(s) = status {
                        s.status.is_ok()
                    } else {
                        false
                    }
                })
                .count(),
            valid_txs,
            "incorrect number of valid txs"
        );
        assert_eq!(
            res.value
                .iter()
                .filter(|status| if let Some(s) = status {
                    s.status.is_err()
                } else {
                    true
                })
                .count(),
            invalid_txs,
            "incorrect number of invalid txs"
        );
    }

    #[test]
    fn test_request_airdrop() {
        let pk = Pubkey::new_unique();
        let lamports = 1000;
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .request_airdrop(Some(setup.context.clone()), pk.to_string(), lamports, None)
            .unwrap();
        let sig = Signature::from_str(res.as_str()).unwrap();
        let state_reader = setup.context.svm_locker.0.blocking_read();
        assert_eq!(
            state_reader.inner.get_account(&pk).unwrap().lamports,
            lamports,
            "airdropped amount is incorrect"
        );
        assert!(
            state_reader.inner.get_transaction(&sig).is_some(),
            "transaction is not found in the SVM"
        );
        assert!(
            state_reader.transactions.get(&sig).is_some(),
            "transaction is not found in the history"
        );
    }

    #[test_case(TransactionVersion::Legacy(Legacy::Legacy) ; "Legacy transactions")]
    #[test_case(TransactionVersion::Number(0) ; "V0 transactions")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_transaction(version: TransactionVersion) {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let (mempool_tx, mempool_rx) = crossbeam_channel::unbounded();
        let setup = TestSetup::new_with_mempool(SurfpoolFullRpc, mempool_tx);
        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let tx = match version {
            TransactionVersion::Legacy(_) => build_legacy_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(
                    &payer.pubkey(),
                    &pk,
                    LAMPORTS_PER_SOL,
                )],
                &recent_blockhash,
            ),
            TransactionVersion::Number(0) => build_v0_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(
                    &payer.pubkey(),
                    &pk,
                    LAMPORTS_PER_SOL,
                )],
                &recent_blockhash,
            ),
            _ => unimplemented!(),
        };

        let _ = setup
            .context
            .svm_locker
            .0
            .write()
            .await
            .airdrop(&payer.pubkey(), 2 * LAMPORTS_PER_SOL);

        let handle = send_and_await_transaction(tx.clone(), setup.clone(), mempool_rx).await;
        assert_eq!(
            handle.join().unwrap(),
            tx.signatures[0].to_string(),
            "incorrect signature"
        );
    }

    #[test_case(TransactionVersion::Legacy(Legacy::Legacy) ; "Legacy transactions")]
    #[test_case(TransactionVersion::Number(0) ; "V0 transactions")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_simulate_transaction(version: TransactionVersion) {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let lamports = LAMPORTS_PER_SOL;
        let setup = TestSetup::new(SurfpoolFullRpc);
        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let _ = setup
            .rpc
            .request_airdrop(
                Some(setup.context.clone()),
                payer.pubkey().to_string(),
                2 * lamports,
                None,
            )
            .unwrap();

        let tx = match version {
            TransactionVersion::Legacy(_) => build_legacy_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
                &recent_blockhash,
            ),
            TransactionVersion::Number(0) => build_v0_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
                &recent_blockhash,
            ),
            _ => unimplemented!(),
        };

        let simulation_res = setup
            .rpc
            .simulate_transaction(
                Some(setup.context),
                bs58::encode(bincode::serialize(&tx).unwrap()).into_string(),
                Some(RpcSimulateTransactionConfig {
                    sig_verify: true,
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::finalized()),
                    encoding: None,
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: None,
                        addresses: vec![pk.to_string()],
                    }),
                    min_context_slot: None,
                    inner_instructions: false,
                }),
            )
            .await
            .unwrap();

        assert_eq!(
            simulation_res.value.err, None,
            "Unexpected simulation error"
        );
        assert_eq!(
            simulation_res.value.accounts,
            Some(vec![Some(UiAccount {
                lamports,
                data: UiAccountData::Binary(BASE64_STANDARD.encode(""), UiAccountEncoding::Base64),
                owner: system_program::id().to_string(),
                executable: false,
                rent_epoch: 0,
                space: Some(0),
            })]),
            "Wrong account content"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_simulate_transaction_no_signers() {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let lamports = LAMPORTS_PER_SOL;
        let setup = TestSetup::new(SurfpoolFullRpc);
        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let _ = setup
            .rpc
            .request_airdrop(
                Some(setup.context.clone()),
                payer.pubkey().to_string(),
                2 * lamports,
                None,
            )
            .unwrap();
        //build_legacy_transaction
        let mut msg = LegacyMessage::new(
            &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
            Some(&payer.pubkey()),
        );
        msg.recent_blockhash = recent_blockhash;
        let tx = Transaction::new_unsigned(msg);

        let simulation_res = setup
            .rpc
            .simulate_transaction(
                Some(setup.context),
                bs58::encode(bincode::serialize(&tx).unwrap()).into_string(),
                Some(RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::finalized()),
                    encoding: None,
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: None,
                        addresses: vec![pk.to_string()],
                    }),
                    min_context_slot: None,
                    inner_instructions: false,
                }),
            )
            .await
            .unwrap();

        assert_eq!(
            simulation_res.value.err, None,
            "Unexpected simulation error"
        );
        assert_eq!(
            simulation_res.value.accounts,
            Some(vec![Some(UiAccount {
                lamports,
                data: UiAccountData::Binary(BASE64_STANDARD.encode(""), UiAccountEncoding::Base64),
                owner: system_program::id().to_string(),
                executable: false,
                rent_epoch: 0,
                space: Some(0),
            })]),
            "Wrong account content"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_block() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_block(Some(setup.context), 0, None)
            .await
            .unwrap();

        assert_eq!(res, None);
    }

    #[tokio::test]
    async fn test_get_block_time() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_block_time(Some(setup.context), 0)
            .await
            .unwrap();

        assert_eq!(res, None);
    }

    #[test_case(TransactionVersion::Legacy(Legacy::Legacy) ; "Legacy transactions")]
    #[test_case(TransactionVersion::Number(0) ; "V0 transactions")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_transaction(version: TransactionVersion) {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let lamports = LAMPORTS_PER_SOL;
        let mut setup = TestSetup::new(SurfpoolFullRpc);
        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let _ = setup
            .rpc
            .request_airdrop(
                Some(setup.context.clone()),
                payer.pubkey().to_string(),
                2 * lamports,
                None,
            )
            .unwrap();

        let tx = match version {
            TransactionVersion::Legacy(_) => build_legacy_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
                &recent_blockhash,
            ),
            TransactionVersion::Number(0) => build_v0_transaction(
                &payer.pubkey(),
                &[&payer.insecure_clone()],
                &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
                &recent_blockhash,
            ),
            _ => unimplemented!(),
        };

        setup.process_txs(vec![tx.clone()]).await;

        let res = setup
            .rpc
            .get_transaction(
                Some(setup.context.clone()),
                tx.signatures[0].to_string(),
                None,
            )
            .await
            .unwrap()
            .unwrap();

        let instructions = match tx.message.clone() {
            VersionedMessage::Legacy(message) => message
                .instructions
                .iter()
                .map(|ix| UiCompiledInstruction::from(ix, None))
                .collect(),
            VersionedMessage::V0(message) => message
                .instructions
                .iter()
                .map(|ix| UiCompiledInstruction::from(ix, None))
                .collect(),
        };

        assert_eq!(
            res,
            EncodedConfirmedTransactionWithStatusMeta {
                slot: 0,
                transaction: EncodedTransactionWithStatusMeta {
                    transaction: EncodedTransaction::Json(UiTransaction {
                        signatures: vec![tx.signatures[0].to_string()],
                        message: UiMessage::Raw(UiRawMessage {
                            header: MessageHeader {
                                num_required_signatures: 1,
                                num_readonly_signed_accounts: 0,
                                num_readonly_unsigned_accounts: 1
                            },
                            account_keys: vec![
                                payer.pubkey().to_string(),
                                pk.to_string(),
                                system_program::id().to_string()
                            ],
                            recent_blockhash: recent_blockhash.to_string(),
                            instructions,
                            address_table_lookups: match tx.message {
                                VersionedMessage::Legacy(_) => None,
                                VersionedMessage::V0(_) => Some(vec![]),
                            },
                        })
                    }),
                    meta: res.transaction.clone().meta, // Using the same values to avoid reintroducing processing logic errors
                    version: Some(version)
                },
                block_time: res.block_time // Using the same values to avoid flakyness
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[allow(deprecated)]
    async fn test_get_first_available_block() {
        let setup = TestSetup::new(SurfpoolFullRpc);

        {
            let mut svm_writer = setup.context.svm_locker.0.write().await;

            let previous_chain_tip = svm_writer.chain_tip.clone();

            let latest_entries = svm_writer
                .inner
                .get_sysvar::<solana_sdk::sysvar::recent_blockhashes::RecentBlockhashes>(
            );
            let latest_entry = latest_entries.first().unwrap();

            svm_writer.chain_tip = BlockIdentifier::new(
                svm_writer.chain_tip.index + 1,
                latest_entry.blockhash.to_string().as_str(),
            );

            let hash = svm_writer.chain_tip.hash.clone();
            let block_height = svm_writer.chain_tip.index;
            let parent_slot = svm_writer.get_latest_absolute_slot();

            svm_writer.blocks.insert(
                parent_slot,
                BlockHeader {
                    hash,
                    previous_blockhash: previous_chain_tip.hash.clone(),
                    block_time: chrono::Utc::now().timestamp_millis(),
                    block_height,
                    parent_slot,
                    signatures: Vec::new(),
                },
            );
        }

        let res = setup
            .rpc
            .get_first_available_block(Some(setup.context))
            .unwrap();

        assert_eq!(res, 123);
    }

    #[test]
    fn test_get_latest_blockhash() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_latest_blockhash(Some(setup.context.clone()), None)
            .unwrap();

        assert_eq!(
            res.value.blockhash,
            setup
                .context
                .svm_locker
                .0
                .blocking_read()
                .latest_blockhash()
                .to_string()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_recent_prioritization_fees() {
        let (mempool_tx, mempool_rx) = crossbeam_channel::unbounded();
        let setup = TestSetup::new_with_mempool(SurfpoolFullRpc, mempool_tx);

        let recent_blockhash = setup
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        let payer_1 = Keypair::new();
        let payer_2 = Keypair::new();
        let receiver_pubkey = Pubkey::new_unique();
        let random_pubkey = Pubkey::new_unique();

        // setup accounts
        {
            let _ = setup
                .rpc
                .request_airdrop(
                    Some(setup.context.clone()),
                    payer_1.pubkey().to_string(),
                    2 * LAMPORTS_PER_SOL,
                    None,
                )
                .unwrap();
            let _ = setup
                .rpc
                .request_airdrop(
                    Some(setup.context.clone()),
                    payer_2.pubkey().to_string(),
                    2 * LAMPORTS_PER_SOL,
                    None,
                )
                .unwrap();

            setup.context.svm_locker.confirm_current_block().unwrap();
        }

        // send two transactions that include a compute budget instruction
        {
            let tx_1 = build_legacy_transaction(
                &payer_1.pubkey(),
                &[&payer_1.insecure_clone()],
                &[
                    system_instruction::transfer(
                        &payer_1.pubkey(),
                        &receiver_pubkey,
                        LAMPORTS_PER_SOL,
                    ),
                    compute_budget::ComputeBudgetInstruction::set_compute_unit_price(1000),
                ],
                &recent_blockhash,
            );
            let tx_2 = build_legacy_transaction(
                &payer_2.pubkey(),
                &[&payer_2.insecure_clone()],
                &[
                    system_instruction::transfer(
                        &payer_2.pubkey(),
                        &receiver_pubkey,
                        LAMPORTS_PER_SOL,
                    ),
                    compute_budget::ComputeBudgetInstruction::set_compute_unit_price(1002),
                ],
                &recent_blockhash,
            );

            send_and_await_transaction(tx_1, setup.clone(), mempool_rx.clone())
                .await
                .join()
                .unwrap();
            send_and_await_transaction(tx_2, setup.clone(), mempool_rx)
                .await
                .join()
                .unwrap();
            setup.context.svm_locker.confirm_current_block().unwrap();
        }

        // sending the get_recent_prioritization_fees request with an account
        // should filter the results to only include fees for that account
        let res = setup
            .rpc
            .get_recent_prioritization_fees(
                Some(setup.context.clone()),
                Some(vec![payer_1.pubkey().to_string()]),
            )
            .await
            .unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].prioritization_fee, 1000);

        // sending the get_recent_prioritization_fees request without an account
        // should return all prioritization fees
        let res = setup
            .rpc
            .get_recent_prioritization_fees(Some(setup.context.clone()), None)
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].prioritization_fee, 1000);
        assert_eq!(res[1].prioritization_fee, 1002);

        // sending the get_recent_prioritization_fees request with some random account
        // to filter should return no results
        let res = setup
            .rpc
            .get_recent_prioritization_fees(
                Some(setup.context.clone()),
                Some(vec![random_pubkey.to_string()]),
            )
            .await
            .unwrap();
        assert!(
            res.is_empty(),
            "Expected no prioritization fees for random account"
        );
    }
}
