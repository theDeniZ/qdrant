//! Writes `qdrant/sopack-rs/conformance/packs/rust_v2_sample.sopack` — the
//! M2 pack-conformance leg's "Rust writes, Python reads" artifact
//! (SOPACK-1.0-PLAN.md §4). See `conformance/README.md`.
//!
//! Uses the real, committed calibration fixture both for the probe (as
//! instructed: "set a valid self_check using the fixture vectors themselves
//! as the probe") and, for simplicity and determinism, as the book points'
//! vectors too — this is a *format* conformance pack, not an embedding one,
//! so the vectors' content doesn't need to come from a real model run, only
//! be real 1024-d floats of the right shape.
//!
//! Run: `cargo run -p sopack-format --example make_v2_conformance_pack`

use sopack_contract::Contract;
use sopack_format::{EmbeddingProvenance, PackWriter, ProbeEntryInput};

fn main() {
    let contract = Contract::embedded("e5-large-v1")
        .expect("the embedded e5-large-v1 contract (and its calibration.json fixture) must load");
    let fixture = contract
        .fixture
        .as_ref()
        .expect("this example needs a generated calibration.json — see contracts/e5-large-v1/");

    let sop_entries: Vec<_> = fixture.entries_for_profile("sop").take(3).collect();
    assert!(
        sop_entries.len() >= 2,
        "need at least 2 'sop' fixture entries to build a sample pack, found {}",
        sop_entries.len()
    );

    let out_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/packs");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("rust_v2_sample.sopack");
    // A previous run's checkpoint (if any) would otherwise make `create`
    // refuse — this example always starts fresh.
    let checkpoint = std::path::PathBuf::from(format!("{}.sopack.partial", out_path.display()));
    if checkpoint.exists() {
        std::fs::remove_dir_all(&checkpoint).unwrap();
    }
    let _ = std::fs::remove_file(&out_path);

    let provenance = EmbeddingProvenance {
        runtime: format!(
            "sopack-rs {} (conformance example, no real model run)",
            env!("CARGO_PKG_VERSION")
        ),
        device: "cpu".to_string(),
        threads: 1,
        batch_tokens: contract.embedding.max_tokens,
    };

    let mut w = PackWriter::create(
        &out_path,
        &contract,
        "sop",
        "conformance-v2-sample".to_string(),
        format!(
            "sopack-rs {} conformance example",
            env!("CARGO_PKG_VERSION")
        ),
        None,
        provenance,
    )
    .expect("PackWriter::create");

    for (i, entry) in sop_entries.iter().enumerate() {
        let payload = serde_json::json!({
            "lang": entry.lang.clone().unwrap_or_else(|| "en".to_string()),
            "book_code": "CONF",
            "book_pair": "CONF",
            "page": 1,
            "para": i + 1,
            "para_key": format!("1.{}", i + 1),
            "raw_text": entry.text,
            "aligned": null,
        });
        let uid = format!(
            "{}:CONF:1.{}#0",
            entry.lang.as_deref().unwrap_or("en"),
            i + 1
        );
        w.add(&uid, payload.as_object().unwrap().clone(), &entry.vector)
            .unwrap_or_else(|e| panic!("add point {i}: {e}"));
    }

    w.set_books(vec![sopack_format::BookEntry {
        book_code: "CONF".to_string(),
        lang: Some("en".to_string()),
        points: w.count(),
        first_id: None,
        title: Some("sopack-rs conformance sample".to_string()),
        author: None,
        year: None,
        corpus: Some("conformance".to_string()),
        slug: Some("conformance-sample".to_string()),
        book_pair: Some("CONF".to_string()),
        id_rule: "sop/seq".to_string(),
        book_sha256: None,
    }]);

    // Probe: every fixture entry, embedded (here: reused verbatim) as "this
    // pack's own" vectors — cosine against the fixture is exactly 1.0,
    // trivially clearing pack_min_cosine, which is the point: this pack
    // proves the *format*, not a real embedding run.
    let probe_entries: Vec<ProbeEntryInput> = fixture
        .entries
        .iter()
        .map(|e| ProbeEntryInput {
            id: e.id.clone(),
            profile: e.profile.clone(),
        })
        .collect();
    let probe_vectors: Vec<Vec<f32>> = fixture.entries.iter().map(|e| e.vector.clone()).collect();
    let n = probe_vectors.len() as u64;
    w.set_probe(probe_entries, probe_vectors, n, 1.0, 1.0)
        .expect("set_probe");

    let path = w.finish().expect("finish");
    println!("wrote {}", path.display());
}
