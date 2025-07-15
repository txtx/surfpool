use schemars::JsonSchema;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum Admin {
    #[schemars(description = "Immediately shuts down the RPC server.")]
    Exit,
    #[schemars(description = "Reloads a runtime plugin with new configuration.")]
    ReloadPlugin(ReloadPlugin),
    #[schemars(description = "Unloads a runtime plugin.")]
    UnloadPlugin(UnloadPlugin),
    #[schemars(
        description = "Dynamically loads a new plugin into the runtime from a configuration file."
    )]
    LoadPlugin(LoadPlugin),
    #[schemars(description = "Returns a list of all currently loaded plugin names.")]
    ListPlugins,
    #[schemars(description = "Returns the address of the RPC server.")]
    RpcAddress,
    #[schemars(description = "Sets a filter for log messages in the system.")]
    SetLogFilter(SetLogFilter),
    #[schemars(description = "Returns the system start time.")]
    StartTime,
    #[schemars(description = "Adds an authorized voter to the system.")]
    AddAuthorizedVoter(AddAuthorizedVoter),
    #[schemars(
        description = "Adds an authorized voter to the system using a byte-encoded keypair."
    )]
    AddAuthorizedVoterFromBytes(AddAuthorizedVoterFromBytes),
    #[schemars(description = "Removes all authorized voters from the system.")]
    RemoveAllAuthorizedVoters,
    #[schemars(description = "Sets the identity for the system using the provided keypair.")]
    SetIdentity(SetIdentity),
    #[schemars(
        description = "Sets the identity for the system using a keypair provided as a byte array."
    )]
    SetIdentityFromBytes(SetIdentityFromBytes),
    #[schemars(description = "Sets the overrides for staked nodes using a specified path.")]
    SetStakedNodesOverrides(SetStakedNodesOverrides),
    #[schemars(description = "Repairs a shred from a peer node in the network.")]
    RepairShredFromPeer(RepairShredFromPeer),
    #[schemars(description = "Sets the whitelist of nodes allowed to repair shreds.")]
    SetRepairWhitelist(SetRepairWhitelist),
    #[schemars(description = "Retrieves the size of the secondary index key for a given account.")]
    GetSecondaryIndexKeySize(GetSecondaryIndexKeySize),
    #[schemars(description = "Sets the public TPU (Transaction Processing Unit) address.")]
    SetPublicTpuAddress(SetPublicTpuAddress),
    #[schemars(description = "Sets the public TPU forwards address.")]
    SetPublicTpuForwardsAddress(SetPublicTpuForwardsAddress),
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReloadPlugin {
    #[schemars(description = "The name of the plugin to reload.")]
    pub name: String,
    #[schemars(description = "The path to the new configuration file for the plugin.")]
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnloadPlugin {
    #[schemars(description = "The name of the plugin to unload.")]
    pub name: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadPlugin {
    #[schemars(description = "The path to the configuration file for the new plugin.")]
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetLogFilter {
    #[schemars(description = "The log filter string to apply.")]
    pub filter: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorizedVoter {
    #[schemars(description = "Path to the keypair file for the authorized voter.")]
    pub keypair_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorizedVoterFromBytes {
    #[schemars(description = "Byte array representing the keypair for the authorized voter.")]
    pub keypair: Vec<u8>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentity {
    #[schemars(description = "Path to the keypair file to be used as the node's identity.")]
    pub keypair_file: String,
    #[schemars(description = "Boolean indicating if a tower is required for this identity.")]
    pub require_tower: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentityFromBytes {
    #[schemars(description = "Byte array representing the identity keypair.")]
    pub identity_keypair: Vec<u8>,
    #[schemars(description = "Boolean indicating if a tower is required for this identity.")]
    pub require_tower: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetStakedNodesOverrides {
    #[schemars(description = "Path to the file containing staked nodes overrides.")]
    pub path: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepairShredFromPeer {
    #[schemars(
        description = "The public key of the peer to repair from, as a base-58 encoded string."
    )]
    pub pubkey: Option<String>,
    #[schemars(description = "The slot of the shred to repair.")]
    pub slot: u64,
    #[schemars(description = "The index of the shred to repair.")]
    pub shred_index: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetRepairWhitelist {
    #[schemars(
        description = "A list of public keys (base-58 encoded strings) to set as the repair whitelist."
    )]
    pub whitelist: Vec<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSecondaryIndexKeySize {
    #[schemars(
        description = "The public key of the account to get the secondary index key size for, as a base-58 encoded string."
    )]
    pub pubkey_str: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetPublicTpuAddress {
    #[schemars(description = "The public TPU address as a string.")]
    pub public_tpu_addr: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetPublicTpuForwardsAddress {
    #[schemars(description = "The public TPU forwards address as a string.")]
    pub public_tpu_forwards_addr: String,
}
