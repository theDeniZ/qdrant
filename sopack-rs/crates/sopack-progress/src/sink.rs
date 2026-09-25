//! `ProgressSink` — the adapter trait every long-running sopack command
//! reports through (SOPACK-1.0-PLAN.md §3.4). `Progress` (`engine.rs`) is
//! the concrete, mode-aware implementation; `NullSink` is for callers (and
//! tests) that don't want progress at all.

use crate::unit::Unit;

/// Must be `Send + Sync`: the embedding pipeline reports from worker
/// threads (tokenising batch N+1 while batch N runs through ONNX Runtime),
/// so every implementation has to tolerate concurrent calls.
pub trait ProgressSink: Send + Sync {
    /// Begins a new stage with a known total and unit. Implementations
    /// should treat this as also ending whatever stage was previously
    /// open, without requiring a separate `stage_end()` call first — a
    /// caller that forgets to call `stage_end()` before starting the next
    /// stage must not corrupt the overall percentage.
    fn stage_start(&self, stage: &str, total: u64, unit: Unit);

    /// Advances the current stage's `done` counter by `n` units.
    fn advance(&self, n: u64);

    /// A non-fatal problem worth surfacing without stopping the command.
    fn warn(&self, msg: &str);

    /// Marks the current stage as fully done (`done = total`).
    fn stage_end(&self);

    /// Marks the whole command as finished. Guarantees the overall
    /// percentage a caller last observed is exactly 100 after this call.
    fn done(&self);
}

/// A `ProgressSink` that does nothing — for library callers (and tests)
/// that don't want progress reporting at all.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl ProgressSink for NullSink {
    fn stage_start(&self, _stage: &str, _total: u64, _unit: Unit) {}
    fn advance(&self, _n: u64) {}
    fn warn(&self, _msg: &str) {}
    fn stage_end(&self) {}
    fn done(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_never_panics() {
        let s = NullSink;
        s.stage_start("x", 10, Unit::Items);
        s.advance(3);
        s.warn("hi");
        s.stage_end();
        s.done();
    }
}
