//! Port of `qdrant/sopack/tests/test_book.py` — same test names (minus the
//! `test_` prefix's Python-class grouping, folded into Rust module names),
//! same behaviour asserted.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use sopack_book::{
    dump, get_profile, load, point_id, to_payload, uid, uid_for, validate, validate_payload, Block,
    Book, SCHEMA_BOOK,
};

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => panic!("expected object"),
    }
}

/// Port of `_plain_sop_book()`. `overrides` lets a caller replace individual
/// `book` metadata keys, mirroring the Python `**overrides` kwarg.
fn plain_sop_book(overrides: &[(&str, Value)]) -> Book {
    let mut meta = obj(json!({
        "book_code": "ABC",
        "lang": "en",
        "book_pair": "ABC",
        "title": "A Book",
        "author": null,
        "year": null,
        "slug": "a-book",
        "corpus": null,
        "page_kind": null,
    }));
    for (k, v) in overrides {
        meta.insert((*k).to_string(), v.clone());
    }
    let blocks = vec![
        Block::new("1.1", 1, 1, 0, 1, "Hello world today.", 3, None),
        Block::new("1.2", 1, 2, 0, 1, "A second paragraph here.", 4, None),
    ];
    Book {
        schema: SCHEMA_BOOK.to_string(),
        profile: "sop".to_string(),
        source: obj(json!({
            "file": "x.json", "sha256": "abc", "kind": "sop_json",
            "acquired_from": null, "rights": null,
        })),
        book: meta,
        id_rule: "sop/plain".to_string(),
        alignment: None,
        stats: obj(json!({
            "blocks_in": 2, "blocks_out": 2, "dropped": 0, "damage": 0.0,
            "words": 7, "dropped_detail": [],
        })),
        blocks,
    }
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    // Leak the tempdir so the file survives for the caller; tests are
    // short-lived processes so this is fine.
    std::mem::forget(dir);
    path
}

// ── DumpLoadRoundTrip ────────────────────────────────────────────────────

#[test]
fn test_round_trip_preserves_content() {
    let book = plain_sop_book(&[]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    assert!(path.exists());
    let back = load(&path).unwrap();
    assert_eq!(back.schema, book.schema);
    assert_eq!(back.profile, book.profile);
    assert_eq!(back.book, book.book);
    assert_eq!(back.id_rule, book.id_rule);
    assert_eq!(back.alignment, book.alignment);
    assert_eq!(back.stats, book.stats);
    assert_eq!(back.blocks, book.blocks);
}

#[test]
fn test_dump_has_stable_key_order_and_indent() {
    let book = plain_sop_book(&[]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let doc: Value = serde_json::from_str(&raw).unwrap();
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec![
            "schema",
            "profile",
            "source",
            "book",
            "id_rule",
            "alignment",
            "stats",
            "blocks"
        ]
    );
    assert!(raw.contains("\n  "));
    let block_keys: Vec<&String> = doc["blocks"][0].as_object().unwrap().keys().collect();
    assert_eq!(
        block_keys,
        vec!["para_key", "page", "para", "seq", "chunk", "chunks", "text", "words"]
    );
}

#[test]
fn test_load_defaults_chunk_to_seq_for_pre_0_1_4_files() {
    let book = plain_sop_book(&[]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut doc: Value = serde_json::from_str(&raw).unwrap();
    for b in doc["blocks"].as_array_mut().unwrap() {
        b.as_object_mut().unwrap().remove("chunk");
    }
    doc["blocks"][1]["seq"] = json!(1);
    doc["blocks"][1]["para_key"] = json!("1.1");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let loaded = load(&path).unwrap();
    let chunks: Vec<i64> = loaded.blocks.iter().map(|b| b.chunk).collect();
    assert_eq!(chunks, vec![0, 1]);
}

#[test]
fn test_dump_load_round_trips_chunk() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("3.1", 3, 1, 0, 1, "a heading", 2, Some(0)),
        Block::new("3.1", 3, 1, 1, 2, "piece one", 2, Some(0)),
        Block::new("3.1", 3, 1, 2, 2, "piece two", 2, Some(1)),
    ];
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let loaded = load(&path).unwrap();
    let got: Vec<(i64, i64)> = loaded.blocks.iter().map(|b| (b.seq, b.chunk)).collect();
    assert_eq!(got, vec![(0, 0), (1, 0), (2, 1)]);
    assert_eq!(validate(&loaded), Vec::<String>::new());
}

