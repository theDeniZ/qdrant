//! A JSON serializer that reproduces Python's `json.dumps(obj,
//! ensure_ascii=False)` byte-for-byte, for one thing only: `points.jsonl`
//! lines.
//!
//! Two default behaviours of `serde_json`'s compact writer differ from
//! Python's `json.dumps` defaults:
//!
//! 1. **Separators.** `json.dumps`'s default `(item_separator, key_separator)`
//!    is `(", ", ": ")` — a space after both the comma and the colon.
//!    `serde_json`'s `CompactFormatter` writes `(",", ":")` — no spaces.
//! 2. **Everything else already matches**, given `serde_json = { features =
//!    ["preserve_order"] }` (set at the workspace level): object key order
//!    follows `serde_json::Map`/struct insertion order exactly like a Python
//!    `dict`, non-ASCII text is written as raw UTF-8 (matching
//!    `ensure_ascii=False`), and both encoders escape only `"`, `\` and the
//!    C0 control characters (`\b \f \n \r \t` as short escapes, the rest as
//!    `\u00XX`) — see [`tests::matches_cpython_json_dumps_ensure_ascii_false`]
//!    for the exact byte comparison this rests on.
//!
//! `PyFormatter` overrides only the three `Formatter` methods that emit
//! separators; every other byte (numbers, string escaping, structural
//! brackets) comes from `serde_json`'s default implementation unchanged.

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

#[derive(Debug, Clone, Copy, Default)]
struct PyFormatter;

impl Formatter for PyFormatter {
    #[inline]
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    #[inline]
    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    #[inline]
    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}

/// Serialize *value* exactly as CPython's `json.dumps(value,
/// ensure_ascii=False)` would (no trailing newline). Used for every
/// `points.jsonl` line — see the module docs for what "exactly" rests on.
pub fn to_python_json_bytes<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, PyFormatter);
    value.serialize(&mut ser)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Golden strings captured from a real interpreter:
    /// `/workspaces/sdarm/.venv/bin/python3.11 -c 'import json; print(json.dumps(obj, ensure_ascii=False))'`
    /// on CPython 3.11, covering: nested objects, `null`, `true`, negative
    /// ints, an array, a quote/backslash/newline/tab needing escapes, and
    /// two flavours of non-ASCII (accented Latin + combining, and CJK +
    /// emoji + raw control characters needing `\u00XX`).
    #[test]
    fn matches_cpython_json_dumps_ensure_ascii_false() {
        let case0 = json!({
            "uid": "en:WDYS:1.1#0",
            "id": "41265457-5d67-50cc-a5ad-fd24b69403d9",
            "payload": {
                "lang": "en", "book_code": "WDYS", "book_pair": "WDYS", "page": 1,
                "para": 1, "para_key": "1.1",
                "raw_text": "hello \"world\"\nline2\ttab",
                "aligned": null
            }
        });
        let want0 = "{\"uid\": \"en:WDYS:1.1#0\", \"id\": \"41265457-5d67-50cc-a5ad-fd24b69403d9\", \"payload\": {\"lang\": \"en\", \"book_code\": \"WDYS\", \"book_pair\": \"WDYS\", \"page\": 1, \"para\": 1, \"para_key\": \"1.1\", \"raw_text\": \"hello \\\"world\\\"\\nline2\\ttab\", \"aligned\": null}}";
        assert_eq!(
            String::from_utf8(to_python_json_bytes(&case0).unwrap()).unwrap(),
            want0
        );

        let case1 = json!({
            "uid": "de:WDYS:1.1#0",
            "id": "x",
            "payload": {
                "lang": "de", "book_code": "WDYS",
                "raw_text": "Hallo Welt äöüß — “Zitat” café",
                "flag": true, "n": -5, "chunks": 3,
                "bible_refs": ["Gen.1.1", "Ps.23.1"]
            }
        });
        let want1 = "{\"uid\": \"de:WDYS:1.1#0\", \"id\": \"x\", \"payload\": {\"lang\": \"de\", \"book_code\": \"WDYS\", \"raw_text\": \"Hallo Welt äöüß — “Zitat” café\", \"flag\": true, \"n\": -5, \"chunks\": 3, \"bible_refs\": [\"Gen.1.1\", \"Ps.23.1\"]}}";
        assert_eq!(
            String::from_utf8(to_python_json_bytes(&case1).unwrap()).unwrap(),
            want1
        );

        let case2 = json!({
            "uid": "ja:X:1#0",
            "id": "y",
            "payload": {
                "lang": "ja",
                "raw_text": "こんにちは世界 🎉 emoji test \u{0001}\u{001f}"
            }
        });
        let want2 = "{\"uid\": \"ja:X:1#0\", \"id\": \"y\", \"payload\": {\"lang\": \"ja\", \"raw_text\": \"こんにちは世界 🎉 emoji test \\u0001\\u001f\"}}";
        assert_eq!(
            String::from_utf8(to_python_json_bytes(&case2).unwrap()).unwrap(),
            want2
        );
    }

    #[test]
    fn object_key_order_is_preserved_not_sorted() {
        let v = json!({"z": 1, "a": 2, "m": 3});
        let s = String::from_utf8(to_python_json_bytes(&v).unwrap()).unwrap();
        assert_eq!(s, "{\"z\": 1, \"a\": 2, \"m\": 3}");
    }
}
