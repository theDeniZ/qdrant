//! Shared test support: run the built `sopack` binary as a subprocess, and a
//! small, dependency-free structural JSON Schema checker (`SOPACK-1.0-PLAN.md`
//! §3.5: "validated against a committed JSON Schema… otherwise structural
//! checks" — the `jsonschema` crate was left out of this crate's
//! dev-dependencies to keep the workspace's disk/network footprint down, per
//! `common.md`'s shared-disk note; this covers `type`, `required`,
//! `properties`, `enum`, `const`, `$ref` to `#/$defs/*`, `items` and `oneOf`,
//! which is everything every schema under `qdrant/sopack-rs/schemas/`
//! actually uses).

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sopack"))
}

pub struct Run {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn stdout_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not valid JSON: {e}\nstdout was:\n{}",
                self.stdout
            )
        })
    }
}

pub fn run(args: &[&str]) -> Run {
    let out: Output = Command::new(bin())
        .args(args)
        .output()
        .expect("failed to run the sopack binary");
    Run {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

pub fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

pub fn load_schema(name: &str) -> serde_json::Value {
    let path = schemas_dir().join(format!("{name}.v1.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read schema {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{}: not valid JSON: {e}", path.display()))
}

/// Structural validation: every problem found, empty means *value* satisfies
/// *schema* well enough for this checker's supported keywords (see the
/// module doc for the subset).
pub fn validate(schema: &serde_json::Value, value: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", &mut errors);
    errors
}

pub fn assert_valid(schema_name: &str, value: &serde_json::Value) {
    let schema = load_schema(schema_name);
    let errors = validate(&schema, value);
    assert!(
        errors.is_empty(),
        "{schema_name}: value does not match schema:\n{}\nvalue was:\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(value).unwrap()
    );
}

fn resolve_ref<'a>(root: &'a serde_json::Value, r#ref: &str) -> &'a serde_json::Value {
    let path = r#ref
        .strip_prefix("#/")
        .unwrap_or_else(|| panic!("only local #/... $refs are supported, got {ref}"));
    let mut cur = root;
    for part in path.split('/') {
        cur = cur
            .get(part)
            .unwrap_or_else(|| panic!("$ref {ref} does not resolve (missing {part})"));
    }
    cur
}

fn type_matches(ty: &str, value: &serde_json::Value) -> bool {
    match ty {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        other => panic!("unsupported schema type {other:?}"),
    }
}

fn check(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let schema = if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        resolve_ref(root, r)
    } else {
        schema
    };

    if let Some(one_of) = schema.get("oneOf").and_then(|v| v.as_array()) {
        let mut sub_errors_by_variant = Vec::new();
        let mut matched = false;
        for variant in one_of {
            let mut sub = Vec::new();
            check(root, variant, value, path, &mut sub);
            if sub.is_empty() {
                matched = true;
                break;
            }
            sub_errors_by_variant.push(sub);
        }
        if !matched {
            errors.push(format!(
                "{path}: matched none of {} oneOf variants (first variant's errors: {:?})",
                one_of.len(),
                sub_errors_by_variant.first()
            ));
        }
        return;
    }

    if let Some(c) = schema.get("const") {
        if value != c {
            errors.push(format!("{path}: expected const {c}, got {value}"));
        }
    }

    if let Some(en) = schema.get("enum").and_then(|v| v.as_array()) {
        if !en.contains(value) {
            errors.push(format!("{path}: {value} is not one of {en:?}"));
        }
    }

    match schema.get("type") {
        Some(serde_json::Value::String(ty)) => {
            if !type_matches(ty, value) {
                errors.push(format!("{path}: expected type {ty}, got {value}"));
            }
        }
        Some(serde_json::Value::Array(types)) => {
            let ok = types
                .iter()
                .filter_map(|t| t.as_str())
                .any(|t| type_matches(t, value));
            if !ok {
                errors.push(format!(
                    "{path}: expected one of types {types:?}, got {value}"
                ));
            }
        }
        _ => {}
    }

    if let Some(obj) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for r in required {
                let key = r.as_str().unwrap();
                if !obj.contains_key(key) {
                    errors.push(format!("{path}: missing required key {key:?}"));
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, sub_schema) in props {
                if let Some(v) = obj.get(key) {
                    check(root, sub_schema, v, &format!("{path}.{key}"), errors);
                }
            }
        }
    }

    if let Some(arr) = value.as_array() {
        if let Some(items_schema) = schema.get("items") {
            for (i, item) in arr.iter().enumerate() {
                check(root, items_schema, item, &format!("{path}[{i}]"), errors);
            }
        }
    }
}
