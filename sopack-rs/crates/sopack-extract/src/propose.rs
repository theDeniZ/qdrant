//! `propose` — the metadata-*draft* engine (`SOPACK-1.0-PLAN.md` §3.5, §6).
//!
//! Reads a source the same way [`crate::extract`] eventually will, but
//! instead of committing to one metadata value per field, it collects every
//! **candidate** it can find, tags each with where it came from, and only
//! ever fills a field's `value` when every candidate for it agrees *and* at
//! least one of them comes from a source the plan calls out as trustworthy
//! enough to stand alone (the printed title page, or — for `sop_json` — the
//! file's own reviewed `meta` block). Every other field is left `null` and
//! listed in `unresolved`. This is the plan's own risk mitigation for
//! "`propose` looks authoritative when it is not" (§6): it is not a
//! guesser, it is an evidence collector, and [`Proposal`] cannot represent
//! a value with no candidate behind it.
//!
//! `book_code` in particular is expected to end up `null` on almost every
//! real book: the historical SDARM/pioneer codes (`ATNW`, `CIS`, `SOGO`, …
//! — see `pd-books/converted/_results_pioneers2026.json`) are hand-picked
//! mnemonics, not a mechanical function of the title, so
//! [`acronym_candidates`]'s best-effort acronym is offered purely as a
//! *starting point* for a human/agent, checked against the offline
//! [`crate::registry::Registry`] for collisions — it is not expected to
//! reproduce the historical code, and `propose` never claims it has.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::common::sha256_file;
use crate::epub::{blocks_of, spine_docs, BOILERPLATE_RE, NUMERIC_RE};
use crate::error::ExtractError;
use crate::options::Kind;
use crate::progress::{NoopProgress, ProgressSink, Stage};
use crate::registry::{Registry, TitleMatchKind};

// --------------------------------------------------------------------- types

/// One proposed value for a field, with where it came from.
///
/// `from` is a short, stable tag (`"title_page"`, `"opf:dc:title"`,
/// `"filename"`, `"sop_json_meta"`, …) — documented per call site below,
/// and asserted on by the golden test so the set doesn't drift silently.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Candidate {
    pub value: Value,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Only ever set on a `book_code` candidate: whether this literal code
    /// string is already a row in the offline registry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collision: Option<bool>,
}

impl Candidate {
    fn new(value: impl Into<Value>, from: impl Into<String>) -> Self {
        Candidate {
            value: value.into(),
            from: from.into(),
            evidence: None,
            warning: None,
            collision: None,
        }
    }

    fn with_evidence(mut self, evidence: impl Into<String>) -> Self {
        self.evidence = Some(evidence.into());
        self
    }

    fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warning = Some(warning.into());
        self
    }

    fn with_collision(mut self, collision: bool) -> Self {
        self.collision = Some(collision);
        self
    }
}

/// `from` tags that are trusted enough, on their own, to resolve a field's
/// `value` when every candidate for that field agrees. Everything else
/// (OPF metadata, filenames, heuristics) is corroborating evidence only.
const TITLE_PAGE: &str = "title_page";
const SOP_JSON_META: &str = "sop_json_meta";
const SOP_JSON_DIR: &str = "sop_json_dir";
fn is_authoritative(from: &str) -> bool {
    matches!(from, TITLE_PAGE | SOP_JSON_META | SOP_JSON_DIR)
}

/// One field's candidates plus the value `propose` was willing to commit to
/// (`null` unless [`is_authoritative`] and every candidate agrees).
#[derive(Debug, Clone, Serialize, Default)]
pub struct FieldProposal {
    pub value: Value,
    pub candidates: Vec<Candidate>,
}

impl FieldProposal {
    fn from_candidates(candidates: Vec<Candidate>) -> Self {
        let value = resolve(&candidates);
        FieldProposal { value, candidates }
    }
}

/// `value = candidates[0].value` iff every candidate's value is equal *and*
/// at least one candidate's `from` is authoritative — otherwise `Null`.
fn resolve(candidates: &[Candidate]) -> Value {
    if candidates.is_empty() {
        return Value::Null;
    }
    let first = &candidates[0].value;
    let all_agree = candidates.iter().all(|c| &c.value == first);
    let has_authoritative = candidates.iter().any(|c| is_authoritative(&c.from));
    if all_agree && has_authoritative {
        first.clone()
    } else {
        Value::Null
    }
}

/// The `ExtractOptions`-named fields a `meta.toml` sidecar resolves,
/// in the fixed order the JSON/TOML output uses.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Fields {
    pub book_code: FieldProposal,
    pub lang: FieldProposal,
    pub title: FieldProposal,
    pub author: FieldProposal,
    pub year: FieldProposal,
    pub corpus: FieldProposal,
    pub slug: FieldProposal,
    pub book_pair: FieldProposal,
    pub acquired_from: FieldProposal,
    pub rights: FieldProposal,
}

/// The 10 field names, in the same fixed order as [`Fields`]'s members —
/// exactly [`crate::options::ExtractOptions`]'s field set, since a
/// `meta.toml` sidecar resolves this struct one field at a time.
pub const FIELD_NAMES: [&str; 10] = [
    "book_code",
    "lang",
    "title",
    "author",
    "year",
    "corpus",
    "slug",
    "book_pair",
    "acquired_from",
    "rights",
];

impl Fields {
    /// `(name, &FieldProposal)` pairs in [`FIELD_NAMES`] order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &FieldProposal)> {
        [
            ("book_code", &self.book_code),
            ("lang", &self.lang),
            ("title", &self.title),
            ("author", &self.author),
            ("year", &self.year),
            ("corpus", &self.corpus),
            ("slug", &self.slug),
            ("book_pair", &self.book_pair),
            ("acquired_from", &self.acquired_from),
            ("rights", &self.rights),
        ]
        .into_iter()
    }

    /// Field name -> resolved value, for the fields that have one. Used by
    /// [`crate::meta::write_template`] to pre-fill a sidecar.
    pub fn resolved(&self) -> HashMap<&'static str, Value> {
        self.iter()
            .filter(|(_, f)| !f.value.is_null())
            .map(|(name, f)| (name, f.value.clone()))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub path: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The metadata draft `propose` hands to the agent loop
/// (`SOPACK-1.0-PLAN.md` §3.5).
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub source: SourceInfo,
    pub fields: Fields,
    pub unresolved: Vec<String>,
    pub warnings: Vec<String>,
}

// ------------------------------------------------------------------ entry

