//! `Block` / `Book` — the `sopack.book/1` in-memory model. Mirrors
//! `sopack.book.Block` / `sopack.book.Book` field-for-field.

use serde_json::{Map, Value};

/// One emitted point.
///
/// `seq` and `chunk` are **not** the same number, and conflating them was
/// the bug fixed in Python 0.1.4. `seq` disambiguates every block sharing a
/// `para_key` — it is what `sop/seq` hashes into the point id — and runs
/// 0,1,2… across the whole `para_key`, including across two *different*
/// source paragraphs that happen to key the same (an EPUB with an inline
/// citation scheme keys cited blocks from the citation and uncited ones —
/// headings, mostly — from a chapter ordinal, and the two collide). `chunk`
/// is the piece's index *within its own paragraph*, 0…`chunks`-1, and is
/// what the payload carries. They are equal only while a `para_key` holds
/// one paragraph.
///
/// `chunk` defaults to `seq` when not given explicitly — exactly right for
/// every book.json written before 0.1.4, since a book with a collision
/// could not be written at all under the old rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub para_key: String,
    pub page: i64,
    pub para: i64,
    pub seq: i64,
    pub chunks: i64,
    pub text: String,
    pub words: i64,
    pub chunk: i64,
}

impl Block {
    /// Mirrors the dataclass constructor + `__post_init__`: `chunk: None`
    /// resolves to `seq`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        para_key: impl Into<String>,
        page: i64,
        para: i64,
        seq: i64,
        chunks: i64,
        text: impl Into<String>,
        words: i64,
        chunk: Option<i64>,
    ) -> Self {
        let seq_v = seq;
        Block {
            para_key: para_key.into(),
            page,
            para,
            seq,
            chunks,
            text: text.into(),
            words,
            chunk: chunk.unwrap_or(seq_v),
        }
    }
}

/// The reviewable intermediate between a source and a `.sopack`.
///
/// `source`, `book`, `stats` and `alignment` are kept as raw ordered JSON
/// maps (not fixed structs) exactly like the Python dataclass keeps plain
/// `dict`s: an extractor builds them as literal dicts in a fixed key order,
/// and `load()` passes an existing book.json's `source`/`book`/`stats`
/// objects through untouched — including their on-disk key order, which
/// `dump()` must reproduce byte-for-byte for a round trip. Semantic
/// completeness (required metadata, etc.) is `validate()`'s job, not this
/// struct's.
#[derive(Debug, Clone)]
pub struct Book {
    pub schema: String,
    pub profile: String,
    pub source: Map<String, Value>,
    pub book: Map<String, Value>,
    pub id_rule: String,
    pub alignment: Option<Map<String, Value>>,
    pub stats: Map<String, Value>,
    pub blocks: Vec<Block>,
}

impl Book {
    /// A `book.book[key]` string, or `None` if absent, `null`, or not a
    /// string. Mirrors `meta.get(key)` used by `to_payload`/`uid` on a
    /// schema field that is always `[string, null]`.
    pub fn meta_str(&self, key: &str) -> Option<String> {
        match self.book.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }
}
