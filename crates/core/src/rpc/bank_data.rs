use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_client::rpc_config::{RpcBlockProductionConfig, RpcContextConfig};
use solana_client::rpc_response::{RpcBlockProduction, RpcInflationGovernor, RpcInflationRate};
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_epoch_schedule::EpochSchedule;
use solana_rpc_client_api::response::Response as RpcResponse;

use super::{not_implemented_err, RunloopContext, State};

#[rpc]
pub trait BankData {
    type Metadata;

    /// Returns the minimum balance required for rent exemption based on the given data length.
    ///
    /// This RPC method calculates the minimum balance required for an account to be exempt from
    /// rent charges. It uses the data length of the account to determine the balance. The result
    /// can help users manage their accounts by ensuring they have enough balance to cover rent
    /// exemption, preventing accounts from being purged by the system.
    ///
    /// ## Parameters
    /// - `data_len`: The length (in bytes) of the account data. This is used to determine the
    ///   minimum balance required for rent exemption.
    /// - `commitment`: (Optional) A `CommitmentConfig` that allows specifying the level of
    ///   commitment for querying. If not provided, the default commitment level will be used.
    ///
    /// ## Returns
    /// - `Result<u64>`: The method returns the minimum balance required for rent exemption
    ///   as a `u64`. If successful, it will be wrapped in `Ok`, otherwise an error will be
    ///   returned.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getMinimumBalanceForRentExemption",
    ///   "params": [128]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": 2039280,
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is commonly used to determine the required balance when creating new accounts
    ///   or performing account setup operations that need rent exemption.
    /// - The `commitment` parameter allows users to specify the level of assurance they want
    ///   regarding the state of the ledger. For example, using `Confirmed` or `Finalized` ensures
    ///   that the state is more reliable.
    ///
    /// ## Errors
    /// - If there is an issue with the `data_len` or `commitment` parameter (e.g., invalid data),
    ///   an error will be returned.
    #[rpc(meta, name = "getMinimumBalanceForRentExemption")]
    fn get_minimum_balance_for_rent_exemption(
        &self,
        meta: Self::Metadata,
        data_len: usize,
        commitment: Option<CommitmentConfig>,
    ) -> Result<u64>;

    /// Retrieves the inflation governor settings for the network.
    ///
    /// This RPC method returns the current inflation governor configuration, which controls the
    /// inflation rate of the network. The inflation governor is responsible for adjusting the
    /// inflation rate over time, with parameters like the initial and terminal inflation rates,
    /// the taper rate, the foundation amount, and the foundation term.
    ///
    /// ## Parameters
    /// - `commitment`: (Optional) A `CommitmentConfig` that specifies the commitment level for
    ///   querying the inflation governor settings. If not provided, the default commitment level
    ///   is used. Valid commitment levels include `Processed`, `Confirmed`, or `Finalized`.
    ///
    /// ## Returns
    /// - `Result<RpcInflationGovernor>`: The method returns an `RpcInflationGovernor` struct that
    ///   contains the inflation parameters if successful. Otherwise, an error will be returned.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getInflationGovernor",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "initial": 0.15,
    ///     "terminal": 0.05,
    ///     "taper": 0.9,
    ///     "foundation": 0.02,
    ///     "foundation_term": 5.0
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - The inflation governor defines how inflation changes over time, ensuring the network's
    ///   growth remains stable and sustainable.
    /// - The `commitment` parameter allows users to define how strongly they want to ensure the
    ///   inflation data is confirmed or finalized when queried. For example, using `Confirmed` or
    ///   `Finalized` ensures a more reliable inflation state.
    ///
    /// ## Errors
    /// - If there is an issue with the `commitment` parameter or an internal error occurs,
    ///   an error will be returned.
    #[rpc(meta, name = "getInflationGovernor")]
    fn get_inflation_governor(
        &self,
        meta: Self::Metadata,
        commitment: Option<CommitmentConfig>,
    ) -> Result<RpcInflationGovernor>;

