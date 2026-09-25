//! Adapts `sopack_extract::ProgressSink` (a `&mut self`, `Option<u64>`-total
//! trait local to that crate — see its module docs on why it doesn't depend
//! on `sopack-progress`) onto the shared `sopack_progress::ProgressSink`
//! (a `&self`, `u64`-total trait) every other command reports through, so
//! `extract`/`propose` render with the exact same bar/NDJSON/plain-line
//! machinery as `pack`/`verify`/`doctor`.

use std::cell::Cell;

use sopack_extract::{ProgressSink as ExtractSink, Stage};
use sopack_progress::{ProgressSink as SharedSink, Unit};

pub struct ExtractProgressAdapter<'a> {
    inner: &'a dyn SharedSink,
    last_done: Cell<u64>,
}

impl<'a> ExtractProgressAdapter<'a> {
    pub fn new(inner: &'a dyn SharedSink) -> Self {
        ExtractProgressAdapter {
            inner,
            last_done: Cell::new(0),
        }
    }
}

impl ExtractSink for ExtractProgressAdapter<'_> {
    fn stage_start(&mut self, stage: Stage, total: Option<u64>) {
        self.last_done.set(0);
        self.inner
            .stage_start(stage.as_str(), total.unwrap_or(0), Unit::Items);
    }

    fn stage_progress(&mut self, _stage: Stage, done: u64, _total: Option<u64>) {
        let prev = self.last_done.replace(done);
        if done > prev {
            self.inner.advance(done - prev);
        }
    }

    fn stage_end(&mut self, _stage: Stage) {
        self.inner.stage_end();
    }

    fn warning(&mut self, message: &str) {
        self.inner.warn(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        starts: Mutex<Vec<(String, u64)>>,
        advanced: AtomicU64,
        ends: AtomicU64,
        warnings: Mutex<Vec<String>>,
    }

    impl SharedSink for Recorder {
        fn stage_start(&self, stage: &str, total: u64, _unit: Unit) {
            self.starts.lock().unwrap().push((stage.to_string(), total));
        }
        fn advance(&self, n: u64) {
            self.advanced.fetch_add(n, Ordering::SeqCst);
        }
        fn warn(&self, msg: &str) {
            self.warnings.lock().unwrap().push(msg.to_string());
        }
        fn stage_end(&self) {
            self.ends.fetch_add(1, Ordering::SeqCst);
        }
        fn done(&self) {}
    }

    #[test]
    fn translates_absolute_progress_into_deltas() {
        let rec = Recorder::default();
        let mut adapter = ExtractProgressAdapter::new(&rec);
        adapter.stage_start(Stage::ParseSpineItems, Some(10));
        adapter.stage_progress(Stage::ParseSpineItems, 3, Some(10));
        adapter.stage_progress(Stage::ParseSpineItems, 7, Some(10));
        adapter.stage_progress(Stage::ParseSpineItems, 10, Some(10));
        adapter.stage_end(Stage::ParseSpineItems);
        assert_eq!(rec.advanced.load(Ordering::SeqCst), 10);
        assert_eq!(rec.ends.load(Ordering::SeqCst), 1);
        assert_eq!(rec.starts.lock().unwrap()[0].0, "parse_spine_items");
    }

    #[test]
    fn warnings_pass_through() {
        let rec = Recorder::default();
        let mut adapter = ExtractProgressAdapter::new(&rec);
        adapter.warning("low signal");
        assert_eq!(rec.warnings.lock().unwrap()[0], "low signal");
    }
}
