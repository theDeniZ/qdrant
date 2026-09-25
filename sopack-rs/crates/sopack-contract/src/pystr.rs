//! A byte-for-byte reproduction of CPython's `str.isspace()` / `str.strip()`
//! character set.
//!
//! `sopack.contract.validate_payload` (Python) rejects a text field that is
//! empty *after* `str.strip()`. Rust's `char::is_whitespace()` follows the
//! Unicode `White_Space` property, which is close to but not identical to
//! what CPython's `Py_UNICODE_ISSPACE` accepts — notably CPython also treats
//! the C1 control characters U+001C–U+001F (file/group/record/unit
//! separator) as whitespace (their bidirectional category is `B`/`S`, which
//! CPython folds into `isspace()`), while Unicode's `White_Space` property
//! does not. Getting this wrong means the Rust and Python sides could accept
//! or reject the same payload differently, so the exact CPython table is
//! reproduced here instead of delegating to `char::is_whitespace()`.
//!
//! Table (from CPython's `Tools/unicode/makeunicodedata.py` /
//! `Objects/unicodetype_db.h`, "additional category" whitespace ∪ Unicode
//! `Zs`/`Zl`/`Zp`): the ASCII/Latin-1 control-and-space set plus the modern
//! Unicode space separators.
const PY_WHITESPACE: &[char] = &[
    '\u{0009}', '\u{000A}', '\u{000B}', '\u{000C}', '\u{000D}', // \t \n \v \f \r
    '\u{001C}', '\u{001D}', '\u{001E}', '\u{001F}', // FS GS RS US
    '\u{0020}', // space
    '\u{0085}', // NEL
    '\u{00A0}', // no-break space
    '\u{1680}', // ogham space mark
    '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
    '\u{2008}', '\u{2009}', '\u{200A}', // en/em spaces etc.
    '\u{2028}', // line separator
    '\u{2029}', // paragraph separator
    '\u{202F}', // narrow no-break space
    '\u{205F}', // medium mathematical space
    '\u{3000}', // ideographic space
];

/// `True` exactly where CPython's `str.isspace()` / `str.strip()` would
/// treat *c* as whitespace.
pub fn is_python_space(c: char) -> bool {
    PY_WHITESPACE.contains(&c)
}

/// Equivalent to Python's `s.strip()` with no arguments.
pub fn python_strip(s: &str) -> &str {
    s.trim_matches(is_python_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ascii_space_and_common_whitespace() {
        assert_eq!(python_strip("  hi \t\n"), "hi");
        assert_eq!(python_strip(""), "");
        assert_eq!(python_strip("   "), "");
    }

    #[test]
    fn strips_nbsp_and_separators_python_treats_as_space() {
        assert_eq!(python_strip("\u{00A0}hi\u{00A0}"), "hi");
        assert_eq!(python_strip("\u{2028}hi\u{2029}"), "hi");
        assert_eq!(python_strip("\u{001C}hi\u{001F}"), "hi");
    }

    #[test]
    fn does_not_strip_interior_whitespace() {
        assert_eq!(python_strip("  hi there  "), "hi there");
    }

    #[test]
    fn non_breaking_space_only_string_is_empty_after_strip() {
        assert!(python_strip("\u{00A0}\u{00A0}").is_empty());
    }
}
