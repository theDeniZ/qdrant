//! [`Contract`] — a loaded, validated `contract.toml` (+ optional
//! `calibration.json`). See the module docs in [`crate`] for the big
//! picture; this module is the loading + validation + accessor surface.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::de::Error as _;
use sha2::{Digest, Sha256};

use crate::calibration::Calibration;
use crate::data::ContractDoc;
use crate::error::{ContractError, Result};
use crate::idrule::IdRule;

pub const CONTRACT_SCHEMA: &str = "sopack.contract/1";

// ── the one released contract this crate ships with ─────────────────────────
//
// `contract.toml` always exists (committed, reviewed data) so it is
// `include_str!`-ed directly. `calibration.json` does not exist yet as of
// this crate's initial build — see `build.rs` for why it goes through
// `OUT_DIR` instead of a direct `include_str!`, which would fail to compile
// until the fixture lands.
const E5_LARGE_V1_TOML: &str = include_str!("../../../contracts/e5-large-v1/contract.toml");
const E5_LARGE_V1_CALIBRATION: &str =
    include_str!(concat!(env!("OUT_DIR"), "/calibration_e5_large_v1.json"));

/// Model-defining fields any embedding-space setting depends on. Plain
/// fields, no methods — every value here is exactly what `contract.toml`
/// says, just typed and validated.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    pub model: String,
    pub pooling: String,
    pub normalized: bool,
    pub dim: u32,
    pub distance: String,
    pub max_tokens: u32,
    pub passage_prefix: String,
    pub query_prefix: String,
    pub reference_runtimes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub source: String,
    pub repo: String,
    pub revision: String,
    pub onnx: String,
    pub output: String,
    pub files: BTreeMap<String, ModelFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunker {
    pub max_words: u32,
    pub target_words: u32,
    pub min_words: u32,
    pub max_block_damage: f64,
    pub max_junk_chars: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationConfig {
    pub file: String,
    /// May be empty: the fixture has not been generated yet, or the
    /// contract predates recording its hash. [`Contract::from_dir`] /
    /// [`Contract::embedded`] only *require* a match when this is non-empty
    /// (SOPACK-2-FORMAT.md §3).
    pub sha256: String,
    pub pack_min_cosine: f64,
    pub probe_min_cosine: f64,
    pub probe_expect_cosine: f64,
}

/// A validated `[profiles.<name>]` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub id_rules: Vec<String>,
    pub default_id_rule: String,
    pub identity: String,
    pub text_field: String,
    pub reimport_is_normal: bool,
    pub filterable: BTreeMap<String, String>,
}

/// The flattened view [`sopack-embed`] (or any embedding engine) needs: the
/// model to fetch, how to run it, and the chunker limits that depend on its
/// window — without digging through [`Contract::model`] /
/// [`Contract::embedding`] / [`Contract::chunker`] separately.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbedSpec {
    pub repo: String,
    pub revision: String,
    pub onnx_file: String,
    pub output_name: String,
    pub files: BTreeMap<String, ModelFile>,
    pub pooling: String,
    pub normalized: bool,
    pub dim: u32,
    pub max_tokens: u32,
    pub passage_prefix: String,
    pub query_prefix: String,
    pub max_words: u32,
    pub target_words: u32,
    pub min_words: u32,
}

/// A loaded, validated embedding contract: `contract.toml` plus (usually)
/// its `calibration.json` fixture.
#[derive(Debug, Clone)]
pub struct Contract {
    pub schema: String,
    pub id: String,
    pub description: String,
    pub embedding: Embedding,
    pub model: Model,
    pub chunker: Chunker,
    pub calibration: CalibrationConfig,
    pub ids_namespace: String,
    pub id_rules: BTreeMap<String, IdRule>,
    pub profiles: BTreeMap<String, Profile>,
    /// The parsed calibration fixture, if one was found. `None` only when
    /// `calibration.sha256` is empty too (an in-progress contract) — see
    /// [`Contract::from_dir`]/[`Contract::embedded`], which refuse to load a
    /// contract that declares a fixture sha256 but cannot find the file.
    pub fixture: Option<Calibration>,
    toml_bytes: Vec<u8>,
}

