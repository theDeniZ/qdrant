//! `validate_payload` — semantic checks on one point's payload against its
//! profile, mirroring `sopack.contract.validate_payload` exactly (including
//! the CPython `str.strip()` whitespace set — see [`crate::pystr`]).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::contract::Profile;
use crate::pystr::python_strip;

/// Every problem with *payload* under *profile*. Empty means well formed.
///
/// - Every `profile.required` key must be **present** (its value may
///   legitimately be `null` — Python: `key not in payload`, not a truthiness
///   check).
/// - No key outside `required ∪ optional` may appear — an unknown key is
///   usually a typo that would otherwise be silently written and never read.
/// - `payload[profile.text_field]` must be a JSON string that is non-empty
///   after [`python_strip`].
pub fn validate_payload(profile: &Profile, payload: &Map<String, Value>) -> Vec<String> {
    let mut errors = Vec::new();

    for key in &profile.required {
        if !payload.contains_key(key) {
            errors.push(format!("missing required payload key {key:?}"));
        }
    }

    let known: BTreeSet<&str> = profile
        .required
        .iter()
        .chain(profile.optional.iter())
        .map(String::as_str)
        .collect();
    for key in payload.keys() {
        if !known.contains(key.as_str()) {
            errors.push(format!("unknown payload key {key:?}"));
        }
    }

    let text_ok = match payload.get(&profile.text_field) {
        Some(Value::String(s)) => !python_strip(s).is_empty(),
        _ => false,
    };
    if !text_ok {
        errors.push(format!("{:?} is empty", profile.text_field));
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Contract;
    use serde_json::json;

    fn sop_profile() -> Profile {
        Contract::embedded("e5-large-v1").unwrap().profiles["sop"].clone()
    }

    fn valid_payload() -> Map<String, Value> {
        json!({
            "lang": "en", "book_code": "WDYS", "book_pair": "WDYS", "page": 1,
            "para": 1, "para_key": "1.1", "raw_text": "hello", "aligned": null
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn accepts_a_well_formed_payload() {
        let profile = sop_profile();
        assert_eq!(
            validate_payload(&profile, &valid_payload()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn required_key_present_with_null_value_is_fine() {
        // `aligned` is required but legitimately null.
        let profile = sop_profile();
        let payload = valid_payload();
        assert!(payload.get("aligned").unwrap().is_null());
        assert_eq!(validate_payload(&profile, &payload), Vec::<String>::new());
    }

    #[test]
    fn missing_required_key_is_reported() {
        let profile = sop_profile();
        let mut payload = valid_payload();
        payload.remove("aligned");
        let errors = validate_payload(&profile, &payload);
        assert!(errors.iter().any(|e| e.contains("aligned")), "{errors:?}");
    }

    #[test]
    fn unknown_key_is_reported() {
        let profile = sop_profile();
        let mut payload = valid_payload();
        payload.insert("typo_field".to_string(), json!("x"));
        let errors = validate_payload(&profile, &payload);
        assert!(
            errors.iter().any(|e| e.contains("typo_field")),
            "{errors:?}"
        );
    }

    #[test]
    fn whitespace_only_text_is_empty() {
        let profile = sop_profile();
        let mut payload = valid_payload();
        payload.insert("raw_text".to_string(), json!("\u{00A0}  \u{2028}"));
        let errors = validate_payload(&profile, &payload);
        assert!(errors.iter().any(|e| e.contains("raw_text")), "{errors:?}");
    }

    #[test]
    fn non_string_text_field_is_empty() {
        let profile = sop_profile();
        let mut payload = valid_payload();
        payload.insert("raw_text".to_string(), json!(42));
        let errors = validate_payload(&profile, &payload);
        assert!(errors.iter().any(|e| e.contains("raw_text")), "{errors:?}");
    }
}
