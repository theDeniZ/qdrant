//! Sentence-boundary chunker and OCR-damage/junk gates.
//!
//! Ported from `sopack/extract/chunk.py` (itself ported from
//! `pd-books/qdrant/build_pioneers_corpus.py`, battle-tested against 51 real
//! pioneer-era OCR'd books). Kept faithful to that reference — same
//! constants, same regexes, same arithmetic — so a re-extraction of the
//! pioneer corpus through `sopack` reproduces the same chunking decisions.

use fancy_regex::Regex;
use std::sync::LazyLock;

/// The chunker's word-count and quality-gate limits. Defaults equal the
/// Python module's constants (`MAX_WORDS`, `TARGET_WORDS`, `MIN_WORDS`,
/// `MAX_BLOCK_DAMAGE`, `MAX_JUNK_CHARS`) — multilingual-e5-large truncates
/// at 512 tokens ≈ 380 English words, so blocks longer than `max_words` are
/// split on sentence boundaries so nothing is silently lost.
///
/// Kept as parameters (not module constants) so a future caller can load
/// them from the embedding contract's `[chunker]` table
/// (`contracts/e5-large-v1/contract.toml`) instead of hardcoding them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkLimits {
    pub max_words: usize,
    pub target_words: usize,
    /// Drops page numbers, running heads, short headings.
    pub min_words: usize,
    /// Per-block gate. A work can score clean overall and still open on a
    /// scrambled title page. Those blocks are dropped on their own.
    pub max_block_damage: f64,
    pub max_junk_chars: f64,
}

impl Default for ChunkLimits {
    fn default() -> Self {
        ChunkLimits {
            max_words: 300,
            target_words: 220,
            min_words: 8,
            max_block_damage: 0.25,
            max_junk_chars: 0.02,
        }
    }
}

// `(?<=[.!?;:])["”’']?\s+` — split just after a sentence-ending punctuation
// mark, optionally consuming one closing quote, then whitespace. Needs a
// lookbehind, which the `regex` crate does not support; fancy-regex is a
// backtracking engine (like CPython's `re`) that does.
static SENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?<=[.!?;:])["”’']?\s+"#).expect("valid SENT_RE"));

// Characters a normal book actually uses. Anything else — box-drawing
// leftovers, stray diacritics, scanner artefacts — is damage. A scrambled
// title page reads as a handful of one-letter "words" and so scores clean
// on damage_score(); it is the character mix that gives it away.
static CLEAN_CHARS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z0-9\s.,;:!?'’‘"“”()\[\]{}—–\-/&%$#*@+=°£§¶†‡]"#)
        .expect("valid CLEAN_CHARS_RE")
});

static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z’']+").expect("valid word re"));
static JUNK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\^~`|\\{}<>]").expect("valid junk re"));

/// Regex-based split, matching Python's `SENT_RE.split(text)` exactly
/// (including the fancy-regex lookbehind semantics).
fn sent_split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0usize;
    for m in SENT_RE.find_iter(text) {
        let m = m.expect("regex match");
        out.push(text[last..m.start()].to_string());
        last = m.end();
    }
    out.push(text[last..].to_string());
    out
}

/// Sentence-boundary split of a block that would overflow the encoder.
pub fn split_long(text: &str, limits: &ChunkLimits) -> Vec<String> {
    if word_count(text) <= limits.max_words {
        return vec![text.to_string()];
    }
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut n = 0usize;
    for sent in sent_split(text) {
        let w = word_count(&sent);
        if !cur.is_empty() && n + w > limits.target_words {
            parts.push(cur.join(" "));
            cur.clear();
            n = 0;
        }
        cur.push(sent);
        n += w;
    }
    if !cur.is_empty() {
        parts.push(cur.join(" "));
    }
    // A single sentence longer than max_words still has to be cut somewhere.
    let mut final_parts = Vec::new();
    for p in parts {
        let words: Vec<&str> = p.split_whitespace().collect();
        if words.len() <= limits.max_words {
            final_parts.push(p);
        } else {
            let mut i = 0;
            while i < words.len() {
                let end = (i + limits.target_words).min(words.len());
                final_parts.push(words[i..end].join(" "));
                i += limits.target_words;
            }
        }
    }
    final_parts.retain(|p| !p.trim().is_empty());
    final_parts
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Share of tokens that look like OCR wreckage. Crude, but it separates
/// 'ButastoJesus' and ';^^^UR country's' from ordinary prose.
pub fn damage_score(text: &str) -> f64 {
    let words: Vec<String> = WORD_RE
        .find_iter(text)
        .filter_map(|m| m.ok())
        .map(|m| m.as_str().to_string())
        .collect();
    if words.is_empty() {
        return 1.0;
    }
    let mut bad = 0usize;
    for w in &words {
        let chars: Vec<char> = w.chars().collect();
        let has_upper_after_first =
            chars.len() > 1 && chars[1..].iter().any(|c| c.is_ascii_uppercase());
        // Python `str.isupper()`: at least one cased character, and every
        // cased character is uppercase.
        let has_alpha = chars.iter().any(|c| c.is_alphabetic());
        let is_all_upper = has_alpha
            && chars
                .iter()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase());
        // Python: `if A: bad += 1 elif B: bad += 1` — both branches do the
        // same thing, so this is just `if A or B` (kept as two named
        // conditions, one per Python line, for traceability).
        let looks_mixed_case = chars.len() > 2 && !is_all_upper && has_upper_after_first;
        let looks_vowelless = chars.len() > 3 && !chars.iter().any(|c| "aeiouyAEIOUY".contains(*c));
        if looks_mixed_case || looks_vowelless {
            bad += 1;
        }
    }
    let junk = JUNK_RE.find_iter(text).filter_map(|m| m.ok()).count();
    (bad + junk) as f64 / words.len() as f64
}

pub fn junk_char_ratio(text: &str) -> f64 {
    if text.is_empty() {
        return 1.0;
    }
    let clean = CLEAN_CHARS_RE
        .find_iter(text)
        .filter_map(|m| m.ok())
        .count();
    let total = text.chars().count();
    1.0 - (clean as f64 / total as f64)
}

/// `None` if *text* passes the per-block gates; else a short drop reason
/// suitable for a `stats["dropped_detail"]` entry (R8 — nothing silently
/// discarded).
pub fn quality_gate(text: &str, limits: &ChunkLimits) -> Option<String> {
    let words = word_count(text);
    if words < limits.min_words {
        return Some(format!(
            "too short ({words} word(s) < {})",
            limits.min_words
        ));
    }
    let d = damage_score(text);
    if d > limits.max_block_damage {
        return Some(format!(
            "OCR damage {:.1}% over {:.0}% gate",
            d * 100.0,
            limits.max_block_damage * 100.0
        ));
    }
    let j = junk_char_ratio(text);
    if j > limits.max_junk_chars {
        return Some(format!(
            "junk-char ratio {:.1}% over {:.1}% gate",
            j * 100.0,
            limits.max_junk_chars * 100.0
        ));
    }
    None
}
