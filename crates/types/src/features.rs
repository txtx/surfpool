use std::str::FromStr;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};

/// SVM feature flags that can be enabled or disabled via CLI.
///
/// These correspond to the fields in `solana_svm_feature_set::SVMFeatureSet`.
/// Use kebab-case on the CLI (e.g., `--feature disable-fees-sysvar`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumIter,
    EnumString,
    Display,
    IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum SvmFeature {
    MovePrecompileVerificationToSvm,
    StricterAbiAndRuntimeConstraints,
    EnableBpfLoaderSetAuthorityCheckedIx,
    EnableLoaderV4,
    DepleteCuMeterOnVmFailure,
    AbortOnInvalidCurve,
    Blake3SyscallEnabled,
    Curve25519SyscallEnabled,
    DisableDeployOfAllocFreeSyscall,
    DisableFeesSysvar,
    DisableSbpfV0Execution,
    EnableAltBn128CompressionSyscall,
    EnableAltBn128Syscall,
    EnableBigModExpSyscall,
    EnableGetEpochStakeSyscall,
    EnablePoseidonSyscall,
    EnableSbpfV1DeploymentAndExecution,
    EnableSbpfV2DeploymentAndExecution,
    EnableSbpfV3DeploymentAndExecution,
    GetSysvarSyscallEnabled,
    LastRestartSlotSysvar,
    ReenableSbpfV0Execution,
    RemainingComputeUnitsSyscallEnabled,
    RemoveBpfLoaderIncorrectProgramId,
    MoveStakeAndMoveLamportsIxs,
    StakeRaiseMinimumDelegationTo1Sol,
    DeprecateLegacyVoteIxs,
    MaskOutRentEpochInVmSerialization,
    SimplifyAltBn128SyscallErrorCodes,
    FixAltBn128MultiplicationInputLength,
    IncreaseTxAccountLockLimit,
    EnableExtendProgramChecked,
    FormalizeLoadedTransactionDataSize,
    DisableZkElgamalProofProgram,
    ReenableZkElgamalProofProgram,
    RaiseCpiNestingLimitTo8,
}

impl SvmFeature {
    /// Returns an iterator over all available SVM features.
    pub fn all() -> impl Iterator<Item = SvmFeature> {
        SvmFeature::iter()
    }

    /// Returns the kebab-case string representation used in CLI.
    pub fn as_str(&self) -> &'static str {
        self.into()
    }
}

/// Parse an SvmFeature from a string, with a custom error message.
pub fn parse_svm_feature(s: &str) -> Result<SvmFeature, String> {
    SvmFeature::from_str(s).map_err(|_| {
        format!(
            "Unknown SVM feature: '{}'. Use --help to see available features.",
            s
        )
    })
}

/// Configuration for SVM features, specifying which features to enable or disable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvmFeatureConfig {
    /// Features to explicitly enable (override defaults)
    pub enable: Vec<SvmFeature>,
    /// Features to explicitly disable (override defaults)
    pub disable: Vec<SvmFeature>,
}

impl SvmFeatureConfig {
    /// Creates a new empty feature configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the default mainnet feature configuration.
    ///
    /// This reflects features currently active on Solana mainnet-beta.
    /// Note: This may need periodic updates as mainnet features change.
    /// Last updated: 2025-01-25 (queried from mainnet RPC)
    pub fn default_mainnet_features() -> Self {
        // Features that are NOT yet active on mainnet (should be disabled)
        let disable = vec![
            // Blake3 syscall not yet on mainnet
            SvmFeature::Blake3SyscallEnabled,
            // Legacy vote deprecation not yet on mainnet
            SvmFeature::DeprecateLegacyVoteIxs,
            // SBPF v0 disable/reenable not yet on mainnet
            SvmFeature::DisableSbpfV0Execution,
            SvmFeature::ReenableSbpfV0Execution,
            // ZK ElGamal disable not yet on mainnet (reenable IS active)
            SvmFeature::DisableZkElgamalProofProgram,
            // Extended program checked not yet on mainnet
            SvmFeature::EnableExtendProgramChecked,
            // Loader v4 not yet on mainnet
            SvmFeature::EnableLoaderV4,
            // SBPF v1 not yet on mainnet (v2 and v3 ARE active)
            SvmFeature::EnableSbpfV1DeploymentAndExecution,
            // Transaction data size formalization not yet on mainnet
            SvmFeature::FormalizeLoadedTransactionDataSize,
            // Precompile verification move not yet on mainnet
            SvmFeature::MovePrecompileVerificationToSvm,
            // Stake move instructions not yet on mainnet
            SvmFeature::MoveStakeAndMoveLamportsIxs,
            // Stake minimum delegation raise not yet on mainnet
            SvmFeature::StakeRaiseMinimumDelegationTo1Sol,
        ];

        Self {
            enable: vec![],
            disable,
        }
    }

