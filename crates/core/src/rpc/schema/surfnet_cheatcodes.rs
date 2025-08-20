use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_tuple::Serialize_tuple;
use solana_pubkey::Pubkey;
use solana_sdk::system_program;

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    #[schemars(description = "The new balance in lamports (1 SOL = 1,000,000,000 lamports).")]
    pub lamports: Option<u64>,
    #[schemars(
        description = "The new owner program ID that controls this account, as a base-58 encoded string."
    )]
    pub owner: Option<String>,
    #[schemars(
        description = "Whether the account should be executable (true for program accounts, false for data accounts)."
    )]
    pub executable: Option<bool>,
    #[schemars(description = "The new rent epoch when this account will next owe rent.")]
    pub rent_epoch: Option<u64>,
    #[schemars(
        description = "The new account data, as a base-64 encoded string. This contains the actual data stored in the account."
    )]
    pub data: Option<String>,
}

impl AccountUpdate {
    pub fn example() -> Self {
        Self {
            lamports: Some(1_000_000_000), // 1 SOL
            owner: Some(system_program::id().to_string()),
            executable: Some(false),
            rent_epoch: Some(0),
            data: Some("0x123456".to_string()),
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    #[schemars(
        description = "The new token balance amount in the smallest unit (e.g., lamports for SOL, or the smallest token unit)."
    )]
    pub amount: Option<u64>,
    #[schemars(
        description = "The new delegate account that can spend tokens on behalf of the owner."
    )]
    pub delegate: Option<SetSomeAccount>,
    #[schemars(description = "The new account state (e.g., 'initialized', 'frozen', 'closed').")]
    pub state: Option<String>,
    #[schemars(description = "The new delegated amount that the delegate is authorized to spend.")]
    pub delegated_amount: Option<u64>,
    #[schemars(
        description = "The new close authority that can close the account and recover rent."
    )]
    pub close_authority: Option<SetSomeAccount>,
}

impl TokenAccountUpdate {
    pub fn example() -> Self {
        Self {
            amount: Some(1_000_000_000),
            delegate: Some(SetSomeAccount::Account(Pubkey::new_unique().to_string())),
            state: Some("initialized".to_string()),
            delegated_amount: Some(1_000_000_000),
            close_authority: Some(SetSomeAccount::Account(Pubkey::new_unique().to_string())),
        }
    } 
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetSomeAccount {
    #[schemars(
        description = "Specifies an account, as a base-58 encoded string. Use this to set a specific account as the delegate or authority."
    )]
    Account(String),
    #[schemars(
        description = "Specifies no account. Use this to remove a delegate or authority (set to None)."
    )]
    NoAccount,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplyUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The new total supply of SOL in lamports across the entire network.")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "The new circulating supply of SOL in lamports (total supply minus non-circulating)."
    )]
    pub circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "The new non-circulating supply of SOL in lamports (locked in non-circulating accounts)."
    )]
    pub non_circulating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "The new list of non-circulating account addresses that hold locked SOL."
    )]
    pub non_circulating_accounts: Option<Vec<String>>,
}

impl SupplyUpdate {
    pub fn example() -> Self {
        Self {
            total: Some(1_000_000_000),
            circulating: Some(1_000_000_000),
            non_circulating: Some(1_000_000_000),
            non_circulating_accounts: Some(vec![]),
        }
    } 
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcProfileResultConfig {
    #[schemars(
        description = "The encoding format for returned account data (e.g., 'base64', 'base58', 'jsonParsed')."
    )]
    pub encoding: Option<String>,
    #[schemars(
        description = "The depth of profiling - 'transaction' for overall transaction profile, 'instruction' for per-instruction breakdown."
    )]
    pub depth: Option<String>,
}

