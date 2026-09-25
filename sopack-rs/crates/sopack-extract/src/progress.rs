//! A minimal progress hook every extractor reports through. Kept as a
//! plain trait in this crate (no dependency on `sopack-progress`, which
//! adapts it) so `sopack-extract` stays a leaf with respect to progress
//! rendering, per `docs/SOPACK-1.0-PLAN.md` §3.4/§3.1.

/// The four stages the plan calls out for `extract`: opening/reading the
/// source container, resolving its parseable items (EPUB spine documents;
/// for markdown/text/sop_json, the one file itself), chunking overlong
/// paragraphs, and running the quality gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    ReadContainer,
    ParseSpineItems,
    Chunk,
    Gate,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::ReadContainer => "read_container",
            Stage::ParseSpineItems => "parse_spine_items",
            Stage::Chunk => "chunk",
            Stage::Gate => "gate",
        }
    }
}

/// Receives progress events for one `extract` call. Default methods are
/// no-ops, so a caller that only cares about one stage can override just
/// that method.
pub trait ProgressSink {
    fn stage_start(&mut self, _stage: Stage, _total: Option<u64>) {}
    fn stage_progress(&mut self, _stage: Stage, _done: u64, _total: Option<u64>) {}
    fn stage_end(&mut self, _stage: Stage) {}
    fn warning(&mut self, _message: &str) {}
}

/// The default sink: discards every event. What every extractor uses when
/// a caller (a test, a library user who does not care) passes nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopProgress;

impl ProgressSink for NoopProgress {}
