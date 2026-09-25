//! `Progress` — the concrete, mode-aware `ProgressSink` (SOPACK-1.0-PLAN.md
//! §3.4). One instance drives a whole command: stages are pre-declared with
//! weights (`StageWeight`), their totals/units arrive as each stage starts,
//! and the overall percentage is a weighted composite across the plan that
//! is guaranteed monotonic and lands on exactly 100 when `done()` is
//! called.

use crate::clock::{Clock, SystemClock};
use crate::event;
use crate::mode::Mode;
use crate::plan::StageWeight;
use crate::sink::ProgressSink;
use crate::unit::Unit;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct StageRuntime {
    name: String,
    weight: f64,
    total: u64,
    done: u64,
    unit: Unit,
    started_at: Option<Instant>,
    ended: bool,
}

/// What one `advance()` call needs to emit, computed once under the lock so
/// the emission code below runs lock-free.
struct Snapshot {
    stage: String,
    done: u64,
    total: u64,
    unit: Unit,
    pct: f64,
    rate: Option<f64>,
    eta: Option<f64>,
    emit_plain: bool,
}

struct State {
    stages: Vec<StageRuntime>,
    current: Option<usize>,
    total_weight: f64,
    completed_weight: f64,
    overall_start: Instant,
    last_pct: f64,
    finished: bool,
    plain_last_emit: Instant,
    plain_last_pct: f64,
}

impl State {
    fn find_or_add_stage(&mut self, name: &str) -> usize {
        if let Some(i) = self.stages.iter().position(|s| s.name == name) {
            i
        } else {
            // An unplanned stage (not in the `Vec<StageWeight>` the caller
            // declared up front) is accepted rather than rejected — it just
            // carries zero weight, so it reports its own done/total in
            // every event but never moves the overall percentage. A
            // correctly written caller never hits this path; it exists so
            // a bug in the caller's plan degrades gracefully instead of
            // panicking mid-command.
            self.stages.push(StageRuntime {
                name: name.to_string(),
                weight: 0.0,
                total: 0,
                done: 0,
                unit: Unit::Items,
                started_at: None,
                ended: false,
            });
            self.stages.len() - 1
        }
    }

    fn start_stage(&mut self, name: &str, total: u64, unit: Unit, now: Instant) {
        let idx = self.find_or_add_stage(name);
        if self.stages[idx].ended {
            // Restarting a stage that already ended (a retry) — undo its
            // earlier contribution to `completed_weight` first.
            self.completed_weight -= self.stages[idx].weight;
        }
        if let Some(cur) = self.current {
            if cur != idx && !self.stages[cur].ended {
                // The caller started a new stage without ending the last
                // one; close it so its weight isn't silently lost from the
                // overall percentage.
                self.completed_weight += self.stages[cur].weight;
                self.stages[cur].ended = true;
            }
        }
        let s = &mut self.stages[idx];
        s.total = total;
        s.done = 0;
        s.unit = unit;
        s.started_at = Some(now);
        s.ended = false;
        self.current = Some(idx);
    }

    fn advance(
        &mut self,
        n: u64,
        now: Instant,
        min_interval: Duration,
        min_pct_delta: f64,
    ) -> Option<Snapshot> {
        let idx = self.current?;
        {
            let s = &mut self.stages[idx];
            s.done = s.done.saturating_add(n);
            if s.total > 0 {
                s.done = s.done.min(s.total);
            }
        }
        let raw_pct = self.overall_pct();
        let pct = raw_pct.max(self.last_pct);
        self.last_pct = pct;
        let (rate, eta) = Self::stage_rate_eta(&self.stages[idx], now);
        let emit_plain = now.saturating_duration_since(self.plain_last_emit) >= min_interval
            || (pct - self.plain_last_pct) >= min_pct_delta;
        if emit_plain {
            self.plain_last_emit = now;
            self.plain_last_pct = pct;
        }
        let s = &self.stages[idx];
        Some(Snapshot {
            stage: s.name.clone(),
            done: s.done,
            total: s.total,
            unit: s.unit,
            pct,
            rate,
            eta,
            emit_plain,
        })
    }