impl Contract {
    /// Load one of the contracts this binary ships with, by `id`
    /// (`contract.toml`'s `id` key). Only `"e5-large-v1"` is registered
    /// today; a new released contract is added by embedding its files here.
    pub fn embedded(id: &str) -> Result<Contract> {
        match id {
            "e5-large-v1" => {
                let toml_bytes = E5_LARGE_V1_TOML.as_bytes().to_vec();
                let doc = parse_doc(&toml_bytes, "<embedded e5-large-v1/contract.toml>")?;
                let calib_bytes = if E5_LARGE_V1_CALIBRATION.is_empty() {
                    None
                } else {
                    Some(E5_LARGE_V1_CALIBRATION.as_bytes().to_vec())
                };
                build_contract(
                    doc,
                    toml_bytes,
                    calib_bytes,
                    "<embedded e5-large-v1/calibration.json>".to_string(),
                )
            }
            other => Err(ContractError::UnknownEmbeddedContract(other.to_string())),
        }
    }

    /// Load a contract directory (`--contract <path>`): `contract.toml`
    /// plus, if present next to it, the `calibration.json` the toml's
    /// `[calibration].file` names (normally `"calibration.json"`).
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Contract> {
        let dir = dir.as_ref();
        let toml_path = dir.join("contract.toml");
        let toml_bytes = fs::read(&toml_path).map_err(|source| ContractError::Io {
            path: toml_path.display().to_string(),
            source,
        })?;
        let doc = parse_doc(&toml_bytes, &toml_path.display().to_string())?;

        let calib_path = dir.join(&doc.calibration.file);
        let calib_bytes = match fs::read(&calib_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ContractError::Io {
                    path: calib_path.display().to_string(),
                    source,
                })
            }
        };

        build_contract(
            doc,
            toml_bytes,
            calib_bytes,
            calib_path.display().to_string(),
        )
    }

    /// sha256 of the exact `contract.toml` bytes this was loaded from —
    /// recorded in a pack's `manifest.contract.sha256`.
    pub fn contract_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.toml_bytes))
    }

    /// sha256 of the loaded `calibration.json`, if one is loaded. `None`
    /// means this contract has no fixture yet ([`Contract::fixture`] is also
    /// `None` in that case).
    pub fn calibration_sha256(&self) -> Option<String> {
        self.fixture.as_ref().map(Calibration::sha256)
    }

    pub fn get_profile(&self, name: &str) -> Result<&Profile> {
        self.profiles
            .get(name)
            .ok_or_else(|| ContractError::UnknownProfile(name.to_string()))
    }

    pub fn get_id_rule(&self, name: &str) -> Result<&IdRule> {
        self.id_rules
            .get(name)
            .ok_or_else(|| ContractError::UnknownIdRule {
                rule: name.to_string(),
                available: self.id_rules.keys().cloned().collect(),
            })
    }

    /// `uuid5(NAMESPACE_DNS, uid)` of *fields* under id rule *rule_name* —
    /// the Rust equivalent of `sopack.contract.point_id`.
    pub fn point_id(
        &self,
        rule_name: &str,
        fields: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.get_id_rule(rule_name)?.point_id(fields)
    }

    /// The flattened spec an embedding engine needs. See [`EmbedSpec`].
    pub fn embed_spec(&self) -> EmbedSpec {
        EmbedSpec {
            repo: self.model.repo.clone(),
            revision: self.model.revision.clone(),
            onnx_file: self.model.onnx.clone(),
            output_name: self.model.output.clone(),
            files: self.model.files.clone(),
            pooling: self.embedding.pooling.clone(),
            normalized: self.embedding.normalized,
            dim: self.embedding.dim,
            max_tokens: self.embedding.max_tokens,
            passage_prefix: self.embedding.passage_prefix.clone(),
            query_prefix: self.embedding.query_prefix.clone(),
            max_words: self.chunker.max_words,
            target_words: self.chunker.target_words,
            min_words: self.chunker.min_words,
        }
    }
}

fn parse_doc(bytes: &[u8], path: &str) -> Result<ContractDoc> {
    let text = std::str::from_utf8(bytes).map_err(|e| ContractError::Toml {
        path: path.to_string(),
        source: Box::new(toml::de::Error::custom(format!("invalid utf-8: {e}"))),
    })?;
    let doc: ContractDoc = toml::from_str(text).map_err(|source| ContractError::Toml {
        path: path.to_string(),
        source: Box::new(source),
    })?;
    if doc.schema != CONTRACT_SCHEMA {
        return Err(ContractError::UnsupportedSchema {
            path: path.to_string(),
            got: doc.schema,
        });
    }
    Ok(doc)
}

