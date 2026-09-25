//! Every committed JSON Schema, embedded with `include_str!`
//! (`SOPACK-1.0-PLAN.md` §3.5: "`--json` on every command… validated
//! against a committed JSON Schema (`sopack schema <name>` prints any of
//! them)"). `qdrant/sopack-rs/schemas/<name>.v1.json`, one per command
//! result, plus `error`, `progress-event`, `propose`, and `book`
//! (`sopack_book::SCHEMA_JSON`, which lives at
//! `qdrant/sopack/schemas/book.schema.json` — the one schema not under this
//! crate's own `schemas/` dir, since `sopack-book` already embeds and owns
//! it).

pub struct SchemaEntry {
    pub name: &'static str,
    pub text: &'static str,
}

macro_rules! schema {
    ($name:literal, $path:literal) => {
        SchemaEntry {
            name: $name,
            text: include_str!(concat!("../../../schemas/", $path)),
        }
    };
}

pub const SCHEMAS: &[SchemaEntry] = &[
    schema!("error", "error.v1.json"),
    schema!("progress-event", "progress-event.v1.json"),
    schema!("propose", "propose.v1.json"),
    schema!("extract", "extract.v1.json"),
    schema!("inspect", "inspect.v1.json"),
    schema!("pack", "pack.v1.json"),
    schema!("calibrate", "calibrate.v1.json"),
    schema!("verify", "verify.v1.json"),
    schema!("doctor", "doctor.v1.json"),
    schema!("model", "model.v1.json"),
    schema!("model-path", "model-path.v1.json"),
    schema!("commands", "commands.v1.json"),
    schema!("contract-show", "contract-show.v1.json"),
    schema!("contract-list", "contract-list.v1.json"),
    schema!("schema-list", "schema-list.v1.json"),
];

/// Every schema name `sopack schema <name>` accepts, including `book` (the
/// one whose text comes from `sopack_book::SCHEMA_JSON` rather than this
/// crate's own `schemas/` dir).
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = SCHEMAS.iter().map(|s| s.name).collect();
    v.push("book");
    v
}

pub fn get(name: &str) -> Option<&'static str> {
    if name == "book" {
        return Some(sopack_book::SCHEMA_JSON);
    }
    SCHEMAS.iter().find(|s| s.name == name).map(|s| s.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_valid_json() {
        for entry in SCHEMAS {
            let v: serde_json::Value = serde_json::from_str(entry.text)
                .unwrap_or_else(|e| panic!("{}: not valid JSON: {e}", entry.name));
            assert!(v.is_object(), "{}: top level must be an object", entry.name);
        }
        let _: serde_json::Value = serde_json::from_str(sopack_book::SCHEMA_JSON).unwrap();
    }

    #[test]
    fn get_resolves_every_declared_name_including_book() {
        for name in names() {
            assert!(get(name).is_some(), "{name} did not resolve");
        }
        assert!(get("no-such-schema").is_none());
    }
}
