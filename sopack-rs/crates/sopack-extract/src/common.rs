//! Small helpers shared by every extractor.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::ExtractError;

pub fn sha256_file(path: &Path) -> Result<String, ExtractError> {
    let bytes = std::fs::read(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Python `text[:80]` truncates by *character* (Unicode codepoint), not
/// byte — every extractor's dropped-detail preview text does this.
pub fn char_truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// A `stats["dropped_detail"]` entry: `{"para_key", "reason", "text"?}`.
/// `text` is omitted entirely (not written as `null`) when absent, same as
/// the Python dicts that only include the key when they have one.
pub fn dropped_entry(para_key: &str, reason: &str, text: Option<&str>) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert(
        "para_key".into(),
        serde_json::Value::String(para_key.to_string()),
    );
    m.insert(
        "reason".into(),
        serde_json::Value::String(reason.to_string()),
    );
    if let Some(t) = text {
        m.insert("text".into(), serde_json::Value::String(t.to_string()));
    }
    serde_json::Value::Object(m)
}

/// `stats` dict, in the fixed key order every extractor writes it in
/// (`blocks_in`, `blocks_out`, `dropped`, `damage`, `words`,
/// `dropped_detail`).
#[allow(clippy::too_many_arguments)]
pub fn build_stats(
    blocks_in: i64,
    blocks_out: i64,
    dropped: i64,
    damage: f64,
    words: i64,
    dropped_detail: Vec<serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("blocks_in".into(), serde_json::Value::from(blocks_in));
    m.insert("blocks_out".into(), serde_json::Value::from(blocks_out));
    m.insert("dropped".into(), serde_json::Value::from(dropped));
    m.insert("damage".into(), serde_json::Value::from(damage));
    m.insert("words".into(), serde_json::Value::from(words));
    m.insert(
        "dropped_detail".into(),
        serde_json::Value::Array(dropped_detail),
    );
    m
}

/// `source` dict, in the fixed key order every extractor writes it in
/// (`file`, `sha256`, `kind`, `acquired_from`, `rights`).
pub fn build_source(
    file: &str,
    sha256: Option<&str>,
    kind: &str,
    acquired_from: Option<&str>,
    rights: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("file".into(), serde_json::Value::String(file.to_string()));
    m.insert(
        "sha256".into(),
        sha256
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    m.insert("kind".into(), serde_json::Value::String(kind.to_string()));
    m.insert(
        "acquired_from".into(),
        acquired_from
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    m.insert(
        "rights".into(),
        rights
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    m
}

/// `book` dict, in the fixed key order every extractor writes it in
/// (`book_code`, `lang`, `book_pair`, `title`, `author`, `year`, `slug`,
/// `corpus`, `page_kind`).
#[allow(clippy::too_many_arguments)]
pub fn build_book_meta(
    book_code: Option<&str>,
    lang: Option<&str>,
    book_pair: Option<&str>,
    title: Option<&str>,
    author: Option<&str>,
    year: Option<i64>,
    slug: Option<&str>,
    corpus: Option<&str>,
    page_kind: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::Value;
    let mut m = serde_json::Map::new();
    m.insert(
        "book_code".into(),
        book_code
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "lang".into(),
        lang.map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "book_pair".into(),
        book_pair
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "title".into(),
        title
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "author".into(),
        author
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert("year".into(), year.map(Value::from).unwrap_or(Value::Null));
    m.insert(
        "slug".into(),
        slug.map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "corpus".into(),
        corpus
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "page_kind".into(),
        page_kind
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    );
    m
}
