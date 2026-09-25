//! M2 exit test for SOPACK-1.0-PLAN.md §3.6: "a new model means a new
//! contract directory … It needs **no code change** in sopack as long as
//! the model fits the 'ONNX encoder + pooling' shape. M2 tests this with a
//! small second model (e.g. `multilingual-e5-small`) as a test-only
//! contract."
//!
//! `tests/fixtures/test-e5-small/` is a complete, self-consistent contract
//! for `intfloat/multilingual-e5-small` (384-d, vs. the released
//! `e5-large-v1`'s 1024-d) with its own synthetic `calibration.json` — model
//! weights are never downloaded (`model.files` sha256/bytes are
//! placeholders; nothing in `sopack-contract` fetches them). This test
//! loads it with **only** [`Contract::from_dir`] — the same entry point
//! `--contract <path>` uses — and exercises every piece of behavior the
//! released contract's own tests exercise, proving none of it is hardcoded
//! to `e5-large-v1`'s shape (its id, dimension, or model name appear
//! nowhere in `sopack-contract`'s source).

use std::path::PathBuf;

use serde_json::json;
use sopack_contract::{validate_payload, Contract};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-e5-small")
}

#[test]
fn a_second_model_loads_and_validates_with_no_code_change() {
    let c = Contract::from_dir(fixture_dir()).expect("test-e5-small contract must load");

    assert_eq!(c.id, "e5-small-test");
    assert_eq!(c.embedding.model, "intfloat/multilingual-e5-small");
    assert_eq!(c.embedding.dim, 384);
    assert_ne!(
        c.embedding.dim, 1024,
        "must not be silently reusing e5-large-v1's dimension"
    );

    // Calibration: loaded and sha256-verified against [calibration].sha256,
    // exactly like the released contract.
    let fixture = c.fixture.as_ref().expect("calibration.json must load");
    assert_eq!(c.calibration_sha256().unwrap(), fixture.sha256());
    assert_eq!(fixture.entries.len(), 2);
    assert!(fixture.entries.iter().all(|e| e.vector.len() == 384));

    // Id rules: same templates as e5-large-v1 (a model swap doesn't change
    // id rules, but they still come from this contract's own [ids.rules],
    // not a hardcoded table).
    let rule = c.get_id_rule("sop/seq").unwrap();
    let fields = json!({"lang": "en", "book_code": "TT", "para_key": "1.1", "seq": 0})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(rule.build_uid(&fields).unwrap(), "en:TT:1.1#0");
    let id = c.point_id("sop/seq", &fields).unwrap();
    assert!(!id.is_empty());

    // Payload validation against this contract's own profile.
    let profile = c.get_profile("sop").unwrap();
    let payload = json!({
        "lang": "en", "book_code": "TT", "book_pair": "TT", "page": 1,
        "para": 1, "para_key": "1.1", "raw_text": "hello", "aligned": null
    });
    assert_eq!(
        validate_payload(profile, payload.as_object().unwrap()),
        Vec::<String>::new()
    );

    // check_embedding: a declared block matching *this* contract passes;
    // one declaring e5-large-v1's dim fails — proves the check is driven by
    // the loaded contract, not a compiled-in constant.
    let matching = sopack_contract::DeclaredEmbedding {
        model: c.embedding.model.clone(),
        pooling: c.embedding.pooling.clone(),
        normalized: c.embedding.normalized,
        dim: c.embedding.dim,
        distance: c.embedding.distance.clone(),
        max_tokens: c.embedding.max_tokens,
        passage_prefix: c.embedding.passage_prefix.clone(),
    };
    assert_eq!(
        sopack_contract::check_embedding(&c, &matching),
        Vec::<String>::new()
    );
    let mut wrong_dim = matching;
    wrong_dim.dim = 1024;
    assert!(!sopack_contract::check_embedding(&c, &wrong_dim).is_empty());

    // embed_spec(): the flattened accessors an embedding engine would read.
    let spec = c.embed_spec();
    assert_eq!(spec.repo, "intfloat/multilingual-e5-small");
    assert_eq!(spec.dim, 384);
    assert_eq!(spec.passage_prefix, "passage: ");
}