#[test]
fn test_load_rejects_bad_json() {
    let path = tmp_path("book.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load(&path).is_err());
}

#[test]
fn test_load_rejects_missing_top_level_key() {
    let path = tmp_path("book.json");
    let doc = json!({"schema": SCHEMA_BOOK, "profile": "sop"});
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(load(&path).is_err());
}

#[test]
fn test_load_rejects_unknown_profile() {
    let book = plain_sop_book(&[("book_code", Value::Null)]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut doc: Value = serde_json::from_str(&raw).unwrap();
    doc["profile"] = json!("nonsense");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(load(&path).is_err());
}

#[test]
fn test_load_rejects_malformed_block() {
    let book = plain_sop_book(&[]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut doc: Value = serde_json::from_str(&raw).unwrap();
    doc["blocks"][0].as_object_mut().unwrap().remove("words");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(load(&path).is_err());
}

#[test]
fn test_load_accepts_null_metadata() {
    let book = plain_sop_book(&[("book_code", Value::Null), ("title", Value::Null)]);
    let path = tmp_path("book.json");
    dump(&book, &path).unwrap();
    let back = load(&path).unwrap(); // must not raise
    assert!(back.book.get("book_code").unwrap().is_null());
}

// ── Validate ─────────────────────────────────────────────────────────────

#[test]
fn test_valid_book_has_no_errors() {
    assert_eq!(validate(&plain_sop_book(&[])), Vec::<String>::new());
}

#[test]
fn test_missing_required_metadata() {
    let book = plain_sop_book(&[
        ("book_code", Value::Null),
        ("lang", Value::Null),
        ("title", Value::Null),
    ]);
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("book_code")));
    assert!(errors.iter().any(|e| e.contains("lang")));
    assert!(errors.iter().any(|e| e.contains("title")));
}

#[test]
fn test_egw_exempt_from_author_year() {
    let book = plain_sop_book(&[
        ("corpus", Value::Null),
        ("author", Value::Null),
        ("year", Value::Null),
    ]);
    let errors = validate(&book);
    assert!(!errors.iter().any(|e| e.contains("author")));
    assert!(!errors.iter().any(|e| e.contains("year")));
}

#[test]
fn test_non_egw_requires_author_year() {
    let book = plain_sop_book(&[
        ("corpus", json!("pioneers")),
        ("author", Value::Null),
        ("year", Value::Null),
    ]);
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("author")));
    assert!(errors.iter().any(|e| e.contains("year")));
}

#[test]
fn test_non_egw_with_author_year_is_clean() {
    let book = plain_sop_book(&[
        ("corpus", json!("pioneers")),
        ("author", json!("J. N. Andrews")),
        ("year", json!(1873)),
    ]);
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_rejects_bad_id_rule_for_profile() {
    let mut book = plain_sop_book(&[]);
    book.id_rule = "bible/v1".to_string();
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("id_rule")));
}

#[test]
fn test_rejects_duplicate_para_key_seq() {
    let mut book = plain_sop_book(&[]);
    book.blocks
        .push(Block::new("1.1", 1, 1, 0, 1, "duplicate", 1, None));
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("duplicate")));
}

#[test]
fn test_rejects_empty_text() {
    let mut book = plain_sop_book(&[]);
    book.blocks[0].text = "   ".to_string();
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("empty text")));
}

#[test]
fn test_rejects_chunks_seq_inconsistency_gap() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("9.1", 9, 1, 0, 3, "part one here", 3, None),
        Block::new("9.1", 9, 1, 2, 3, "part three here", 3, None),
    ];
    let errors = validate(&book);
    assert!(errors
        .iter()
        .any(|e| e.contains("9.1") && e.contains("seq")));
}

#[test]
fn test_rejects_inconsistent_chunks_value() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("9.1", 9, 1, 0, 2, "part one here", 3, None),
        Block::new("9.1", 9, 1, 1, 3, "part two here", 3, None),
    ];
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("inconsistent")));
}

