use std::collections::BTreeMap;

use surfpool_types::{OverrideTemplate, YamlOverrideTemplateCollection};

pub const PYTH_V2_IDL_CONTENT: &str = include_str!("./protocols/pyth/v2/idl.json");
pub const PYTH_V2_OVERRIDES_CONTENT: &str = include_str!("./protocols/pyth/v2/overrides.yaml");

pub const RAYDIUM_CLMM_IDL_CONTENT: &str = include_str!("./protocols/raydium/v3/idl.json");
pub const RAYDIUM_CLMM_OVERRIDES_CONTENT: &str = include_str!("./protocols/raydium/v3/overrides.yaml");


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
        default.load_raydium_overrides();
        default
    }

    pub fn load_pyth_overrides(&mut self) {
        let idl = match serde_json::from_str(PYTH_V2_IDL_CONTENT) {
            Ok(idl) => idl,
            Err(e) => panic!("unable to load pyth idl: {}", e),
        };

        let Ok(collection) =
            serde_yaml::from_str::<YamlOverrideTemplateCollection>(PYTH_V2_OVERRIDES_CONTENT)
        else {
            panic!("unable to load pyth overrides");
        };

        // Convert all templates in the collection
        let templates = collection.to_override_templates(idl);

        // Register each template
        for template in templates {
            let template_id = template.id.clone();
            self.templates.insert(template_id.clone(), template);
        }
    }

    pub fn load_raydium_overrides(&mut self){
        let idl = match serde_json::from_str(RAYDIUM_CLMM_IDL_CONTENT) {
            Ok(idl) => idl,
            Err(e) => panic!("unable to load raydium idl: {}", e),
        };

        let Ok(collection) =
            serde_yaml::from_str::<YamlOverrideTemplateCollection>(RAYDIUM_CLMM_OVERRIDES_CONTENT)
        else {
            panic!("unable to load raydium overrides");
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