    fn end_stage(&mut self, now: Instant) -> Option<(String, u64, u64, Unit, f64)> {
        let idx = self.current.take()?;
        let elapsed = {
            let s = &mut self.stages[idx];
            if s.total > 0 {
                s.done = s.total;
            }
            s.ended = true;
            now.saturating_duration_since(s.started_at.unwrap_or(now))
                .as_secs_f64()
        };
        self.completed_weight += self.stages[idx].weight;
        let raw_pct = self.overall_pct();
        self.last_pct = raw_pct.max(self.last_pct);
        let s = &self.stages[idx];
        Some((s.name.clone(), s.done, s.total, s.unit, elapsed))
    }

    fn finish(&mut self, now: Instant) -> f64 {
        if let Some(idx) = self.current {
            if !self.stages[idx].ended {
                let s = &mut self.stages[idx];
                if s.total > 0 {
                    s.done = s.total;
                }
                s.ended = true;
                self.completed_weight += s.weight;
            }
        }
        self.current = None;
        self.finished = true;
        self.last_pct = 100.0;
        now.saturating_duration_since(self.overall_start)
            .as_secs_f64()
    }

    fn overall_pct(&self) -> f64 {
        if self.finished {
            return 100.0;
        }
        if self.total_weight <= 0.0 {
            return 0.0;
        }
        let mut pct = (self.completed_weight / self.total_weight) * 100.0;
        if let Some(idx) = self.current {
            let s = &self.stages[idx];
            if !s.ended {
                let frac = if s.total > 0 {
                    (s.done as f64 / s.total as f64).min(1.0)
                } else {
                    0.0
                };
                pct += (s.weight / self.total_weight) * 100.0 * frac;
            }
        }
        pct.clamp(0.0, 100.0)
    }

    fn stage_rate_eta(s: &StageRuntime, now: Instant) -> (Option<f64>, Option<f64>) {
        let Some(started) = s.started_at else {
            return (None, None);
        };
        let elapsed = now.saturating_duration_since(started).as_secs_f64();
        if elapsed <= 0.0 || s.done == 0 {
            return (None, None);
        }
        let rate = s.done as f64 / elapsed;
        let eta = if s.total > s.done && rate > 0.0 {
            Some((s.total - s.done) as f64 / rate)
        } else {
            Some(0.0)
        };
        (Some(rate), eta)
    }
}

/// The shared, mode-aware progress engine. Cheap to share across threads
/// via `Arc<Progress>` (it is `Send + Sync`) — every `ProgressSink` method
/// takes `&self`.
pub struct Progress {
    state: Mutex<State>,
    mode: Mode,
    writer: Option<Mutex<Box<dyn Write + Send>>>,
    bar: Option<ProgressBar>,
    clock: Box<dyn Clock>,
    plain_min_interval: Duration,
    plain_min_pct_delta: f64,
}

impl Progress {
    /// Auto-detects the mode (`Mode::detect`) — `sopack-cli` overrides this
    /// with an explicit `--progress` value when one was given.
    pub fn new(plan: Vec<StageWeight>) -> Self {
        Self::with_mode(plan, Mode::detect())
    }

    pub fn with_mode(plan: Vec<StageWeight>, mode: Mode) -> Self {
        Self::build(plan, mode, Box::new(SystemClock), Box::new(io::stderr()))
    }

