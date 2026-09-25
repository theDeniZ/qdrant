//! Shared between [`crate::writer`] and [`crate::reader`]: derive the
//! id-rule input fields from a payload plus a uid's `#<seq>` suffix —
//! `sopack.format._fields` in the Python reference.

use serde_json::{Map, Value};

/// `"#" in uid` in Python means "anywhere in the string", not "at the end",
/// so this mirrors `uid.rsplit("#", 1)` with [`str::rsplit_once`] (which
/// splits on the *last* `#`, matching `rsplit`).
pub(crate) fn fields_from(payload: &Map<String, Value>, uid: &str) -> Map<String, Value> {
    let mut fields = payload.clone();
    if let Some((_, tail)) = uid.rsplit_once('#') {
        if let Ok(seq) = tail.parse::<i64>() {
            fields.insert("seq".to_string(), Value::from(seq));
        }
        // else: matches Python's `except ValueError: pass` — leave fields as-is.
    } else {
        fields.entry("seq".to_string()).or_insert(Value::from(0));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_seq_from_hash_suffix() {
        let payload = json!({"lang": "en"}).as_object().unwrap().clone();
        let f = fields_from(&payload, "en:TT:1.1#3");
        assert_eq!(f.get("seq"), Some(&json!(3)));
    }

    #[test]
    fn defaults_seq_to_zero_without_hash() {
        let payload = json!({"lang": "en"}).as_object().unwrap().clone();
        let f = fields_from(&payload, "en:TT:1.1");
        assert_eq!(f.get("seq"), Some(&json!(0)));
    }

    #[test]
    fn non_numeric_suffix_leaves_fields_unchanged() {
        let payload = json!({"lang": "en"}).as_object().unwrap().clone();
        let f = fields_from(&payload, "en:TT:1.1#abc");
        assert_eq!(f.get("seq"), None);
    }

    #[test]
    fn payload_seq_kept_when_suffix_is_not_numeric() {
        let payload = json!({"lang": "en", "seq": 9}).as_object().unwrap().clone();
        let f = fields_from(&payload, "en:TT:1.1#abc");
        assert_eq!(f.get("seq"), Some(&json!(9)));
    }
}
