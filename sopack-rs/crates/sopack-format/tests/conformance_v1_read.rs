//! M2 pack conformance, direction 1: Rust reads a real `sopack/1` pack
//! (`qdrant/packs/wdys.sopack`, produced by the Python pipeline) and
//! verifies it cleanly. Proves `PackReader`/`verify` are not just
//! self-consistent with the Rust writer, but interoperate with a pack the
//! Python side actually built and ships. See `conformance/README.md`.

use std::path::PathBuf;

use sopack_contract::Contract;
use sopack_format::{verify, PackReader};

fn wdys_pack_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packs/wdys.sopack")
}

#[test]
fn reads_and_checks_a_real_v1_pack() {
    let path = wdys_pack_path();
    assert!(
        path.exists(),
        "{} not found — is qdrant/packs/wdys.sopack still there?",
        path.display()
    );

    let contract = Contract::embedded("e5-large-v1").unwrap();
    let mut reader = PackReader::open(&path).unwrap();
    assert_eq!(reader.manifest.schema, "sopack/1");
    assert_eq!(reader.manifest.profile, "sop");
    assert_eq!(reader.manifest.id_rule, "sop/seq");
    assert_eq!(reader.count(), 23);

    let errors = reader.check(&contract);
    assert_eq!(
        errors,
        Vec::<String>::new(),
        "check() found problems in a real /1 pack: {errors:?}"
    );

    let mut seen = 0u64;
    for batch in reader.batches(&contract, 8, true).unwrap() {
        let (points, vectors) = batch.unwrap();
        assert_eq!(points.len(), vectors.len());
        for v in &vectors {
            assert_eq!(v.len(), reader.dim());
        }
        seen += points.len() as u64;
    }
    assert_eq!(seen, 23);
}

#[test]
fn verify_reports_a_real_v1_pack_as_clean() {
    let contract = Contract::embedded("e5-large-v1").unwrap();
    let report = verify(wdys_pack_path(), &contract);
    assert!(
        report.is_clean(),
        "verify() found problems in a real /1 pack: {:?}",
        report.errors
    );
}
