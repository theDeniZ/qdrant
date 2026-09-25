//! sopack-progress — the shared progress engine every long-running sopack
//! command reports through (SOPACK-1.0-PLAN.md §3.1, §3.4).
//!
//! One [`Progress`] drives a whole command. Stages are declared up front as
//! `Vec<StageWeight>` (name + relative weight), so an overall percentage
//! exists even before the first stage's total is known; each stage then
//! reports its own `total`/`done` in a [`Unit`] (bytes, items, blocks,
//! tokens or checks) through the [`ProgressSink`] trait.
//!
//! ```
//! use sopack_progress::{Mode, Progress, ProgressSink, StageWeight, Unit};
//!
//! let progress = Progress::with_mode(
//!     vec![StageWeight::new("tokenise", 1.0), StageWeight::new("embed", 4.0)],
//!     Mode::Plain,
//! );
//! progress.stage_start("tokenise", 100, Unit::Blocks);
//! progress.advance(100);
//! progress.stage_end();
//! progress.stage_start("embed", 1000, Unit::Tokens);
//! progress.advance(1000);
//! progress.stage_end();
//! progress.done();
//! assert_eq!(progress.pct(), 100.0);
//! ```
//!
//! Output mode ([`Mode`]) is auto-detected (`Mode::detect`: a TTY gets a
//! live `indicatif` bar, anything else gets throttled plain lines) or
//! forced with `Mode::Json` for NDJSON on stderr — see `event` for the
//! event shapes and `qdrant/sopack-rs/schemas/progress-event.v1.json` for
//! the committed JSON Schema they follow. [`NullSink`] is a no-op
//! `ProgressSink` for callers that don't want progress reporting.
//!
//! Thread safety: the embedding pipeline reports from worker threads
//! (tokenising the next batch while ONNX Runtime runs the current one), so
//! every `ProgressSink` method takes `&self` and [`Progress`] is
//! `Send + Sync`.

mod clock;
mod engine;
mod event;
mod mode;
mod plan;
mod sink;
mod unit;

pub use engine::Progress;
pub use mode::Mode;
pub use plan::StageWeight;
pub use sink::{NullSink, ProgressSink};
pub use unit::Unit;

/// The NDJSON event builders used by `--progress json`
/// (SOPACK-1.0-PLAN.md §3.4). Public so a caller assembling its own JSON
/// output (e.g. `sopack commands --json`) can reuse the same schema
/// version constant and shapes.
pub mod events {
    pub use crate::event::{done, progress, stage_end, stage_start, warning, SCHEMA_VERSION};
}
