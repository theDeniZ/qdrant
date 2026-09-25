//! `BookError` — the single error type `sopack-book` raises, matching
//! `sopack.book.BookError` in the Python reference: a book.json is
//! malformed, structurally inconsistent, or refers to an unknown profile /
//! id_rule.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookError(pub String);

impl BookError {
    pub fn new(msg: impl Into<String>) -> Self {
        BookError(msg.into())
    }
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BookError {}

impl From<String> for BookError {
    fn from(s: String) -> Self {
        BookError(s)
    }
}

impl From<&str> for BookError {
    fn from(s: &str) -> Self {
        BookError(s.to_string())
    }
}
