//! OSIS verse keys for the `bible` profile — the `para_key` of every bible
//! block, stored as the point's `osis` payload and hashed into its id
//! (`bible/v1` = `bible:{bible}:{osis}`).
//!
//! A key is `<Book>.<chapter>.<verse>` in the edition's **own** numbering
//! (`Ps.51.3` in a Hebrew-numbered Bible is "Have mercy upon me"), with an
//! optional OSIS grain suffix for a verse the edition splits (`Exod.22.3!a`,
//! as in the live `japkougo`). `<Book>` is one of the 66 canonical OSIS book
//! names the bible-sop read tools parse — the same set every live Bible uses.

/// The 66 canonical OSIS book names, in canonical order.
pub const OSIS_BOOKS: [&str; 66] = [
    "Gen", "Exod", "Lev", "Num", "Deut", "Josh", "Judg", "Ruth", "1Sam", "2Sam", "1Kgs", "2Kgs",
    "1Chr", "2Chr", "Ezra", "Neh", "Esth", "Job", "Ps", "Prov", "Eccl", "Song", "Isa", "Jer",
    "Lam", "Ezek", "Dan", "Hos", "Joel", "Amos", "Obad", "Jonah", "Mic", "Nah", "Hab", "Zeph",
    "Hag", "Zech", "Mal", "Matt", "Mark", "Luke", "John", "Acts", "Rom", "1Cor", "2Cor", "Gal",
    "Eph", "Phil", "Col", "1Thess", "2Thess", "1Tim", "2Tim", "Titus", "Phlm", "Heb", "Jas",
    "1Pet", "2Pet", "1John", "2John", "3John", "Jude", "Rev",
];

/// One parsed verse key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsisVerse {
    /// Index into [`OSIS_BOOKS`].
    pub book: usize,
    pub chapter: i64,
    pub verse: i64,
    /// The `!a` grain of a split verse, without the `!`.
    pub grain: Option<String>,
}

impl OsisVerse {
    /// Canonical sort key: book order, chapter, verse, grain.
    pub fn sort_key(&self) -> (usize, i64, i64, String) {
        (
            self.book,
            self.chapter,
            self.verse,
            self.grain.clone().unwrap_or_default(),
        )
    }
}

fn positive(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.starts_with('0') {
        return None;
    }
    s.parse().ok().filter(|&n: &i64| n > 0)
}

/// Parses `Book.C.V` / `Book.C.V!x` (`x` = lowercase ASCII letters).
/// `None` for anything else: an unknown book, a chapter-only key, `0` or a
/// leading zero, a range.
pub fn parse_verse(key: &str) -> Option<OsisVerse> {
    let (base, grain) = match key.split_once('!') {
        Some((b, g)) => {
            if g.is_empty() || !g.bytes().all(|c| c.is_ascii_lowercase()) {
                return None;
            }
            (b, Some(g.to_string()))
        }
        None => (key, None),
    };
    let mut parts = base.split('.');
    let (b, c, v) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(OsisVerse {
        book: OSIS_BOOKS.iter().position(|&n| n == b)?,
        chapter: positive(c)?,
        verse: positive(v)?,
        grain,
    })
}

/// A bible id (the `book_code` of a bible book, stored as the `bible`
/// payload): lowercase ASCII letters and digits, `-`/`_` after the first
/// character — the shape of every live id (`kjv`, `luther1912`, `ukrogienko`).
pub fn is_bible_id(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_split_verses() {
        let v = parse_verse("Ps.51.3").unwrap();
        assert_eq!(
            (OSIS_BOOKS[v.book], v.chapter, v.verse, v.grain),
            ("Ps", 51, 3, None)
        );
        let v = parse_verse("Exod.22.3!a").unwrap();
        assert_eq!((v.chapter, v.verse, v.grain.as_deref()), (22, 3, Some("a")));
        assert_eq!(parse_verse("1John.5.7").unwrap().book, 61);
    }

    #[test]
    fn rejects_non_verse_keys() {
        for k in [
            "Gen.1",
            "Gen.1.1.1",
            "Gen.0.1",
            "Gen.1.0",
            "Gen.01.1",
            "Tob.1.1",
            "gen.1.1",
            "Gen.1.1-3",
            "Gen.1.1!",
            "Gen.1.1!A",
            "1.1",
            "",
        ] {
            assert!(parse_verse(k).is_none(), "{k}");
        }
    }

    #[test]
    fn bible_ids() {
        for ok in ["kjv", "luther1912", "cep", "nkjv-2", "a_b"] {
            assert!(is_bible_id(ok), "{ok}");
        }
        for bad in ["", "KJV", "Cep", "-x", "c p", "čep"] {
            assert!(!is_bible_id(bad), "{bad}");
        }
    }
}
