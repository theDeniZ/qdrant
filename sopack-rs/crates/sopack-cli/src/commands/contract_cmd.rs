//! `sopack contract show|list`.

use serde::Serialize;

use crate::args::{Cli, ContractAction, ContractArgs};
use crate::contract_load::load_contract;
use crate::exit::{CliError, EXIT_OK};
use crate::output::{print_json, print_text};

#[derive(Serialize)]
struct EmbeddingInfo {
    model: String,
    pooling: String,
    normalized: bool,
    dim: u32,
    distance: String,
    max_tokens: u32,
    passage_prefix: String,
    query_prefix: String,
}

#[derive(Serialize)]
struct ModelInfo {
    repo: String,
    revision: String,
    onnx: String,
    files_total_bytes: u64,
}

#[derive(Serialize)]
struct ChunkerInfo {
    max_words: u32,
    target_words: u32,
    min_words: u32,
}

#[derive(Serialize)]
struct CalibrationInfo {
    pack_min_cosine: f64,
    probe_min_cosine: f64,
    probe_expect_cosine: f64,
    fixture_entries: usize,
}

#[derive(Serialize)]
struct ContractShow {
    id: String,
    description: String,
    contract_sha256: String,
    calibration_sha256: Option<String>,
    embedding: EmbeddingInfo,
    model: ModelInfo,
    chunker: ChunkerInfo,
    calibration: CalibrationInfo,
    profiles: Vec<String>,
    id_rules: Vec<String>,
}

pub fn run(cli: &Cli, args: &ContractArgs) -> Result<i32, CliError> {
    match args.action {
        ContractAction::Show => show(cli),
        ContractAction::List => list(cli),
    }
}

fn show(cli: &Cli) -> Result<i32, CliError> {
    let contract = load_contract(&cli.contract)?;
    let out = ContractShow {
        id: contract.id.clone(),
        description: contract.description.clone(),
        contract_sha256: contract.contract_sha256(),
        calibration_sha256: contract.calibration_sha256(),
        embedding: EmbeddingInfo {
            model: contract.embedding.model.clone(),
            pooling: contract.embedding.pooling.clone(),
            normalized: contract.embedding.normalized,
            dim: contract.embedding.dim,
            distance: contract.embedding.distance.clone(),
            max_tokens: contract.embedding.max_tokens,
            passage_prefix: contract.embedding.passage_prefix.clone(),
            query_prefix: contract.embedding.query_prefix.clone(),
        },
        model: ModelInfo {
            repo: contract.model.repo.clone(),
            revision: contract.model.revision.clone(),
            onnx: contract.model.onnx.clone(),
            files_total_bytes: contract.model.files.values().map(|f| f.bytes).sum(),
        },
        chunker: ChunkerInfo {
            max_words: contract.chunker.max_words,
            target_words: contract.chunker.target_words,
            min_words: contract.chunker.min_words,
        },
        calibration: CalibrationInfo {
            pack_min_cosine: contract.calibration.pack_min_cosine,
            probe_min_cosine: contract.calibration.probe_min_cosine,
            probe_expect_cosine: contract.calibration.probe_expect_cosine,
            fixture_entries: contract
                .fixture
                .as_ref()
                .map(|f| f.entries.len())
                .unwrap_or(0),
        },
        profiles: contract.profiles.keys().cloned().collect(),
        id_rules: contract.id_rules.keys().cloned().collect(),
    };
    if cli.json {
        print_json(&serde_json::to_value(&out).unwrap());
    } else {
        print_text(&format!("id: {}", out.id));
        print_text(&format!("description: {}", out.description));
        print_text(&format!("contract_sha256: {}", out.contract_sha256));
        print_text(&format!(
            "calibration_sha256: {}",
            out.calibration_sha256.as_deref().unwrap_or("(not loaded)")
        ));
        print_text(&format!(
            "embedding: {} ({}, dim={}, max_tokens={})",
            out.embedding.model, out.embedding.pooling, out.embedding.dim, out.embedding.max_tokens
        ));
        print_text(&format!("profiles: {}", out.profiles.join(", ")));
        print_text(&format!("id_rules: {}", out.id_rules.join(", ")));
    }
    Ok(EXIT_OK)
}

fn list(cli: &Cli) -> Result<i32, CliError> {
    let embedded = vec!["e5-large-v1".to_string()];
    if cli.json {
        print_json(&serde_json::json!({"embedded": embedded, "default": "e5-large-v1"}));
    } else {
        for id in &embedded {
            print_text(id);
        }
    }
    Ok(EXIT_OK)
}