    fn build(
        plan: Vec<StageWeight>,
        mode: Mode,
        clock: Box<dyn Clock>,
        writer: Box<dyn Write + Send>,
    ) -> Self {
        let total_weight = plan.iter().map(|s| s.weight).sum();
        let stages = plan
            .into_iter()
            .map(|p| StageRuntime {
                name: p.name.to_string(),
                weight: p.weight,
                total: 0,
                done: 0,
                unit: Unit::Items,
                started_at: None,
                ended: false,
            })
            .collect();
        let now = clock.now();
        let state = State {
            stages,
            current: None,
            total_weight,
            completed_weight: 0.0,
            overall_start: now,
            last_pct: 0.0,
            finished: false,
            plain_last_emit: now,
            plain_last_pct: 0.0,
        };
        let bar = if mode == Mode::Tty {
            let b = ProgressBar::new(10_000);
            if let Ok(style) = ProgressStyle::with_template("{msg}") {
                b.set_style(style);
            }
            Some(b)
        } else {
            None
        };
        Progress {
            state: Mutex::new(state),
            mode,
            writer: if mode == Mode::Tty {
                None
            } else {
                Some(Mutex::new(writer))
            },
            bar,
            clock,
            plain_min_interval: Duration::from_secs(5),
            plain_min_pct_delta: 5.0,
        }
    }

    /// The last overall percentage reported (monotonic, 0..=100).
    pub fn pct(&self) -> f64 {
        self.state.lock().unwrap().last_pct
    }

    pub fn is_finished(&self) -> bool {
        self.state.lock().unwrap().finished
    }

