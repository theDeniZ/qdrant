//! The measurement a stage's `total`/`done` counters are in
//! (SOPACK-1.0-PLAN.md §3.4's per-command table: bytes, items, blocks,
//! tokens, checks).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Bytes,
    Items,
    Blocks,
    Tokens,
    Checks,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Bytes => "bytes",
            Unit::Items => "items",
            Unit::Blocks => "blocks",
            Unit::Tokens => "tokens",
            Unit::Checks => "checks",
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_schema_vocabulary() {
        assert_eq!(Unit::Bytes.as_str(), "bytes");
        assert_eq!(Unit::Items.as_str(), "items");
        assert_eq!(Unit::Blocks.as_str(), "blocks");
        assert_eq!(Unit::Tokens.as_str(), "tokens");
        assert_eq!(Unit::Checks.as_str(), "checks");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(Unit::Tokens.to_string(), "tokens");
    }
}
