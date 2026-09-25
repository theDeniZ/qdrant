//! `cargo run --release --example bench -p sopack-embed -- <model_dir> [text_file]`
//!
//! Reports blocks/s and tokens/s at 1, 2, 4 and `available_parallelism()`
//! threads on a given book.json-like text list (SOPACK-1.0-PLAN.md §3.3
//! "Benchmark helper"). `text_file` is one text per line; without it, a
//! small built-in sample is repeated to give the batcher something to sort.
//!
//! Needs `ORT_DYLIB_PATH` set (or the dylib next to the binary — see
//! `runtime::resolve_ort_dylib`) and, per this job's shared rules, must run
//! under `flock .../model.lock` since it loads the ~2.5 GB model.
//!
//! This is the helper the integration agent uses later to measure a real
//! pack run; it prints numbers, it doesn't assert anything.

use sopack_contract::Contract;
use sopack_embed::device::Device;
use sopack_embed::engine::{Engine, EngineOptions};
use sopack_embed::spec::{EmbedSpec, Role};
use sopack_progress::NullSink;
use std::path::PathBuf;
use std::time::Instant;

fn load_spec_from_contract(_contract_dir: &std::path::Path) -> EmbedSpec {
    // The embedded e5-large-v1 contract is exactly the one under
    // `contracts/e5-large-v1/` this workspace ships — loading it via
    // `sopack-contract` (instead of hand-parsing contract.toml a second
    // time) is the single source of truth for the mapping (see
    // `sopack_embed::contract_spec`).
    let contract = Contract::embedded("e5-large-v1").expect("load embedded e5-large-v1 contract");
    EmbedSpec::from(&contract)
}

fn sample_texts() -> Vec<String> {
    let base = [
        "For God so loved the world, that he gave his only begotten Son, that whosoever believeth in him should not perish, but have everlasting life.",
        "The great plan of redemption results in fully bringing back the world into God's favor.",
        "Blessed are the poor in spirit: for theirs is the kingdom of heaven. Blessed are they that mourn: for they shall be comforted.",
        "Short text.",
        "In the beginning God created the heaven and the earth. And the earth was without form, and void; and darkness was upon the face of the deep.",
    ];
    (0..40).map(|i| base[i % base.len()].to_string()).collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let model_dir = PathBuf::from(args.next().expect("usage: bench <model_dir> [text_file]"));
    let contract_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/e5-large-v1");
    let spec = load_spec_from_contract(&contract_dir);

    let texts: Vec<String> = match args.next() {
        Some(path) => std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {path}: {e}"))
            .lines()
            .map(str::to_string)
            .collect(),
        None => sample_texts(),
    };
    println!(
        "bench: {} texts, model_dir={}",
        texts.len(),
        model_dir.display()
    );

    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut thread_counts = vec![1usize, 2, 4];
    if !thread_counts.contains(&cores) {
        thread_counts.push(cores);
    }
    thread_counts.retain(|&t| t <= cores || t == 1);

    for threads in thread_counts {
        let mut engine = Engine::new(
            spec.clone(),
            &model_dir,
            EngineOptions {
                device: Device::Cpu,
                threads: Some(threads),
                batch_tokens: None,
            },
            &NullSink,
        )
        .unwrap_or_else(|e| panic!("load engine at threads={threads}: {e}"));
        let total_tokens = engine
            .count_tokens(&texts)
            .unwrap_or_else(|e| panic!("count tokens at threads={threads}: {e}"));
        let t0 = Instant::now();
        let vecs = engine
            .embed(&texts, Role::Passage, &NullSink)
            .unwrap_or_else(|e| panic!("embed at threads={threads}: {e}"));
        let elapsed = t0.elapsed().as_secs_f64();
        println!(
            "threads={threads:<2} blocks={:<4} tokens={total_tokens:<7} {:>7.2}s  {:>6.2} blocks/s  {:>8.0} tokens/s  (dim={})",
            vecs.len(),
            elapsed,
            vecs.len() as f64 / elapsed,
            total_tokens as f64 / elapsed,
            vecs.first().map(|v| v.len()).unwrap_or(0)
        );
    }
}
