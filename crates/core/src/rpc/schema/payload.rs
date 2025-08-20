use schemars::JsonSchema;

use super::surfnet_cheatcodes::{
    CloneProgramAccount, GetIdl, GetLocalSignatures, GetProfileResults, GetTransactionProfile,
    PauseClock, ProfileTransaction, RegisterIdl, ResumeClock, SetAccount, SetProgramAuthority,
    SetSupply, SetTokenAccount, TimeTravel,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "Request payload types for all Surfnet cheatcodes JSON-RPC endpoints")]
pub struct SurfnetCheatcodesRequestPayloads {
    #[schemars(description = "Payload for surfnet_setAccount")]
    pub set_account: SetAccount,

    #[schemars(description = "Payload for surfnet_setTokenAccount")]
    pub set_token_account: SetTokenAccount,

    #[schemars(description = "Payload for surfnet_cloneProgramAccount")]
    pub clone_program_account: CloneProgramAccount,

    #[schemars(description = "Payload for surfnet_profileTransaction")]
    pub profile_transaction: ProfileTransaction,

    #[schemars(description = "Payload for surfnet_getProfileResultsByTag")]
    pub get_profile_results: GetProfileResults,

    #[schemars(description = "Payload for surfnet_setSupply")]
    pub set_supply: SetSupply,

    #[schemars(description = "Payload for surfnet_setProgramAuthority")]
    pub set_program_authority: SetProgramAuthority,

    #[schemars(description = "Payload for surfnet_getTransactionProfile")]
    pub get_transaction_profile: GetTransactionProfile,

    #[schemars(description = "Payload for surfnet_registerIdl")]
    pub register_idl: RegisterIdl,

    #[schemars(description = "Payload for surfnet_getActiveIdl")]
    pub get_idl: GetIdl,

    #[schemars(description = "Payload for surfnet_getLocalSignatures")]
    pub get_local_signatures: GetLocalSignatures,

    #[schemars(description = "Payload for surfnet_timeTravel")]
    pub time_travel: TimeTravel,

    #[schemars(description = "Payload for surfnet_pauseClock")]
    pub pause_clock: PauseClock,

    #[schemars(description = "Payload for surfnet_resumeClock")]
    pub resume_clock: ResumeClock,
}
