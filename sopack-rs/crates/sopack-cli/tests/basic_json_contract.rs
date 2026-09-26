//! `--json` output validates against its committed schema, for every
//! command that needs no model/ORT (`SOPACK-1.0-PLAN.md` §3.5).

mod support;

use support::{assert_valid, run};

#[test]
fn version_prints_the_documented_shape() {
    let r = run(&["--version"]);
    assert_eq!(r.status, 0);
    assert_eq!(
        r.stdout.trim(),
        format!("sopack {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(r.stderr.is_empty());
}

#[test]
fn doctor_quick_json_matches_schema_and_exits_cleanly_with_no_model() {
    // The exact scenario release-sopack.yml's "Doctor --quick (no model)"
    // step relies on: must succeed with no model downloaded and, on that
    // job, no ONNX Runtime library present either.
    let r = run(&["doctor", "--quick", "--json"]);
    // stdout must carry exactly one JSON document — any progress output
    // (this subprocess's stderr is not a TTY, so `--progress auto` renders
    // plain throttled lines there, not JSON) stays off stdout entirely.
    let v = r.stdout_json();
    assert_valid("doctor", &v);
}

#[test]
fn contract_show_json_matches_schema() {
    let r = run(&["contract", "show", "--json"]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_valid("contract-show", &r.stdout_json());
}

#[test]
fn contract_list_json_matches_schema() {
    let r = run(&["contract", "list", "--json"]);
    assert_eq!(r.status, 0);
    let v = r.stdout_json();
    assert_valid("contract-list", &v);
    assert_eq!(v["default"], "e5-large-v1");
}

#[test]
fn schema_list_json_matches_schema_and_every_name_resolves() {
    let r = run(&["schema", "--list", "--json"]);
    assert_eq!(r.status, 0);
    let v = r.stdout_json();
    assert_valid("schema-list", &v);
    let names: Vec<String> = v["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"book".to_string()));
    assert!(names.contains(&"error".to_string()));
    assert!(!names.contains(&"propose".to_string()));
    for name in &names {
        let r = run(&["schema", name]);
        assert_eq!(r.status, 0, "sopack schema {name} failed");
        let _: serde_json::Value = serde_json::from_str(&r.stdout)
            .unwrap_or_else(|e| panic!("sopack schema {name} did not print valid JSON: {e}"));
    }
}

#[test]
fn unknown_schema_name_is_a_usage_error() {
    let r = run(&["schema", "no-such-schema"]);
    assert_eq!(r.status, 2);
    assert!(r.stdout.is_empty());
    assert!(r.stderr.contains("no-such-schema"));
}

#[test]
fn commands_json_matches_schema() {
    let r = run(&["commands", "--json"]);
    assert_eq!(r.status, 0);
    let v = r.stdout_json();
    assert_valid("commands", &v);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["exit_codes"].as_array().unwrap().len(), 8);
}

#[test]
fn commands_json_covers_every_documented_subcommand() {
    let r = run(&["commands", "--json"]);
    let v = r.stdout_json();
    let names: Vec<String> = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "extract",
        "inspect",
        "pack",
        "calibrate",
        "verify",
        "doctor",
        "model fetch",
        "model import",
        "model verify",
        "model path",
        "schema",
        "commands",
        "contract show",
        "contract list",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected} in {names:?}"
        );
    }
    // Every leaf command has a result_schema and it resolves.
    for c in v["commands"].as_array().unwrap() {
        if let Some(schema_name) = c["result_schema"].as_str() {
            let sr = run(&["schema", schema_name]);
            assert_eq!(
                sr.status, 0,
                "{}'s result_schema {schema_name:?} does not exist",
                c["name"]
            );
        }
    }
}

#[test]
fn no_arguments_is_a_usage_error() {
    let r = run(&[]);
    assert_eq!(r.status, 2);
}
