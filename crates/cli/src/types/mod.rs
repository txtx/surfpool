use txtx_addon_network_svm::templates::{
    get_interpolated_anchor_program_deployment_template, get_interpolated_anchor_subgraph_template,
    get_interpolated_native_program_deployment_template,
};

#[derive(Debug, Clone)]
pub enum Framework {
    Anchor,
    Native,
    Steel,
    Typhoon,
    Pinocchio,
}

impl Framework {
    pub fn to_string(&self) -> String {
        match self {
            Framework::Anchor => "anchor".to_string(),
            Framework::Native => "native".to_string(),
            Framework::Steel => "steel".to_string(),
            Framework::Typhoon => "typhoon".to_string(),
            Framework::Pinocchio => "pinocchio".to_string(),
        }
    }
    pub fn get_interpolated_program_deployment_template(&self, program_name: &str) -> String {
        match self {
            Framework::Anchor => get_interpolated_anchor_program_deployment_template(&program_name),
            Framework::Typhoon => todo!(),
            Framework::Native | Framework::Steel | Framework::Pinocchio => {
                get_interpolated_native_program_deployment_template(&program_name)
            }
        }
    }
    pub fn get_interpolated_subgraph_template(
        &self,
        program_name: &str,
        idl: Option<&String>,
    ) -> Result<Option<String>, String> {
        let Some(idl) = idl else {
            return Ok(None);
        };

        match self {
            Framework::Anchor | Framework::Native | Framework::Pinocchio => {
                let some_template = get_interpolated_anchor_subgraph_template(&program_name, &idl)
                    .map_err(|e| {
                        format!("failed to generate subgraph infrastructure as code: {}", e)
                    })?;
                Ok(some_template)
            }
            Framework::Steel => todo!(),
            Framework::Typhoon => todo!(),
        }
    }
}
impl std::fmt::Display for Framework {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}
impl std::str::FromStr for Framework {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "anchor" => Ok(Framework::Anchor),
            "native" => Ok(Framework::Native),
            "steel" => Ok(Framework::Steel),
            "typhoon" => Ok(Framework::Typhoon),
            _ => Err(format!("Unknown framework: {}", s)),
        }
    }
}
