//! `inspect` — counts, damage, codes for a book.json. Library form of
//! `sopack.cli._cmd_inspect`'s report, returning a struct that can be
//! JSON-serialised (`serde::Serialize`) or rendered as the same human text
//! the Python CLI prints.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::model::Book;

/// The `inspect` report for one book.json. Field order matches the order
/// the Python CLI prints them in (see [`InspectReport::to_text`]).
#[derive(Debug, Clone, Serialize)]
pub struct InspectReport {
    pub file: String,
    pub profile: String,
    pub id_rule: String,
    pub blocks: usize,
    pub stats: Map<String, Value>,
    /// Total paragraphs (blocks with `chunk == 0`), not raw block count.
    pub paragraphs: usize,
    /// How many `para_key`s are shared by more than one paragraph — a
    /// legitimate key collision, worth seeing since it is why `seq` and
    /// `chunk` diverge for this book.
    pub collided_para_keys: usize,
    pub split_blocks: usize,
    pub book_code: Option<Value>,
    pub lang: Option<Value>,
    pub title: Option<Value>,
}

impl InspectReport {
    /// Build a report for *book*, whose original path is *file* (kept only
    /// for display — this function does not re-read the file).
    pub fn from_book(file: impl Into<String>, book: &Book) -> Self {
        // Paragraph counts by para_key: only the aggregate (total paragraphs,
        // how many para_keys collided) is needed, so insertion order does
        // not matter here — unlike `validate`'s grouping, nothing downstream
        // iterates this map.
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for b in &book.blocks {
            if b.chunk == 0 {
                *counts.entry(b.para_key.clone()).or_insert(0) += 1;
            }
        }
        let paragraphs: usize = counts.values().sum();
        let collided = counts.values().filter(|&&n| n > 1).count();
        let split_blocks = book.blocks.iter().filter(|b| b.chunks > 1).count();

        InspectReport {
            file: file.into(),
            profile: book.profile.clone(),
            id_rule: book.id_rule.clone(),
            blocks: book.blocks.len(),
            stats: book.stats.clone(),
            paragraphs,
            collided_para_keys: collided,
            split_blocks,
            book_code: book.book.get("book_code").cloned(),
            lang: book.book.get("lang").cloned(),
            title: book.book.get("title").cloned(),
        }
    }

    /// Human text matching `sopack.cli._cmd_inspect`'s stdout, line for
    /// line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("file: {}\n", self.file));
        out.push_str(&format!("profile: {}\n", self.profile));
        out.push_str(&format!("id_rule: {}\n", self.id_rule));
        out.push_str(&format!("blocks: {}\n", self.blocks));
        for (k, v) in self.stats.iter() {
            out.push_str(&format!("  {k}: {}\n", python_repr_value(v)));
        }
        out.push_str(&format!(
            "paragraphs: {} ({} para_key(s) shared by more than one)\n",
            self.paragraphs, self.collided_para_keys
        ));
        out.push_str(&format!("split blocks: {}\n", self.split_blocks));
        out.push_str(&format!(
            "book_code: {}  lang: {}  title: {}\n",
            python_str_opt(self.book_code.as_ref()),
            python_str_opt(self.lang.as_ref()),
            python_repr_value(self.title.as_ref().unwrap_or(&Value::Null)),
        ));
        out
    }
}

/// `str(x)` for the plain values this report ever holds (used where the
/// Python CLI f-strings a value without `!r}`).
fn python_str_opt(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => python_repr_value(other),
    }
}

/// `repr(x)` for a JSON value, close enough to CPython's `repr`/`str` for
/// the types a book.json ever holds (string, number, bool, null, list,
/// object) — this is a human-text convenience for `inspect`, not
/// conformance-gated.
fn python_repr_value(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(python_repr_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(o) => {
            let items: Vec<String> = o
                .iter()
                .map(|(k, v)| format!("'{}': {}", k.replace('\'', "\\'"), python_repr_value(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}
