use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use txtx_addon_kit::types::types::Value;
use txtx_addon_network_svm_types::{SvmValue, subgraph::IndexedSubgraphSourceTypeName};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubgraphDataEntry {
    // The UUID of the entry
    pub uuid: Uuid,
    // A map of field names and their values
    pub values: HashMap<String, Value>,
    /// Default values for every entry, depending on the type of data being indexed
    pub table_defaults: SubgraphDataEntryTableDefaults,
}

impl SubgraphDataEntry {
    pub fn from_data_sourcing_entry(
        subgraph_uuid: &Uuid,
        entry: SchemaDataSourcingEventEntry,
    ) -> Result<Vec<Self>, String> {
        let err_ctx = "Failed to apply new database entry to subgraph";
        let mut result = vec![];
        match entry {
            SchemaDataSourcingEventEntry::CpiEvent(cpi) => {
                let entries: Vec<HashMap<String, Value>> = serde_json::from_slice(&cpi.data).map_err(|e| {
                    format!("{err_ctx}: Failed to deserialize new cpi event database entry for subgraph {}: {}", subgraph_uuid, e)
                })?;
                for entry in entries.into_iter() {
                    result.push(Self::cpi_event(
                        Uuid::new_v4(),
                        entry,
                        cpi.slot,
                        cpi.transaction_signature.into(),
                    ));
                }
            }
            SchemaDataSourcingEventEntry::Pda(pda_entry) => {
                let entries: Vec<HashMap<String, Value>> = serde_json::from_slice(&pda_entry.data).map_err(|e| {
                    format!("{err_ctx}: Failed to deserialize new pda database entry for subgraph {}: {}", subgraph_uuid, e)
                })?;
                for entry in entries.into_iter() {
                    result.push(Self::pda(
                        Uuid::new_v4(),
                        entry,
                        pda_entry.slot,
                        pda_entry.transaction_signature.into(),
                        pda_entry.pubkey.into(),
                        pda_entry.owner.into(),
                        pda_entry.lamports,
                        pda_entry.write_version,
                    ));
                }
            }
        }
        Ok(result)
    }