/// Read *source* and draft a [`Proposal`]. *kind* is inferred from the file
/// extension when `None` (`.epub` -> epub, `.md`/`.markdown` -> markdown,
/// `.json` -> sop_json, everything else -> text) — pass it explicitly when
/// the extension is unreliable. *registry* enables offline `book_code`
/// collision checks (§3.2); pass `None` to skip them (every candidate then
/// reports `collision: null`, not `false` — "not checked" is not the same
/// claim as "checked, no collision").
pub fn propose(
    source: &Path,
    kind: Option<Kind>,
    registry: Option<&Registry>,
    progress: &mut dyn ProgressSink,
) -> Result<Proposal, ExtractError> {
    if !source.exists() {
        return Err(ExtractError::input_invalid(format!(
            "no such file: {}",
            source.display()
        )));
    }
    let kind = match kind {
        Some(k) => k,
        None => infer_kind(source)?,
    };

    progress.stage_start(Stage::ReadContainer, None);
    let sha256 = sha256_file(source)?;
    let bytes = std::fs::metadata(source)
        .map_err(|e| ExtractError::input_invalid(format!("cannot stat {}: {e}", source.display())))?
        .len();
    progress.stage_end(Stage::ReadContainer);

    let mut warnings: Vec<String> = Vec::new();
    let mut fields = match kind {
        Kind::Epub => propose_epub(source, registry, &mut warnings, progress)?,
        Kind::Markdown => propose_markdown_or_text(source, registry, true)?,
        Kind::Text => propose_markdown_or_text(source, registry, false)?,
        Kind::SopJson => propose_sop_json(source, &mut warnings)?,
    };

    // book_pair mirrors whatever book_code candidates were found (epub.rs's
    // own extractor default is `book_pair = book_code` too — see `epub::extract`).
    // A mirrored candidate keeps the original's `warning`/`collision` too —
    // a registry re-import hit on book_code (SOPACK-1.0-PLAN.md §3.2) is
    // exactly as true of book_pair, which defaults to the same value.
    if fields.book_pair.candidates.is_empty() {
        let mirrored: Vec<Candidate> = fields
            .book_code
            .candidates
            .iter()
            .map(|c| {
                let mut m = Candidate::new(c.value.clone(), "derived:book_code");
                m.evidence = Some("mirrors the book_code candidate of the same value".to_string());
                m.warning = c.warning.clone();
                m.collision = c.collision;
                m
            })
            .collect();
        fields.book_pair = FieldProposal::from_candidates(mirrored);
    }

    let unresolved: Vec<String> = fields
        .iter()
        .filter(|(_, f)| f.value.is_null())
        .map(|(name, _)| name.to_string())
        .collect();

    Ok(Proposal {
        source: SourceInfo {
            path: source.display().to_string(),
            kind: kind.as_str().to_string(),
            sha256,
            bytes,
        },
        fields,
        unresolved,
        warnings,
    })
}

fn infer_kind(source: &Path) -> Result<Kind, ExtractError> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("epub") => Ok(Kind::Epub),
        Some("md") | Some("markdown") => Ok(Kind::Markdown),
        Some("json") => Ok(Kind::SopJson),
        Some("txt") | Some("text") => Ok(Kind::Text),
        _ => Err(ExtractError::input_invalid(format!(
            "cannot infer a source kind from {} — pass --kind explicitly",
            source.display()
        ))),
    }
}

// -------------------------------------------------------------- shared text heuristics

static BY_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^by[:\s]+(.{3,80})$").expect("valid BY_LINE_RE"));
static YEAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(1[5-9]\d{2}|20[0-4]\d)").expect("valid YEAR_RE"));
static HEADING_MD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#{1,6}\s+(.+)$").expect("valid HEADING_MD_RE"));

