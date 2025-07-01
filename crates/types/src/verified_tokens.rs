use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(rename = "logoURI")]
    pub logo_uri: Option<String>,
}

pub static VERIFIED_TOKENS_BY_SYMBOL: Lazy<HashMap<String, TokenInfo>> = Lazy::new(|| {
    let json = include_str!("verified_tokens.json");
    let tokens: Vec<TokenInfo> = serde_json::from_str(json).expect("invalid verified_tokens.json");
    tokens.into_iter().map(|t| (t.symbol.clone(), t)).collect()
});