    /// Adds a feature to enable.
    pub fn enable(mut self, feature: SvmFeature) -> Self {
        if !self.enable.contains(&feature) {
            self.enable.push(feature);
        }
        // Remove from disable if present
        self.disable.retain(|f| f != &feature);
        self
    }

    /// Adds a feature to disable.
    pub fn disable(mut self, feature: SvmFeature) -> Self {
        if !self.disable.contains(&feature) {
            self.disable.push(feature);
        }
        // Remove from enable if present
        self.enable.retain(|f| f != &feature);
        self
    }

    /// Checks if a feature should be enabled based on this configuration.
    /// Returns None if not explicitly configured (use default).
    pub fn is_enabled(&self, feature: &SvmFeature) -> Option<bool> {
        if self.enable.contains(feature) {
            Some(true)
        } else if self.disable.contains(feature) {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== SvmFeature parsing tests ====================

    #[test]
    fn test_feature_from_str_valid() {
        assert_eq!(
            SvmFeature::from_str("disable-fees-sysvar").unwrap(),
            SvmFeature::DisableFeesSysvar
        );
        assert_eq!(
            SvmFeature::from_str("enable-loader-v4").unwrap(),
            SvmFeature::EnableLoaderV4
        );
        assert_eq!(
            SvmFeature::from_str("blake3-syscall-enabled").unwrap(),
            SvmFeature::Blake3SyscallEnabled
        );
        assert_eq!(
            SvmFeature::from_str("enable-sbpf-v2-deployment-and-execution").unwrap(),
            SvmFeature::EnableSbpfV2DeploymentAndExecution
        );
    }

    #[test]
    fn test_feature_from_str_invalid() {
        assert!(SvmFeature::from_str("invalid-feature").is_err());
        assert!(SvmFeature::from_str("").is_err());
        assert!(SvmFeature::from_str("disable_fees_sysvar").is_err()); // underscore instead of hyphen
        assert!(SvmFeature::from_str("DISABLE-FEES-SYSVAR").is_err()); // uppercase
    }

    #[test]
    fn test_parse_svm_feature_error_message() {
        let err = parse_svm_feature("not-a-feature").unwrap_err();
        assert!(err.contains("Unknown SVM feature"));
        assert!(err.contains("not-a-feature"));
    }

    // ==================== SvmFeature display tests ====================

    #[test]
    fn test_feature_display() {
        assert_eq!(
            SvmFeature::DisableFeesSysvar.to_string(),
            "disable-fees-sysvar"
        );
        assert_eq!(SvmFeature::EnableLoaderV4.to_string(), "enable-loader-v4");
        assert_eq!(
            SvmFeature::EnableSbpfV3DeploymentAndExecution.to_string(),
            "enable-sbpf-v3-deployment-and-execution"
        );
    }

    #[test]
    fn test_feature_as_str() {
        assert_eq!(
            SvmFeature::DisableFeesSysvar.as_str(),
            "disable-fees-sysvar"
        );
        assert_eq!(SvmFeature::EnableLoaderV4.as_str(), "enable-loader-v4");
    }

    #[test]
    fn test_feature_roundtrip() {
        for feature in SvmFeature::all() {
            let s = feature.to_string();
            let parsed = SvmFeature::from_str(&s).unwrap();
            assert_eq!(feature, parsed, "Roundtrip failed for {:?}", feature);
        }
    }

    // ==================== SvmFeature::all() tests ====================

    #[test]
    fn test_feature_all_count() {
        // Ensure we have all 36 features
        assert_eq!(SvmFeature::all().count(), 36);
    }

    #[test]
    fn test_feature_all_unique() {
        let all: Vec<_> = SvmFeature::all().collect();
        let mut seen = std::collections::HashSet::new();
        for feature in &all {
            assert!(
                seen.insert(feature),
                "Duplicate feature in all(): {:?}",
                feature
            );
        }
    }

    // ==================== SvmFeatureConfig basic tests ====================

    #[test]
    fn test_feature_config_new_is_empty() {
        let config = SvmFeatureConfig::new();
        assert!(config.enable.is_empty());
        assert!(config.disable.is_empty());
    }

    #[test]
    fn test_feature_config_default_is_empty() {
        let config = SvmFeatureConfig::default();
        assert!(config.enable.is_empty());
        assert!(config.disable.is_empty());
    }

    #[test]
    fn test_feature_config_enable() {
        let config = SvmFeatureConfig::new().enable(SvmFeature::EnableLoaderV4);

        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(true));
        assert_eq!(config.enable.len(), 1);
        assert!(config.disable.is_empty());
    }

    #[test]
    fn test_feature_config_disable() {
        let config = SvmFeatureConfig::new().disable(SvmFeature::DisableFeesSysvar);

        assert_eq!(
            config.is_enabled(&SvmFeature::DisableFeesSysvar),
            Some(false)
        );
        assert!(config.enable.is_empty());
        assert_eq!(config.disable.len(), 1);
    }

    #[test]
    fn test_feature_config_is_enabled_not_configured() {
        let config = SvmFeatureConfig::new();
        assert_eq!(config.is_enabled(&SvmFeature::Blake3SyscallEnabled), None);
    }

    // ==================== SvmFeatureConfig complex scenarios ====================

    #[test]
    fn test_feature_config_enable_then_disable() {
        // Enabling then disabling should result in disabled
        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .disable(SvmFeature::EnableLoaderV4);

        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(false));
        assert!(config.enable.is_empty());
        assert_eq!(config.disable.len(), 1);
    }