/// Strips a trailing "BY <NAME>" byline down to a plausible author-name
/// string: trims surrounding whitespace/punctuation, collapses internal
/// runs of whitespace. Titlecases nothing — archive.org bylines are often
/// already in the right case ("BY REV. J. N. ANDREWS, OF N. C.") and
/// guessing a case transform would be exactly the kind of silent invention
/// rule #9 forbids.
fn clean_byline(raw: &str) -> String {
    raw.trim()
        .trim_end_matches(['.', ',', ';'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// Common nouns a "BY <...>" line's remainder is made of when the line is
// actually a fragment of the printed *title* (or some other non-name
// description), not a byline — plus the ordinary function words a personal
// name never consists entirely of. Real incident: "AN EXPLANATION / OF THE
// / TYPICAL AND ANTI-TYPICAL / SANCTUARY / BY THE SCRIPTURES. / WITH A
// CHART. / BY JOSEPH BATES." is one continuous title-page title split
// across separate headings by the EPUB conversion; the *second* "BY ..."
// line is the real byline, but a bare `BY_LINE_RE` match takes the first.
const AUTHOR_NON_NAME_WORDS: &[&str] = &[
    // function words: a real byline is a name, not a sentence fragment.
    "the",
    "a",
    "an",
    "of",
    "in",
    "on",
    "at",
    "to",
    "and",
    "or",
    "its",
    "from",
    "with",
    "for",
    "is",
    "as",
    "be",
    "it",
    "his",
    "her",
    "this",
    "that",
    "who",
    "which",
    // nouns this corpus's known false positives are made of — descriptions
    // of the book/publisher, not people.
    "scriptures",
    "scripture",
    "bible",
    "chart",
    "committee",
    "society",
    "association",
    "church",
    "conference",
    "publishers",
    "publisher",
    "editor",
    "editors",
    "author",
    "authors",
    "authority",
    "request",
    "order",
    "people",
    "public",
    "friend",
    "friends",
    "reader",
    "readers",
];

/// Whether *text* (the captured remainder of a "BY ..." line, already
/// through [`clean_byline`]) plausibly names a *person*, as opposed to
/// being a fragment of the book's own title or some other non-name
/// description that happens to start with "BY".
///
/// Three independent rejections (any one is disqualifying):
/// 1. *text* reappears verbatim (case-insensitively) inside *opf_title* —
///    the "BY ..." line is part of the printed title, not a standalone
///    byline, so accepting it would misread the title as an author.
/// 2. *text* opens with an article ("the "/"a "/"an ") — a real byline
///    names someone, it doesn't open with a description.
/// 3. Every word in *text* is an ordinary function word or one of this
///    corpus's known non-name nouns — no word looks like it could be part
///    of a personal name (a short alphabetic token is read as a run of
///    initials, e.g. "J."/"N."; a longer one must not be in the stoplist).
fn is_plausible_author_text(text: &str, opf_title: Option<&str>) -> bool {
    if let Some(title) = opf_title {
        let text_trimmed = text.trim_end_matches(['.', ',', ';']);
        if !text_trimmed.is_empty() && title.to_lowercase().contains(&text_trimmed.to_lowercase()) {
            return false;
        }
    }
    let lower = text.to_lowercase();
    if lower.starts_with("the ") || lower.starts_with("a ") || lower.starts_with("an ") {
        return false;
    }
    text.split_whitespace().any(|w| {
        let core: String = w.chars().filter(|c| c.is_alphabetic()).collect();
        let core_len = core.chars().count();
        if core_len == 0 {
            return false;
        }
        if core_len == 1 {
            return true; // a bare initial, e.g. "J"
        }
        if core_len == 2 && w.contains('.') {
            return true; // initials run together, e.g. "Jn."
        }
        if core_len <= 2 {
            return false; // "of", "to", "is", "by", … — a 2-letter function
                          // word with no period, not an initial
        }
        !AUTHOR_NON_NAME_WORDS.contains(&core.to_lowercase().as_str())
    })
}

/// The first line in *lines* that looks like a title-page byline and whose
/// remainder [`is_plausible_author_text`] accepts as a personal name, as a
/// `(author_text, matched_line)` pair. *opf_title* (when known) feeds that
/// plausibility check so a "BY ..." fragment of the book's own title is
/// skipped rather than misread as the author.
fn find_byline(lines: &[String], opf_title: Option<&str>) -> Option<(String, String)> {
    for line in lines {
        if let Some(cap) = BY_LINE_RE.captures(line.trim()).ok().flatten() {
            let author = clean_byline(cap.get(1).unwrap().as_str());
            if !author.is_empty() && is_plausible_author_text(&author, opf_title) {
                return Some((author, line.clone()));
            }
        }
    }
    None
}

/// The first 4-digit year found in *lines* that is not itself part of a
/// line [`find_byline`] already claimed, as a `(year, matched_line)` pair.
fn find_year_line(lines: &[String], skip: &[String]) -> Option<(i64, String)> {
    for line in lines {
        if skip.iter().any(|s| s == line) {
            continue;
        }
        if let Some(cap) = YEAR_RE.captures(line).ok().flatten() {
            if let Ok(y) = cap.get(1).unwrap().as_str().parse::<i64>() {
                return Some((y, line.clone()));
            }
        }
    }
    None
}

/// The first line that reads like a title: not empty, not boilerplate, not
/// a numeric/TOC line, not itself a byline, at least 2 words.
fn find_title_line<'a>(lines: &'a [String], byline: Option<&str>) -> Option<&'a str> {
    lines.iter().map(String::as_str).find(|line| {
        let t = line.trim();
        if t.is_empty() || Some(t) == byline {
            return false;
        }
        if BOILERPLATE_RE.is_match(t).unwrap_or(false) || NUMERIC_RE.is_match(t).unwrap_or(false) {
            return false;
        }
        if BY_LINE_RE.is_match(t).unwrap_or(false) {
            return false;
        }
        t.split_whitespace().count() >= 2
    })
}

// Very small stopword lists — enough to separate the languages this corpus
// actually contains (en/de plus a few likely future ones), not a general
// language-id model. Script-based detection (Cyrillic/Hangul/Han+Kana)
// handles ru/ko/ja without any wordlist at all.
const STOPWORDS_EN: &[&str] = &[
    "the", "and", "of", "to", "in", "is", "that", "it", "was", "for",
];
const STOPWORDS_DE: &[&str] = &[
    "der", "die", "das", "und", "ist", "nicht", "ich", "sie", "mit", "den",
];
const STOPWORDS_FR: &[&str] = &[
    "le", "la", "les", "et", "de", "un", "une", "est", "dans", "que",
];
const STOPWORDS_ES: &[&str] = &["el", "la", "de", "y", "en", "que", "un", "una", "es", "los"];

/// A best-effort `lang` guess from body text: script detection for
/// Cyrillic/Hangul/CJK, a stopword count for the Latin-script languages
/// this corpus is likely to hold. `None` when no signal at all (e.g. the
/// sample was too short) — no candidate is better than a wrong one.
fn lang_heuristic(text: &str) -> Option<&'static str> {
    let sample: String = text.chars().take(4000).collect();
    let mut cyrillic = 0usize;
    let mut hangul = 0usize;
    let mut cjk = 0usize;
    for c in sample.chars() {
        match c as u32 {
            0x0400..=0x04FF => cyrillic += 1,
            0xAC00..=0xD7A3 => hangul += 1,
            0x3040..=0x30FF | 0x4E00..=0x9FFF => cjk += 1,
            _ => {}
        }
    }
    if hangul > 20 {
        return Some("ko");
    }
    if cjk > 20 {
        return Some("ja");
    }
    if cyrillic > 20 {
        return Some("ru");
    }

    let words: Vec<String> = sample
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    if words.len() < 20 {
        return None;
    }
    let score = |list: &[&str]| words.iter().filter(|w| list.contains(&w.as_str())).count();
    let scores = [
        ("en", score(STOPWORDS_EN)),
        ("de", score(STOPWORDS_DE)),
        ("fr", score(STOPWORDS_FR)),
        ("es", score(STOPWORDS_ES)),
    ];
    let (best_lang, best_score) = scores.iter().copied().max_by_key(|(_, s)| *s)?;
    if best_score < 3 {
        None
    } else {
        Some(best_lang)
    }
}

// -------------------------------------------------------------- book_code / corpus

const STOPWORDS_TITLE: &[&str] = &[
    "the", "a", "an", "of", "in", "on", "at", "to", "and", "or", "its", "from", "with", "by",
    "for", "is", "as", "be", "it",
];

/// A best-effort acronym from *title*'s significant words (stopwords
/// dropped), up to 4 letters, checked against *registry* for a collision.
/// **Not** expected to reproduce a historical hand-picked code (see this
/// module's doc comment) — it is a starting point, always just a
/// candidate, never a `value`.
fn acronym_candidates(title: &str, registry: Option<&Registry>) -> Vec<Candidate> {
    let significant: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !STOPWORDS_TITLE.contains(&w.to_lowercase().as_str()))
        .collect();
    if significant.is_empty() {
        return Vec::new();
    }
    let base: String = significant
        .iter()
        .take(4)
        .filter_map(|w| w.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if base.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let collision = registry.map(|r| r.contains(&base)).unwrap_or(false);
    let mut cand = Candidate::new(base.clone(), "title_heuristic").with_evidence(format!(
        "first letter of each significant word in the title, up to 4: {}",
        significant
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    ));
    if registry.is_some() {
        cand = cand.with_collision(collision);
    }
    out.push(cand);

    if collision {
        // Alternative: extend to a 5th significant word's initial.
        if let Some(fifth) = significant.get(4).and_then(|w| w.chars().next()) {
            let alt = format!("{base}{}", fifth.to_ascii_uppercase());
            let alt_collision = registry.map(|r| r.contains(&alt)).unwrap_or(false);
            out.push(
                Candidate::new(alt, "title_heuristic_alt")
                    .with_evidence("base acronym collided; extended with a 5th word's initial")
                    .with_collision(alt_collision),
            );
        }
    }
    out
}

/// `from` tag for a [`Registry::find_by_title`] hit — never authoritative
/// (see [`is_authoritative`]): a title match is strong evidence that the
/// work is *already in the store*, but the plan's resolution rule for
/// `book_code` stays "title-page evidence required", so this can propose
/// re-using the existing code, never silently commit to it.
const REGISTRY_TITLE_MATCH: &str = "registry:title_match";