    fn write_line(&self, line: &str) {
        if let Some(w) = &self.writer {
            let mut w = w.lock().unwrap();
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

fn fmt_eta(eta: Option<f64>) -> String {
    match eta {
        Some(s) if s.is_finite() => format!(" eta={s:.0}s"),
        _ => String::new(),
    }
}

impl ProgressSink for Progress {
    fn stage_start(&self, stage: &str, total: u64, unit: Unit) {
        let now = self.clock.now();
        self.state
            .lock()
            .unwrap()
            .start_stage(stage, total, unit, now);
        match self.mode {
            Mode::Json => self.write_line(&event::stage_start(stage, total, unit).to_string()),
            Mode::Plain => self.write_line(&format!("[{stage}] starting (0/{total} {unit})")),
            Mode::Tty => {
                if let Some(b) = &self.bar {
                    b.set_message(format!("{stage}: starting"));
                }
            }
        }
    }

    fn advance(&self, n: u64) {
        let now = self.clock.now();
        let snap = {
            self.state.lock().unwrap().advance(
                n,
                now,
                self.plain_min_interval,
                self.plain_min_pct_delta,
            )
        };
        let Some(snap) = snap else { return };
        match self.mode {
            Mode::Json => self.write_line(
                &event::progress(
                    &snap.stage,
                    snap.done,
                    snap.total,
                    snap.unit,
                    snap.pct,
                    snap.rate,
                    snap.eta,
                )
                .to_string(),
            ),
            Mode::Plain => {
                if snap.emit_plain {
                    self.write_line(&format!(
                        "[{}] {:.1}% ({}/{} {}){}",
                        snap.stage,
                        snap.pct,
                        snap.done,
                        snap.total,
                        snap.unit,
                        fmt_eta(snap.eta)
                    ));
                }
            }
            Mode::Tty => {
                if let Some(b) = &self.bar {
                    b.set_position((snap.pct * 100.0).round() as u64);
                    b.set_message(format!(
                        "{} {:.1}% ({}/{} {}){}",
                        snap.stage,
                        snap.pct,
                        snap.done,
                        snap.total,
                        snap.unit,
                        fmt_eta(snap.eta)
                    ));
                }
            }
        }
    }

    fn warn(&self, msg: &str) {
        let stage = {
            let st = self.state.lock().unwrap();
            st.current.map(|i| st.stages[i].name.clone())
        };
        match self.mode {
            Mode::Json => self.write_line(&event::warning(stage.as_deref(), msg).to_string()),
            Mode::Plain => self.write_line(&format!(
                "warning: {}{}",
                stage
                    .as_ref()
                    .map(|s| format!("[{s}] "))
                    .unwrap_or_default(),
                msg
            )),
            Mode::Tty => {
                if let Some(b) = &self.bar {
                    b.println(format!("warning: {msg}"));
                } else {
                    eprintln!("warning: {msg}");
                }
            }
        }
    }

    fn stage_end(&self) {
        let now = self.clock.now();
        let ended = { self.state.lock().unwrap().end_stage(now) };
        let Some((stage, done, total, unit, elapsed)) = ended else {
            return;
        };
        match self.mode {
            Mode::Json => {
                self.write_line(&event::stage_end(&stage, done, total, unit, elapsed).to_string())
            }
            Mode::Plain => self.write_line(&format!(
                "[{stage}] done ({done}/{total} {unit}) in {elapsed:.1}s"
            )),
            Mode::Tty => {
                if let Some(b) = &self.bar {
                    b.println(format!("[{stage}] done in {elapsed:.1}s"));
                }
            }
        }
    }

    fn done(&self) {
        let now = self.clock.now();
        let elapsed = { self.state.lock().unwrap().finish(now) };
        match self.mode {
            Mode::Json => self.write_line(&event::done(elapsed, 100.0).to_string()),
            Mode::Plain => self.write_line(&format!("done in {elapsed:.1}s")),
            Mode::Tty => {
                if let Some(b) = &self.bar {
                    b.finish_with_message(format!("done in {elapsed:.1}s"));
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Arc;

    /// A `Write` sink backed by a shared buffer, so a test can build a
    /// `Progress` and then read back every line it emitted.
    #[derive(Clone)]
    pub struct SharedBuf(pub Arc<Mutex<Vec<u8>>>);

    impl SharedBuf {
        pub fn new() -> Self {
            SharedBuf(Arc::new(Mutex::new(Vec::new())))
        }

        pub fn lines(&self) -> Vec<String> {
            let buf = self.0.lock().unwrap();
            String::from_utf8_lossy(&buf)
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Progress {
        /// Test-only constructor: explicit mode, clock and writer, so
        /// output can be captured and time can be advanced without
        /// sleeping.
        pub fn for_test(
            plan: Vec<StageWeight>,
            mode: Mode,
            clock: Box<dyn Clock>,
            buf: SharedBuf,
        ) -> Self {
            Progress::build(plan, mode, clock, Box::new(buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::SharedBuf;
    use super::*;
    use crate::clock::fake::FakeClock;
    use std::sync::Arc;

    fn plan() -> Vec<StageWeight> {
        vec![
            StageWeight::new("tokenise", 1.0),
            StageWeight::new("embed", 4.0),
            StageWeight::new("write", 0.5),
        ]
    }

    #[test]
    fn pct_is_zero_before_anything_starts() {
        let p = Progress::for_test(
            plan(),
            Mode::Json,
            Box::new(FakeClock::new()),
            SharedBuf::new(),
        );
        assert_eq!(p.pct(), 0.0);
    }

    #[test]
    fn pct_reaches_exactly_100_after_a_full_run() {
        let p = Progress::for_test(
            plan(),
            Mode::Json,
            Box::new(FakeClock::new()),
            SharedBuf::new(),
        );
        p.stage_start("tokenise", 100, Unit::Blocks);
        p.advance(100);
        p.stage_end();
        p.stage_start("embed", 1000, Unit::Tokens);
        p.advance(1000);
        p.stage_end();
        p.stage_start("write", 1, Unit::Items);
        p.advance(1);
        p.stage_end();
        p.done();
        assert_eq!(p.pct(), 100.0);
        assert!(p.is_finished());
    }

    #[test]
    fn done_forces_100_even_if_a_stage_is_left_short() {
        let p = Progress::for_test(
            plan(),
            Mode::Json,
            Box::new(FakeClock::new()),
            SharedBuf::new(),
        );
        p.stage_start("embed", 1000, Unit::Tokens);
        p.advance(3); // far short of 1000
        p.done();
        assert_eq!(p.pct(), 100.0);
    }

    #[test]
    fn pct_is_monotonic_across_many_updates() {
        let p = Progress::for_test(
            plan(),
            Mode::Json,
            Box::new(FakeClock::new()),
            SharedBuf::new(),
        );
        let mut last = 0.0f64;
        p.stage_start("tokenise", 50, Unit::Blocks);
        for _ in 0..50 {
            p.advance(1);
            let now = p.pct();
            assert!(now >= last, "pct went backwards: {now} < {last}");
            last = now;
        }
        p.stage_end();
        p.stage_start("embed", 200, Unit::Tokens);
        for _ in 0..200 {
            p.advance(1);
            let now = p.pct();
            assert!(now >= last, "pct went backwards: {now} < {last}");
            last = now;
        }
        p.stage_end();
        p.stage_start("write", 1, Unit::Items);
        p.advance(1);
        p.stage_end();
        p.done();
        assert_eq!(p.pct(), 100.0);
        assert!(last <= 100.0);
    }

    #[test]
    fn json_events_are_well_formed_ndjson() {
        let buf = SharedBuf::new();
        let p = Progress::for_test(plan(), Mode::Json, Box::new(FakeClock::new()), buf.clone());
        p.stage_start("embed", 10, Unit::Tokens);
        p.advance(5);
        p.warn("half way, just checking");
        p.stage_end();
        p.done();
        let lines = buf.lines();
        assert!(
            lines.len() >= 4,
            "expected at least start/progress/warning/stage_end/done, got {lines:?}"
        );
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("not valid JSON: {line} ({e})"));
            assert_eq!(v["v"], 1);
            assert!(v["event"].is_string());
        }
        let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(last["event"], "done");
        assert_eq!(last["pct"], 100.0);
    }

    #[test]
    fn plain_mode_emits_no_carriage_returns() {
        let buf = SharedBuf::new();
        let p = Progress::for_test(plan(), Mode::Plain, Box::new(FakeClock::new()), buf.clone());
        p.stage_start("embed", 10, Unit::Tokens);
        p.advance(10);
        p.stage_end();
        p.done();
        let raw = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            !raw.contains('\r'),
            "plain mode must not use carriage returns: {raw:?}"
        );
    }

    #[test]
    fn plain_mode_throttles_by_time_not_every_advance() {
        let buf = SharedBuf::new();
        let clock = Arc::new(FakeClock::new());
        // Progress owns a `Box<dyn Clock>`, so hand it a boxed clone-alike
        // that shares the same underlying FakeClock via the Arc.
        struct Shared(Arc<FakeClock>);
        impl Clock for Shared {
            fn now(&self) -> Instant {
                self.0.now()
            }
        }
        let p = Progress::for_test(
            plan(),
            Mode::Plain,
            Box::new(Shared(clock.clone())),
            buf.clone(),
        );
        p.stage_start("embed", 1_000_000, Unit::Tokens);
        // Many tiny advances within the same instant / well under the 5%
        // threshold should collapse to very few emitted lines, not one per call.
        for _ in 0..1000 {
            p.advance(1);
        }
        let lines_before = buf.lines().len();
        clock.advance(Duration::from_secs(6));
        p.advance(1);
        let lines_after = buf.lines().len();
        assert!(
            lines_after > lines_before,
            "an update past the time threshold should emit a new line"
        );
        assert!(lines_before < 1000, "plain mode should throttle, not emit one line per advance() call: {lines_before} lines for 1000 calls");
    }

    #[test]
    fn unplanned_stage_does_not_panic_and_reports_its_own_progress() {
        let buf = SharedBuf::new();
        let p = Progress::for_test(
            vec![StageWeight::new("only", 1.0)],
            Mode::Json,
            Box::new(FakeClock::new()),
            buf.clone(),
        );
        p.stage_start("surprise", 10, Unit::Checks);
        p.advance(10);
        p.stage_end();
        // Zero weight: an unplanned stage doesn't move the overall pct.
        assert_eq!(p.pct(), 0.0);
        let lines = buf.lines();
        let progress_line = lines
            .iter()
            .find(|l| l.contains("\"event\":\"progress\""))
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(progress_line).unwrap();
        assert_eq!(v["stage"], "surprise");
        assert_eq!(v["done"], 10);
    }
}