impl RpcProfileResultConfig {
    pub fn example() -> Self {
        Self {
            encoding: Some("base64".to_string()),
            depth: Some("transaction".to_string()),
        }
    } 
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UuidOrSignature {
    #[schemars(
        description = "A UUID string that uniquely identifies a transaction profile in the system."
    )]
    Uuid(String),
    #[schemars(
        description = "A transaction signature as a base-58 encoded string that identifies a specific transaction."
    )]
    Signature(String),
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeTravelConfig {
    #[schemars(
        description = "Move to the specified absolute epoch number (each epoch = 432,000 slots)."
    )]
    AbsoluteEpoch(u64),
    #[schemars(description = "Move to the specified absolute slot number in the blockchain.")]
    AbsoluteSlot(u64),
    #[schemars(description = "Move to the specified UNIX timestamp in milliseconds since epoch.")]
    AbsoluteTimestamp(u64),
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Idl {
    #[schemars(
        description = "The program address that this IDL describes, as a base-58 encoded string."
    )]
    pub address: String,
    #[schemars(
        description = "The IDL metadata containing program name, version, and description information."
    )]
    pub metadata: serde_json::Value,
    #[schemars(
        description = "The IDL instructions array defining all available program instructions and their parameters."
    )]
    pub instructions: Vec<serde_json::Value>,
    #[schemars(
        description = "The IDL accounts array defining all account types that the program can create or interact with."
    )]
    pub accounts: Vec<serde_json::Value>,
    #[schemars(
        description = "The IDL types array defining custom data types used by the program."
    )]
    pub types: Vec<serde_json::Value>,
    #[schemars(description = "The IDL events array defining events that the program can emit.")]
    pub events: Vec<serde_json::Value>,
    #[schemars(
        description = "The IDL errors array defining custom error types that the program can return."
    )]
    pub errors: Vec<serde_json::Value>,
    #[schemars(
        description = "The IDL constants array defining constant values used by the program."
    )]
    pub constants: Vec<serde_json::Value>,
    #[schemars(
        description = "The IDL state object defining the program's state account structure (if applicable)."
    )]
    pub state: Option<serde_json::Value>,
}

impl Idl {
    pub fn example() -> Self {
        Self {
            address: Pubkey::new_unique().to_string(),
            metadata: serde_json::json!({
                "name": "Test Program",
                "version": "1.0.0",
                "description": "A test program for the Surfnet Cheatcodes"
            }),
            instructions: vec![],
            accounts: vec![],
            types: vec![],
            events: vec![],
            errors: vec![],
            constants: vec![],
            state: None,
        }
    }
    
}

#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum SurfnetCheatcodes {
    #[schemars(
        description = "Sets or updates an account's properties including lamports, data, owner, and executable status."
    )]
    SetAccount(SetAccount),
    #[schemars(
        description = "Sets or updates a token account's properties including balance, delegate, state, and authorities."
    )]
    SetTokenAccount(SetTokenAccount),
    #[schemars(
        description = "Clones a program account from one program ID to another, including its associated program data."
    )]
    CloneProgramAccount(CloneProgramAccount),
    #[schemars(
        description = "Profiles a transaction to analyze compute units, account changes, and execution details."
    )]
    ProfileTransaction(ProfileTransaction),
    #[schemars(
        description = "Retrieves all profiling results for transactions tagged with a specific identifier."
    )]
    GetProfileResults(GetProfileResults),
    #[schemars(
        description = "Configures the network supply information including total, circulating, and non-circulating amounts."
    )]
    SetSupply(SetSupply),
    #[schemars(
        description = "Sets or removes the upgrade authority for a program's ProgramData account."
    )]
    SetProgramAuthority(SetProgramAuthority),
    #[schemars(
        description = "Retrieves the detailed profile of a specific transaction by signature or UUID."
    )]
    GetTransactionProfile(GetTransactionProfile),
    #[schemars(
        description = "Registers an IDL (Interface Definition Language) for a program to enable account data parsing."
    )]
    RegisterIdl(RegisterIdl),
    #[schemars(
        description = "Retrieves the registered IDL for a specific program ID at a given slot."
    )]
    GetIdl(GetIdl),
    #[schemars(
        description = "Retrieves the most recent transaction signatures from the local network."
    )]
    GetLocalSignatures(GetLocalSignatures),
    #[schemars(
        description = "Advances or rewinds the local network's clock to test time-sensitive logic."
    )]
    TimeTravel(TimeTravel),
    #[schemars(
        description = "Pauses the local network's clock progression, halting all time-based operations."
    )]
    PauseClock(PauseClock),
    #[schemars(description = "Resumes the local network's clock progression after it was paused.")]
    ResumeClock(ResumeClock),
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct SetAccount {
    #[schemars(
        description = "The public key of the account to update, as a base-58 encoded string. This identifies which account will be modified."
    )]
    pub pubkey: String, 
    #[schemars(
        description = "The account data to update. Contains the new values for lamports, owner, executable status, rent epoch, and data."
    )]
    pub update: AccountUpdate,
}