/// Evidence text for a registry title match: the stored title plus, when
/// the registry carries them, the stored author and year — what an agent
/// compares against the source's byline and date before reusing the code.
fn registry_evidence(registry: &Registry, hit: &crate::registry::TitleMatch, lang: &str) -> String {
    let mut ev = format!("registry title {:?} ({lang})", hit.entry_title);
    if let Some(entry) = registry.get(&hit.code) {
        if let Some(author) = &entry.author {
            ev.push_str(&format!("; author {author:?}"));
        }
        if let Some(year) = entry.year {
            ev.push_str(&format!("; year {year}"));
        }
    }
    ev
}

/// `book_code` candidates from matching *title* against the offline
/// registry's own titles (`SOPACK-1.0-PLAN.md` §3.2) — the **re-import
/// detector**. Unlike [`acronym_candidates`] (which only checks whether a
/// freshly *minted* code string collides with a registry row), this checks
/// whether the *book itself* is already in the store under some other code
/// — the gap the TATS/BP3 incident exposed: propose reported "no
/// collision" for a fresh acronym while the live store already held the
/// identical work as `BP3`.
///
/// Pushes one proposal-level *warning* per matched code into *warnings*
/// (not just a per-candidate one) so a human/agent scanning the top-level
/// `warnings` array — not just this one field's candidates — still catches
/// a re-import.
fn registry_title_candidates(
    title: &str,
    lang: Option<&str>,
    registry: Option<&Registry>,
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    let Some(registry) = registry else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for hit in registry.find_by_title(title, lang) {
        let lang_str = lang.unwrap_or("?");
        let mut cand = Candidate::new(hit.code.as_str(), REGISTRY_TITLE_MATCH)
            .with_evidence(registry_evidence(registry, &hit, lang_str))
            .with_collision(true);
        if hit.kind == TitleMatchKind::Short {
            cand = cand.with_warning("short-title match only — confirm author/year");
        }
        out.push(cand);
        warnings.push(format!(
            "book_code: already in the store as {} — this is a RE-IMPORT; reuse the code",
            hit.code
        ));
    }
    out
}

/// "pioneer heuristics: author != Ellen G. White -> 'pioneers'; EGW ->
/// none" (spec wording, `SOPACK-1.0-PLAN.md` §3.5). Only ever emits a
/// candidate when an author string is in hand — EGW works keep `corpus`
/// entirely unresolved rather than assert a `null`/`none` "value" for it,
/// same "never guess" posture as every other field.
fn corpus_candidate(author: &str) -> Option<Candidate> {
    let lower = author.to_lowercase();
    let is_egw = lower.contains("ellen") && (lower.contains("white") || lower.contains("g. white"));
    if is_egw {
        None
    } else {
        Some(
            Candidate::new("pioneers", "author_heuristic")
                .with_evidence(format!("author {author:?} does not match Ellen G. White")),
        )
    }
}

fn normalize_rights(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

static ARCHIVE_DOWNLOAD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"archive\.org/download/([^/]+)/").expect("valid ARCHIVE_DOWNLOAD_RE")
});

/// `https://archive.org/download/<id>/...pdf` -> `archive.org/details/<id>`
/// — the normalised form the reviewed goldens record (see
/// `conformance/extract/goldens/wdys.book.json`'s `acquired_from`).
fn acquired_from_candidates(source_url: &str) -> Vec<Candidate> {
    let mut out = vec![Candidate::new(source_url, "opf:dc:source")];
    if let Some(cap) = ARCHIVE_DOWNLOAD_RE.captures(source_url).ok().flatten() {
        let id = cap.get(1).unwrap().as_str();
        out.push(
            Candidate::new(
                format!("archive.org/details/{id}"),
                "opf:dc:source_normalized",
            )
            .with_evidence("archive.org download URL rewritten to its details-page form"),
        );
    }
    out
}

// -------------------------------------------------------------- filename convention

/// `<author_key>__<title_kebab>__<year>__<source>` — the pd-books pioneer
/// acquisition's naming convention (`pd-books/converted/MANIFEST.md`,
/// `_results_pioneers2026.json`'s `slug`). Not every source follows it —
/// [`parse_pioneer_filename`] returns `None` rather than a wrong guess when
/// the stem doesn't have a plausible 4-digit year segment.
struct PioneerFilename {
    author_key: String,
    title_kebab: String,
    year: i64,
    #[allow(dead_code)]
    source: String,
}

fn parse_pioneer_filename(stem: &str) -> Option<PioneerFilename> {
    let parts: Vec<&str> = stem.split("__").collect();
    if parts.len() < 3 {
        return None;
    }
    let year_idx = parts.len() - 2;
    let maybe_year = parts[year_idx];
    if maybe_year.len() != 4 || !maybe_year.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year: i64 = maybe_year.parse().ok()?;
    let source = parts[parts.len() - 1].to_string();
    let title_kebab = parts[1..year_idx].join("__");
    if title_kebab.is_empty() {
        return None;
    }
    Some(PioneerFilename {
        author_key: parts[0].to_string(),
        title_kebab,
        year,
        source,
    })
}

/// `waggoner-jh` -> `"J. H. Waggoner"`, `bates-joseph` -> `"Joseph Bates"`,
/// `litch` -> `"Litch"`. A token of length <= 3 is read as run-together
/// initials (one period per letter); a longer token is read as a given
/// name and titlecased. Matches every author form recorded in
/// `pd-books/converted/_results_pioneers2026.json` for the two-token case.
fn author_from_key(author_key: &str) -> Option<String> {
    let tokens: Vec<&str> = author_key.split('-').filter(|t| !t.is_empty()).collect();
    let (surname, rest) = tokens.split_first()?;
    if !surname.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let surname_tc = titlecase_word(surname);
    if rest.is_empty() {
        return Some(surname_tc);
    }
    let mut given_parts: Vec<String> = Vec::new();
    for tok in rest {
        if !tok.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        if tok.len() <= 3 {
            for c in tok.chars() {
                given_parts.push(format!("{}.", c.to_ascii_uppercase()));
            }
        } else {
            given_parts.push(titlecase_word(tok));
        }
    }
    given_parts.push(surname_tc);
    Some(given_parts.join(" "))
}

