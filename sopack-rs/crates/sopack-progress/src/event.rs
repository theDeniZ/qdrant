//! NDJSON event shapes for `--progress json` (SOPACK-1.0-PLAN.md §3.4).
//! Each event is a flat JSON object carrying `"v": 1`
//! (`SCHEMA_VERSION`) and an `"event"` discriminator. The shapes here and
//! `qdrant/sopack-rs/schemas/progress-event.v1.json` must be kept in sync —
//! `tests::events_match_schema_shape` is the tripwire.

use crate::unit::Unit;
use serde_json::{json, Value};

/// Bump only when adding a new event *shape* that an old consumer could not
/// safely ignore. A new optional field does not need a bump.
pub const SCHEMA_VERSION: u32 = 1;

pub fn stage_start(stage: &str, total: u64, unit: Unit) -> Value {
    json!({"v": SCHEMA_VERSION, "event": "stage_start", "stage": stage, "total": total, "unit": unit.as_str()})
}

#[allow(clippy::too_many_arguments)]
pub fn progress(
    stage: &str,
    done: u64,
    total: u64,
    unit: Unit,
    pct: f64,
    rate_per_s: Option<f64>,
    eta_s: Option<f64>,
) -> Value {
    json!({
        "v": SCHEMA_VERSION,
        "event": "progress",
        "stage": stage,
        "done": done,
        "total": total,
        "unit": unit.as_str(),
        "pct": round2(pct),
        "rate_per_s": rate_per_s.map(round2),
        "eta_s": eta_s.map(round2),
    })
}

pub fn stage_end(stage: &str, done: u64, total: u64, unit: Unit, elapsed_s: f64) -> Value {
    json!({
        "v": SCHEMA_VERSION,
        "event": "stage_end",
        "stage": stage,
        "done": done,
        "total": total,
        "unit": unit.as_str(),
        "elapsed_s": round2(elapsed_s),
    })
}

pub fn warning(stage: Option<&str>, message: &str) -> Value {
    json!({"v": SCHEMA_VERSION, "event": "warning", "stage": stage, "message": message})
}

pub fn done(elapsed_s: f64, pct: f64) -> Value {
    json!({"v": SCHEMA_VERSION, "event": "done", "elapsed_s": round2(elapsed_s), "pct": round2(pct)})
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required_keys(event: &str) -> &'static [&'static str] {
        match event {
            "stage_start" => &["v", "event", "stage", "total", "unit"],
            "progress" => &["v", "event", "stage", "done", "total", "unit", "pct"],
            "stage_end" => &["v", "event", "stage", "done", "total", "unit", "elapsed_s"],
            "warning" => &["v", "event", "message"],
            "done" => &["v", "event", "elapsed_s", "pct"],
            _ => panic!("unknown event {event}"),
        }
    }

    #[test]
    fn events_match_schema_shape() {
        let events = [
            stage_start("embed", 210_000, Unit::Tokens),
            progress(
                "embed",
                81_234,
                210_000,
                Unit::Tokens,
                38.7,
                Some(660.5),
                Some(412.0),
            ),
            stage_end("embed", 210_000, 210_000, Unit::Tokens, 123.4),
            warning(Some("embed"), "low cosine"),
            done(611.2, 100.0),
        ];
        for ev in events {
            let name = ev["event"].as_str().unwrap().to_string();
            assert_eq!(ev["v"], SCHEMA_VERSION);
            for key in required_keys(&name) {
                assert!(ev.get(*key).is_some(), "{name} event missing {key}: {ev}");
            }
        }
    }

    #[test]
    fn progress_event_matches_plan_example() {
        // SOPACK-1.0-PLAN.md §3.4's literal example (bar aside — v/pct rounding
        // is this crate's own addition on top of it).
        let ev = progress(
            "embed",
            81234,
            210000,
            Unit::Tokens,
            38.7,
            None,
            Some(412.0),
        );
        assert_eq!(ev["event"], "progress");
        assert_eq!(ev["stage"], "embed");
        assert_eq!(ev["done"], 81234);
        assert_eq!(ev["total"], 210000);
        assert_eq!(ev["unit"], "tokens");
        assert_eq!(ev["pct"], 38.7);
        assert_eq!(ev["eta_s"], 412.0);
    }

    #[test]
    fn warning_without_a_stage_is_null_not_missing() {
        let ev = warning(None, "no fixture entries for this profile");
        assert!(ev["stage"].is_null());
    }
}
