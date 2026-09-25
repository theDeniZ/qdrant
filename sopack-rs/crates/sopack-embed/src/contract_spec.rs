//! `impl From<&sopack_contract::Contract> for EmbedSpec` — the one place
//! that maps a loaded `contract.toml` (`sopack-contract`'s job: parsing +
//! validation) onto this crate's own, contract-independent [`EmbedSpec`]
//! (see `spec.rs`'s module doc for why the two types are kept separate).
//!
//! Before this module existed, every caller that had a
//! `sopack_contract::Contract` and needed an `EmbedSpec` (the CLI, this
//! crate's own `examples/bench.rs` and `tests/model.rs`) hand-parsed
//! `contract.toml` a second time with the `toml` crate directly, repeating
//! the exact field mapping `spec.rs`'s doc comment already documents. This
//! is that mapping, written once.

use sopack_contract::Contract;

use crate::calibration::{CalibrationEntry as EmbedCalibrationEntry, CalibrationFixture};
use crate::spec::{EmbedSpec, ModelFile, Pooling};

/// Parses `[embedding].pooling` the same way `spec.rs`'s table promises:
/// `"mean"` / `"cls"`. Any other value is a contract bug that should be
/// caught at load time, not silently defaulted — panics with the offending
/// value, matching the two hand-parsing sites this replaces (`bench.rs` /
/// `tests/model.rs`), which both `panic!("unknown pooling {other:?}")`.
fn parse_pooling(s: &str) -> Pooling {
    match s {
        "mean" => Pooling::Mean,
        "cls" => Pooling::Cls,
        other => panic!("unknown pooling {other:?} in contract.toml (expected mean or cls)"),
    }
}

impl From<&Contract> for EmbedSpec {
    fn from(contract: &Contract) -> EmbedSpec {
        let files = contract
            .model
            .files
            .iter()
            .map(|(name, f)| ModelFile {
                name: name.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            })
            .collect();
        EmbedSpec {
            model_id: contract.embedding.model.clone(),
            repo: contract.model.repo.clone(),
            revision: contract.model.revision.clone(),
            onnx_file: contract.model.onnx.clone(),
            output_name: contract.model.output.clone(),
            files,
            pooling: parse_pooling(&contract.embedding.pooling),
            normalize: contract.embedding.normalized,
            dim: contract.embedding.dim as usize,
            max_tokens: contract.embedding.max_tokens as usize,
            passage_prefix: contract.embedding.passage_prefix.clone(),
            query_prefix: contract.embedding.query_prefix.clone(),
        }
    }
}

/// `impl From<&sopack_contract::Calibration> for CalibrationFixture` — the
/// same "one mapping, written once" reasoning as `EmbedSpec` above, for the
/// calibration fixture a loaded `Contract` already parsed
/// (`Contract::fixture`). A caller that has a `Contract` (the CLI's
/// `calibrate`/`pack`/device-fallback paths) never needs to re-read
/// `calibration.json` from disk itself.
impl From<&sopack_contract::Calibration> for CalibrationFixture {
    fn from(calibration: &sopack_contract::Calibration) -> CalibrationFixture {
        CalibrationFixture {
            schema: sopack_contract::CALIBRATION_SCHEMA.to_string(),
            contract: calibration.contract_id.clone(),
            created_at: Some(calibration.created_at.clone()),
            source: None,
            entries: calibration
                .entries
                .iter()
                .map(|e| EmbedCalibrationEntry {
                    id: e.id.clone(),
                    profile: e.profile.clone(),
                    uid: e.uid.clone(),
                    lang: e.lang.clone(),
                    note: Some(e.note.clone()),
                    text: e.text.clone(),
                    vector: e.vector.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_e5_large_v1_converts_to_the_expected_spec() {
        let contract = Contract::embedded("e5-large-v1").unwrap();
        let spec = EmbedSpec::from(&contract);
        assert_eq!(spec.model_id, "intfloat/multilingual-e5-large");
        assert_eq!(spec.repo, "qdrant/multilingual-e5-large-onnx");
        assert_eq!(spec.onnx_file, "model.onnx");
        assert_eq!(spec.output_name, "last_hidden_state");
        assert_eq!(spec.pooling, Pooling::Mean);
        assert!(spec.normalize);
        assert_eq!(spec.dim, 1024);
        assert_eq!(spec.max_tokens, 512);
        assert_eq!(spec.passage_prefix, "passage: ");
        assert_eq!(spec.query_prefix, "query: ");
        assert!(spec.file("model.onnx").is_some());
        assert!(spec.file("model.onnx_data").is_some());
        assert_eq!(spec.files.len(), contract.model.files.len());
    }

    #[test]
    #[should_panic(expected = "unknown pooling")]
    fn unknown_pooling_panics_rather_than_silently_defaulting() {
        parse_pooling("first_token");
    }

    #[test]
    fn calibration_fixture_conversion_preserves_entries_when_a_fixture_is_loaded() {
        let contract = Contract::embedded("e5-large-v1").unwrap();
        let Some(fixture) = &contract.fixture else {
            // The fixture may not have been generated yet by the concurrent
            // Python-side agent (see sopack-contract's own tests for the
            // same guard) — this conversion is exercised for real once it
            // has landed; until then there's nothing to convert.
            return;
        };
        let embed_fixture = CalibrationFixture::from(fixture);
        assert_eq!(embed_fixture.contract, "e5-large-v1");
        assert_eq!(embed_fixture.entries.len(), fixture.entries.len());
        assert_eq!(embed_fixture.entries[0].id, fixture.entries[0].id);
        assert_eq!(embed_fixture.entries[0].vector, fixture.entries[0].vector);
    }
}