fn titlecase_word(w: &str) -> String {
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn year_trap_warning(
    candidate_year: i64,
    title_page_year: Option<i64>,
    is_pioneer: bool,
) -> Option<String> {
    if let Some(tp) = title_page_year {
        if candidate_year > tp {
            return Some("digital-edition date?".to_string());
        }
    }
    if is_pioneer && candidate_year > 1950 {
        return Some("digital-edition date?".to_string());
    }
    None
}

// ------------------------------------------------------------------- epub

fn propose_epub(
    path: &Path,
    registry: Option<&Registry>,
    warnings: &mut Vec<String>,
    progress: &mut dyn ProgressSink,
) -> Result<Fields, ExtractError> {
    let file = File::open(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot open {}: {e}", path.display())))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        ExtractError::input_invalid(format!("{}: not a valid EPUB/zip: {e}", path.display()))
    })?;
    let (docs, _opf_path, opf_xml) = spine_docs(&mut archive)?;

    progress.stage_start(Stage::ParseSpineItems, Some(docs.len().min(2) as u64));
    let mut front_lines: Vec<String> = Vec::new();
    for (i, d) in docs.iter().take(2).enumerate() {
        front_lines.extend(blocks_of(&mut archive, d)?);
        progress.stage_progress(
            Stage::ParseSpineItems,
            (i + 1) as u64,
            Some(docs.len().min(2) as u64),
        );
    }
    progress.stage_end(Stage::ParseSpineItems);

    // Body sample for the lang heuristic: first 3 spine docs, joined.
    let mut body_sample = String::new();
    for d in docs.iter().take(3) {
        for b in blocks_of(&mut archive, d)? {
            body_sample.push_str(&b);
            body_sample.push(' ');
            if body_sample.len() > 6000 {
                break;
            }
        }
    }

    let opf_title = extract_dc_tag(&opf_xml, "title");
    let opf_creator = extract_dc_tag(&opf_xml, "creator");
    let opf_date = extract_dc_tag(&opf_xml, "date");
    let opf_rights = extract_dc_tag(&opf_xml, "rights");
    let opf_language = extract_dc_tag(&opf_xml, "language");
    let opf_source = extract_dc_tag(&opf_xml, "source");
    let opf_identifier = extract_dc_tag(&opf_xml, "identifier");

    let byline = find_byline(&front_lines, opf_title.as_deref());
    let title_line = find_title_line(&front_lines, byline.as_ref().map(|(_, l)| l.as_str()));
    let title_page_year = find_year_line(
        &front_lines,
        &byline.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(),
    );

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let pf = parse_pioneer_filename(stem);

    // ---- title
    let mut title_candidates = Vec::new();
    if let Some(t) = title_line {
        title_candidates.push(
            Candidate::new(t, TITLE_PAGE)
                .with_evidence("first plausible title-like line in the opening spine document(s)"),
        );
    }
    if let Some(t) = &opf_title {
        title_candidates.push(Candidate::new(t.as_str(), "opf:dc:title"));
    }
    let title_field = FieldProposal::from_candidates(title_candidates);
    let title_for_heuristics: Option<String> = match &title_field.value {
        Value::String(s) => Some(s.clone()),
        _ => opf_title.clone().or_else(|| title_line.map(str::to_string)),
    };

    // ---- author
    let mut author_candidates = Vec::new();
    if let Some((a, _)) = &byline {
        author_candidates.push(
            Candidate::new(a.as_str(), TITLE_PAGE)
                .with_evidence("a \"BY ...\" byline in the opening spine document(s)"),
        );
    }
    if let Some(a) = &opf_creator {
        author_candidates.push(Candidate::new(a.as_str(), "opf:dc:creator"));
    }
    if let Some(pf) = &pf {
        if let Some(a) = author_from_key(&pf.author_key) {
            author_candidates.push(Candidate::new(a, "filename").with_evidence(format!(
                "author_key {:?} in the pd-books filename convention",
                pf.author_key
            )));
        }
    }
    let author_field = FieldProposal::from_candidates(author_candidates);
    let author_for_heuristics: Option<String> = match &author_field.value {
        Value::String(s) => Some(s.clone()),
        _ => opf_creator.clone(),
    };

    // ---- year (with the year-trap warning)
    let is_pioneer_guess = author_for_heuristics
        .as_deref()
        .map(|a| corpus_candidate(a).is_some())
        .unwrap_or(false);
    let title_page_year_only: Option<i64> = title_page_year.as_ref().map(|(y, _)| *y);
    let mut year_candidates = Vec::new();
    if let Some((y, _)) = &title_page_year {
        year_candidates.push(
            Candidate::new(*y, TITLE_PAGE)
                .with_evidence("a 4-digit year in the opening spine document(s)"),
        );
    }
    if let Some(d) = &opf_date {
        if let Some(cap) = YEAR_RE.captures(d).ok().flatten() {
            if let Ok(y) = cap.get(1).unwrap().as_str().parse::<i64>() {
                let mut c = Candidate::new(y, "opf:dc:date");
                if let Some(w) = year_trap_warning(y, title_page_year_only, is_pioneer_guess) {
                    c = c.with_warning(w.clone());
                    warnings.push(format!("year: opf:dc:date={y} — {w}"));
                }
                year_candidates.push(c);
            }
        }
    }
    if let Some(pf) = &pf {
        let mut c = Candidate::new(pf.year, "filename")
            .with_evidence("year segment of the pd-books filename convention");
        if let Some(w) = year_trap_warning(pf.year, title_page_year_only, is_pioneer_guess) {
            c = c.with_warning(w);
        }
        year_candidates.push(c);
    }
    let year_field = FieldProposal::from_candidates(year_candidates);

    // ---- lang
    let mut lang_candidates = Vec::new();
    if let Some(l) = &opf_language {
        lang_candidates.push(Candidate::new(l.as_str(), "opf:dc:language"));
    }
    if let Some(l) = lang_heuristic(&body_sample) {
        lang_candidates.push(Candidate::new(l, "lang_heuristic"));
    }
    let lang_field = FieldProposal::from_candidates(lang_candidates);

    // ---- corpus
    let corpus_candidates: Vec<Candidate> = author_for_heuristics
        .as_deref()
        .and_then(corpus_candidate)
        .into_iter()
        .collect();
    let corpus_field = FieldProposal::from_candidates(corpus_candidates);

    // ---- book_code
    let mut book_code_candidates = title_for_heuristics
        .as_deref()
        .map(|t| acronym_candidates(t, registry))
        .unwrap_or_default();
    if let Some(t) = title_for_heuristics.as_deref() {
        let lang_guess: Option<String> = match &lang_field.value {
            Value::String(s) => Some(s.clone()),
            _ => opf_language.clone(),
        };
        book_code_candidates.extend(registry_title_candidates(
            t,
            lang_guess.as_deref(),
            registry,
            warnings,
        ));
    }
    let book_code_field = FieldProposal::from_candidates(book_code_candidates);

    // ---- slug
    let mut slug_candidates = Vec::new();
    if !stem.is_empty() {
        slug_candidates.push(Candidate::new(stem, "filename"));
    }
    if let Some(pf) = &pf {
        slug_candidates.push(
            Candidate::new(
                format!("{}-{}", pf.author_key, pf.title_kebab),
                "filename_normalized",
            )
            .with_evidence(
                "author_key-title_kebab, year/source suffix dropped (observed convention)",
            ),
        );
    }
    let slug_field = FieldProposal::from_candidates(slug_candidates);

    // ---- acquired_from
    let acquired_candidates = opf_source
        .as_deref()
        .map(acquired_from_candidates)
        .unwrap_or_default();
    let acquired_field = FieldProposal::from_candidates(acquired_candidates);

    // ---- rights
    let mut rights_candidates = Vec::new();
    if let Some(r) = &opf_rights {
        rights_candidates.push(Candidate::new(r.as_str(), "opf:dc:rights"));
        rights_candidates.push(
            Candidate::new(normalize_rights(r), "opf:dc:rights_normalized")
                .with_evidence("lowercased, spaces to hyphens"),
        );
    }
    let rights_field = FieldProposal::from_candidates(rights_candidates);

    if let Some(id) = &opf_identifier {
        if id.trim().is_empty() {
            warnings.push("opf:dc:identifier is present but empty".to_string());
        }
    }

    Ok(Fields {
        book_code: book_code_field,
        lang: lang_field,
        title: title_field,
        author: author_field,
        year: year_field,
        corpus: corpus_field,
        slug: slug_field,
        book_pair: FieldProposal::default(),
        acquired_from: acquired_field,
        rights: rights_field,
    })
}

