use std::collections::BTreeMap;

use surfpool_types::{OverrideTemplate, YamlOverrideTemplateCollection};

pub const PYTH_V2_IDL_CONTENT: &str = include_str!("./protocols/pyth/v2/idl.json");
pub const PYTH_V2_OVERRIDES_CONTENT: &str = include_str!("./protocols/pyth/v2/overrides.yaml");

pub const JUPITER_V6_IDL_CONTENT: &str = include_str!("./protocols/jupiter/v6/idl.json");
pub const JUPITER_V6_OVERRIDES_CONTENT: &str =
    include_str!("./protocols/jupiter/v6/overrides.yaml");

pub const RAYDIUM_CLMM_IDL_CONTENT: &str = include_str!("./protocols/raydium/v3/idl.json");
pub const RAYDIUM_CLMM_OVERRIDES_CONTENT: &str =
    include_str!("./protocols/raydium/v3/overrides.yaml");

pub const KAMINO_V1_IDL_CONTENT: &str = include_str!("./protocols/kamino/v1/idl.json");
pub const KAMINO_V1_OVERRIDES_CONTENT: &str = include_str!("./protocols/kamino/v1/overrides.yaml");

/// Registry for managing override templates loaded from YAML files
#[derive(Clone, Debug, Default)]
pub struct TemplateRegistry {
    /// Map of template ID to template
    pub templates: BTreeMap<String, OverrideTemplate>,
}

impl TemplateRegistry {
    /// Create a new template registry
    pub fn new() -> Self {
        let mut default = Self::default();
        default.load_pyth_overrides();
        default.load_jupiter_overrides();
        default.load_raydium_overrides();
        default.load_kamino_overrides();
        default
    }

    pub fn load_pyth_overrides(&mut self) {
        self.load_protocol_overrides(PYTH_V2_IDL_CONTENT, PYTH_V2_OVERRIDES_CONTENT, "pyth");
    }

    pub fn load_jupiter_overrides(&mut self) {
        self.load_protocol_overrides(
            JUPITER_V6_IDL_CONTENT,
            JUPITER_V6_OVERRIDES_CONTENT,
            "jupiter",
        );
    }

    pub fn load_raydium_overrides(&mut self) {
        self.load_protocol_overrides(
            RAYDIUM_CLMM_IDL_CONTENT,
            RAYDIUM_CLMM_OVERRIDES_CONTENT,
            "raydium",
        );
    }

    pub fn load_kamino_overrides(&mut self) {
        self.load_protocol_overrides(KAMINO_V1_IDL_CONTENT, KAMINO_V1_OVERRIDES_CONTENT, "kamino");
    }

    fn load_protocol_overrides(
        &mut self,
        idl_content: &str,
        overrides_content: &str,
        protocol_name: &str,
    ) {
        let idl = match serde_json::from_str(idl_content) {
            Ok(idl) => idl,
            Err(e) => panic!("unable to load {} idl: {}", protocol_name, e),
        };

        let Ok(collection) =
            serde_yaml::from_str::<YamlOverrideTemplateCollection>(overrides_content)
        else {
            panic!("unable to load {} overrides", protocol_name);
        };

        // Convert all templates in the collection
        let templates = collection.to_override_templates(idl);

        // Register each template
        for template in templates {
            let template_id = template.id.clone();
            self.templates.insert(template_id.clone(), template);
        }
    }

    /// Get a template by ID
    pub fn get(&self, template_id: &str) -> Option<&OverrideTemplate> {
        self.templates.get(template_id)
    }

    /// Get all templates
    pub fn all(&self) -> Vec<&OverrideTemplate> {
        self.templates.values().collect()
    }

    /// Get templates for a specific protocol
    pub fn by_protocol(&self, protocol: &str) -> Vec<&OverrideTemplate> {
        self.templates
            .values()
            .filter(|t| t.protocol.eq_ignore_ascii_case(protocol))
            .collect()
    }

    /// Get templates matching any of the given tags
    pub fn by_tags(&self, tags: &[String]) -> Vec<&OverrideTemplate> {
        self.templates
            .values()
            .filter(|t| t.tags.iter().any(|tag| tags.contains(tag)))
            .collect()
    }