fn build_contract(
    doc: ContractDoc,
    toml_bytes: Vec<u8>,
    calib_bytes: Option<Vec<u8>>,
    calib_path_for_errors: String,
) -> Result<Contract> {
    let mut id_rules = BTreeMap::new();
    for (name, template) in &doc.ids.rules {
        id_rules.insert(
            name.clone(),
            IdRule::parse(name, template, &doc.ids.namespace)?,
        );
    }

    let mut profiles = BTreeMap::new();
    for (name, p) in &doc.profiles {
        if !p.id_rules.contains(&p.default_id_rule) {
            return Err(ContractError::ProfileDefaultRuleNotAllowed {
                profile: name.clone(),
                rule: p.default_id_rule.clone(),
                allowed: p.id_rules.clone(),
            });
        }
        for (index, rule_name) in p.id_rules.iter().enumerate() {
            if !id_rules.contains_key(rule_name) {
                return Err(ContractError::ProfileIdRuleUndefined {
                    profile: name.clone(),
                    index,
                    rule: rule_name.clone(),
                });
            }
        }
        profiles.insert(
            name.clone(),
            Profile {
                name: name.clone(),
                required: p.required.clone(),
                optional: p.optional.clone(),
                id_rules: p.id_rules.clone(),
                default_id_rule: p.default_id_rule.clone(),
                identity: p.identity.clone(),
                text_field: p.text_field.clone(),
                reimport_is_normal: p.reimport_is_normal,
                filterable: p.filterable.clone(),
            },
        );
    }

    let embedding = Embedding {
        model: doc.embedding.model.clone(),
        pooling: doc.embedding.pooling.clone(),
        normalized: doc.embedding.normalized,
        dim: doc.embedding.dim,
        distance: doc.embedding.distance.clone(),
        max_tokens: doc.embedding.max_tokens,
        passage_prefix: doc.embedding.passage_prefix.clone(),
        query_prefix: doc.embedding.query_prefix.clone(),
        reference_runtimes: doc.embedding.reference_runtimes.clone(),
    };
    let model = Model {
        source: doc.model.source.clone(),
        repo: doc.model.repo.clone(),
        revision: doc.model.revision.clone(),
        onnx: doc.model.onnx.clone(),
        output: doc.model.output.clone(),
        files: doc
            .model
            .files
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    ModelFile {
                        sha256: v.sha256.clone(),
                        bytes: v.bytes,
                    },
                )
            })
            .collect(),
    };
    let chunker = Chunker {
        max_words: doc.chunker.max_words,
        target_words: doc.chunker.target_words,
        min_words: doc.chunker.min_words,
        max_block_damage: doc.chunker.max_block_damage,
        max_junk_chars: doc.chunker.max_junk_chars,
    };
    let calibration_config = CalibrationConfig {
        file: doc.calibration.file.clone(),
        sha256: doc.calibration.sha256.clone(),
        pack_min_cosine: doc.calibration.pack_min_cosine,
        probe_min_cosine: doc.calibration.probe_min_cosine,
        probe_expect_cosine: doc.calibration.probe_expect_cosine,
    };

    let fixture = match calib_bytes {
        Some(bytes) if !bytes.is_empty() => {
            let calib = Calibration::parse(bytes, &calib_path_for_errors)?;
            if !calibration_config.sha256.is_empty() {
                let actual = calib.sha256();
                if actual != calibration_config.sha256 {
                    return Err(ContractError::CalibrationSha256Mismatch {
                        contract_id: doc.id.clone(),
                        declared: calibration_config.sha256.clone(),
                        actual,
                    });
                }
            }
            Some(calib)
        }
        _ => {
            if !calibration_config.sha256.is_empty() {
                return Err(ContractError::CalibrationNotYetGenerated {
                    contract_id: doc.id.clone(),
                    expected_path: calib_path_for_errors,
                });
            }
            None
        }
    };

    Ok(Contract {
        schema: doc.schema,
        id: doc.id,
        description: doc.description,
        embedding,
        model,
        chunker,
        calibration: calibration_config,
        ids_namespace: doc.ids.namespace,
        id_rules,
        profiles,
        fixture,
        toml_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn embedded_e5_large_v1_loads() {
        let c = Contract::embedded("e5-large-v1").unwrap();
        assert_eq!(c.id, "e5-large-v1");
        assert_eq!(c.embedding.dim, 1024);
        assert_eq!(c.embedding.model, "intfloat/multilingual-e5-large");
        assert!(c.profiles.contains_key("sop"));
        assert!(c.profiles.contains_key("bible"));
        assert_eq!(
            c.get_id_rule("sop/seq").unwrap().template,
            "{lang}:{book_code}:{para_key}#{seq}"
        );
    }

    #[test]
    fn unknown_embedded_id_is_an_error() {
        let err = Contract::embedded("no-such-contract").unwrap_err();
        assert!(matches!(err, ContractError::UnknownEmbeddedContract(_)));
    }

    #[test]
    fn contract_sha256_is_stable_and_matches_manual_hash() {
        let c = Contract::embedded("e5-large-v1").unwrap();
        let want = hex::encode(Sha256::digest(E5_LARGE_V1_TOML.as_bytes()));
        assert_eq!(c.contract_sha256(), want);
    }

    /// Rewrite the `[calibration]` section's `sha256 = "..."` line, whatever
    /// its current value, without touching the six `model.files` `sha256`
    /// entries that also match a naive substring search. Tests build their
    /// own contract fixtures against *this* rather than a literal sentinel
    /// string, because the real `calibration.json`/`[calibration].sha256`
    /// can (and, mid-workspace, did) appear at any point — another agent is
    /// generating them concurrently (see `common.md`).
    fn replace_calibration_sha(toml: &str, new_sha: &str) -> String {
        let mut out = String::new();
        let mut in_calibration = false;
        let mut replaced = false;
        for line in toml.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('[') {
                in_calibration = trimmed.starts_with("[calibration]");
            }
            if in_calibration && !replaced && trimmed.starts_with("sha256") {
                out.push_str(&format!("sha256 = \"{new_sha}\"\n"));
                replaced = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        assert!(
            replaced,
            "no [calibration] sha256 line found in the given toml"
        );
        out
    }

    #[test]
    fn declared_calibration_sha256_but_missing_file_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let toml = replace_calibration_sha(E5_LARGE_V1_TOML, "deadbeef");
        fs::write(dir.path().join("contract.toml"), toml).unwrap();
        // deliberately no calibration.json written

        let err = Contract::from_dir(dir.path()).unwrap_err();
        assert!(
            matches!(err, ContractError::CalibrationNotYetGenerated { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn calibration_sha256_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let toml = replace_calibration_sha(E5_LARGE_V1_TOML, "deadbeef");
        fs::write(dir.path().join("contract.toml"), toml).unwrap();
        fs::write(dir.path().join("calibration.json"), br#"{"schema":"sopack.calibration/1","contract":"e5-large-v1","created_at":"x","entries":[]}"#).unwrap();

        let err = Contract::from_dir(dir.path()).unwrap_err();
        assert!(
            matches!(err, ContractError::CalibrationSha256Mismatch { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn calibration_loads_and_matches_when_sha256_is_correct() {
        let dir = tempfile::tempdir().unwrap();
        let calib_bytes = br#"{"schema":"sopack.calibration/1","contract":"e5-large-v1","created_at":"x","entries":[{"id":"p1","profile":"sop","lang":"en","text":"hello","vector":[0.1,0.2]}]}"#;
        let sha = hex::encode(Sha256::digest(calib_bytes));
        let toml = replace_calibration_sha(E5_LARGE_V1_TOML, &sha);
        fs::write(dir.path().join("contract.toml"), toml).unwrap();
        let mut f = fs::File::create(dir.path().join("calibration.json")).unwrap();
        f.write_all(calib_bytes).unwrap();
        drop(f);

        let c = Contract::from_dir(dir.path()).unwrap();
        assert_eq!(c.calibration_sha256().unwrap(), sha);
        assert_eq!(c.fixture.unwrap().entries.len(), 1);
    }

    #[test]
    fn embedded_calibration_fixture_loads_and_matches_declared_sha256() {
        // As of this test running, the fixture may or may not have been
        // generated yet by the concurrent Python-side agent (see
        // `build.rs`) — assert whichever state is consistent, rather than
        // assuming one or the other, so this test is correct before *and*
        // after that fixture lands.
        let c = Contract::embedded("e5-large-v1").unwrap();
        if c.calibration.sha256.is_empty() {
            assert!(c.fixture.is_none());
        } else {
            let fixture = c
                .fixture
                .as_ref()
                .expect("declared sha256 but no fixture loaded");
            assert_eq!(fixture.sha256(), c.calibration.sha256);
            assert!(!fixture.entries.is_empty());
        }
    }

    #[test]
    fn profile_default_id_rule_must_be_in_its_own_id_rules() {
        let dir = tempfile::tempdir().unwrap();
        let toml = E5_LARGE_V1_TOML.replace(
            "default_id_rule = \"sop/seq\"",
            "default_id_rule = \"bible/v1\"",
        );
        assert_ne!(toml, E5_LARGE_V1_TOML);
        fs::write(dir.path().join("contract.toml"), toml).unwrap();
        let err = Contract::from_dir(dir.path()).unwrap_err();
        assert!(
            matches!(err, ContractError::ProfileDefaultRuleNotAllowed { .. }),
            "{err:?}"
        );
    }
}