/// `<dc:tag>...</dc:tag>` (or self-closing) text content, HTML-entity
/// decoded. A plain substring scan, not full XML parsing: every converted
/// EPUB in this corpus writes the `dc:` prefix literally (no namespace
/// remapping — see any file under `pd-books/converted/*.epub`'s
/// `content.opf`), and `propose` only needs *candidates*, not the
/// authoritative parse `crate::epub::opf_metadata` already provides for
/// the four fields the deterministic extractor uses.
fn extract_dc_tag(opf_xml: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<dc:{tag}");
    let start = opf_xml.find(&open_tag)?;
    let after_open = &opf_xml[start..];
    let gt = after_open.find('>')?;
    if after_open.as_bytes()[gt.saturating_sub(1)] == b'/' {
        return None; // self-closing, no text content
    }
    let content_start = start + gt + 1;
    let close_tag = format!("</dc:{tag}>");
    let content_end = opf_xml[content_start..].find(&close_tag)? + content_start;
    let raw = &opf_xml[content_start..content_end];
    let decoded = html_escape::decode_html_entities(raw).trim().to_string();
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

// -------------------------------------------------------------- markdown / text

fn propose_markdown_or_text(
    path: &Path,
    _registry: Option<&Registry>,
    is_markdown: bool,
) -> Result<Fields, ExtractError> {
    let raw_bytes = std::fs::read(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {}: {e}", path.display())))?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

    let (front_matter, body) = split_front_matter(&raw);
    let lines: Vec<String> = body
        .lines()
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .take(20)
        .collect();

    let mut title_candidates = Vec::new();
    if let Some(t) = front_matter.get("title") {
        title_candidates.push(Candidate::new(t.as_str(), "front_matter"));
    }
    if is_markdown {
        if let Some(first) = lines.first() {
            if let Some(cap) = HEADING_MD_RE.captures(first).ok().flatten() {
                title_candidates.push(Candidate::new(
                    cap.get(1).unwrap().as_str().trim(),
                    "first_lines",
                ));
            }
        }
    } else if let Some(t) = find_title_line(&lines, None) {
        title_candidates.push(Candidate::new(t, "first_lines"));
    }
    let title_field = FieldProposal::from_candidates(title_candidates);

    let mut author_candidates = Vec::new();
    if let Some(a) = front_matter.get("author") {
        author_candidates.push(Candidate::new(a.as_str(), "front_matter"));
    }
    if let Some((a, _)) = find_byline(&lines, title_field.value.as_str()) {
        author_candidates.push(Candidate::new(a, "first_lines"));
    }
    let author_field = FieldProposal::from_candidates(author_candidates);

    let mut year_candidates = Vec::new();
    if let Some(y) = front_matter.get("year").and_then(|v| v.parse::<i64>().ok()) {
        year_candidates.push(Candidate::new(y, "front_matter"));
    }
    if let Some((y, _)) = find_year_line(&lines, &[]) {
        year_candidates.push(Candidate::new(y, "first_lines"));
    }
    let year_field = FieldProposal::from_candidates(year_candidates);

    let mut lang_candidates = Vec::new();
    if let Some(l) = front_matter.get("lang") {
        lang_candidates.push(Candidate::new(l.as_str(), "front_matter"));
    }
    if let Some(l) = lang_heuristic(body) {
        lang_candidates.push(Candidate::new(l, "lang_heuristic"));
    }
    let lang_field = FieldProposal::from_candidates(lang_candidates);

    let mut book_code_candidates = Vec::new();
    if let Some(c) = front_matter.get("book_code") {
        book_code_candidates.push(Candidate::new(c.as_str(), "front_matter"));
    }
    let book_code_field = FieldProposal::from_candidates(book_code_candidates);

    let mut corpus_candidates = Vec::new();
    if let Some(c) = front_matter.get("corpus") {
        corpus_candidates.push(Candidate::new(c.as_str(), "front_matter"));
    }
    let corpus_field = FieldProposal::from_candidates(corpus_candidates);

    let mut slug_candidates = Vec::new();
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        slug_candidates.push(Candidate::new(stem, "filename"));
    }
    let slug_field = FieldProposal::from_candidates(slug_candidates);

    let mut rights_candidates = Vec::new();
    if let Some(r) = front_matter.get("rights") {
        rights_candidates.push(Candidate::new(r.as_str(), "front_matter"));
    }
    let rights_field = FieldProposal::from_candidates(rights_candidates);

    let mut acquired_candidates = Vec::new();
    if let Some(a) = front_matter.get("acquired_from") {
        acquired_candidates.push(Candidate::new(a.as_str(), "front_matter"));
    }
    let acquired_field = FieldProposal::from_candidates(acquired_candidates);

    Ok(Fields {
        book_code: book_code_field,
        lang: lang_field,
        title: title_field,
        author: author_field,
        year: year_field,
        corpus: corpus_field,
        slug: slug_field,
        book_pair: FieldProposal::default(),
        acquired_from: acquired_field,
        rights: rights_field,
    })
}

/// A minimal `---\nkey: value\n---` front-matter block, if *raw* opens with
/// one. Not a YAML parser — one `key: value` per line, quotes stripped —
/// which is all a hand-written sidecar-free markdown/text source is likely
/// to carry. Returns `(fields, remaining_body)`; *raw* unchanged when there
/// is no front matter.
fn split_front_matter(raw: &str) -> (HashMap<String, String>, &str) {
    let mut fields = HashMap::new();
    let trimmed = raw.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return (fields, raw);
    }
    // Everything after the opening "---" line.
    let after_open = &trimmed[3..];
    let after_open = after_open
        .strip_prefix("\r\n")
        .or_else(|| after_open.strip_prefix('\n'))
        .unwrap_or(after_open);
    let Some(close_rel) = after_open.find("\n---") else {
        return (fields, raw);
    };
    let block = &after_open[..close_rel];
    for line in block.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_lowercase();
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !k.is_empty() && !v.is_empty() {
                fields.insert(k, v);
            }
        }
    }
    // Skip past the "\n---" that closed the block, then the rest of that
    // closing delimiter line itself (e.g. a trailing "---" with no `\n` at
    // all, if the block ends the file).
    let after_close = &after_open[close_rel + 4..];
    let body = match after_close.find('\n') {
        Some(nl) => &after_close[nl + 1..],
        None => "",
    };
    (fields, body)
}

