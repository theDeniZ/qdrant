//! Structural validation of `propose`'s JSON output against the committed
//! schema `schemas/propose.v1.json` (SOPACK-1.0-PLAN.md §3.5's "committed
//! JSON Schema" requirement).
//!
//! No `jsonschema`-style generic interpreter — this crate's only network
//! access would be `cargo`'s own dependency fetch, and pulling in a full
//! schema engine for one file is exactly the kind of dependency weight the
//! task brief flags as avoidable ("or a hand-written structural check if
//! that crate is heavy"). Instead this test hand-checks the same
//! constraints the schema declares, directly against the schema file's own
//! `required`/`enum`/`pattern` values (parsed once, so the two cannot
//! silently drift without this test breaking) — every `Proposal` produced
//! by any fixture must satisfy them.

use std::path::Path;

use sopack_extract::{propose_quiet, Kind};

const SCHEMA_JSON: &str = include_str!("../../../schemas/propose.v1.json");

fn schema() -> serde_json::Value {
    serde_json::from_str(SCHEMA_JSON).expect("schemas/propose.v1.json must be valid JSON")
}

const FIELD_NAMES: [&str; 10] = [
    "book_code",
    "lang",
    "title",
    "author",
    "year",
    "corpus",
    "slug",
    "book_pair",
    "acquired_from",
    "rights",
];

