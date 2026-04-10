use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{SurfnetError, SurfnetResult};

const DEFAULT_REPORT_DIR: &str = "target/surfpool-reports";
const DEFAULT_OUTPUT_PATH: &str = "target/surfpool-report.html";
const REPORT_DATA_PLACEHOLDER: &str = "__SURFPOOL_REPORT_DATA_PLACEHOLDER__";
#[cfg(rust_analyzer)]
const REPORT_TEMPLATE: &str = include_str!("report-template.dev.html");
#[cfg(not(rust_analyzer))]
const REPORT_TEMPLATE: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/surfpool-report-viewer/index.html"
));

/// Transaction data from a single Surfnet instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfnetReportData {
    pub instance_id: String,
    pub test_name: Option<String>,
    pub rpc_url: String,
    pub transactions: Vec<TransactionReportEntry>,
    pub timestamp: String,
}

/// A single transaction with its full profile data.
///
/// Profiles are stored as raw JSON values in both `jsonParsed` and `base64`
/// encodings, matching the static report viewer bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionReportEntry {
    pub signature: String,
    pub slot: u64,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub profile_json_parsed: Option<serde_json::Value>,
    pub profile_base64: Option<serde_json::Value>,
}

/// A consolidated report covering all Surfnet instances from a test suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfpoolReport {
    pub instances: Vec<SurfnetReportData>,
    pub generated_at: String,
}

/// Options for consolidating per-instance report JSON and writing the static HTML report.
#[derive(Debug, Clone)]
pub struct SurfpoolReportOptions {
    pub report_dir: PathBuf,
    pub output_path: PathBuf,
}

impl Default for SurfpoolReportOptions {
    fn default() -> Self {
        Self {
            report_dir: PathBuf::from(DEFAULT_REPORT_DIR),
            output_path: PathBuf::from(DEFAULT_OUTPUT_PATH),
        }
    }
}

/// Read report JSON from `options.report_dir` and write the consolidated HTML report to
/// `options.output_path`.
pub fn generate(options: SurfpoolReportOptions) -> SurfnetResult<PathBuf> {
    let report = SurfpoolReport::from_directory(&options.report_dir)?;
    report.write_html(&options.output_path)
}

/// Generate the Surfpool report using the default report and output paths.
pub fn generate_default() -> SurfnetResult<PathBuf> {
    generate(SurfpoolReportOptions::default())
}

impl SurfpoolReport {
    /// Read and consolidate all report JSON files from a directory.
    ///
    /// Each file was written by a `Surfnet` instance via `write_report_data()`.
    pub fn from_directory(dir: impl AsRef<Path>) -> SurfnetResult<Self> {
        let dir = dir.as_ref();

        if !dir.exists() {
            return Err(SurfnetError::Report(format!(
                "report directory does not exist: {}",
                dir.display()
            )));
        }

        let mut instances = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| SurfnetError::Report(format!("failed to read report dir: {e}")))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            let content = std::fs::read_to_string(&path).map_err(|e| {
                SurfnetError::Report(format!("failed to read {}: {e}", path.display()))
            })?;

            let data: SurfnetReportData = serde_json::from_str(&content).map_err(|e| {
                SurfnetError::Report(format!("failed to parse {}: {e}", path.display()))
            })?;

            instances.push(data);
        }

        Ok(Self {
            instances,
            generated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub fn total_transactions(&self) -> usize {
        self.instances
            .iter()
            .map(|instance| instance.transactions.len())
            .sum()
    }

    pub fn failed_transactions(&self) -> usize {
        self.instances
            .iter()
            .flat_map(|instance| &instance.transactions)
            .filter(|transaction| transaction.error.is_some())
            .count()
    }

    pub fn total_compute_units(&self) -> u64 {
        self.instances
            .iter()
            .flat_map(|instance| &instance.transactions)
            .map(|transaction| {
                transaction
                    .profile_json_parsed
                    .as_ref()
                    .and_then(|profile| profile.get("transactionProfile"))
                    .and_then(|profile| profile.get("computeUnitsConsumed"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0)
            })
            .sum()
    }

    /// Generate the HTML report as a string using the embedded report-viewer bundle.
    pub fn generate_html(&self) -> SurfnetResult<String> {
        if !REPORT_TEMPLATE.contains(REPORT_DATA_PLACEHOLDER) {
            return Err(SurfnetError::Report(
                "embedded report viewer template is missing the report data placeholder".into(),
            ));
        }

        let serialized = serde_json::to_string(self)
            .map_err(|e| SurfnetError::Report(format!("failed to serialize report: {e}")))?;
        let escaped_json = escape_json_for_script_tag(&serialized);

        Ok(REPORT_TEMPLATE.replacen(REPORT_DATA_PLACEHOLDER, &escaped_json, 1))
    }

    /// Generate the HTML report and write it to a file.
    pub fn write_html(&self, path: impl AsRef<Path>) -> SurfnetResult<PathBuf> {
        let html = self.generate_html()?;
        let path = path.as_ref();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SurfnetError::Report(format!("failed to create output dir: {e}")))?;
        }

        std::fs::write(path, html)
            .map_err(|e| SurfnetError::Report(format!("failed to write report: {e}")))?;

        log::info!("Surfpool report written to {}", path.display());
        Ok(path.to_path_buf())
    }
}