// -------------------------------------------------------------- sop_json

fn propose_sop_json(path: &Path, warnings: &mut Vec<String>) -> Result<Fields, ExtractError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {}: {e}", path.display())))?;
    let data: Value = serde_json::from_str(&raw).map_err(|e| {
        ExtractError::input_invalid(format!("{}: not valid JSON: {e}", path.display()))
    })?;
    let Value::Object(data) = data else {
        return Err(ExtractError::input_invalid(format!(
            "{}: top level must be a JSON object",
            path.display()
        )));
    };

    let dir_lang = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned());
    let resolved_lang = match &dir_lang {
        Some(l) if data.contains_key(l) => Some(l.clone()),
        _ => data
            .keys()
            .find(|k| k.as_str() != "meta" && k.as_str() != "en_reverse")
            .cloned(),
    };
    if resolved_lang.is_none() {
        warnings.push(
            "sop_json: could not determine which top-level key holds the paragraphs".to_string(),
        );
    }

    let mut lang_candidates = Vec::new();
    if let Some(l) = &resolved_lang {
        let from = if dir_lang.as_deref() == Some(l.as_str()) {
            SOP_JSON_DIR
        } else {
            "sop_json_key_guess"
        };
        let mut c = Candidate::new(l.as_str(), from);
        if from != SOP_JSON_DIR {
            c = c.with_warning(
                "directory name did not match any top-level key; guessed from the file's own keys",
            );
        }
        lang_candidates.push(c);
    }
    let lang_field = FieldProposal::from_candidates(lang_candidates);

    let meta = match data.get("meta") {
        Some(Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    let meta_str = |key: &str| -> Option<String> {
        match meta.get(key) {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    };

    let mut book_code_candidates = Vec::new();
    if let Some(l) = &resolved_lang {
        if let Some(c) = meta_str(&format!("{l}_code")) {
            book_code_candidates
                .push(Candidate::new(c, SOP_JSON_META).with_evidence(format!("meta.{l}_code")));
        } else if let Some(c) = meta_str("en_code") {
            book_code_candidates
                .push(Candidate::new(c, SOP_JSON_META).with_evidence("meta.en_code"));
        }
    }
    let book_code_field = FieldProposal::from_candidates(book_code_candidates);

    let mut title_candidates = Vec::new();
    if let Some(l) = &resolved_lang {
        if let Some(t) = meta_str(&format!("{l}_title")) {
            title_candidates
                .push(Candidate::new(t, SOP_JSON_META).with_evidence(format!("meta.{l}_title")));
        } else if let Some(t) = meta_str("en_title") {
            title_candidates.push(Candidate::new(t, SOP_JSON_META).with_evidence("meta.en_title"));
        }
    }
    let title_field = FieldProposal::from_candidates(title_candidates);

    let mut year_candidates = Vec::new();
    match meta.get("year") {
        Some(Value::Number(n)) if n.as_i64().is_some() => {
            year_candidates.push(
                Candidate::new(n.as_i64().unwrap(), SOP_JSON_META).with_evidence("meta.year"),
            );
        }
        Some(Value::String(s))
            if s.trim().chars().all(|c| c.is_ascii_digit()) && !s.trim().is_empty() =>
        {
            if let Ok(y) = s.trim().parse::<i64>() {
                year_candidates.push(Candidate::new(y, SOP_JSON_META).with_evidence("meta.year"));
            }
        }
        _ => {}
    }
    let year_field = FieldProposal::from_candidates(year_candidates);

    // `author`/`corpus`/`slug`/`book_pair`/`acquired_from`/`rights`: the
    // sop_json extractor never reads these from `meta` (author is
    // "intentionally left None unless given explicitly" — `sop_json.rs`'s
    // doc comment), but `propose` still surfaces them as authoritative
    // candidates when a reviewed file happens to carry them, since the
    // meta block is defined as authoritative for this kind regardless of
    // which subset the deterministic extractor currently consumes.
    let passthrough = |key: &str| -> FieldProposal {
        match meta_str(key) {
            Some(v) => FieldProposal::from_candidates(vec![
                Candidate::new(v, SOP_JSON_META).with_evidence(format!("meta.{key}"))
            ]),
            None => FieldProposal::default(),
        }
    };

    Ok(Fields {
        book_code: book_code_field,
        lang: lang_field,
        title: title_field,
        author: passthrough("author"),
        year: year_field,
        corpus: passthrough("corpus"),
        slug: passthrough("slug"),
        book_pair: FieldProposal::default(),
        acquired_from: passthrough("acquired_from"),
        rights: passthrough("rights"),
    })
}

/// Convenience wrapper for callers that don't care about progress.
pub fn propose_quiet(
    source: &Path,
    kind: Option<Kind>,
    registry: Option<&Registry>,
) -> Result<Proposal, ExtractError> {
    propose(source, kind, registry, &mut NoopProgress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_requires_agreement_and_an_authoritative_source() {
        let agree_no_auth = vec![
            Candidate::new("X", "opf:dc:title"),
            Candidate::new("X", "filename"),
        ];
        assert_eq!(resolve(&agree_no_auth), Value::Null);

        let auth_alone = vec![Candidate::new("X", TITLE_PAGE)];
        assert_eq!(resolve(&auth_alone), Value::from("X"));

        let disagree_with_auth = vec![
            Candidate::new("X", TITLE_PAGE),
            Candidate::new("Y", "opf:dc:title"),
        ];
        assert_eq!(resolve(&disagree_with_auth), Value::Null);

        let agree_with_auth = vec![
            Candidate::new("X", TITLE_PAGE),
            Candidate::new("X", "opf:dc:title"),
        ];
        assert_eq!(resolve(&agree_with_auth), Value::from("X"));
    }

    #[test]
    fn author_from_key_matches_manifest_forms() {
        assert_eq!(
            author_from_key("waggoner-jh").as_deref(),
            Some("J. H. Waggoner")
        );
        assert_eq!(
            author_from_key("haskell-sn").as_deref(),
            Some("S. N. Haskell")
        );
        assert_eq!(
            author_from_key("andrews-jn").as_deref(),
            Some("J. N. Andrews")
        );
        assert_eq!(
            author_from_key("crosier-orl").as_deref(),
            Some("O. R. L. Crosier")
        );
        assert_eq!(
            author_from_key("bates-joseph").as_deref(),
            Some("Joseph Bates")
        );
        assert_eq!(
            author_from_key("white-james").as_deref(),
            Some("James White")
        );
        assert_eq!(author_from_key("litch").as_deref(), Some("Litch"));
    }

    #[test]
    fn parse_pioneer_filename_matches_real_stems() {
        let pf = parse_pioneer_filename(
            "waggoner-jh__the-atonement-examination-remedial-system__1884__archive",
        )
        .unwrap();
        assert_eq!(pf.author_key, "waggoner-jh");
        assert_eq!(pf.title_kebab, "the-atonement-examination-remedial-system");
        assert_eq!(pf.year, 1884);
        assert_eq!(pf.source, "archive");

        assert!(parse_pioneer_filename("plain_no_convention").is_none());
    }

    #[test]
    fn corpus_candidate_egw_vs_pioneer() {
        assert!(corpus_candidate("Ellen G. White").is_none());
        assert!(corpus_candidate("Ellen White").is_none());
        assert_eq!(
            corpus_candidate("J. H. Waggoner").unwrap().value,
            Value::from("pioneers")
        );
    }

    #[test]
    fn normalize_rights_matches_golden_convention() {
        assert_eq!(normalize_rights("Public domain"), "public-domain");
    }

    #[test]
    fn acquired_from_normalizes_archive_download_urls_like_the_golden() {
        let cands = acquired_from_candidates(
            "https://archive.org/download/whydoyouswear00andr/whydoyouswear00andr.pdf",
        );
        assert!(cands
            .iter()
            .any(|c| c.value.as_str() == Some("archive.org/details/whydoyouswear00andr")));
    }

    #[test]
    fn lang_heuristic_detects_script_languages_without_wordlists() {
        assert_eq!(lang_heuristic(&"\u{d55c}".repeat(30)), Some("ko"));
        assert_eq!(lang_heuristic(&"\u{3042}".repeat(30)), Some("ja"));
        assert_eq!(lang_heuristic(&"\u{0430}".repeat(30)), Some("ru"));
    }

    #[test]
    fn lang_heuristic_detects_en_vs_de_by_stopwords() {
        let en = "the quick brown fox and the lazy dog it was in the of the that it is to the "
            .repeat(3);
        let de = "der und die das ist nicht ich sie mit den der und die das ist ".repeat(3);
        assert_eq!(lang_heuristic(&en), Some("en"));
        assert_eq!(lang_heuristic(&de), Some("de"));
    }

    #[test]
    fn split_front_matter_parses_simple_block() {
        let raw = "---\ntitle: Hello World\nyear: 1900\n---\nBody text here.\n";
        let (fields, body) = split_front_matter(raw);
        assert_eq!(fields.get("title").map(String::as_str), Some("Hello World"));
        assert_eq!(fields.get("year").map(String::as_str), Some("1900"));
        assert_eq!(body.trim(), "Body text here.");
    }

    #[test]
    fn split_front_matter_absent_returns_whole_body() {
        let raw = "No front matter here.\n";
        let (fields, body) = split_front_matter(raw);
        assert!(fields.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn find_byline_and_title_line() {
        let lines = vec![
            "THE ATONEMENT".to_string(),
            "BY J. H. WAGGONER".to_string(),
            "Battle Creek, Mich., 1884".to_string(),
        ];
        let (author, matched) = find_byline(&lines, None).unwrap();
        assert_eq!(author, "J. H. WAGGONER");
        let title = find_title_line(&lines, Some(matched.as_str())).unwrap();
        assert_eq!(title, "THE ATONEMENT");
        let (year, _) = find_year_line(&lines, &[matched]).unwrap();
        assert_eq!(year, 1884);
    }

    /// Real title-page text from
    /// `bates-joseph__explanation-of-the-typical-and-anti-typical-sanctuary__1850__archive.epub`
    /// (`OEBPS/ch002.xhtml`'s block sequence — every `h1`/`h2`/`p` in
    /// document order) — the incident this fix addresses: the title itself
    /// is split across separate headings by the EPUB conversion, and one
    /// of those fragments, "BY THE SCRIPTURES.", happens to match
    /// `BY_LINE_RE` before the real byline, "BY JOSEPH BATES.", is reached.
    fn bates_title_page_lines() -> Vec<String> {
        [
            "AN EXPLANATION",
            "OF THE",
            "TYPICAL AND ANTI-TYPICAL",
            "SANCTUARY",
            "BY THE SCRIPTURES.",
            "WITH A CHART.",
            "BY JOSEPH BATES.",
            "EXPLANATION OF THE CHART:",
            "IT appears that God\u{2019}s instruction to Moses on Mount Sinai, was the most simple and yet the most impressive imaginable, viz: SHADOWS.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    const BATES_OPF_TITLE: &str =
        "An Explanation of the Typical and Anti-typical Sanctuary by the Scriptures, with a Chart";

    #[test]
    fn find_byline_skips_a_title_fragment_and_finds_the_real_byline() {
        let lines = bates_title_page_lines();
        let (author, matched) = find_byline(&lines, Some(BATES_OPF_TITLE)).unwrap();
        assert_eq!(author, "JOSEPH BATES");
        assert_eq!(matched, "BY JOSEPH BATES.");
        assert_ne!(author, "THE SCRIPTURES");
    }

    #[test]
    fn find_byline_without_opf_title_still_rejects_the_non_name_fragment() {
        // Even with no dc:title to cross-check against, "THE SCRIPTURES" is
        // rejected on its own: it opens with an article and contains no
        // word that isn't a function word / this corpus's known non-name
        // noun.
        let lines = bates_title_page_lines();
        let (author, matched) = find_byline(&lines, None).unwrap();
        assert_eq!(author, "JOSEPH BATES");
        assert_eq!(matched, "BY JOSEPH BATES.");
    }

    #[test]
    fn is_plausible_author_text_rejects_title_fragments_and_bare_descriptions() {
        assert!(!is_plausible_author_text(
            "THE SCRIPTURES",
            Some(BATES_OPF_TITLE)
        ));
        assert!(!is_plausible_author_text("THE SCRIPTURES", None));
        assert!(!is_plausible_author_text("A FRIEND OF TRUTH", None));
        assert!(!is_plausible_author_text("ORDER OF THE COMMITTEE", None));
    }

    #[test]
    fn is_plausible_author_text_accepts_real_name_forms() {
        assert!(is_plausible_author_text(
            "JOSEPH BATES",
            Some(BATES_OPF_TITLE)
        ));
        assert!(is_plausible_author_text("J. H. WAGGONER", None));
        assert!(is_plausible_author_text("ELD. J. H. WAGGONER", None));
        assert!(is_plausible_author_text(
            "REV. J. N. ANDREWS, OF N. C.",
            None
        ));
    }
}
