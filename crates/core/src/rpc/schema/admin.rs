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
    pub name: String,
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnloadPlugin {
    pub name: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadPlugin {
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetLogFilter {
    pub filter: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorizedVoter {
    pub keypair_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorizedVoterFromBytes {
    pub keypair: Vec<u8>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentity {
    pub keypair_file: String,
    pub require_tower: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentityFromBytes {
    pub identity_keypair: Vec<u8>,
    pub require_tower: bool,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetStakedNodesOverrides {
    pub path: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepairShredFromPeer {
    pub pubkey: Option<String>,
    pub slot: u64,
    pub shred_index: u64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetRepairWhitelist {
    pub whitelist: Vec<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSecondaryIndexKeySize {
    pub pubkey_str: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetPublicTpuAddress {
    pub public_tpu_addr: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetPublicTpuForwardsAddress {
    pub public_tpu_forwards_addr: String,
}
