//! `load` / `dump` / `book_sha256` — reading and writing book.json.
//!
//! Mirrors `sopack.book.load` and `sopack.book.dump`: `load` raises on any
//! structurally bad input (not valid JSON, missing top-level keys, an
//! unknown profile, a malformed block) but never on merely incomplete
//! metadata — that is `validate`'s job, so a book.json with `null`
//! metadata still loads. `dump` writes a stable key order with `indent=2`
//! so it diffs cleanly in review, and is byte-identical to
//! `json.dump(doc, f, ensure_ascii=False, indent=2)` + a trailing newline.

use std::path::Path;

use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::contract_bridge;
use crate::error::BookError;
use crate::model::{Block, Book};

const REQUIRED_TOP: [&str; 7] = [
    "schema", "profile", "source", "book", "id_rule", "stats", "blocks",
];
const REQUIRED_BLOCK: [&str; 7] = ["para_key", "page", "para", "seq", "chunks", "text", "words"];

/// `str()` on a JSON value, matching CPython's `str(x)` for the handful of
/// types a book.json field can legitimately hold (string, number, bool,
/// null). Not a general Python `repr`/`str` implementation.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// `int()` on a JSON value, matching CPython's `int(x)` closely enough for
/// every field this module converts (`page`, `para`, `seq`, `chunks`,
/// `words`, `chunk`): accepts an integer, truncates a float toward zero,
/// coerces bool, and parses a base-10 integer string.
fn py_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if let Some(u) = n.as_u64() {
                i64::try_from(u).map_err(|_| "int too large".to_string())
            } else if let Some(f) = n.as_f64() {
                Ok(f.trunc() as i64)
            } else {
                Err("not a number".to_string())
            }
        }
        Value::Bool(true) => Ok(1),
        Value::Bool(false) => Ok(0),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("invalid literal for int() with base 10: {s:?}")),
        Value::Null => Err("int() argument must not be None".to_string()),
        other => Err(format!("int() argument must be a number, not {other:?}")),
    }
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Sha256 hex digest of a book.json file's raw bytes — what `pack.py`
/// records as a source book's `book_sha256` in the pack manifest (see
/// `docs/SOPACK-2-FORMAT.md`). Independent of parsing: hashes the file
/// exactly as it sits on disk.
pub fn book_sha256(path: impl AsRef<Path>) -> Result<String, BookError> {
    let path = path.as_ref();
    let raw = std::fs::read(path)
        .map_err(|exc| BookError::new(format!("cannot read {}: {exc}", path.display())))?;
    Ok(sha256_hex_bytes(&raw))
}

