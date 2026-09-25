//! `to_payload` / `uid` — the profile-aware Qdrant payload for one block,
//! and the pre-hash uid string for its point id. Mirrors
//! `sopack.book.to_payload` / `sopack.book.uid`.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::contract_bridge;
use crate::error::BookError;
use crate::model::{Block, Book};

/// The paragraph key(s) *para_key* aligns to in the other language.
///
/// `alignment["en_reverse"]` is always keyed by **English** para_key,
/// mapping to a list of the other language's para_keys (the shape
/// `sop_json` files carry: `en_reverse["7.1"] == ["5.1"]` means EN 7.1
/// aligns to DE 5.1). So a book whose own language *is* English looks it up
/// directly; any other language inverts it once to go from its own
/// para_key back to the aligned EN para_key(s).
fn aligned_for(book: &Book, para_key: &str) -> Option<Vec<String>> {
    let alignment = book.alignment.as_ref()?;
    let rev = match alignment.get("en_reverse") {
        Some(Value::Object(m)) if !m.is_empty() => m,
        _ => return None,
    };
    let lang = book.meta_str("lang");
    if lang.as_deref() == Some("en") {
        return match rev.get(para_key) {
            Some(Value::Array(a)) if !a.is_empty() => Some(
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
            ),
            _ => None,
        };
    }
    let mut hits = Vec::new();
    for (en_pk, own_pks) in rev.iter() {
        let contains = match own_pks {
            Value::Array(a) => a.iter().any(|v| v.as_str() == Some(para_key)),
            _ => false,
        };
        if contains {
            hits.push(en_pk.clone());
        }
    }
    if hits.is_empty() {
        None
    } else {
        Some(hits)
    }
}

/// The profile-aware Qdrant payload for one block of *book*.
///
/// Produces exactly the payload keys `contract_bridge::validate_payload`
/// accepts for the book's profile — required keys are always present (even
/// as `null`, which is legitimate for `aligned`/`book_pair`), optional keys
/// are included only when known.
pub fn to_payload(book: &Book, block: &Block) -> Result<Map<String, Value>, BookError> {
    let profile = contract_bridge::get_profile(&book.profile).map_err(BookError::new)?;

    if profile.name == "sop" {
        let mut payload = Map::new();
        payload.insert(
            "lang".into(),
            book.meta_str("lang")
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "book_code".into(),
            book.meta_str("book_code")
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "book_pair".into(),
            book.meta_str("book_pair")
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        payload.insert("page".into(), Value::Number(block.page.into()));
        payload.insert("para".into(), Value::Number(block.para.into()));
        payload.insert("para_key".into(), Value::String(block.para_key.clone()));
        payload.insert("raw_text".into(), Value::String(block.text.clone()));
        payload.insert(
            "aligned".into(),
            match aligned_for(book, &block.para_key) {
                Some(v) => Value::Array(v.into_iter().map(Value::String).collect()),
                None => Value::Null,
            },
        );
        for key in ["corpus", "author", "title", "year", "slug", "page_kind"] {
            if let Some(v) = book.book.get(key) {
                if !v.is_null() {
                    payload.insert(key.to_string(), v.clone());
                }
            }
        }
        if block.chunks > 1 {
            payload.insert("chunk".into(), Value::Number(block.chunk.into()));
            payload.insert("chunks".into(), Value::Number(block.chunks.into()));
        }
        return Ok(payload);
    }

    if profile.name == "bible" {
        let mut payload = Map::new();
        let bible = book
            .meta_str("book_code")
            .filter(|s| !s.is_empty())
            .or_else(|| book.meta_str("bible"));
        payload.insert(
            "bible".into(),
            bible.map(Value::String).unwrap_or(Value::Null),
        );
        payload.insert("osis".into(), Value::String(block.para_key.clone()));
        payload.insert("text".into(), Value::String(block.text.clone()));
        for key in ["canonical_osis", "versification_offset"] {
            if let Some(v) = book.book.get(key) {
                if !v.is_null() {
                    payload.insert(key.to_string(), v.clone());
                }
            }
        }
        return Ok(payload);
    }

    Err(BookError::new(format!(
        "to_payload: no payload builder for profile {:?}",
        profile.name
    )))
}

/// The pre-hash uid string for *block*, agreeing with
/// `contract_bridge::uid_for(book.id_rule, fields)`.
pub fn uid(book: &Book, block: &Block) -> Result<String, BookError> {
    let profile = contract_bridge::get_profile(&book.profile).map_err(BookError::new)?;
    let mut fields: HashMap<&str, Option<String>> = HashMap::new();
    if profile.name == "bible" {
        let bible = book
            .meta_str("book_code")
            .filter(|s| !s.is_empty())
            .or_else(|| book.meta_str("bible"));
        fields.insert("bible", bible);
        fields.insert("osis", Some(block.para_key.clone()));
    } else {
        fields.insert("lang", book.meta_str("lang"));
        fields.insert("book_code", book.meta_str("book_code"));
        fields.insert("para_key", Some(block.para_key.clone()));
        fields.insert("seq", Some(block.seq.to_string()));
    }
    contract_bridge::uid_for(&book.id_rule, &fields)
}
