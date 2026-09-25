//! Ignored-by-default tests against the real `e5-large-v1` model
//! (SOPACK-1.0-PLAN.md §3.1's "ignored-by-default model tests"). Enabled by
//! env `SOPACK_TEST_MODEL_DIR` (+ `ORT_DYLIB_PATH`, since `ort` is built
//! with `load-dynamic`). Every test that loads the model must run under
//! `flock /home/vscode/.claude/jobs/00f226f6/tmp/model.lock <cmd>` per this
//! job's shared rules — only one model in memory at a time on a 12 GB box.
//!
//! Run:
//! ```text
//! SOPACK_TEST_MODEL_DIR=<model dir> ORT_DYLIB_PATH=<libonnxruntime.so> \
//!   cargo test -p sopack-embed --test model -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! This crate now depends on `sopack-contract` (see
//! `src/contract_spec.rs`'s `impl From<&Contract> for EmbedSpec`), so
//! `load_spec` below loads the embedded contract instead of hand-parsing
//! `contract.toml` a second time.

use sopack_contract::Contract;
use sopack_embed::calibration::{cosine, CalibrationFixture};
use sopack_embed::device::Device;
use sopack_embed::engine::{Engine, EngineOptions};
use sopack_embed::spec::{EmbedSpec, Pooling, Role};
use sopack_progress::NullSink;
use std::path::PathBuf;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/e5-large-v1")
}

fn load_spec() -> EmbedSpec {
    let contract = Contract::embedded("e5-large-v1").expect("load embedded e5-large-v1 contract");
    EmbedSpec::from(&contract)
}

fn load_fixture() -> CalibrationFixture {
    CalibrationFixture::load(&contracts_dir().join("calibration.json"))
        .expect("load calibration.json")
}

fn model_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("SOPACK_TEST_MODEL_DIR")
            .expect("set SOPACK_TEST_MODEL_DIR to run the ignored model tests"),
    )
}

/// A handful of real texts of varying length — enough that length-sorted
/// batching actually has more than one bucket to plan, but small enough
/// that these tests run in seconds rather than minutes (unlike the M0
/// spike's 281-point/192-paragraph fixtures, these aren't measuring
/// throughput, just vector agreement).
fn bench_texts() -> Vec<String> {
    vec![
        "For God so loved the world, that he gave his only begotten Son.".to_string(),
        "In the beginning God created the heaven and the earth. ".repeat(3),
        "Blessed are the poor in spirit: for theirs is the kingdom of heaven.".to_string(),
        "The Spirit of Prophecy points forward to the great controversy between Christ \
         and Satan, a struggle that has spanned six thousand years of human history and \
         will culminate in the final triumph of righteousness over sin and death."
            .to_string(),
        "Short text.".to_string(),
    ]
}

#[test]
#[ignore]
fn calibration_passes_on_cpu() {
    let spec = load_spec();
    let fixture = load_fixture();
    let mut engine = Engine::new(
        spec,
        &model_dir(),
        EngineOptions {
            device: Device::Cpu,
            threads: Some(1),
            batch_tokens: None,
        },
        &NullSink,
    )
    .expect("load engine");
    println!(
        "onnxruntime version loaded: {}",
        engine.onnxruntime_version()
    );
    assert!(
        engine.onnxruntime_version().starts_with("1.30"),
        "expected ONNX Runtime 1.30.x (what M0 measured), got {}",
        engine.onnxruntime_version()
    );

    let result = engine.calibrate(&fixture, &NullSink).expect("calibrate");
    println!(
        "calibration: n={} min={:.10} mean={:.10}",
        result.n, result.min_cosine, result.mean_cosine
    );
    assert!(
        result.min_cosine >= 0.9999,
        "min cosine {} below 0.9999, worst 3: {:?}",
        result.min_cosine,
        result.worst(3)
    );
}

#[test]
#[ignore]
fn cls_pooling_fails_the_calibration_gate() {
    let mut spec = load_spec();
    spec.pooling = Pooling::Cls;
    let fixture = load_fixture();
    let mut engine = Engine::new(
        spec,
        &model_dir(),
        EngineOptions {
            device: Device::Cpu,
            threads: Some(1),
            batch_tokens: None,
        },
        &NullSink,
    )
    .expect("load engine");
    let result = engine.calibrate(&fixture, &NullSink).expect("calibrate");
    println!(
        "cls pooling calibration: n={} min={:.6} mean={:.6}",
        result.n, result.min_cosine, result.mean_cosine
    );
    assert!(!result.passes(0.9999), "cls pooling must NOT pass the calibration gate (that's the point of the gate) — got min={}", result.min_cosine);
}

#[test]
#[ignore]
fn vectors_are_bitwise_identical_across_thread_counts() {
    let spec = load_spec();
    let texts = bench_texts();
    let mut reference: Option<Vec<Vec<f32>>> = None;
    for threads in [1usize, 2, 4] {
        let mut engine = Engine::new(
            spec.clone(),
            &model_dir(),
            EngineOptions {
                device: Device::Cpu,
                threads: Some(threads),
                batch_tokens: None,
            },
            &NullSink,
        )
        .expect("load engine");
        let vecs = engine
            .embed(&texts, Role::Passage, &NullSink)
            .expect("embed");
        match &reference {
            None => reference = Some(vecs),
            Some(r) => {
                for (i, (a, b)) in vecs.iter().zip(r).enumerate() {
                    assert_eq!(a, b, "threads={threads} text[{i}] differs from the threads=1 reference (must be bitwise identical, per M0)");
                }
            }
        }
    }
}

#[test]
#[ignore]
fn vectors_are_cosine_stable_across_batch_budgets() {
    let spec = load_spec();
    let texts = bench_texts();
    let mut reference: Option<Vec<Vec<f32>>> = None;
    for budget in [512usize, 2048] {
        let mut engine = Engine::new(
            spec.clone(),
            &model_dir(),
            EngineOptions {
                device: Device::Cpu,
                threads: Some(1),
                batch_tokens: Some(budget),
            },
            &NullSink,
        )
        .expect("load engine");
        let vecs = engine
            .embed(&texts, Role::Passage, &NullSink)
            .expect("embed");
        match &reference {
            None => reference = Some(vecs),
            Some(r) => {
                for (i, (a, b)) in vecs.iter().zip(r).enumerate() {
                    let cos = cosine(a, b);
                    println!("budget={budget} text[{i}] cosine vs budget=512: {cos:.9}");
                    assert!(cos >= 0.9999, "budget={budget} text[{i}] cosine {cos} < 0.9999 vs the budget=512 reference");
                }
            }
        }
    }
}