    #[test]
    fn test_feature_config_disable_then_enable() {
        // Disabling then enabling should result in enabled
        let config = SvmFeatureConfig::new()
            .disable(SvmFeature::EnableLoaderV4)
            .enable(SvmFeature::EnableLoaderV4);

        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(true));
        assert_eq!(config.enable.len(), 1);
        assert!(config.disable.is_empty());
    }

    #[test]
    fn test_feature_config_enable_idempotent() {
        // Enabling the same feature twice should not duplicate
        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .enable(SvmFeature::EnableLoaderV4);

        assert_eq!(config.enable.len(), 1);
    }

    #[test]
    fn test_feature_config_disable_idempotent() {
        // Disabling the same feature twice should not duplicate
        let config = SvmFeatureConfig::new()
            .disable(SvmFeature::EnableLoaderV4)
            .disable(SvmFeature::EnableLoaderV4);

        assert_eq!(config.disable.len(), 1);
    }

    #[test]
    fn test_feature_config_multiple_features() {
        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .enable(SvmFeature::Blake3SyscallEnabled)
            .disable(SvmFeature::DisableFeesSysvar)
            .disable(SvmFeature::DisableSbpfV0Execution);

        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(true));
        assert_eq!(
            config.is_enabled(&SvmFeature::Blake3SyscallEnabled),
            Some(true)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::DisableFeesSysvar),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::DisableSbpfV0Execution),
            Some(false)
        );
        assert_eq!(config.enable.len(), 2);
        assert_eq!(config.disable.len(), 2);
    }

    // ==================== Mainnet defaults tests ====================

    #[test]
    fn test_mainnet_features_disabled_list() {
        let config = SvmFeatureConfig::default_mainnet_features();

        // All these should be disabled on mainnet (queried 2025-01-25)
        assert_eq!(
            config.is_enabled(&SvmFeature::Blake3SyscallEnabled),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::DeprecateLegacyVoteIxs),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::DisableSbpfV0Execution),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::ReenableSbpfV0Execution),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::DisableZkElgamalProofProgram),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::EnableExtendProgramChecked),
            Some(false)
        );
        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(false));
        assert_eq!(
            config.is_enabled(&SvmFeature::EnableSbpfV1DeploymentAndExecution),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::FormalizeLoadedTransactionDataSize),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::MovePrecompileVerificationToSvm),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::MoveStakeAndMoveLamportsIxs),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::StakeRaiseMinimumDelegationTo1Sol),
            Some(false)
        );
    }

    #[test]
    fn test_mainnet_features_has_no_enables() {
        let config = SvmFeatureConfig::default_mainnet_features();
        assert!(config.enable.is_empty());
    }

    #[test]
    fn test_mainnet_features_override_with_enable() {
        // Start with mainnet defaults, then enable a disabled feature
        let config =
            SvmFeatureConfig::default_mainnet_features().enable(SvmFeature::EnableLoaderV4);

        // Should now be enabled
        assert_eq!(config.is_enabled(&SvmFeature::EnableLoaderV4), Some(true));
        // Other mainnet-disabled features should still be disabled
        assert_eq!(
            config.is_enabled(&SvmFeature::Blake3SyscallEnabled),
            Some(false)
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::EnableExtendProgramChecked),
            Some(false)
        );
    }

    #[test]
    fn test_mainnet_features_active_features_not_in_disable() {
        let config = SvmFeatureConfig::default_mainnet_features();

        // Features that ARE active on mainnet should not be in disable list
        // These should return None (use default, which is enabled)
        assert_eq!(config.is_enabled(&SvmFeature::DisableFeesSysvar), None);
        assert_eq!(
            config.is_enabled(&SvmFeature::Curve25519SyscallEnabled),
            None
        );
        assert_eq!(config.is_enabled(&SvmFeature::EnableAltBn128Syscall), None);
        assert_eq!(config.is_enabled(&SvmFeature::EnablePoseidonSyscall), None);
        assert_eq!(
            config.is_enabled(&SvmFeature::EnableSbpfV2DeploymentAndExecution),
            None
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::EnableSbpfV3DeploymentAndExecution),
            None
        );
        assert_eq!(
            config.is_enabled(&SvmFeature::RaiseCpiNestingLimitTo8),
            None
        );
    }

    // ==================== Serialization tests ====================

    #[test]
    fn test_feature_serde_roundtrip() {
        let feature = SvmFeature::EnableLoaderV4;
        let json = serde_json::to_string(&feature).unwrap();
        let parsed: SvmFeature = serde_json::from_str(&json).unwrap();
        assert_eq!(feature, parsed);
    }

    #[test]
    fn test_feature_config_serde_roundtrip() {
        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .disable(SvmFeature::DisableFeesSysvar);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: SvmFeatureConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, parsed);
    }

    #[test]
    fn test_feature_json_format() {
        let feature = SvmFeature::EnableLoaderV4;
        let json = serde_json::to_string(&feature).unwrap();
        // Should use kebab-case due to serde rename_all
        assert_eq!(json, "\"enable-loader-v4\"");
    }

    #[test]
    fn test_feature_config_json_format() {
        let config = SvmFeatureConfig::new().enable(SvmFeature::EnableLoaderV4);

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enable\""));
        assert!(json.contains("enable-loader-v4"));
    }

    // ==================== Edge cases ====================

    #[test]
    fn test_feature_equality() {
        assert_eq!(SvmFeature::EnableLoaderV4, SvmFeature::EnableLoaderV4);
        assert_ne!(SvmFeature::EnableLoaderV4, SvmFeature::DisableFeesSysvar);
    }

    #[test]
    fn test_feature_clone() {
        let feature = SvmFeature::EnableLoaderV4;
        let cloned = feature.clone();
        assert_eq!(feature, cloned);
    }

    #[test]
    fn test_feature_config_clone() {
        let config = SvmFeatureConfig::new()
            .enable(SvmFeature::EnableLoaderV4)
            .disable(SvmFeature::DisableFeesSysvar);

        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    #[test]
    fn test_feature_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(SvmFeature::EnableLoaderV4);
        set.insert(SvmFeature::DisableFeesSysvar);
        set.insert(SvmFeature::EnableLoaderV4); // duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&SvmFeature::EnableLoaderV4));
        assert!(set.contains(&SvmFeature::DisableFeesSysvar));
    }

    #[test]
    fn test_feature_debug() {
        let feature = SvmFeature::EnableLoaderV4;
        let debug_str = format!("{:?}", feature);
        assert_eq!(debug_str, "EnableLoaderV4");
    }
}