impl SetAccount {
    pub fn example() -> Self {
        Self {
            pubkey: Pubkey::new_unique().to_string(),
            update: AccountUpdate::example(),
        }
    }
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct SetTokenAccount {
    #[schemars(
        description = "The public key of the token account owner, as a base-58 encoded string. This is the wallet that owns the token account."
    )]
    pub owner: String,
    #[schemars(
        description = "The public key of the token mint, as a base-58 encoded string. This identifies the specific token type (e.g., USDC, SOL)."
    )]
    pub mint: String,
    #[schemars(
        description = "The token account data to update. Contains new values for balance, delegate, state, and authorities."
    )]
    pub update: TokenAccountUpdate,
    #[schemars(
        description = "The token program ID, as a base-58 encoded string. Defaults to SPL Token program if not specified."
    )]
    pub token_program: Option<String>,
}

impl SetTokenAccount {
    pub fn example() -> Self {
        Self {
            owner: Pubkey::new_unique().to_string(),
            mint: Pubkey::new_unique().to_string(),
            update: TokenAccountUpdate::example(),
            token_program: Some(spl_token::ID.to_string()),
        }
    }
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct CloneProgramAccount {
    #[schemars(
        description = "The public key of the source program to clone, as a base-58 encoded string. This program's account and data will be copied."
    )]
    pub source_program_id: String,
    #[schemars(
        description = "The public key of the destination program, as a base-58 encoded string. This is where the cloned program will be placed."
    )]
    pub destination_program_id: String,
}

impl CloneProgramAccount {
    pub fn example() -> Self {
        Self {
            source_program_id: Pubkey::new_unique().to_string(),
            destination_program_id: Pubkey::new_unique().to_string(),
        }
    }
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTransaction {
    #[schemars(
        description = "The transaction data to profile, as a base-64 encoded string. This should be a serialized VersionedTransaction."
    )]
    pub transaction_data: String,
    #[schemars(
        description = "An optional tag to identify the profiling results. Useful for grouping related transaction profiles."
    )]
    pub tag: Option<String>,
    #[schemars(
        description = "Configuration for the profile result, including encoding format and profiling depth."
    )]
    pub config: Option<RpcProfileResultConfig>,
}

impl ProfileTransaction {
    pub fn example() -> Self {
        Self {
            transaction_data: "0x123456".to_string(),
            tag: Some("Tag".to_string()),
            config: Some(RpcProfileResultConfig::example()),
        }
    }
    
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct GetProfileResults {
    #[schemars(
        description = "The tag to retrieve profiling results for. Returns all transaction profiles that were tagged with this identifier."
    )]
    pub tag: String,
    #[schemars(
        description = "Configuration for the profile result, including encoding format and profiling depth."
    )]
    pub config: Option<RpcProfileResultConfig>,
}

