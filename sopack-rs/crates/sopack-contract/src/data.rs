//! The as-written shape of `contract.toml`, deserialized with `serde` +
//! `toml`. Kept separate from [`crate::contract::Contract`] (the validated,
//! ergonomic wrapper other crates use) so the "all keys typed" requirement is
//! satisfied purely by these struct definitions: a `contract.toml` whose
//! values don't match these types fails to parse, before any semantic
//! validation runs.
//!
//! Unknown keys are deliberately **not** rejected (no `deny_unknown_fields`):
//! this file is read by both the Rust and Python sides, and SOPACK-2-FORMAT.md
//! §1 asks readers elsewhere in the format to ignore unknown keys for forward
//! compatibility. The same spirit applies here — a future key an older
//! `sopack-contract` doesn't know about should not break loading.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ContractDoc {
    pub schema: String,
    pub id: String,
    pub description: String,
    pub embedding: EmbeddingDoc,
    pub model: ModelDoc,
    pub chunker: ChunkerDoc,
    pub calibration: CalibrationConfigDoc,
    pub ids: IdsDoc,
    pub profiles: BTreeMap<String, ProfileDoc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingDoc {
    pub model: String,
    pub pooling: String,
    pub normalized: bool,
    pub dim: u32,
    pub distance: String,
    pub max_tokens: u32,
    pub passage_prefix: String,
    pub query_prefix: String,
    #[serde(default)]
    pub reference_runtimes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelDoc {
    pub source: String,
    pub repo: String,
    pub revision: String,
    pub onnx: String,
    pub output: String,
    pub files: BTreeMap<String, ModelFileDoc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelFileDoc {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkerDoc {
    pub max_words: u32,
    pub target_words: u32,
    pub min_words: u32,
    pub max_block_damage: f64,
    pub max_junk_chars: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalibrationConfigDoc {
    pub file: String,
    #[serde(default)]
    pub sha256: String,
    pub pack_min_cosine: f64,
    pub probe_min_cosine: f64,
    pub probe_expect_cosine: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdsDoc {
    pub namespace: String,
    #[serde(default)]
    pub rules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDoc {
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    pub id_rules: Vec<String>,
    pub default_id_rule: String,
    pub identity: String,
    pub text_field: String,
    pub reimport_is_normal: bool,
    #[serde(default)]
    pub filterable: BTreeMap<String, String>,
}

/// The as-written shape of `calibration.json` (`sopack.calibration/1`,
/// SOPACK-2-FORMAT.md §3).
#[derive(Debug, Clone, Deserialize)]
pub struct CalibrationDoc {
    pub schema: String,
    pub contract: String,
    pub created_at: String,
    /// Provenance only (`{"store": "qdrant", "note": "..."}` today) — never
    /// consulted by validation or packing, kept only because it is part of
    /// the committed file's schema.
    #[serde(default)]
    #[allow(dead_code)]
    pub source: serde_json::Value,
    pub entries: Vec<CalibrationEntryDoc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalibrationEntryDoc {
    pub id: String,
    pub profile: String,
    #[serde(default)]
    pub uid: Option<String>,
    /// `null` for several real fixture entries (e.g. Bible translations,
    /// where the language lives in `note` instead — `"bible kjv"`).
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub note: String,
    pub text: String,
    pub vector: Vec<f32>,
}
