//! sopack-extract — deterministic, NO-LLM source → [`sopack_book::Book`]
//! extraction. Port of `qdrant/sopack/extract/` — see
//! `qdrant/docs/SOPACK-1.0-PLAN.md` §3.1, §3.5, §4, §6.
//!
//! [`extract`] dispatches on [`Kind`]: `epub`, `markdown`, `text`,
//! `sop_json` (`pdf` is left room for — see `Kind`'s doc comment). Every
//! extractor never invents metadata (rule #9) — what it cannot read from
//! the source is left `None` in `Book.book` and reported missing by
//! `sopack_book::validate`.
//!
//! Every extractor produces `Book.stats` with `blocks_in`, `blocks_out`,
//! `dropped`, `damage`, `words` and `dropped_detail` (a list of what was
//! dropped and why — R8, nothing silently discarded).
//!
//! This crate does not implement the CLI binary — `sopack-cli` (built by
//! another agent) wires `sopack extract`/`inspect` over this crate's
//! public API. The API surface is designed for that: structured
//! [`ExtractError`]s with a `code`/`message`/`hint`/`field`, and a
//! [`ProgressSink`] hook for the read/parse/chunk/gate stages.

pub mod chunk;
mod common;
mod epub;
mod error;
mod markdown;
pub mod meta;
mod options;
mod progress;
pub mod propose;
pub mod registry;
mod sop_json;
mod text;

pub use error::{ErrorCode, ExtractError};
pub use meta::{
    check_complete, discover_sidecars, load as load_meta, merge as merge_options, required_fields,
    write_template, MetaFile,
};
pub use options::{check_options, ExtractOptions, Kind};
pub use progress::{NoopProgress, ProgressSink, Stage};
pub use propose::{
    propose, propose_quiet, Candidate, FieldProposal, Fields, Proposal, SourceInfo, FIELD_NAMES,
};
pub use registry::{Registry, RegistryEntry};

// Re-exported so a caller that wants epub-specific regexes for its own
// pre-checks (e.g. a CLI `inspect --epub`) does not have to duplicate them.
pub use epub::{BOILERPLATE_RE, NUMERIC_RE};

use std::path::Path;

use sopack_book::Book;

/// Extract *source_path* (any [`Kind`]) into a reviewable
/// [`sopack_book::Book`]. Enforces the per-kind accepted-option table (see
/// [`check_options`]) before dispatching, then runs that kind's extractor
/// with *progress* as its stage hook (pass [`NoopProgress`] if you do not
/// care).
pub fn extract(
    source_path: &Path,
    kind: Kind,
    opts: &ExtractOptions,
    progress: &mut dyn ProgressSink,
) -> Result<Book, ExtractError> {
    check_options(kind, opts)?;
    match kind {
        Kind::Epub => epub::extract(source_path, opts, progress),
        Kind::Markdown => markdown::extract(source_path, opts, progress),
        Kind::Text => text::extract(source_path, opts, progress),
        Kind::SopJson => sop_json::extract(source_path, opts, progress),
    }
}
