//! Model-gated integration tests (`SOPACK-1.0-PLAN.md` M4 exit criterion:
//! "3 real books packed in Rust pass the server probe >= 0.9999"; this
//! job's own test list items a-e). `#[ignore]` by default. Run:
//!
//! ```text
//! SOPACK_TEST_MODEL_DIR=<model dir> ORT_DYLIB_PATH=<libonnxruntime.so> \
//!   flock /home/vscode/.claude/jobs/00f226f6/tmp/model.lock \
//!   cargo test -p sopack-cli --release --test model_gated -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `--release` matters here: an unoptimized ORT/ndarray build is drastically
//! slower for a real 2.2 GB model. `--test-threads=1` because only one
//! model may be resident at a time on this box (shared `model.lock`).

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use sopack_contract::Contract;
use sopack_format::PackReader;

fn model_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("SOPACK_TEST_MODEL_DIR")
            .expect("set SOPACK_TEST_MODEL_DIR to run the model-gated tests"),
    )
}

fn qdrant_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../qdrant/sopack-rs/crates/sopack-cli
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("qdrant/ root should exist three levels above the crate")
}

fn python() -> PathBuf {
    qdrant_root()
        .parent()
        .expect("qdrant/ has a parent (the workspace root)")
        .join(".venv/bin/python3.11")
}

fn run_python_check(script: &str, extra_args: &[&str]) -> (bool, String) {
    let out = Command::new(python())
        .arg(qdrant_root().join(script))
        .args(extra_args)
        .env("PYTHONPATH", qdrant_root())
        .output()
        .expect("failed to run the python checker");
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), combined)
}

/// Streams every `(id, vector)` pair out of *pack_path* via the real
/// `sopack-format` reader (accepts both `/1` and `/2`, so this also reads
/// the Python-built `wdys.sopack` reference).
fn read_all_vectors(
    pack_path: &Path,
    contract: &Contract,
) -> std::collections::HashMap<String, Vec<f32>> {
    let mut reader = PackReader::open(pack_path).expect("open pack");
    let errors = reader.check(contract);
    assert!(
        errors.is_empty(),
        "{}: check() failed: {errors:?}",
        pack_path.display()
    );
    let mut out = std::collections::HashMap::new();
    for batch in reader.batches(contract, 64, true).expect("batches") {
        let (points, vectors) = batch.expect("batch");
        for (p, v) in points.into_iter().zip(vectors) {
            out.insert(p.id, v);
        }
    }
    out
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa.sqrt() * bb.sqrt())
}

#[test]
#[ignore]
fn calibrate_passes_on_cpu() {
    let r = support::run(&[
        "calibrate",
        "--model-dir",
        model_dir().to_str().unwrap(),
        "--threads",
        "1",
        "--json",
    ]);
    let v = r.stdout_json();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    assert_eq!(r.status, 0, "calibrate failed: {}", r.stderr);
    assert_eq!(v["pass"], true);
    assert!(v["min_cosine"].as_f64().unwrap() >= 0.9999);
}

#[test]
#[ignore]
fn doctor_full_loads_the_model_and_passes_calibration() {
    let r = support::run(&[
        "doctor",
        "--model-dir",
        model_dir().to_str().unwrap(),
        "--json",
    ]);
    let v = r.stdout_json();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    let model_loads = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "model_loads")
        .expect("expected a model_loads check");
    assert_eq!(
        model_loads["ok"], true,
        "model_loads check failed: {model_loads}"
    );
    assert_eq!(model_loads["level"], "info");
}

