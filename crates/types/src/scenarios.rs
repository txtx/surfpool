use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use solana_clock::Slot;
use uuid::Uuid;

use crate::Idl;

// ========================================
// Core Scenarios Types
// ========================================

/// Defines how an account address should be determined
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[doc = "Defines how an account address should be determined"]
pub enum AccountAddress {
    /// A specific public key
    #[doc = "A specific public key"]
    Pubkey(String),
    /// A Program Derived Address with seeds
    #[doc = "A Program Derived Address with seeds"]
    Pda {
        program_id: String,
        seeds: Vec<PdaSeed>,
    },
}

/// Seeds used for PDA derivation
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[doc = "Seeds used for PDA derivation"]
pub enum PdaSeed {
    Pubkey(String),
    String(String),
    Bytes(Vec<u8>),
    /// Reference to a property value
    PropertyRef(String),
}

/// A reusable template for creating account overrides
/// Values are mapped directly to IDL fields using dot notation (e.g., "agg.price", "expo")
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverrideTemplate {
    /// Unique identifier for the template
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this template does
    pub description: String,
    /// Protocol this template is for (e.g., "Pyth", "Switchboard")
    pub protocol: String,
    /// IDL for the account structure - defines all available fields and types
    pub idl: Idl,
    /// How to determine the account address
    pub address: AccountAddress,
    /// Account type name from the IDL (e.g., "PriceAccount")
    /// This specifies which account struct in the IDL to use
    pub account_type: String,
    pub properties: Vec<String>,
    /// Tags for categorization and search
    pub tags: Vec<String>,
}

impl OverrideTemplate {
    pub fn new(
        id: String,
        name: String,
        description: String,
        protocol: String,
        idl: Idl,
        address: AccountAddress,
        properties: Vec<String>,
        account_type: String,
    ) -> Self {
        Self {
            id,
            name,
            description,
            protocol,
            idl,
            address,
            account_type,
            properties,
            tags: Vec::new(),
        }
    }
}

/// A concrete instance of an override template with specific values
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverrideInstance {
    /// Unique identifier for this instance
    #[doc = "Unique identifier for the scenario"]
    pub id: String,
    /// Reference to the template being used
    #[doc = "Reference to the template being used"]
    pub template_id: String,
    /// Values for the template properties (flat key-value map with dot notation, e.g., "price_message.price_value")
    #[doc = "Values for the template properties (flat key-value map with dot notation, e.g., 'price_message.price_value')"]
    pub values: HashMap<String, serde_json::Value>,
    /// Relative slot when this override should be applied (relative to scenario registration slot)
    #[doc = "Relative slot when this override should be applied (relative to scenario registration slot)"]
    pub scenario_relative_slot: Slot,
    /// Optional label for this instance
    #[doc = "Optional label for this instance"]
    pub label: Option<String>,
    /// Whether this override is enabled
    #[doc = "Whether this override is enabled"]
    pub enabled: bool,
    /// Whether to fetch fresh account data just before transaction execution
    #[doc = "Whether to fetch fresh account data just before transaction execution"]
    #[serde(default)]
    pub fetch_before_use: bool,
    /// Account address to override
    #[doc = "Account address to override"]
    pub account: AccountAddress,
}

impl OverrideInstance {
    pub fn new(template_id: String, scenario_relative_slot: Slot, account: AccountAddress) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            template_id,
            values: HashMap::new(),
            scenario_relative_slot,
            label: None,
            enabled: true,
            fetch_before_use: false,
            account,
        }
    }

    pub fn with_values(mut self, values: HashMap<String, serde_json::Value>) -> Self {
        self.values = values;
        self
    }

    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }
}

/// A scenario containing a timeline of overrides
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    /// Unique identifier for the scenario
    #[doc = "Unique identifier for the scenario"]
    pub id: String,
    /// Human-readable name
    #[doc = "Human-readable name"]
    pub name: String,
    /// Description of this scenario
    #[doc = "Description of this scenario"]
    pub description: String,
    /// List of override instances in this scenario
    #[doc = "List of override instances in this scenario"]
    pub overrides: Vec<OverrideInstance>,
    /// Tags for categorization
    #[doc = "Tags for categorization"]
    pub tags: Vec<String>,
}