#[test]
fn test_accepts_two_paragraphs_sharing_one_para_key() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("3.1", 3, 1, 0, 1, "Revelation 18 verse one", 4, Some(0)),
        Block::new(
            "3.1",
            3,
            1,
            1,
            1,
            "II. WHAT ARE WE TO UNDERSTAND",
            6,
            Some(0),
        ),
    ];
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_accepts_split_paragraph_after_a_colliding_one() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("3.1", 3, 1, 0, 1, "the heading here", 3, Some(0)),
        Block::new("3.1", 3, 1, 1, 2, "a long paragraph", 3, Some(0)),
        Block::new("3.1", 3, 1, 2, 2, "split in two", 3, Some(1)),
    ];
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_rejects_seq_gap_across_colliding_paragraphs() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("3.1", 3, 1, 0, 1, "first here", 2, Some(0)),
        Block::new("3.1", 3, 1, 2, 1, "third here", 2, Some(0)),
    ];
    let errors = validate(&book);
    assert!(errors
        .iter()
        .any(|e| e.contains("3.1") && e.contains("seq")));
}

#[test]
fn test_rejects_truncated_split_paragraph() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![Block::new("3.1", 3, 1, 0, 2, "only piece", 2, Some(0))];
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("chunks=2")));
}

#[test]
fn test_valid_chunked_block_is_clean() {
    let mut book = plain_sop_book(&[]);
    book.blocks = vec![
        Block::new("9.1", 9, 1, 0, 2, "part one here", 3, None),
        Block::new("9.1", 9, 1, 1, 2, "part two here", 3, None),
    ];
    assert_eq!(validate(&book), Vec::<String>::new());
}

// ── Payload ──────────────────────────────────────────────────────────────

#[test]
fn test_sop_payload_matches_contract() {
    let book = plain_sop_book(&[]);
    let profile = get_profile("sop").unwrap();
    for block in &book.blocks {
        let payload = to_payload(&book, block).unwrap();
        assert_eq!(validate_payload(profile, &payload), Vec::<String>::new());
    }
}

#[test]
fn test_sop_payload_carries_required_keys_even_when_none() {
    let book = plain_sop_book(&[("book_pair", Value::Null)]);
    let payload = to_payload(&book, &book.blocks[0]).unwrap();
    assert!(payload.contains_key("book_pair"));
    assert!(payload["book_pair"].is_null());
    assert!(payload.contains_key("aligned"));
    assert!(payload["aligned"].is_null());
}

#[test]
fn test_sop_payload_omits_absent_optional_keys() {
    let book = plain_sop_book(&[
        ("author", Value::Null),
        ("title", json!("Some Title")),
        ("year", Value::Null),
        ("slug", Value::Null),
        ("page_kind", Value::Null),
        ("corpus", Value::Null),
    ]);
    let payload = to_payload(&book, &book.blocks[0]).unwrap();
    assert!(!payload.contains_key("author"));
    assert!(!payload.contains_key("year"));
    assert!(!payload.contains_key("slug"));
    assert!(!payload.contains_key("page_kind"));
    assert!(!payload.contains_key("corpus"));
    assert_eq!(payload["title"], json!("Some Title"));
}

#[test]
fn test_sop_payload_chunk_fields_only_when_split() {
    let book = plain_sop_book(&[]);
    let unsplit = Block::new("2.1", 2, 1, 0, 1, "one piece", 2, None);
    let split = Block::new("2.2", 2, 2, 1, 2, "second piece", 2, None);
    let p1 = to_payload(&book, &unsplit).unwrap();
    let p2 = to_payload(&book, &split).unwrap();
    assert!(!p1.contains_key("chunk"));
    assert!(!p1.contains_key("chunks"));
    assert_eq!(p2["chunk"], json!(1));
    assert_eq!(p2["chunks"], json!(2));
}

#[test]
fn test_sop_payload_chunk_is_paragraph_local_not_seq() {
    let book = plain_sop_book(&[]);
    let block = Block::new("3.1", 3, 1, 1, 2, "first piece", 2, Some(0));
    let payload = to_payload(&book, &block).unwrap();
    assert_eq!(payload["chunk"], json!(0));
    assert_eq!(payload["chunks"], json!(2));
}