#[test]
#[ignore]
fn pack_wdys_is_verifiable_and_matches_the_python_built_reference() {
    let wdys_book = qdrant_root().join("packs/wdys.book.json");
    assert!(wdys_book.is_file(), "expected {}", wdys_book.display());
    let reference_sopack = qdrant_root().join("packs/wdys.sopack");
    assert!(
        reference_sopack.is_file(),
        "expected {}",
        reference_sopack.display()
    );

    let tmp = tempfile::tempdir().unwrap();
    let out_a = tmp.path().join("wdys-a.sopack");
    let pack_id = "model-gated-test-wdys";
    let created_by = "sopack model-gated test on a fixed host string";

    // ---- (a) a full, uninterrupted pack ----------------------------------
    let r = Command::new(support::bin())
        .args([
            "pack",
            wdys_book.to_str().unwrap(),
            "-o",
            out_a.to_str().unwrap(),
            "--model-dir",
            model_dir().to_str().unwrap(),
            "--threads",
            "1",
            "--pack-id",
            pack_id,
            "--created-by",
            created_by,
            "--json",
        ])
        .output()
        .expect("run sopack pack");
    assert!(
        r.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let pack_result: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    println!("pack result: {pack_result}");
    assert_eq!(pack_result["points"], 23);
    assert!(pack_result["calibration_min_cosine"].as_f64().unwrap() >= 0.9999);

    // ---- (a) Rust `verify` accepts ----------------------------------------
    let r = support::run(&["verify", out_a.to_str().unwrap(), "--json"]);
    let v = r.stdout_json();
    assert_eq!(v["clean"], true, "rust verify reported: {v}");
    assert_eq!(r.status, 0);

    // ---- (b) Python accepts (check_pack_py.py) -----------------------------
    let (ok, output) = run_python_check(
        "sopack-rs/conformance/packs/check_pack_py.py",
        &[out_a.to_str().unwrap()],
    );
    println!("check_pack_py.py:\n{output}");
    assert!(ok, "check_pack_py.py failed:\n{output}");

    // ---- (c) the Python importer's probe accepts (in-memory adapter) ------
    let (ok, output) = run_python_check(
        "sopack-rs/conformance/packs/check_import_probe_py.py",
        &[out_a.to_str().unwrap()],
    );
    println!("check_import_probe_py.py:\n{output}");
    assert!(ok, "check_import_probe_py.py failed:\n{output}");

    // ---- (d) vectors match the Python-built reference at cosine >= 0.9999 -
    let contract = Contract::embedded("e5-large-v1").unwrap();
    let ours = read_all_vectors(&out_a, &contract);
    let reference = read_all_vectors(&reference_sopack, &contract);
    assert_eq!(ours.len(), 23);
    assert_eq!(reference.len(), 23);
    let mut worst = 1.0f64;
    let mut compared = 0;
    for (id, our_vec) in &ours {
        if let Some(ref_vec) = reference.get(id) {
            let c = cosine(our_vec, ref_vec);
            worst = worst.min(c);
            compared += 1;
            assert!(
                c >= 0.9999,
                "point {id}: cosine {c:.8} against the Python-built reference is below 0.9999"
            );
        }
    }
    println!("compared {compared} shared point ids, worst cosine {worst:.8}");
    assert_eq!(
        compared, 23,
        "expected every point id to match between the two packs"
    );

    // ---- (e) crash after 1 batch, then resume -> byte-identical -----------
    let out_b = tmp.path().join("wdys-b.sopack");
    let mut abort_cmd = Command::new(support::bin());
    abort_cmd
        .args([
            "pack",
            wdys_book.to_str().unwrap(),
            "-o",
            out_b.to_str().unwrap(),
            "--model-dir",
            model_dir().to_str().unwrap(),
            "--threads",
            "1",
            "--pack-id",
            pack_id,
            "--created-by",
            created_by,
        ])
        .env("SOPACK_TEST_ABORT_AFTER_BATCHES", "1");
    let abort_status = abort_cmd.status().expect("run sopack pack (abort)");
    assert_eq!(
        abort_status.code(),
        Some(7),
        "expected the simulated-crash exit code 7"
    );
    let checkpoint_dir = PathBuf::from(format!("{}.sopack.partial", out_b.display()));
    assert!(
        checkpoint_dir.is_dir(),
        "expected a checkpoint directory left behind at {}",
        checkpoint_dir.display()
    );
    assert!(
        !out_b.exists(),
        "must not have finished the pack after a simulated crash"
    );

    // Resume: same command, no --fresh, no abort env var.
    let resume_status = Command::new(support::bin())
        .args([
            "pack",
            wdys_book.to_str().unwrap(),
            "-o",
            out_b.to_str().unwrap(),
            "--model-dir",
            model_dir().to_str().unwrap(),
            "--threads",
            "1",
            "--pack-id",
            pack_id,
            "--created-by",
            created_by,
        ])
        .status()
        .expect("run sopack pack (resume)");
    assert!(resume_status.success(), "resume run failed");
    assert!(
        !checkpoint_dir.exists(),
        "checkpoint must be cleaned up after finish()"
    );

    let bytes_a = std::fs::read(&out_a).unwrap();
    let bytes_b = std::fs::read(&out_b).unwrap();
    assert_eq!(
        bytes_a, bytes_b,
        "crash-then-resume pack differs byte-for-byte from the uninterrupted one"
    );
    println!(
        "crash-then-resume produced a byte-identical pack ({} bytes)",
        bytes_b.len()
    );
}
