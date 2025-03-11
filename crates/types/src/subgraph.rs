use std::str::FromStr;

use anchor_lang_idl::types::{Idl, IdlDefinedFields, IdlType, IdlTypeDefTy};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_pubkey::Pubkey;
use txtx_addon_kit::{
    diagnosed_error, hex,
    types::{
        diagnostics::Diagnostic,
        frontend::{ProgressBarStatus, ProgressBarStatusColor, StatusUpdater},
        types::{Type, Value},
    },
};

// Subgraph keys
pub const FIELD: &str = "field";
pub const SVM_SUBGRAPH_REQUEST: &str = "svm::subgraph_request";

pub const SVM_PUBKEY: &str = "svm::pubkey";

pub struct SvmValue {}

fn is_hex(str: &str) -> bool {
    decode_hex(str).map(|_| true).unwrap_or(false)
}

fn decode_hex(str: &str) -> Result<Vec<u8>, Diagnostic> {
    let stripped = if str.starts_with("0x") {
        &str[2..]
    } else {
        &str[..]
    };
    hex::decode(stripped)
        .map_err(|e| diagnosed_error!("string '{}' could not be decoded to hex bytes: {}", str, e))
}

impl SvmValue {
    pub fn pubkey(bytes: Vec<u8>) -> Value {
        Value::addon(bytes, SVM_PUBKEY)
    }

    pub fn to_pubkey(value: &Value) -> Result<Pubkey, String> {
        match value.as_string() {
            Some(s) => {
                if is_hex(s) {
                    let hex = decode_hex(s).map_err(|e| e.message)?;
                    let bytes: [u8; 32] = hex[0..32]
                        .try_into()
                        .map_err(|e| format!("could not convert value to pubkey: {e}"))?;
                    return Ok(Pubkey::new_from_array(bytes));
                }
                return Pubkey::from_str(s)
                    .map_err(|e| format!("could not convert value to pubkey: {e}"));
            }
            None => {}
        };
        let bytes = value.to_bytes();
        let bytes: [u8; 32] = bytes[0..32]
            .try_into()
            .map_err(|e| format!("could not convert value to pubkey: {e}"))?;
        Ok(Pubkey::new_from_array(bytes))
    }
}

pub fn get_expected_field_type_from_idl_type_def_ty(
    field_name: &str,
    idl_type_def_ty: &IdlTypeDefTy,
) -> Result<Type, String> {
    let ty = match idl_type_def_ty {
        IdlTypeDefTy::Struct { fields } => {
            let ty = if let Some(fields) = fields {
                let field = match fields {
                    IdlDefinedFields::Named(idl_fields) => idl_fields
                        .iter()
                        .find(|f| f.name == field_name)
                        .ok_or(format!("unable to find field '{}' in struct", field_name))?,
                    IdlDefinedFields::Tuple(idl_types) => {
                        return Err("cannot find field by name for tuple type".to_string())
                    }
                };
                field.ty.clone()
            } else {
                return Err(format!("unable to find field '{}' in struct", field_name));
            };
            ty
        }
        IdlTypeDefTy::Enum { variants } => todo!(),
        IdlTypeDefTy::Type { alias } => todo!(),
    };

    Ok(idl_type_to_txtx_type(ty))
}