    /// Get the number of loaded templates
    pub fn count(&self) -> usize {
        self.templates.len()
    }

    /// Check if a template exists
    pub fn contains(&self, template_id: &str) -> bool {
        self.templates.contains_key(template_id)
    }

    /// List all template IDs
    pub fn list_ids(&self) -> Vec<String> {
        self.templates.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_loads_both_protocols() {
        let registry = TemplateRegistry::new();

        // Should have Pyth (4 templates) + Jupiter (1 template) + Raydium(3 templates) = 5 total
        assert_eq!(
            registry.count(),
            8,
            "Registry should load 5 templates total"
        );

        assert!(registry.contains("pyth-sol-usd-v2"));
        assert!(registry.contains("pyth-btc-usd-v2"));
        assert!(registry.contains("pyth-eth-btc-v2"));
        assert!(registry.contains("pyth-eth-usd-v2"));

        assert!(registry.contains("jupiter-token-ledger-override"));

        assert!(registry.contains("raydium-clmm-sol-usdc"));
        assert!(registry.contains("raydium-clmm-btc-usdc"));
        assert!(registry.contains("raydium-clmm-eth-usdc"));
    }

    #[test]
    fn test_jupiter_template_loads_correctly() {
        let registry = TemplateRegistry::new();

        let jupiter_template = registry
            .get("jupiter-token-ledger-override")
            .expect("Jupiter template should exist");

        assert_eq!(jupiter_template.protocol, "Jupiter");
        assert_eq!(jupiter_template.account_type, "TokenLedger");
        assert_eq!(jupiter_template.name, "Override Jupiter Token Ledger");
        assert_eq!(jupiter_template.properties.len(), 2);

        assert!(
            jupiter_template
                .properties
                .contains(&"tokenAccount".to_string())
        );
        assert!(jupiter_template.properties.contains(&"amount".to_string()));
        assert!(jupiter_template.tags.contains(&"dex".to_string()));
        assert!(jupiter_template.tags.contains(&"aggregator".to_string()));
        assert!(jupiter_template.tags.contains(&"swap".to_string()));
        assert!(jupiter_template.tags.contains(&"defi".to_string()));
    }

    #[test]
    fn test_filter_by_protocol() {
        let registry = TemplateRegistry::new();

        let pyth_templates = registry.by_protocol("Pyth");
        assert_eq!(pyth_templates.len(), 4, "Should have 4 Pyth templates");

        let jupiter_templates = registry.by_protocol("Jupiter");
        assert_eq!(jupiter_templates.len(), 1, "Should have 1 Jupiter template");
    }

    #[test]
    fn test_filter_by_tags() {
        let registry = TemplateRegistry::new();

        let oracle_templates = registry.by_tags(&[vec!["oracle".to_string()]].concat());
        assert_eq!(
            oracle_templates.len(),
            4,
            "Should find 4 oracle templates (Pyth)"
        );

        let dex_templates = registry.by_tags(&[vec!["dex".to_string()]].concat());
        assert_eq!(
            dex_templates.len(),
            1,
            "Should find 1 dex template (Jupiter)"
        );

        let aggregator_templates = registry.by_tags(&[vec!["aggregator".to_string()]].concat());
        assert_eq!(
            aggregator_templates.len(),
            1,
            "Should find 1 aggregator template (Jupiter)"
        );
    }

    #[test]
    fn test_jupiter_idl_has_token_ledger_account() {
        let registry = TemplateRegistry::new();
        let jupiter_template = registry.get("jupiter-token-ledger-override").unwrap();
        let has_token_ledger = jupiter_template
            .idl
            .accounts
            .iter()
            .any(|acc| acc.name == "TokenLedger");

        assert!(has_token_ledger, "IDL should contain TokenLedger account");
    }

    #[test]
    fn test_list_all_template_ids() {
        let registry = TemplateRegistry::new();
        let ids = registry.list_ids();

        assert_eq!(ids.len(), 8);
        assert!(ids.contains(&"raydium-clmm-sol-usdc".to_string()));
        assert!(ids.contains(&"jupiter-token-ledger-override".to_string()));
        assert!(ids.contains(&"pyth-sol-usd-v2".to_string()));
    }
}
