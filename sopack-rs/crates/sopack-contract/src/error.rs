use thiserror::Error;

/// Everything that can go wrong loading, validating or using a contract.
#[derive(Debug, Error)]
pub enum ContractError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}: not valid TOML: {source}")]
    Toml {
        path: String,
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error("{path}: unsupported schema {got:?} (this crate reads \"sopack.contract/1\")")]
    UnsupportedSchema { path: String, got: String },

    #[error("unknown released contract id {0:?} (embedded: e5-large-v1)")]
    UnknownEmbeddedContract(String),

    #[error(
        "calibration fixture for contract {contract_id:?} has not been generated yet \
         (expected at {expected_path}). Run the importer-side admin command that builds \
         calibration.json from the live collections first (SOPACK-2-FORMAT.md §3); \
         until then this contract cannot verify or write sopack/2 probes."
    )]
    CalibrationNotYetGenerated {
        contract_id: String,
        expected_path: String,
    },

    #[error(
        "calibration.json for contract {contract_id:?} does not match \
         [calibration].sha256 in contract.toml (declared {declared}, file hashes to {actual})"
    )]
    CalibrationSha256Mismatch {
        contract_id: String,
        declared: String,
        actual: String,
    },

    #[error("{path}: calibration.json is not valid JSON: {source}")]
    CalibrationJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "{path}: calibration.json has unsupported schema {got:?} \
         (this crate reads \"sopack.calibration/1\")"
    )]
    CalibrationUnsupportedSchema { path: String, got: String },

    #[error("profile {profile:?}: default_id_rule {rule:?} is not in id_rules {allowed:?}")]
    ProfileDefaultRuleNotAllowed {
        profile: String,
        rule: String,
        allowed: Vec<String>,
    },

    #[error("profile {profile:?}: id_rules[{index}] {rule:?} has no template in [ids.rules]")]
    ProfileIdRuleUndefined {
        profile: String,
        index: usize,
        rule: String,
    },

    #[error("[ids.rules] {rule:?}: {reason} in template {template:?}")]
    InvalidIdRuleTemplate {
        rule: String,
        template: String,
        reason: String,
    },

    #[error("unknown profile {0:?}")]
    UnknownProfile(String),

    #[error("unknown id_rule {rule:?} (have: {available:?})")]
    UnknownIdRule {
        rule: String,
        available: Vec<String>,
    },

    #[error(
        "id rule {rule:?}: missing field {field:?} for template {template:?} \
         (fields given: {available:?})"
    )]
    MissingIdField {
        rule: String,
        field: String,
        template: String,
        available: Vec<String>,
    },

    #[error("id rule {rule:?}: field {field:?} is a {kind} and cannot be substituted into a uid")]
    UnsupportedIdFieldType {
        rule: String,
        field: String,
        kind: &'static str,
    },
}

pub type Result<T> = std::result::Result<T, ContractError>;
