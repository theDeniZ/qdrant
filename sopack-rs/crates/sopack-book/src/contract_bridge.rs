//! Bridge from sopack-book's small, stable surface (`get_profile`,
//! `uid_for`, `point_id`, `validate_payload`, `Profile`, `SCHEMA_BOOK`) onto
//! `sopack-contract`, the real owner of profile/id-rule data
//! (`contracts/e5-large-v1/contract.toml`). Replaces the former
//! `contract_lite` duplicate now that `sopack-contract` has landed — see
//! that module's old doc comment (still true of the intent, just no longer
//! of the implementation): the three function names below are kept
//! identical on purpose so every call site in this crate needed no
//! semantic change, only this file.
//!
//! Loads the one contract this crate has ever needed — the embedded
//! `e5-large-v1` contract, the same one every `book.json` in this workspace
//! is written against — once per process, so every call after the first is
//! free.

use std::collections::HashMap;
use std::sync::OnceLock;

pub use sopack_contract::Profile;

use crate::error::BookError;

/// `sopack.contract.SCHEMA_BOOK`.
pub const SCHEMA_BOOK: &str = "sopack.book/1";

fn contract() -> &'static sopack_contract::Contract {
    static CONTRACT: OnceLock<sopack_contract::Contract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        sopack_contract::Contract::embedded("e5-large-v1")
            .expect("the embedded e5-large-v1 contract must load")
    })
}

/// `sopack.contract.get_profile`.
pub fn get_profile(name: &str) -> Result<&'static Profile, String> {
    contract().get_profile(name).map_err(|e| e.to_string())
}

/// The pre-hash uid string for *rule*, agreeing with
/// `sopack.contract.uid_for` / `ID_RULES`.
///
/// `fields` follows the same `dict.get` semantics the old `contract_lite`
/// documented: a key that is *absent* is rendered the same as a key present
/// with value `None` — both become the literal text `"None"` (an f-string
/// embedding Python's `None`). Every field the named rule's template
/// references is populated this way before delegating to
/// `sopack_contract::IdRule::build_uid`, which otherwise requires every
/// referenced field to be genuinely present (no defaulting — see that
/// module's docs); pre-populating here is what preserves the old
/// "never errors on a missing/None field" behavior.
pub fn uid_for(rule: &str, fields: &HashMap<&str, Option<String>>) -> Result<String, BookError> {
    let id_rule = contract()
        .get_id_rule(rule)
        .map_err(|e| BookError::new(e.to_string()))?;
    let mut map = serde_json::Map::new();
    for name in id_rule.fields() {
        let value = match fields.get(name) {
            Some(Some(v)) => v.clone(),
            Some(None) | None => "None".to_string(),
        };
        map.insert(name.to_string(), serde_json::Value::String(value));
    }
    id_rule
        .build_uid(&map)
        .map_err(|e| BookError::new(e.to_string()))
}

/// `sopack.contract.point_id` — `uuid5(NAMESPACE_DNS, uid_for(rule, fields))`.
pub fn point_id(rule: &str, fields: &HashMap<&str, Option<String>>) -> Result<String, BookError> {
    let uid = uid_for(rule, fields)?;
    Ok(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, uid.as_bytes()).to_string())
}

/// `sopack.contract.validate_payload` — problems with one point's payload.
pub fn validate_payload(
    profile: &Profile,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Vec<String> {
    sopack_contract::validate_payload(profile, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_profile_resolves_sop_and_bible() {
        assert_eq!(get_profile("sop").unwrap().name, "sop");
        assert_eq!(get_profile("bible").unwrap().name, "bible");
        assert!(get_profile("nope").is_err());
    }

    #[test]
    fn uid_for_sop_seq_matches_expected_shape() {
        let mut fields = HashMap::new();
        fields.insert("lang", Some("en".to_string()));
        fields.insert("book_code", Some("WDYS".to_string()));
        fields.insert("para_key", Some("1.1".to_string()));
        fields.insert("seq", Some("0".to_string()));
        assert_eq!(uid_for("sop/seq", &fields).unwrap(), "en:WDYS:1.1#0");
    }

    #[test]
    fn uid_for_renders_absent_and_none_fields_as_the_literal_none() {
        let mut fields = HashMap::new();
        fields.insert("lang", Some("en".to_string()));
        fields.insert("book_code", None);
        // para_key entirely absent from the map.
        assert_eq!(uid_for("sop/plain", &fields).unwrap(), "en:None:None");
    }

    #[test]
    fn uid_for_bible_matches_expected_shape() {
        let mut fields = HashMap::new();
        fields.insert("bible", Some("kjv".to_string()));
        fields.insert("osis", Some("Gen.1.1".to_string()));
        assert_eq!(uid_for("bible/v1", &fields).unwrap(), "bible:kjv:Gen.1.1");
    }

    #[test]
    fn point_id_is_a_stable_uuid5() {
        let mut fields = HashMap::new();
        fields.insert("lang", Some("en".to_string()));
        fields.insert("book_code", Some("WDYS".to_string()));
        fields.insert("para_key", Some("1.1".to_string()));
        fields.insert("seq", Some("0".to_string()));
        let id = point_id("sop/seq", &fields).unwrap();
        let expected = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"en:WDYS:1.1#0").to_string();
        assert_eq!(id, expected);
    }

    #[test]
    fn validate_payload_delegates_to_sopack_contract() {
        let profile = get_profile("sop").unwrap();
        let payload = json!({
            "lang": "en", "book_code": "WDYS", "book_pair": "WDYS", "page": 1,
            "para": 1, "para_key": "1.1", "raw_text": "hello", "aligned": null
        });
        assert_eq!(
            validate_payload(profile, payload.as_object().unwrap()),
            Vec::<String>::new()
        );
    }
}
