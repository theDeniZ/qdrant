//! `EmbedSpec` — a plain-data description of one embedding contract's model
//! and vector space, independent of the `sopack-contract` crate.
//!
//! `sopack-contract` is being built by another agent concurrently
//! (SOPACK-1.0-PLAN.md §3.1) and owns `contract.toml` parsing + validation.
//! Rather than depend on its (possibly still-changing) types, this crate
//! defines its own minimal struct that a caller constructs however it
//! likes — from a loaded `sopack_contract::Contract`, from a test fixture,
//! from a second contract file entirely. The mapping from
//! `contracts/<id>/contract.toml` (SOPACK-1.0-PLAN.md §3.6,
//! `contracts/e5-large-v1/contract.toml`) is:
//!
//! | `EmbedSpec` field | `contract.toml` key |
//! |---|---|
//! | `model_id`      | `[embedding].model` |
//! | `pooling`        | `[embedding].pooling` (`"mean"` / `"cls"`) |
//! | `normalize`      | `[embedding].normalized` |
//! | `dim`             | `[embedding].dim` |
//! | `max_tokens`      | `[embedding].max_tokens` |
//! | `passage_prefix`  | `[embedding].passage_prefix` |
//! | `query_prefix`    | `[embedding].query_prefix` |
//! | `repo`            | `[model].repo` |
//! | `revision`        | `[model].revision` |
//! | `onnx_file`       | `[model].onnx` |
//! | `output_name`     | `[model].output` |
//! | `files`           | every `[model.files."<name>"]` entry, as `ModelFile { name, sha256, bytes }` |
//!
//! `[calibration]`'s `pack_min_cosine` / `probe_min_cosine` /
//! `probe_expect_cosine` are passed to `calibration`/`engine` calls
//! directly by the caller rather than folded into `EmbedSpec`, since which
//! threshold applies depends on who is asking (a pack-time self-check vs. a
//! device-fallback probe vs. the importer). `[chunker]`, `[ids]` and
//! `[profiles.*]` are outside this crate's concern entirely.

use serde::{Deserialize, Serialize};

/// How the encoder's per-token `last_hidden_state` becomes one vector.
/// `Mean` (attention-mask-weighted average) is what every shipped contract
/// uses; `Cls` (take the `[CLS]` token, index 0) exists only so a test can
/// prove the calibration gate (`calibration::score`) actually catches a
/// wrong pooling choice against a real model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pooling {
    Mean,
    Cls,
}

/// One file the model needs on disk, with the sha256 `verify::verify_model`
/// checks it against (`contract.toml` `[model.files]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Which prefix to prepend before embedding a text. e5-style models are
/// asymmetric — `passage_prefix` for corpus text going into the index,
/// `query_prefix` for a live search query — and using the wrong one is
/// silent drift, not an error, so the type system is what catches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Passage,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedSpec {
    pub model_id: String,
    pub repo: String,
    pub revision: String,
    pub onnx_file: String,
    pub output_name: String,
    pub files: Vec<ModelFile>,
    pub pooling: Pooling,
    pub normalize: bool,
    pub dim: usize,
    pub max_tokens: usize,
    pub passage_prefix: String,
    pub query_prefix: String,
}

impl EmbedSpec {
    /// The prefix for `role` — see `Role`'s docs.
    pub fn prefix(&self, role: Role) -> &str {
        match role {
            Role::Passage => &self.passage_prefix,
            Role::Query => &self.query_prefix,
        }
    }

    pub fn file(&self, name: &str) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// Total bytes of every file in `files` — the download/verify progress total.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    /// `<cache>/models/<repo>/<revision>` join component, matching
    /// `cache::model_dir_in_cache`'s repo-name mangling — exposed here too
    /// since a caller sometimes has only the spec, not the cache root.
    pub fn repo_dir_name(&self) -> String {
        self.repo.replace('/', "--")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> EmbedSpec {
        EmbedSpec {
            model_id: "intfloat/multilingual-e5-large".into(),
            repo: "qdrant/multilingual-e5-large-onnx".into(),
            revision: "deadbeef".into(),
            onnx_file: "model.onnx".into(),
            output_name: "last_hidden_state".into(),
            files: vec![
                ModelFile {
                    name: "model.onnx".into(),
                    sha256: "aaa".into(),
                    bytes: 100,
                },
                ModelFile {
                    name: "model.onnx_data".into(),
                    sha256: "bbb".into(),
                    bytes: 900,
                },
            ],
            pooling: Pooling::Mean,
            normalize: true,
            dim: 1024,
            max_tokens: 512,
            passage_prefix: "passage: ".into(),
            query_prefix: "query: ".into(),
        }
    }

    #[test]
    fn prefix_selects_by_role() {
        let s = spec();
        assert_eq!(s.prefix(Role::Passage), "passage: ");
        assert_eq!(s.prefix(Role::Query), "query: ");
    }

    #[test]
    fn total_bytes_sums_every_file() {
        assert_eq!(spec().total_bytes(), 1000);
    }

    #[test]
    fn file_looks_up_by_name() {
        let s = spec();
        assert_eq!(s.file("model.onnx").unwrap().bytes, 100);
        assert!(s.file("nope").is_none());
    }

    #[test]
    fn repo_dir_name_replaces_slash() {
        assert_eq!(spec().repo_dir_name(), "qdrant--multilingual-e5-large-onnx");
    }

    #[test]
    fn pooling_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Pooling::Mean).unwrap(), "\"mean\"");
        assert_eq!(serde_json::to_string(&Pooling::Cls).unwrap(), "\"cls\"");
    }
}