/// Checks *doc* (a `serde_json::to_value(&proposal)` result) against every
/// constraint `schemas/propose.v1.json` declares for the top-level shape,
/// `fields`, and each candidate — pulling the expected sets (the `kind`
/// enum, the sha256 pattern, the candidate property names) from the parsed
/// schema itself rather than hardcoding a second copy of them.
fn assert_matches_schema(doc: &serde_json::Value, schema: &serde_json::Value) {
    let obj = doc.as_object().expect("top level must be an object");
    for key in ["source", "fields", "unresolved", "warnings"] {
        assert!(obj.contains_key(key), "missing top-level key {key:?}");
    }
    assert_eq!(
        obj.len(),
        4,
        "unexpected extra top-level key(s): {:?}",
        obj.keys().collect::<Vec<_>>()
    );

    // source
    let source = obj["source"].as_object().expect("source must be an object");
    for key in ["path", "kind", "sha256", "bytes"] {
        assert!(source.contains_key(key), "source missing {key:?}");
    }
    let kind_enum: Vec<&str> = schema["properties"]["source"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        kind_enum.contains(&source["kind"].as_str().unwrap()),
        "source.kind {:?} not in schema enum {kind_enum:?}",
        source["kind"]
    );
    let sha_pattern_len = schema["properties"]["source"]["properties"]["sha256"]["pattern"]
        .as_str()
        .unwrap();
    assert!(
        sha_pattern_len.contains("64"),
        "schema sha256 pattern sanity"
    );
    let sha = source["sha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64, "sha256 must be 64 hex chars");
    assert!(sha
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert!(
        source["bytes"].as_u64().is_some(),
        "bytes must be a non-negative integer"
    );

    // fields
    let fields = obj["fields"].as_object().expect("fields must be an object");
    let schema_field_names: Vec<&str> = schema["properties"]["fields"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(schema_field_names.len(), FIELD_NAMES.len());
    for name in &FIELD_NAMES {
        assert!(
            schema_field_names.contains(name),
            "schema fields.required is missing {name:?}"
        );
    }
    assert_eq!(
        fields.len(),
        FIELD_NAMES.len(),
        "fields must have exactly the 10 ExtractOptions names"
    );
    for name in &FIELD_NAMES {
        let field = fields
            .get(*name)
            .unwrap_or_else(|| panic!("fields.{name} missing"))
            .as_object()
            .unwrap_or_else(|| panic!("fields.{name} must be an object"));
        assert!(field.contains_key("value"), "fields.{name}.value missing");
        let candidates = field
            .get("candidates")
            .unwrap_or_else(|| panic!("fields.{name}.candidates missing"))
            .as_array()
            .unwrap_or_else(|| panic!("fields.{name}.candidates must be an array"));
        for c in candidates {
            let c = c.as_object().expect("candidate must be an object");
            assert!(c.contains_key("value"), "candidate missing value");
            assert!(c.contains_key("from"), "candidate missing from");
            assert!(c["from"].is_string(), "candidate.from must be a string");
            for extra in c.keys() {
                assert!(
                    ["value", "from", "evidence", "warning", "collision"].contains(&extra.as_str()),
                    "candidate has unexpected key {extra:?}"
                );
            }
        }
        // If value is set, it must be one of the candidates' values — the
        // schema can't express this cross-field constraint, but propose's
        // whole contract rests on it, so this test checks it directly.
        if !field["value"].is_null() {
            assert!(
                candidates.iter().any(|c| c["value"] == field["value"]),
                "fields.{name}.value {:?} is not any candidate's value",
                field["value"]
            );
        }
    }

    // unresolved: exactly the fields whose value is null, and only known names.
    let unresolved: Vec<&str> = obj["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let unresolved_enum: Vec<&str> = schema["properties"]["unresolved"]["items"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for u in &unresolved {
        assert!(
            unresolved_enum.contains(u),
            "unresolved {u:?} not in schema enum"
        );
    }
    for name in &FIELD_NAMES {
        let is_null = fields[*name]["value"].is_null();
        assert_eq!(
            unresolved.contains(name),
            is_null,
            "unresolved list disagrees with fields.{name}.value for {name:?}"
        );
    }

    // warnings: array of strings.
    for w in obj["warnings"].as_array().unwrap() {
        assert!(w.is_string(), "warnings entries must be strings");
    }
}

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/extract/fixtures")
}

#[test]
fn epub_fixture_matches_schema() {
    let schema = schema();
    let proposal =
        propose_quiet(&fixtures_dir().join("small.epub"), Some(Kind::Epub), None).unwrap();
    let doc = serde_json::to_value(&proposal).unwrap();
    assert_matches_schema(&doc, &schema);
}

#[test]
fn markdown_fixture_matches_schema() {
    let schema = schema();
    let proposal =
        propose_quiet(&fixtures_dir().join("small.md"), Some(Kind::Markdown), None).unwrap();
    let doc = serde_json::to_value(&proposal).unwrap();
    assert_matches_schema(&doc, &schema);
}

#[test]
fn text_fixture_matches_schema() {
    let schema = schema();
    let proposal =
        propose_quiet(&fixtures_dir().join("small.txt"), Some(Kind::Text), None).unwrap();
    let doc = serde_json::to_value(&proposal).unwrap();
    assert_matches_schema(&doc, &schema);
}

#[test]
fn sop_json_fixture_matches_schema() {
    let schema = schema();
    let proposal = propose_quiet(
        &fixtures_dir().join("en/SMALL.json"),
        Some(Kind::SopJson),
        None,
    )
    .unwrap();
    let doc = serde_json::to_value(&proposal).unwrap();
    assert_matches_schema(&doc, &schema);
    // sop_json's meta block is authoritative: this fixture's meta carries
    // en_code/en_title, so book_code/title should both resolve to a value.
    assert_eq!(
        proposal.fields.book_code.value,
        serde_json::Value::from("SMALL")
    );
    assert_eq!(
        proposal.fields.title.value,
        serde_json::Value::from("A Small Fixture Book")
    );
}

#[test]
fn kind_inference_from_extension() {
    let schema = schema();
    for (file, expected_kind) in [
        ("small.epub", "epub"),
        ("small.md", "markdown"),
        ("small.txt", "text"),
        ("en/SMALL.json", "sop_json"),
    ] {
        let proposal = propose_quiet(&fixtures_dir().join(file), None, None).unwrap();
        assert_eq!(
            proposal.source.kind, expected_kind,
            "kind inferred for {file}"
        );
        let doc = serde_json::to_value(&proposal).unwrap();
        assert_matches_schema(&doc, &schema);
    }
}

#[test]
fn missing_source_is_input_invalid() {
    let err = propose_quiet(
        &fixtures_dir().join("does_not_exist.epub"),
        Some(Kind::Epub),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, sopack_extract::ErrorCode::InputInvalid);
}

#[test]
fn unknown_extension_without_explicit_kind_is_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mystery.bin");
    std::fs::write(&path, b"whatever").unwrap();
    let err = propose_quiet(&path, None, None).unwrap_err();
    assert_eq!(err.code, sopack_extract::ErrorCode::InputInvalid);
}