fn escape_json_for_script_tag(json: &str) -> String {
    json.replace("</script", "<\\/script")
        .replace("<!--", "<\\!--")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        DEFAULT_OUTPUT_PATH, DEFAULT_REPORT_DIR, SurfnetReportData, SurfpoolReport,
        SurfpoolReportOptions, TransactionReportEntry,
    };

    fn sample_transaction(
        signature: &str,
        slot: u64,
        error: Option<&str>,
    ) -> TransactionReportEntry {
        TransactionReportEntry {
            signature: signature.into(),
            slot,
            error: error.map(ToOwned::to_owned),
            logs: vec![
                "Program 11111111111111111111111111111111 invoke [1]".into(),
                "Program log: transfer complete".into(),
            ],
            profile_json_parsed: Some(serde_json::json!({
                "slot": slot,
                "instructionProfiles": [
                    {
                        "computeUnitsConsumed": 1200,
                        "logMessages": [
                            "Program 11111111111111111111111111111111 invoke [1]",
                            "Program log: transfer complete"
                        ],
                        "errorMessage": error,
                        "accountStates": {
                            "Sender1111111111111111111111111111111111111": {
                                "type": "writable",
                                "accountChange": {
                                    "type": "update",
                                    "data": [
                                        {
                                            "lamports": 5000000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["" , "base64"]
                                        },
                                        {
                                            "lamports": 3750000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["" , "base64"]
                                        }
                                    ]
                                }
                            },
                            "Recipient11111111111111111111111111111111111": {
                                "type": "writable",
                                "accountChange": {
                                    "type": "update",
                                    "data": [
                                        {
                                            "lamports": 1000000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["" , "base64"]
                                        },
                                        {
                                            "lamports": 2250000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["" , "base64"]
                                        }
                                    ]
                                }
                            }
                        }
                    }
                ],
                "transactionProfile": {
                    "computeUnitsConsumed": 1200,
                    "accountStates": {},
                    "logMessages": [
                        "Program 11111111111111111111111111111111 invoke [1]",
                        "Program log: transfer complete"
                    ],
                    "errorMessage": error
                },
                "readonlyAccountStates": {
                    "11111111111111111111111111111111": {
                        "lamports": 1,
                        "data": ["", "base64"],
                        "owner": "NativeLoader1111111111111111111111111111111",
                        "executable": true,
                        "rentEpoch": 0,
                        "space": 0
                    }
                }
            })),
            profile_base64: Some(serde_json::json!({
                "slot": slot,
                "instructionProfiles": [
                    {
                        "computeUnitsConsumed": 1200,
                        "logMessages": [
                            "Program 11111111111111111111111111111111 invoke [1]",
                            "Program log: transfer complete"
                        ],
                        "errorMessage": error,
                        "accountStates": {
                            "Sender1111111111111111111111111111111111111": {
                                "type": "writable",
                                "accountChange": {
                                    "type": "update",
                                    "data": [
                                        {
                                            "lamports": 5000000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["", "base64"]
                                        },
                                        {
                                            "lamports": 3750000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["", "base64"]
                                        }
                                    ]
                                }
                            },
                            "Recipient11111111111111111111111111111111111": {
                                "type": "writable",
                                "accountChange": {
                                    "type": "update",
                                    "data": [
                                        {
                                            "lamports": 1000000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["", "base64"]
                                        },
                                        {
                                            "lamports": 2250000000u64,
                                            "owner": "11111111111111111111111111111111",
                                            "executable": false,
                                            "rentEpoch": 0,
                                            "space": 0,
                                            "data": ["", "base64"]
                                        }
                                    ]
                                }
                            }
                        }
                    }
                ],
                "transactionProfile": {
                    "computeUnitsConsumed": 1200,
                    "accountStates": {},
                    "logMessages": [
                        "Program 11111111111111111111111111111111 invoke [1]",
                        "Program log: transfer complete"
                    ],
                    "errorMessage": error
                },
                "readonlyAccountStates": {
                    "11111111111111111111111111111111": {
                        "lamports": 1,
                        "data": ["", "base64"],
                        "owner": "NativeLoader1111111111111111111111111111111",
                        "executable": true,
                        "rentEpoch": 0,
                        "space": 0
                    }
                }
            })),
        }
    }

    fn sample_instance(
        instance_id: &str,
        test_name: &str,
        transactions: Vec<TransactionReportEntry>,
    ) -> SurfnetReportData {
        SurfnetReportData {
            instance_id: instance_id.into(),
            test_name: Some(test_name.into()),
            rpc_url: "http://127.0.0.1:8899".into(),
            transactions,
            timestamp: "2026-04-03T12:00:00Z".into(),
        }
    }

    #[test]
    fn options_default_to_expected_paths() {
        let options = SurfpoolReportOptions::default();
        assert_eq!(
            options.report_dir,
            std::path::PathBuf::from(DEFAULT_REPORT_DIR)
        );
        assert_eq!(
            options.output_path,
            std::path::PathBuf::from(DEFAULT_OUTPUT_PATH)
        );
    }

    #[test]
    fn from_directory_sorts_instances_by_filename() {
        let dir = tempdir().unwrap();
        let alpha = sample_instance("alpha", "alpha_test", vec![]);
        let beta = sample_instance("beta", "beta_test", vec![]);

        fs::write(
            dir.path().join("b.json"),
            serde_json::to_string(&beta).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&alpha).unwrap(),
        )
        .unwrap();

        let report = SurfpoolReport::from_directory(dir.path()).unwrap();
        assert_eq!(report.instances[0].instance_id, "alpha");
        assert_eq!(report.instances[1].instance_id, "beta");
    }

    #[test]
    fn generate_html_injects_script_safe_json() {
        let report = SurfpoolReport {
            instances: vec![sample_instance(
                "alpha",
                "danger_test",
                vec![sample_transaction(
                    "signature",
                    1,
                    Some(r#"closing tag </script><script>alert("x")</script>"#),
                )],
            )],
            generated_at: "2026-04-03T12:00:00Z".into(),
        };

        let html = report.generate_html().unwrap();
        assert!(html.contains(r#"<\/script><script>alert(\"x\")<\/script>"#));
    }

    #[test]
    fn generate_writes_output_file() {
        let dir = tempdir().unwrap();
        let report_dir = dir.path().join("reports");
        let output_path = dir.path().join("report.html");
        fs::create_dir_all(&report_dir).unwrap();

        let instance = sample_instance(
            "alpha",
            "sdk::report",
            vec![sample_transaction("signature", 1, None)],
        );

        fs::write(
            report_dir.join("alpha.json"),
            serde_json::to_string(&instance).unwrap(),
        )
        .unwrap();

        let output = crate::report::generate(SurfpoolReportOptions {
            report_dir,
            output_path: output_path.clone(),
        })
        .unwrap();
        assert_eq!(output, output_path);
        let html = fs::read_to_string(&output).unwrap();
        assert!(html.contains("Surfpool Report"));
        assert!(html.contains("<div id=\"root\"></div>"));
    }

    #[test]
    #[ignore = "manual preview helper: writes a stable HTML file to target/ for browser inspection"]
    fn generate_preview_report_file() {
        let workspace_target =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let report_dir = workspace_target.join("surfpool-preview-report-data");
        let output_path = workspace_target.join("surfpool-preview-report.html");

        if report_dir.exists() {
            fs::remove_dir_all(&report_dir).unwrap();
        }
        fs::create_dir_all(&report_dir).unwrap();

        let instance = sample_instance(
            "preview-instance",
            "sdk::report::preview",
            vec![
                sample_transaction("preview-signature-1", 1, None),
                sample_transaction("preview-signature-2", 2, Some("custom program error: 0x1")),
            ],
        );

        fs::write(
            report_dir.join("preview.json"),
            serde_json::to_string_pretty(&instance).unwrap(),
        )
        .unwrap();

        let output = crate::report::generate(SurfpoolReportOptions {
            report_dir,
            output_path: output_path.clone(),
        })
        .unwrap();

        println!("Preview report written to {}", output.display());
        assert_eq!(output, output_path);
    }
}
