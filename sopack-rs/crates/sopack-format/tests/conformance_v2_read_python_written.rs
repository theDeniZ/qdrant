//! M2 pack conformance, direction 3 (the reverse of
//! `examples/make_v2_conformance_pack.rs` + `conformance/packs/check_pack_py.py`):
//! if a **Python**-written `sopack/2` pack is available anywhere under
//! `qdrant/packs/`, Rust must read and verify it cleanly too.
//!
//! As of this crate's M2 work, no such pack exists yet in this workspace —
//! producing one needs a real model run (`sopack.pack`, fastembed + the
//! ~2.2 GB model), which is M1/M4 work, not this crate's. This test scans
//! for one at run time rather than assuming a fixed path, so it starts
//! proving the reverse direction automatically the moment the Python side
//! (or a human) drops one in, with no test-code change — see
//! `conformance/README.md` for the current status and how to make this test
//! exercise real coverage.

use std::path::PathBuf;

use sopack_contract::Contract;
use sopack_format::{verify, PackReader};

fn qdrant_packs_dir() -> PathBuf {
    // crates/sopack-format -> crates -> sopack-rs -> qdrant -> packs
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packs")
}

/// A `sopack/2` pack written by the *Python* CLI (`sopack.pack`), not by
/// `examples/make_v2_conformance_pack.rs` (that one's already proven, in the
/// other direction, by `check_pack_py.py`).
fn find_python_written_v2_pack() -> Option<PathBuf> {
    let dir = qdrant_packs_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sopack") {
            continue;
        }
        let Ok(mut reader) = PackReader::open(&path) else {
            continue;
        };
        if reader.manifest.schema != "sopack/2" {
            continue;
        }
        // created_by is the one field pack.py/format.py always stamps with
        // "sopack <version> on <platform>" — never mentions "rust" — while
        // this crate's own writer/examples always say "sopack-rs". Good
        // enough to tell the two producers apart without a dedicated marker.
        if reader.manifest.created_by.to_lowercase().contains("rust") {
            continue;
        }
        let _ = reader.check(&Contract::embedded("e5-large-v1").unwrap());
        return Some(path);
    }
    None
}

#[test]
fn reads_a_python_written_v2_pack_if_one_exists() {
    let Some(path) = find_python_written_v2_pack() else {
        eprintln!(
            "no Python-written sopack/2 pack found under {} — skipping (not a failure; \
             see conformance/README.md, 'packs/ — direction 3')",
            qdrant_packs_dir().display()
        );
        return;
    };

    let contract = Contract::embedded("e5-large-v1").unwrap();
    let mut reader = PackReader::open(&path).unwrap();
    let errors = reader.check(&contract);
    assert_eq!(
        errors,
        Vec::<String>::new(),
        "check() found problems in {}: {errors:?}",
        path.display()
    );

    let mut seen = 0u64;
    for batch in reader.batches(&contract, 64, true).unwrap() {
        let (points, vectors) = batch.unwrap();
        seen += points.len() as u64;
        assert_eq!(points.len(), vectors.len());
    }
    assert_eq!(seen, reader.count());

    let report = verify(&path, &contract);
    assert!(
        report.is_clean(),
        "verify() found problems in {}: {:?}",
        path.display(),
        report.errors
    );
}