impl Scenario {
    pub fn new(name: String, description: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            overrides: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub fn add_override(&mut self, override_instance: OverrideInstance) {
        self.overrides.push(override_instance);
        // Sort by slot for efficient lookup
        self.overrides.sort_by_key(|o| o.scenario_relative_slot);
    }

    pub fn remove_override(&mut self, override_id: &str) {
        self.overrides.retain(|o| o.id != override_id);
    }

    pub fn get_overrides_for_slot(&self, slot: Slot) -> Vec<&OverrideInstance> {
        self.overrides
            .iter()
            .filter(|o| o.enabled && o.scenario_relative_slot == slot)
            .collect()
    }
}

/// Configuration for scenario execution
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioConfig {
    /// Whether scenarios are enabled
    pub enabled: bool,
    /// Currently active scenario
    pub active_scenario: Option<String>,
    /// Whether to auto-save scenario changes
    pub auto_save: bool,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active_scenario: None,
            auto_save: true,
        }
    }
}

// ========================================
// YAML Template File Types
// ========================================

/// YAML representation of an override template loaded from file
/// References an external IDL file via idl_file_path
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YamlOverrideTemplateFile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub protocol: String,
    pub version: String,
    pub account_type: String,
    pub properties: Vec<String>,
    pub idl_file_path: String,
    pub address: YamlAccountAddress,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl YamlOverrideTemplateFile {
    /// Convert file-based template to runtime OverrideTemplate with loaded IDL
    pub fn to_override_template(self, idl: Idl) -> OverrideTemplate {
        OverrideTemplate {
            id: self.id,
            name: self.name,
            description: self.description,
            protocol: self.protocol,
            idl,
            address: self.address.into(),
            account_type: self.account_type,
            properties: self.properties,
            tags: self.tags,
        }
    }
}

/// Collection of override templates sharing the same IDL
/// Used when one YAML file defines multiple templates (e.g., multiple Pyth price feeds)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YamlOverrideTemplateCollection {
    /// Protocol these templates are for
    pub protocol: String,
    /// Version identifier
    pub version: String,
    /// Path to shared IDL file
    pub idl_file_path: String,
    /// Common tags for all templates
    #[serde(default)]
    pub tags: Vec<String>,
    /// The templates
    pub templates: Vec<YamlOverrideTemplateEntry>,
}

/// Individual template entry in a collection
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YamlOverrideTemplateEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub idl_account_name: String,
    pub properties: Vec<String>,
    pub address: YamlAccountAddress,
}

impl YamlOverrideTemplateCollection {
    /// Convert collection to runtime OverrideTemplates with loaded IDL
    pub fn to_override_templates(self, idl: Idl) -> Vec<OverrideTemplate> {
        self.templates
            .into_iter()
            .map(|entry| OverrideTemplate {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                protocol: self.protocol.clone(),
                idl: idl.clone(),
                address: entry.address.into(),
                account_type: entry.idl_account_name,
                properties: entry.properties,
                tags: self.tags.clone(),
            })
            .collect()
    }
}

/// YAML representation of an override template with embedded IDL
/// Used for RPC methods where file access is not available
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YamlOverrideTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub protocol: String,
    pub version: String,
    pub account_type: String,
    pub idl: Idl,
    pub address: YamlAccountAddress,
    pub properties: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl YamlOverrideTemplate {
    /// Convert to runtime OverrideTemplate
    pub fn to_override_template(self) -> OverrideTemplate {
        OverrideTemplate {
            id: self.id,
            name: self.name,
            description: self.description,
            protocol: self.protocol,
            idl: self.idl,
            address: self.address.into(),
            account_type: self.account_type,
            properties: self.properties,
            tags: self.tags,
        }
    }
}

/// YAML representation of account address
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum YamlAccountAddress {
    Pubkey {
        #[serde(default)]
        value: Option<String>,
    },
    Pda {
        program_id: String,
        seeds: Vec<YamlPdaSeed>,
    },
}

impl From<YamlAccountAddress> for AccountAddress {
    fn from(yaml: YamlAccountAddress) -> Self {
        match yaml {
            YamlAccountAddress::Pubkey { value } => {
                AccountAddress::Pubkey(value.unwrap_or_default())
            }
            YamlAccountAddress::Pda { program_id, seeds } => AccountAddress::Pda {
                program_id,
                seeds: seeds.into_iter().map(|s| s.into()).collect(),
            },
        }
    }
}

/// YAML representation of PDA seeds
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum YamlPdaSeed {
    String { value: String },
    Bytes { value: Vec<u8> },
    Pubkey { value: String },
    PropertyRef { value: String },
}

impl From<YamlPdaSeed> for PdaSeed {
    fn from(yaml: YamlPdaSeed) -> Self {
        match yaml {
            YamlPdaSeed::String { value } => PdaSeed::String(value),
            YamlPdaSeed::Bytes { value } => PdaSeed::Bytes(value),
            YamlPdaSeed::Pubkey { value } => PdaSeed::Pubkey(value),
            YamlPdaSeed::PropertyRef { value } => PdaSeed::PropertyRef(value),
        }
    }
}