impl GetProfileResults {
    pub fn example() -> Self {
        Self {
            tag: "Tag".to_string(),
            config: Some(RpcProfileResultConfig::example()),
        }
    } 
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct SetSupply {
    #[schemars(
        description = "The supply data to update. Contains new values for total, circulating, and non-circulating SOL amounts."
    )]
    pub update: SupplyUpdate,
}

impl SetSupply {
    pub fn example() -> Self {
        Self {
            update: SupplyUpdate::example(),
        }
    }
    
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct SetProgramAuthority {
    #[schemars(
        description = "The public key of the program, as a base-58 encoded string. This is the program whose upgrade authority will be modified."
    )]
    pub program_id: String,
    #[schemars(
        description = "The public key of the new authority, as a base-58 encoded string. If omitted, the program will have no upgrade authority (immutable)."
    )]
    pub new_authority: Option<String>,
}

impl SetProgramAuthority {
    pub fn example() -> Self {
        Self {
            program_id: Pubkey::new_unique().to_string(),
            new_authority: Some(Pubkey::new_unique().to_string()),
        }
    } 
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct GetTransactionProfile {
    #[schemars(
        description = "The transaction signature (as a base-58 string) or a UUID (as a string) for which to retrieve the profile. This identifies the specific transaction to analyze."
    )]
    pub signature_or_uuid: UuidOrSignature,
    #[schemars(
        description = "Configuration for the profile result, including encoding format and profiling depth."
    )]
    pub config: Option<RpcProfileResultConfig>,
}

impl GetTransactionProfile {
    pub fn example() -> Self {
        Self {
            signature_or_uuid: UuidOrSignature::Signature(Pubkey::new_unique().to_string()),
            config: Some(RpcProfileResultConfig::example()),
        }
    } 
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct RegisterIdl {
    #[schemars(
        description = "The full IDL object to be registered in memory. The `address` field should match the program's public key. This enables account data parsing for the program."
    )]
    pub idl: Idl,
    #[schemars(
        description = "The slot at which to register the IDL. If omitted, uses the latest slot. This determines when the IDL becomes active for account parsing."
    )]
    pub slot: Option<u64>,
}

impl RegisterIdl {
    pub fn example() -> Self {
        Self {
            idl: Idl::example(),
            slot: Some(0),
        }
    }
    
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct GetIdl {
    #[schemars(
        description = "The public key of the program whose IDL is being requested, as a base-58 encoded string. This identifies which program's IDL to retrieve."
    )]
    pub program_id: String,
    #[schemars(
        description = "The slot at which to query the IDL. If omitted, uses the latest slot. This determines which version of the IDL to return."
    )]
    pub slot: Option<u64>,
}

impl GetIdl {
    pub fn example() -> Self {
        Self {
            program_id: Pubkey::new_unique().to_string(),
            slot: Some(0),
        }
    }
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct GetLocalSignatures {
    #[schemars(
        description = "The maximum number of signatures to return. Defaults to 50 if not specified. Returns the most recent signatures first."
    )]
    pub limit: Option<u64>,
}

impl GetLocalSignatures {
    pub fn example() -> Self {
        Self {
            limit: Some(50),
        }
    } 
}

#[derive(JsonSchema, Serialize_tuple)]
#[serde(rename_all = "camelCase")]
pub struct TimeTravel {
    #[schemars(
        description = "Configuration specifying how to modify the clock. Can move to a specific epoch, slot, or timestamp."
    )]
    pub config: Option<TimeTravelConfig>,
}

impl TimeTravel {
    pub fn example() -> Self {
        Self {
            config: Some(TimeTravelConfig::AbsoluteEpoch(0)),
        }
    }
}

impl TimeTravelConfig {
    pub fn example() -> Self {
        Self::AbsoluteEpoch(0)
    }
}

#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseClock {}

impl PauseClock {
    pub fn example() -> Self {
        Self {}
    }
}

#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeClock {}

impl ResumeClock {
    pub fn example() -> Self {
        Self {}
    }
}