pub fn idl_type_to_txtx_type(idl_type: IdlType) -> Type {
    match idl_type {
        IdlType::Bool => Type::bool(),
        IdlType::U8 => Type::integer(),
        IdlType::I8 => Type::integer(),
        IdlType::U16 => Type::integer(),
        IdlType::I16 => Type::integer(),
        IdlType::U32 => Type::integer(),
        IdlType::I32 => Type::integer(),
        IdlType::U64 => Type::integer(),
        IdlType::I64 => Type::integer(),
        IdlType::I128 => Type::integer(),
        IdlType::F32 => Type::float(),
        IdlType::F64 => Type::float(),
        IdlType::U128 => todo!(),
        IdlType::U256 => todo!(),
        IdlType::I256 => todo!(),
        IdlType::Bytes => Type::buffer(),
        IdlType::String => Type::string(),
        IdlType::Pubkey => Type::addon(SVM_PUBKEY),
        IdlType::Option(idl_type) => idl_type_to_txtx_type(*idl_type),
        IdlType::Vec(idl_type) => Type::array(idl_type_to_txtx_type(*idl_type)),
        IdlType::Array(idl_type, ..) => Type::array(idl_type_to_txtx_type(*idl_type)),
        IdlType::Defined { .. } => todo!(),
        IdlType::Generic(_) => todo!(),
        _ => todo!(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub plugin_name: SubgraphPluginType,
    pub data: SubgraphRequest,
    pub workspace: String,
}

impl PluginConfig {
    pub fn new(plugin_name: SubgraphPluginType, data: SubgraphRequest) -> Self {
        Self {
            plugin_name,
            data,
            workspace: "".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubgraphPluginType {
    SurfpoolSubgraph,
}

impl std::fmt::Display for SubgraphPluginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let val = match self {
            SubgraphPluginType::SurfpoolSubgraph => "surfpool-subgraph".to_string(),
        };
        write!(f, "{}", val)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphRequest {
    pub fields: Vec<IndexedSubgraphField>,
    #[serde(with = "crate::utils::serde_with_str")]
    pub program_id: Pubkey,
    pub block_height: u64,
    pub subgraph_name: String,
    pub subgraph_description: Option<String>,
}

impl SubgraphRequest {
    pub fn new(
        subgraph_name: &str,
        subgraph_description: Option<String>,
        program_id: &Pubkey,
        idl_str: &str,
        events: Vec<SubgraphEventDefinition>,
        block_height: u64,
    ) -> Result<Self, Diagnostic> {
        let idl = serde_json::from_str(idl_str)
            .map_err(|e| diagnosed_error!("could not deserialize IDL: {e}"))?;

        let fields = events
            .iter()
            .map(|f| IndexedSubgraphField::from_event_definition(&idl, f.clone()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(Self {
            fields,
            program_id: *program_id,
            block_height,
            subgraph_name: subgraph_name.to_string(),
            subgraph_description,
        })
    }

    pub fn from_value(value: &Value) -> Result<Self, Diagnostic> {
        let addon_data = value.as_addon_data().ok_or(diagnosed_error!(
            "could not deserialize subgraph request: expected addon"
        ))?;
        if addon_data.id != SVM_SUBGRAPH_REQUEST {
            return Err(diagnosed_error!(
                "could not deserialize subgraph request: expected addon type '{}'",
                SVM_SUBGRAPH_REQUEST
            ));
        }
        let bytes = addon_data.bytes.clone();

        serde_json::from_slice(&bytes)
            .map_err(|e| diagnosed_error!("could not deserialize subgraph request: {e}"))
    }

    pub fn to_value(&self) -> Result<Value, Diagnostic> {
        Ok(Value::addon(
            serde_json::to_vec(self)
                .map_err(|e| diagnosed_error!("could not serialize subgraph request: {e}"))?,
            SVM_SUBGRAPH_REQUEST,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubgraphSourceType {
    Instruction(String),
    Account(String),
    Event(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEventDefinition {
    pub name: String,
    pub fields: Vec<SubgraphFieldDefinition>,
}

impl SubgraphEventDefinition {
    pub fn parse_map_values(values: &Vec<Value>) -> Result<Vec<Self>, Diagnostic> {
        if values.len() == 0 {
            return Err(diagnosed_error!("subgraph event should not be empty"));
        }
        let mut events = Vec::new();
        for entry in values.iter() {
            let entry = entry.as_object().ok_or(diagnosed_error!(
                "each entry of a subgraph event should contain an object"
            ))?;
            let name = entry.get("name").ok_or(diagnosed_error!(
                "could not deserialize subgraph event: expected 'name' key"
            ))?;
            let name = name.as_string().ok_or(diagnosed_error!(
                "could not deserialize subgraph event: expected 'name' to be a string"
            ))?;
            let fields = entry.get(FIELD).ok_or(diagnosed_error!(
                "could not deserialize subgraph event: expected 'fields' key"
            ))?;
            let fields = fields.as_array().ok_or(diagnosed_error!(
                "could not deserialize subgraph event: expected 'fields' to be an array"
            ))?;
            let fields = SubgraphFieldDefinition::parse_map_values(fields)?;
            events.push(Self {
                name: name.to_string(),
                fields,
            });
        }
        Ok(events)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphFieldDefinition {
    name: String,
    source: Option<String>,
    description: Option<String>,
}

impl SubgraphFieldDefinition {
    pub fn parse_map_values(values: &Vec<Value>) -> Result<Vec<Self>, Diagnostic> {
        if values.len() == 0 {
            return Err(diagnosed_error!("subgraph field should not be empty"));
        }
        let mut fields = Vec::new();
        for entry in values.iter() {
            let entry = entry.as_object().ok_or(diagnosed_error!(
                "each entry of a subgraph field should contain an object"
            ))?;
            let name = entry.get("name").ok_or(diagnosed_error!(
                "could not deserialize subgraph field: expected 'name' key"
            ))?;
            let name = name.as_string().ok_or(diagnosed_error!(
                "could not deserialize subgraph field: expected 'name' to be a string"
            ))?;
            let source = entry
                .get("idl_key")
                .and_then(|v| v.as_string().map(|s| s.to_string()));
            let description = entry
                .get("description")
                .and_then(|v| v.as_string().map(|s| s.to_string()));
            fields.push(Self {
                name: name.to_string(),
                source,
                description,
            });
        }
        Ok(fields)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedSubgraphField {
    pub display_name: String,
    pub source_key: String,
    pub expected_type: Type,
    pub description: Option<String>,
    pub data_source: IndexedSubgraphSourceType,
}

impl IndexedSubgraphField {
    pub fn from_event_definition(
        idl: &Idl,
        event_def: SubgraphEventDefinition,
    ) -> Result<Vec<Self>, Diagnostic> {
        let mut fields = Vec::new();
        for field_def in event_def.fields.iter() {
            let source_key = field_def.source.clone().unwrap_or(field_def.name.clone());
            let display_name = field_def.name.clone();
            let idl_event =
                idl.events
                    .iter()
                    .find(|e| e.name == event_def.name)
                    .ok_or(diagnosed_error!(
                        "could not find event '{}' in IDL",
                        event_def.name
                    ))?;
            let ty =
                idl.types
                    .iter()
                    .find(|t| t.name == event_def.name)
                    .ok_or(diagnosed_error!(
                        "could not find type '{}' in IDL",
                        event_def.name
                    ))?;

            let expected_type = get_expected_field_type_from_idl_type_def_ty(&source_key, &ty.ty)
                .map_err(|e| {
                diagnosed_error!(
                    "could not determine expected type for subgraph field '{}': {e}",
                    source_key
                )
            })?;

            fields.push(Self {
                display_name,
                source_key,
                expected_type,
                description: field_def.description.clone(),
                data_source: IndexedSubgraphSourceType::Event(EventSubgraphSource {
                    event: idl_event.clone(),
                    ty: ty.clone(),
                }),
            })
        }
        Ok(fields)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexedSubgraphSourceType {
    Instruction(InstructionSubgraphSource),
    Event(EventSubgraphSource),
    // Account(AccountSubgraphSource),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSubgraphSource {
    // The instruction being indexed
    pub instruction: anchor_lang_idl::types::IdlInstruction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSubgraphSource {
    // The event being indexed
    pub event: anchor_lang_idl::types::IdlEvent,
    // The type of the event, found from the IDL
    pub ty: anchor_lang_idl::types::IdlTypeDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSubgraphSource {}