    /// Retrieves the current inflation rate for the network.
    ///
    /// This RPC method returns the current inflation rate, including the breakdown of inflation
    /// allocated to different entities such as validators and the foundation, along with the current
    /// epoch during which the rate applies.
    ///
    /// ## Parameters
    /// - No parameters are required for this method.
    ///
    /// ## Returns
    /// - `Result<RpcInflationRate>`: The method returns an `RpcInflationRate` struct that contains
    ///   the total inflation rate, the validator portion, the foundation portion, and the epoch
    ///   during which this inflation rate applies.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getInflationRate",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "total": 0.10,
    ///     "validator": 0.07,
    ///     "foundation": 0.03,
    ///     "epoch": 1500
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - The total inflation rate is distributed among validators and the foundation based on
    ///   the configuration defined in the inflation governor.
    /// - The epoch field indicates the current epoch number during which this inflation rate applies.
    ///   An epoch is a period during which the network operates under certain parameters.
    /// - Inflation rates can change over time depending on network conditions and governance decisions.
    ///
    /// ## Errors
    /// - If there is an internal error, or if the RPC request is malformed, an error will be returned.
    #[rpc(meta, name = "getInflationRate")]
    fn get_inflation_rate(&self, meta: Self::Metadata) -> Result<RpcInflationRate>;

    /// Retrieves the epoch schedule for the network.
    ///
    /// This RPC method returns the configuration for the network's epoch schedule, including
    /// details on the number of slots per epoch, leader schedule offsets, and epoch warmup.
    ///
    /// ## Parameters
    /// - No parameters are required for this method.
    ///
    /// ## Returns
    /// - `Result<EpochSchedule>`: The method returns an `EpochSchedule` struct, which contains
    ///   information about the slots per epoch, leader schedule offsets, warmup state, and the
    ///   first epoch after the warmup period.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getEpochSchedule",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "slotsPerEpoch": 432000,
    ///     "leaderScheduleSlotOffset": 500,
    ///     "warmup": true,
    ///     "firstNormalEpoch": 8,
    ///     "firstNormalSlot": 1073741824
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// - The `slots_per_epoch` defines the maximum number of slots in each epoch, which determines
    ///   the number of time slots available for network validators to produce blocks.
    /// - The `leader_schedule_slot_offset` specifies how many slots before an epoch’s start the leader
    ///   schedule calculation begins for that epoch.
    /// - The `warmup` field indicates whether the epochs start short and grow over time.
    /// - The `first_normal_epoch` marks the first epoch after the warmup period.
    /// - The `first_normal_slot` gives the first slot after the warmup period in terms of the number of slots
    ///   from the start of the network.
    ///
    /// ## Errors
    /// - If the RPC request is malformed, or if there is an internal error, an error will be returned.
    #[rpc(meta, name = "getEpochSchedule")]
    fn get_epoch_schedule(&self, meta: Self::Metadata) -> Result<EpochSchedule>;