    pub fn from_data_row(
        dynamic_values: HashMap<String, Value>,
        default_values: Vec<Value>,
        source_type: &IndexedSubgraphSourceTypeName,
    ) -> Self {
        let uuid = Uuid::parse_str(default_values[0].expect_string()).unwrap_or(Uuid::nil());

        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => {
                let slot = default_values[1].expect_integer().try_into().unwrap_or(0);
                let transaction_signature = default_values[2]
                    .as_string()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| Signature::from([0u8; 64]));
                Self::cpi_event(uuid, dynamic_values, slot, transaction_signature)
            }
            IndexedSubgraphSourceTypeName::Pda => {
                let slot = default_values[1].expect_integer().try_into().unwrap_or(0);
                let transaction_signature = default_values[2]
                    .as_string()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| Signature::from([0u8; 64]));
                let pubkey = SvmValue::to_pubkey(&default_values[3]).unwrap_or(Pubkey::default());
                let owner = SvmValue::to_pubkey(&default_values[4]).unwrap_or(Pubkey::default());
                let lamports = default_values[5].expect_integer().try_into().unwrap_or(0);
                let write_version = default_values[6].expect_integer().try_into().unwrap_or(0);
                Self::pda(
                    uuid,
                    dynamic_values,
                    slot,
                    transaction_signature,
                    pubkey,
                    owner,
                    lamports,
                    write_version,
                )
            }
        }
    }

    pub fn cpi_event(
        uuid: Uuid,
        values: HashMap<String, Value>,
        slot: u64,
        transaction_signature: Signature,
    ) -> Self {
        Self {
            uuid,
            values,
            table_defaults: SubgraphDataEntryTableDefaults::CpiEvent(CpiEventTableDefaults {
                slot,
                transaction_signature,
            }),
        }
    }

    pub fn pda(
        uuid: Uuid,
        values: HashMap<String, Value>,
        slot: u64,
        transaction_signature: Signature,
        pubkey: Pubkey,
        owner: Pubkey,
        lamports: u64,
        write_version: u64,
    ) -> Self {
        Self {
            uuid,
            values,
            table_defaults: SubgraphDataEntryTableDefaults::Pda(PdaTableDefaults {
                slot,
                transaction_signature,
                pubkey,
                owner,
                lamports,
                write_version,
            }),
        }
    }

    pub fn default_columns(&self) -> Vec<String> {
        self.table_defaults.default_columns()
    }

    pub fn values(self) -> (Vec<String>, HashMap<String, Value>) {
        let uuid = self.uuid;
        let values = self.values;
        let defaults = self.table_defaults.default_values(format!("'{}'", uuid));
        (defaults, values)
    }

    pub fn column_metadata(source_type: &IndexedSubgraphSourceTypeName) -> Vec<String> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => CpiEventTableDefaults::column_metadata(),
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::column_metadata(),
        }
    }

    pub fn default_column_numbers(source_type: &IndexedSubgraphSourceTypeName) -> usize {
        Self::column_metadata(source_type).len() - 1 // -1 to omit the id field
    }

    pub fn default_columns_with_descriptions(
        source_type: &IndexedSubgraphSourceTypeName,
    ) -> Vec<(String, String)> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => {
                CpiEventTableDefaults::columns_with_descriptions()
            }
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::columns_with_descriptions(),
        }
    }
    pub fn default_columns_with_types(source_type: &IndexedSubgraphSourceTypeName) -> Vec<String> {
        match source_type {
            IndexedSubgraphSourceTypeName::Instruction => unimplemented!(),
            IndexedSubgraphSourceTypeName::Event => CpiEventTableDefaults::column_metadata(),
            IndexedSubgraphSourceTypeName::Pda => PdaTableDefaults::column_metadata(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SubgraphDataEntryTableDefaults {
    CpiEvent(CpiEventTableDefaults),
    Pda(PdaTableDefaults),
}

impl SubgraphDataEntryTableDefaults {
    pub fn default_columns(&self) -> Vec<String> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(_) => CpiEventTableDefaults::columns(),
            SubgraphDataEntryTableDefaults::Pda(_) => PdaTableDefaults::columns(),
        }
    }
    pub fn default_values(&self, uuid: String) -> Vec<String> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(defaults) => defaults.default_values(uuid),
            SubgraphDataEntryTableDefaults::Pda(defaults) => defaults.default_values(uuid),
        }
    }
    pub fn slot(&self) -> u64 {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(defaults) => defaults.slot,
            SubgraphDataEntryTableDefaults::Pda(defaults) => defaults.slot,
        }
    }
    pub fn transaction_signature(&self) -> Signature {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(defaults) => defaults.transaction_signature,
            SubgraphDataEntryTableDefaults::Pda(defaults) => defaults.transaction_signature,
        }
    }
    pub fn pubkey(&self) -> Option<Pubkey> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(_) => None,
            SubgraphDataEntryTableDefaults::Pda(defaults) => Some(defaults.pubkey),
        }
    }
    pub fn owner(&self) -> Option<Pubkey> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(_) => None,
            SubgraphDataEntryTableDefaults::Pda(defaults) => Some(defaults.owner),
        }
    }

    pub fn lamports(&self) -> Option<u64> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(_) => None,
            SubgraphDataEntryTableDefaults::Pda(defaults) => Some(defaults.lamports),
        }
    }

    pub fn write_version(&self) -> Option<u64> {
        match self {
            SubgraphDataEntryTableDefaults::CpiEvent(_) => None,
            SubgraphDataEntryTableDefaults::Pda(defaults) => Some(defaults.write_version),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CpiEventTableDefaults {
    // The slot that the transaction that created this entry was processed in
    pub slot: u64,
    // The transaction signature that created this entry
    pub transaction_signature: Signature,
}

impl CpiEventTableDefaults {
    pub fn columns() -> Vec<String> {
        vec!["uuid".into(), "slot".into(), "transactionSignature".into()]
    }

    pub fn columns_with_descriptions() -> Vec<(String, String)> {
        vec![
            ("uuid".into(), "The UUID of the entry".into()),
            (
                "slot".into(),
                "The slot that the transaction was processed in".into(),
            ),
            (
                "transactionSignature".into(),
                "The transaction signature that created this entry".into(),
            ),
        ]
    }

    pub fn column_metadata() -> Vec<String> {
        vec![
            "id INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
            "uuid TEXT".to_string(),
            "slot INTEGER".to_string(),
            "transactionSignature TEXT".to_string(),
        ]
    }
    pub fn default_values(&self, uuid: String) -> Vec<String> {
        vec![
            uuid,
            self.slot.to_string(),
            format!("'{}'", self.transaction_signature),
        ]
    }
}

#[derive(Debug, Clone)]
pub struct PdaTableDefaults {
    // The slot that the transaction that created this entry was processed in
    pub slot: u64,
    // The transaction signature that created this entry
    pub transaction_signature: Signature,
    /// The pubkey of the account
    pub pubkey: Pubkey,
    /// The pubkey of the owner of the account
    pub owner: Pubkey,
    /// The lamports of the account
    pub lamports: u64,
    /// The monotonically increasing index of the account update
    pub write_version: u64,
}

impl PdaTableDefaults {
    pub fn columns() -> Vec<String> {
        vec![
            "uuid".into(),
            "slot".into(),
            "transactionSignature".into(),
            "pubkey".into(),
            "owner".into(),
            "lamports".into(),
            "writeVersion".into(),
        ]
    }
    pub fn columns_with_descriptions() -> Vec<(String, String)> {
        vec![
            ("uuid".into(), "The UUID of the entry".into()),
            (
                "slot".into(),
                "The slot that the transaction was processed in".into(),
            ),
            (
                "transactionSignature".into(),
                "The transaction signature that created this entry".into(),
            ),
            ("pubkey".into(), "The pubkey of the account".into()),
            (
                "owner".into(),
                "The pubkey of the owner of the account".into(),
            ),
            ("lamports".into(), "The lamports of the account".into()),
            (
                "writeVersion".into(),
                "The monotonically increasing index of the account update".into(),
            ),
        ]
    }

    pub fn column_metadata() -> Vec<String> {
        vec![
            "id INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
            "uuid TEXT".to_string(),
            "slot INTEGER".to_string(),
            "transactionSignature TEXT".to_string(),
            "pubkey TEXT".to_string(),
            "owner TEXT".to_string(),
            "lamports INTEGER".to_string(),
            "writeVersion INTEGER".to_string(),
        ]
    }
    pub fn default_values(&self, uuid: String) -> Vec<String> {
        vec![
            uuid,
            self.slot.to_string(),
            format!("'{}'", self.transaction_signature),
            format!("'{}'", self.pubkey),
            format!("'{}'", self.owner),
            self.lamports.to_string(),
            self.write_version.to_string(),
        ]
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SchemaDataSourcingEvent {
    Rountrip(Uuid),
    ApplyEntry(Uuid, SchemaDataSourcingEventEntry),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SchemaDataSourcingEventEntry {
    CpiEvent(CpiEventEntry),
    Pda(PdaEntry),
}
impl SchemaDataSourcingEventEntry {
    pub fn cpi_event(data: Vec<u8>, slot: u64, transaction_signature: Signature) -> Self {
        Self::CpiEvent(CpiEventEntry {
            data,
            slot,
            transaction_signature,
        })
    }
    pub fn pda(
        data: Vec<u8>,
        slot: u64,
        transaction_signature: Signature,
        pubkey: [u8; 32],
        owner: [u8; 32],
        lamports: u64,
        write_version: u64,
    ) -> Self {
        Self::Pda(PdaEntry {
            data,
            slot,
            transaction_signature,
            pubkey,
            owner,
            lamports,
            write_version,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpiEventEntry {
    data: Vec<u8>,
    slot: u64,
    transaction_signature: Signature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdaEntry {
    data: Vec<u8>,
    slot: u64,
    transaction_signature: Signature,
    pubkey: [u8; 32],
    owner: [u8; 32],
    lamports: u64,
    write_version: u64,
}