#[test]
fn test_aligned_pulled_from_alignment_en_reverse() {
    let mut book = plain_sop_book(&[("lang", json!("de")), ("book_code", json!("BH"))]);
    book.alignment = Some(obj(json!({
        "en_code": "SL",
        "en_reverse": {"7.1": ["1.1"]},
    })));
    let payload = to_payload(&book, &book.blocks[0]).unwrap(); // para_key "1.1"
    assert_eq!(payload["aligned"], json!(["7.1"]));
    let payload2 = to_payload(&book, &book.blocks[1]).unwrap(); // "1.2" has no entry
    assert!(payload2["aligned"].is_null());
}

#[test]
fn test_aligned_direct_lookup_for_english_book() {
    let mut book = plain_sop_book(&[("lang", json!("en")), ("book_code", json!("SL"))]);
    book.alignment = Some(obj(json!({
        "en_code": "SL",
        "en_reverse": {"1.1": ["5.1"]},
    })));
    let payload = to_payload(&book, &book.blocks[0]).unwrap(); // para_key "1.1"
    assert_eq!(payload["aligned"], json!(["5.1"]));
}

#[test]
fn test_bible_payload_matches_contract() {
    let mut book = plain_sop_book(&[("book_code", json!("KJV"))]);
    book.profile = "bible".to_string();
    let profile = get_profile("bible").unwrap();
    let payload = to_payload(&book, &book.blocks[0]).unwrap();
    assert_eq!(validate_payload(profile, &payload), Vec::<String>::new());
}

// ── Uid ──────────────────────────────────────────────────────────────────

fn fields(pairs: &[(&'static str, Option<&str>)]) -> HashMap<&'static str, Option<String>> {
    let mut m = HashMap::new();
    for (k, v) in pairs {
        m.insert(*k, v.map(str::to_string));
    }
    m
}

#[test]
fn test_uid_agrees_with_contract_for_sop_plain() {
    let book = plain_sop_book(&[("lang", json!("en")), ("book_code", json!("ABC"))]);
    let block = &book.blocks[0];
    let got = uid(&book, block).unwrap();
    let want = uid_for(
        "sop/plain",
        &fields(&[
            ("lang", Some("en")),
            ("book_code", Some("ABC")),
            ("para_key", Some("1.1")),
        ]),
    )
    .unwrap();
    assert_eq!(got, want);
}

#[test]
fn test_uid_agrees_with_contract_for_sop_seq() {
    let mut book = plain_sop_book(&[("lang", json!("en")), ("book_code", json!("ABC"))]);
    book.id_rule = "sop/seq".to_string();
    let block = Block::new("3.4", 3, 4, 2, 3, "x y z", 3, None);
    let got = uid(&book, &block).unwrap();
    let want = uid_for(
        "sop/seq",
        &fields(&[
            ("lang", Some("en")),
            ("book_code", Some("ABC")),
            ("para_key", Some("3.4")),
            ("seq", Some("2")),
        ]),
    )
    .unwrap();
    assert_eq!(got, want);
}

#[test]
fn test_uid_agrees_with_contract_for_bible() {
    let mut book = plain_sop_book(&[("book_code", json!("KJV"))]);
    book.profile = "bible".to_string();
    book.id_rule = "bible/v1".to_string();
    let block = book.blocks[0].clone();
    let got = uid(&book, &block).unwrap();
    let want = uid_for(
        "bible/v1",
        &fields(&[
            ("bible", Some("KJV")),
            ("osis", Some(block.para_key.as_str())),
        ]),
    )
    .unwrap();
    assert_eq!(got, want);
}

#[test]
fn test_uid_feeds_a_stable_point_id() {
    let book = plain_sop_book(&[("lang", json!("en")), ("book_code", json!("ABC"))]);
    let f = fields(&[
        ("lang", Some("en")),
        ("book_code", Some("ABC")),
        ("para_key", Some("1.1")),
    ]);
    let pid1 = point_id(&book.id_rule, &f).unwrap();
    let pid2 = point_id(&book.id_rule, &f).unwrap();
    assert_eq!(pid1, pid2);
}
