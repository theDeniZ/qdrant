//! `Kind`, `ExtractOptions` and the per-kind accepted-option gate — port of
//! `sopack.cli._EXTRACT_OPTS` and its enforcement in `_cmd_extract`.
//!
//! In the Python reference this table lives in the CLI (`cli.py`), not
//! `sopack/extract/`, because each extractor's own `extract()` function
//! kwargs already encode which options it accepts (a plain Python
//! `TypeError` for an unknown kwarg). This crate has one shared
//! [`ExtractOptions`] struct across all four kinds instead of four
//! distinct function signatures, so the acceptance table has to be
//! checked explicitly — [`check_options`] does that, and the top-level
//! [`crate::extract`] dispatcher calls it before dispatching, so a caller
//! that goes through the public API gets the same "refuse rather than
//! silently drop" behaviour `sopack extract` gives on the Mac.

use std::str::FromStr;

use crate::chunk::ChunkLimits;
use crate::error::{ErrorCode, ExtractError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Epub,
    Markdown,
    Text,
    SopJson,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Epub => "epub",
            Kind::Markdown => "markdown",
            Kind::Text => "text",
            Kind::SopJson => "sop_json",
        }
    }

    /// The `_EXTRACT_OPTS` row for this kind: which [`ExtractOptions`]
    /// fields this source format's extractor actually reads. `sop_json`
    /// derives `book_code`, `title` and `year` from the file's own `meta`
    /// block, so offering them here would silently drop them — a blind
    /// passthrough would be wrong, same reasoning as the Python table's
    /// docstring.
    pub fn allowed_options(&self) -> &'static [&'static str] {
        const CHUNKABLE: &[&str] = &[
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
        const SOP_JSON: &[&str] = &[
            "lang",
            "author",
            "corpus",
            "slug",
            "book_pair",
            "acquired_from",
            "rights",
        ];
        match self {
            Kind::Epub | Kind::Markdown | Kind::Text => CHUNKABLE,
            Kind::SopJson => SOP_JSON,
        }
    }
}

impl FromStr for Kind {
    type Err = ();

    fn from_str(s: &str) -> Result<Kind, ()> {
        match s {
            "epub" => Ok(Kind::Epub),
            "markdown" => Ok(Kind::Markdown),
            "text" => Ok(Kind::Text),
            "sop_json" => Ok(Kind::SopJson),
            _ => Err(()),
        }
    }
}

/// Metadata a caller supplies up front (never guessed from the source —
/// rule #9 in the Python reference's docstrings): what the extractor
/// cannot read from the source itself is left `None` for hand entry in the
/// reviewed book.json. `chunk_limits` is not part of `_EXTRACT_OPTS` (the
/// CLI has no flag for it yet) — it is a chunker parameter every kind that
/// chunks paragraphs reads, defaulting to the values in
/// [`ChunkLimits::default`].
#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    pub book_code: Option<String>,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i64>,
    pub corpus: Option<String>,
    pub slug: Option<String>,
    pub book_pair: Option<String>,
    pub acquired_from: Option<String>,
    pub rights: Option<String>,
    pub chunk_limits: ChunkLimits,
}

impl ExtractOptions {
    /// Which fields this instance has actually set, by the same names
    /// `_EXTRACT_OPTS` uses. `chunk_limits` is excluded — it always has a
    /// value and is not part of the accepted-option table.
    fn supplied(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.book_code.is_some() {
            out.push("book_code");
        }
        if self.lang.is_some() {
            out.push("lang");
        }
        if self.title.is_some() {
            out.push("title");
        }
        if self.author.is_some() {
            out.push("author");
        }
        if self.year.is_some() {
            out.push("year");
        }
        if self.corpus.is_some() {
            out.push("corpus");
        }
        if self.slug.is_some() {
            out.push("slug");
        }
        if self.book_pair.is_some() {
            out.push("book_pair");
        }
        if self.acquired_from.is_some() {
            out.push("acquired_from");
        }
        if self.rights.is_some() {
            out.push("rights");
        }
        out
    }
}

/// Refuses (rather than silently drops) any option *opts* sets that *kind*
/// does not accept — port of `sopack.cli._cmd_extract`'s rejection check.
pub fn check_options(kind: Kind, opts: &ExtractOptions) -> Result<(), ExtractError> {
    let allowed = kind.allowed_options();
    let rejected: Vec<&'static str> = opts
        .supplied()
        .into_iter()
        .filter(|o| !allowed.contains(o))
        .collect();
    if rejected.is_empty() {
        return Ok(());
    }
    let flags: Vec<String> = rejected
        .iter()
        .map(|o| format!("--{}", o.replace('_', "-")))
        .collect();
    Err(ExtractError::new(
        ErrorCode::Usage,
        format!(
            "--kind {} does not accept: {}",
            kind.as_str(),
            flags.join(", ")
        ),
    )
    .with_hint(format!(
        "{} takes these from the source file itself",
        kind.as_str()
    ))
    .with_field(rejected.join(",")))
}
