use txtx_addon_network_svm::templates::{
    AccountDirEntry, AccountEntry, GenesisEntry,
    get_in_memory_interpolated_anchor_program_deployment_template,
    get_in_memory_interpolated_native_program_deployment_template,
    get_interpolated_anchor_program_deployment_template,
    get_interpolated_native_program_deployment_template, get_interpolated_setup_surfnet_template,
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
    pub fn get_interpolated_program_deployment_template(&self, program_name: &str) -> String {
        match self {
            Framework::Anchor => get_interpolated_anchor_program_deployment_template(program_name),
            Framework::Typhoon => todo!(),
            Framework::Native | Framework::Steel | Framework::Pinocchio => {
                get_interpolated_native_program_deployment_template(program_name)
            }
        }
    }
    pub fn get_in_memory_interpolated_program_deployment_template(
        &self,
        program_name: &str,
        artifacts_path: Option<&str>,
    ) -> String {
        let base = match self {
            Framework::Anchor => {
                get_in_memory_interpolated_anchor_program_deployment_template(program_name)
            }
            Framework::Typhoon => todo!(),
            Framework::Native | Framework::Steel | Framework::Pinocchio => {
                get_in_memory_interpolated_native_program_deployment_template(program_name)
            }
        };
        if let Some(artifacts) = artifacts_path {
            let get_program_fn = match self {
                Framework::Anchor => "svm::get_program_from_anchor_project",
                _ => "svm::get_program_from_native_project",
            };
            let bin_path = format!("{}/{}.so", artifacts.trim_end_matches('/'), program_name);
            let keypair_path = format!(
                "{}/{}-keypair.json",
                artifacts.trim_end_matches('/'),
                program_name
            );
            // Override keypair_path (2nd arg) and bin_path (4th arg) to use artifacts dir
            let old = format!("program = {}(\"{}\") ", get_program_fn, program_name);
            let new = format!(
                "program = {}(\"{}\", \"{}\", null, \"{}\") ",
                get_program_fn, program_name, keypair_path, bin_path
            );
            base.replace(&old, &new)
        } else {
            base
        }
    }

    pub fn get_interpolated_setup_surfnet_template(
        &self,
        genesis_accounts: &Vec<GenesisEntry>,
        accounts: &Vec<AccountEntry>,
        accounts_dir: &Vec<AccountDirEntry>,
    ) -> Option<String> {
        get_interpolated_setup_surfnet_template(genesis_accounts, accounts, accounts_dir)
    }
}
impl std::fmt::Display for Framework {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Framework::Anchor => "anchor",
            Framework::Native => "native",
            Framework::Steel => "steel",
            Framework::Typhoon => "typhoon",
            Framework::Pinocchio => "pinocchio",
        };
        write!(f, "{}", s)
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