    /// Retrieves the leader of the current slot.
    ///
    /// This RPC method returns the leader for the current slot in the Solana network. The leader is responsible
    /// for producing blocks for the current slot. The leader is selected based on the Solana consensus mechanism.
    ///
    /// ## Parameters
    /// - `config`: An optional configuration for the request, which can include:
    ///     - `commitment`: A commitment level that defines how "final" the data must be.
    ///     - `min_context_slot`: An optional parameter to specify a minimum slot for the request.
    ///
    /// ## Returns
    /// - `Result<String>`: The method returns a `String` representing the public key of the leader for the current slot.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getSlotLeader",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U"
    /// }
    /// ```
    ///
    /// # Notes
    /// - The leader for a given slot is selected based on the Solana network's consensus mechanism, and this method
    ///   allows you to query the current leader.
    #[rpc(meta, name = "getSlotLeader")]
    fn get_slot_leader(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<String>;

    /// Retrieves the leaders for a specified range of slots.
    ///
    /// This RPC method returns the leaders for a specified range of slots in the Solana network. You can
    /// specify the `start_slot` from which the leaders should be queried and limit the number of results
    /// with the `limit` parameter. The leaders are responsible for producing blocks in the respective slots.
    ///
    /// ## Parameters
    /// - `start_slot`: The starting slot number for which the leaders should be queried.
    /// - `limit`: The number of slots (starting from `start_slot`) for which the leaders should be retrieved.
    ///
    /// ## Returns
    /// - `Result<Vec<String>>`: A vector of `String` values representing the public keys of the leaders for
    ///   the specified slot range.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getSlotLeaders",
    ///   "params": [1000, 5]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     "3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U",
    ///     "BBh1FwXts8EZY6rPZ5kS2ygq99wYjFd5K5daRjc7eF9X",
    ///     "4XYo7yP5J2J8sLNSW3wGYPk3mdS1rbZUy4oFCp7wH1DN",
    ///     "8v1Cp6sHZh8XfGWS7sHZczH3v9NxdgMbo3g91Sh88dcJ",
    ///     "N6bPqwEoD9StS4AnzE27rHyz47tPcsZQjvW9w8p2NhF7"
    ///   ]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The leaders are returned in the order corresponding to the slots queried, starting from `start_slot`
    ///   and continuing for `limit` slots.
    /// - This method provides an efficient way to get multiple leaders for a range of slots, useful for tracking
    ///   leaders over time or for scheduling purposes in decentralized applications.
    #[rpc(meta, name = "getSlotLeaders")]
    fn get_slot_leaders(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        limit: u64,
    ) -> Result<Vec<String>>;

    /// Retrieves block production information for the specified validator identity or range of slots.
    ///
    /// This RPC method returns block production details for a given validator identity or a range of slots
    /// within a certain epoch. If no `identity` is provided, the method returns block production data for all
    /// validators. If a `range` is provided, it will return block production information for the slots within
    /// the specified range.
    ///
    /// ## Parameters
    /// - `config`: An optional configuration object that can include:
    ///     - `identity`: The base-58 encoded public key of a validator to query for block production data. If `None`, results for all validators will be returned.
    ///     - `range`: A range of slots for which block production information is needed. The range will default to the current epoch if `None`.
    ///     - `commitment`: The commitment level (optional) to use when querying for the block production data.
    ///
    /// ## Returns
    /// - `Result<RpcResponse<RpcBlockProduction>>`: The result contains a response object with block production data, including the number of leader slots and blocks produced by each validator.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "getBlockProduction",
    ///   "params": [{
    ///     "identity": "3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U",
    ///     "range": {
    ///       "firstSlot": 1000,
    ///       "lastSlot": 1050
    ///     }
    ///   }]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "byIdentity": {
    ///       "3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U": [10, 8],
    ///       "BBh1FwXts8EZY6rPZ5kS2ygq99wYjFd5K5daRjc7eF9X": [5, 4]
    ///     },
    ///     "range": {
    ///       "firstSlot": 1000,
    ///       "lastSlot": 1050
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// # Notes
    /// - The response contains a map of validator identities to a tuple of two values:
    ///     - The first value is the number of leader slots.
    ///     - The second value is the number of blocks produced by that validator in the queried range.
    /// - The `range` object specifies the range of slots that the block production information applies to, with `first_slot` being the starting slot and `last_slot` being the optional ending slot.
    ///
    /// ## Example Response Interpretation
    /// - In the example response, the identity `3HgA9r8H9z5Pb2L6Pt5Yq1QoFwgr6YwdKKUh9n2ANp5U` produced 10 leader slots and 8 blocks between slots 1000 and 1050.
    /// - Similarly, `BBh1FwXts8EZY6rPZ5kS2ygq99wYjFd5K5daRjc7eF9X` produced 5 leader slots and 4 blocks in the same slot range.
    #[rpc(meta, name = "getBlockProduction")]
    fn get_block_production(
        &self,
        meta: Self::Metadata,
        config: Option<RpcBlockProductionConfig>,
    ) -> Result<RpcResponse<RpcBlockProduction>>;
}

pub struct SurfpoolBankDataRpc;
impl BankData for SurfpoolBankDataRpc {
    type Metadata = Option<RunloopContext>;

    fn get_minimum_balance_for_rent_exemption(
        &self,
        meta: Self::Metadata,
        data_len: usize,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<u64> {
        let ctx = meta.get_state()?;

        Ok(ctx.svm.minimum_balance_for_rent_exemption(data_len))
    }

    fn get_inflation_governor(
        &self,
        _meta: Self::Metadata,
        _commitment: Option<CommitmentConfig>,
    ) -> Result<RpcInflationGovernor> {
        not_implemented_err()
    }

    fn get_inflation_rate(&self, _meta: Self::Metadata) -> Result<RpcInflationRate> {
        not_implemented_err()
    }

    fn get_epoch_schedule(&self, _meta: Self::Metadata) -> Result<EpochSchedule> {
        Ok(EpochSchedule {
            slots_per_epoch: 0,
            leader_schedule_slot_offset: 0,
            warmup: false,
            first_normal_epoch: 0,
            first_normal_slot: 0,
        })
    }

    fn get_slot_leader(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<String> {
        not_implemented_err()
    }

    fn get_slot_leaders(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _limit: u64,
    ) -> Result<Vec<String>> {
        not_implemented_err()
    }

    fn get_block_production(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcBlockProductionConfig>,
    ) -> Result<RpcResponse<RpcBlockProduction>> {
        not_implemented_err()
    }
}