/// Parse *path* into a [`Book`], raising [`BookError`] on any structurally
/// bad input.
pub fn load(path: impl AsRef<Path>) -> Result<Book, BookError> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .map_err(|exc| BookError::new(format!("cannot read {}: {exc}", path.display())))?;
    let data: Value = serde_json::from_str(&raw)
        .map_err(|exc| BookError::new(format!("{}: not valid JSON: {exc}", path.display())))?;
    let data = match data {
        Value::Object(m) => m,
        _ => {
            return Err(BookError::new(format!(
                "{}: top level must be a JSON object",
                path.display()
            )))
        }
    };

    let missing: Vec<&str> = REQUIRED_TOP
        .iter()
        .filter(|k| !data.contains_key(**k))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(BookError::new(format!(
            "{}: missing required key(s): {}",
            path.display(),
            missing.join(", ")
        )));
    }

    let schema = match data.get("schema") {
        Some(Value::String(s)) => s.clone(),
        other => other.map(py_str).unwrap_or_default(),
    };
    if schema != contract_bridge::SCHEMA_BOOK {
        return Err(BookError::new(format!(
            "{}: unsupported schema {schema:?} (this module reads {:?})",
            path.display(),
            contract_bridge::SCHEMA_BOOK
        )));
    }

    let profile = match data.get("profile") {
        Some(Value::String(s)) => s.clone(),
        _ => {
            return Err(BookError::new(format!(
                "{}: 'profile' must be a string",
                path.display()
            )))
        }
    };
    contract_bridge::get_profile(&profile)
        .map_err(|exc| BookError::new(format!("{}: {exc}", path.display())))?;

    let source = match data.get("source") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(BookError::new(format!(
                "{}: 'source' must be an object",
                path.display()
            )))
        }
    };
    let book_meta = match data.get("book") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(BookError::new(format!(
                "{}: 'book' must be an object",
                path.display()
            )))
        }
    };
    let stats = match data.get("stats") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(BookError::new(format!(
                "{}: 'stats' must be an object",
                path.display()
            )))
        }
    };
    let id_rule = match data.get("id_rule") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => {
            return Err(BookError::new(format!(
                "{}: 'id_rule' must be a non-empty string",
                path.display()
            )))
        }
    };
    let alignment = match data.get("alignment") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => Some(m.clone()),
        Some(_) => {
            return Err(BookError::new(format!(
                "{}: 'alignment' must be an object or null",
                path.display()
            )))
        }
    };

    let raw_blocks = match data.get("blocks") {
        Some(Value::Array(a)) => a,
        _ => {
            return Err(BookError::new(format!(
                "{}: 'blocks' must be a list",
                path.display()
            )))
        }
    };

    let mut blocks = Vec::with_capacity(raw_blocks.len());
    for (i, b) in raw_blocks.iter().enumerate() {
        let obj = match b {
            Value::Object(m) => m,
            _ => {
                return Err(BookError::new(format!(
                    "{}: blocks[{i}] is not an object",
                    path.display()
                )))
            }
        };
        let missing_b: Vec<&str> = REQUIRED_BLOCK
            .iter()
            .filter(|k| !obj.contains_key(**k))
            .copied()
            .collect();
        if !missing_b.is_empty() {
            return Err(BookError::new(format!(
                "{}: blocks[{i}] missing key(s): {}",
                path.display(),
                missing_b.join(", ")
            )));
        }
        let field = |name: &str| obj.get(name).unwrap();
        let block_err = |exc: String| {
            BookError::new(format!(
                "{}: blocks[{i}] has a badly typed field: {exc}",
                path.display()
            ))
        };
        let para_key = py_str(field("para_key"));
        let page = py_int(field("page")).map_err(block_err)?;
        let para = py_int(field("para")).map_err(block_err)?;
        let seq = py_int(field("seq")).map_err(block_err)?;
        let chunks = py_int(field("chunks")).map_err(block_err)?;
        let text = py_str(field("text"));
        let words = py_int(field("words")).map_err(block_err)?;
        let chunk = match obj.get("chunk") {
            None | Some(Value::Null) => None,
            Some(v) => Some(py_int(v).map_err(block_err)?),
        };
        blocks.push(Block::new(
            para_key, page, para, seq, chunks, text, words, chunk,
        ));
    }

    Ok(Book {
        schema,
        profile,
        source,
        book: book_meta,
        id_rule,
        alignment,
        stats,
        blocks,
    })
}

fn block_to_value(b: &Block) -> Value {
    let mut m = Map::new();
    m.insert("para_key".into(), Value::String(b.para_key.clone()));
    m.insert("page".into(), Value::Number(Number::from(b.page)));
    m.insert("para".into(), Value::Number(Number::from(b.para)));
    m.insert("seq".into(), Value::Number(Number::from(b.seq)));
    m.insert("chunk".into(), Value::Number(Number::from(b.chunk)));
    m.insert("chunks".into(), Value::Number(Number::from(b.chunks)));
    m.insert("text".into(), Value::String(b.text.clone()));
    m.insert("words".into(), Value::Number(Number::from(b.words)));
    Value::Object(m)
}

/// Write *book* to *path* as book.json with a stable key order and
/// `indent=2`, so it diffs cleanly in review. Byte-identical to
/// `json.dump(doc, f, ensure_ascii=False, indent=2)` followed by `"\n"`.
pub fn dump(book: &Book, path: impl AsRef<Path>) -> Result<(), BookError> {
    let mut doc = Map::new();
    doc.insert("schema".into(), Value::String(book.schema.clone()));
    doc.insert("profile".into(), Value::String(book.profile.clone()));
    doc.insert("source".into(), Value::Object(book.source.clone()));
    doc.insert("book".into(), Value::Object(book.book.clone()));
    doc.insert("id_rule".into(), Value::String(book.id_rule.clone()));
    doc.insert(
        "alignment".into(),
        match &book.alignment {
            Some(m) => Value::Object(m.clone()),
            None => Value::Null,
        },
    );
    doc.insert("stats".into(), Value::Object(book.stats.clone()));
    doc.insert(
        "blocks".into(),
        Value::Array(book.blocks.iter().map(block_to_value).collect()),
    );

    let mut text = serde_json::to_string_pretty(&Value::Object(doc))
        .map_err(|exc| BookError::new(format!("failed to serialise book.json: {exc}")))?;
    text.push('\n');
    std::fs::write(path.as_ref(), text)
        .map_err(|exc| BookError::new(format!("cannot write {}: {exc}", path.as_ref().display())))
}
