//! Calibration fixture (`sopack.calibration/1`, SOPACK-2-FORMAT.md §3): real
//! corpus texts with the vectors the live collection had for them when the
//! fixture was made. `score` embeds `passage_prefix + text` for every entry
//! with the caller's engine and reports per-entry cosine, min, mean — the
//! pass/fail decision (`pack_min_cosine`, `probe_min_cosine`, …) is the
//! caller's, since which threshold applies depends on who is asking
//! (a pack-time self-check vs. a device-fallback probe vs. the importer).

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalibrationEntry {
    pub id: String,
    pub profile: String,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub text: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalibrationFixture {
    pub schema: String,
    pub contract: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
    pub entries: Vec<CalibrationEntry>,
}

impl CalibrationFixture {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// sha256 of the exact file bytes — what a pack records in
    /// `manifest.contract.calibration_sha256` / `probe.fixture_sha256`
    /// (SOPACK-2-FORMAT.md §2/§3), and what `contract.toml`'s
    /// `[calibration].sha256` is filled in with when the fixture is made.
    pub fn sha256_of_file(path: &Path) -> Result<String> {
        crate::verify::sha256_file(path, &sopack_progress::NullSink)
    }
}

/// Per-entry and summary cosine scores from embedding a `CalibrationFixture`
/// with some engine — `Engine::calibrate`'s output, and what a pack's
/// `manifest.probe.self_check` (SOPACK-2-FORMAT.md §2) is built from.
#[derive(Debug, Clone, Serialize)]
pub struct CalibrationResult {
    pub n: usize,
    pub min_cosine: f64,
    pub mean_cosine: f64,
    pub per_entry: Vec<(String, f64)>,
    /// The pack's own embeddings of the fixture entries, fixture order —
    /// what a `.sopack`'s `probe.f32` is written from.
    pub probe_vectors: Vec<Vec<f32>>,
}

impl CalibrationResult {
    pub fn passes(&self, threshold: f64) -> bool {
        self.min_cosine >= threshold
    }

    /// The lowest-scoring entries, worst first — useful in an error message
    /// or a `doctor` report so a failure names *which* text drifted instead
    /// of just the aggregate number.
    pub fn worst(&self, n: usize) -> Vec<(String, f64)> {
        let mut v = self.per_entry.clone();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(n);
        v
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa == 0.0 || bb == 0.0 {
        0.0
    } else {
        ab / (aa.sqrt() * bb.sqrt())
    }
}

/// Scores `produced` (one vector per `fixture.entries`, same order) against
/// the fixture's stored vectors.
pub fn score(fixture: &CalibrationFixture, produced: &[Vec<f32>]) -> CalibrationResult {
    assert_eq!(
        fixture.entries.len(),
        produced.len(),
        "one produced vector is required per fixture entry"
    );
    let per_entry: Vec<(String, f64)> = fixture
        .entries
        .iter()
        .zip(produced)
        .map(|(e, v)| (e.id.clone(), cosine(v, &e.vector)))
        .collect();
    let n = per_entry.len();
    let min_cosine = per_entry
        .iter()
        .map(|(_, c)| *c)
        .fold(f64::INFINITY, f64::min);
    let mean_cosine = if n == 0 {
        0.0
    } else {
        per_entry.iter().map(|(_, c)| *c).sum::<f64>() / n as f64
    };
    CalibrationResult {
        n,
        min_cosine,
        mean_cosine,
        per_entry,
        probe_vectors: produced.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(entries: Vec<(&str, Vec<f32>)>) -> CalibrationFixture {
        CalibrationFixture {
            schema: "sopack.calibration/1".into(),
            contract: "e5-large-v1".into(),
            created_at: None,
            source: None,
            entries: entries
                .into_iter()
                .map(|(id, vector)| CalibrationEntry {
                    id: id.to_string(),
                    profile: "sop".into(),
                    uid: None,
                    lang: None,
                    note: None,
                    text: format!("text for {id}"),
                    vector,
                })
                .collect(),
        }
    }

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_of_opposite_vectors_is_minus_one() {
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn cosine_handles_a_zero_vector_without_dividing_by_zero() {
        let c = cosine(&[0.0, 0.0], &[1.0, 1.0]);
        assert!(c.is_finite());
        assert_eq!(c, 0.0);
    }

    #[test]
    fn score_reports_min_and_mean_across_entries() {
        let fx = fixture(vec![("a", vec![1.0, 0.0]), ("b", vec![1.0, 0.0])]);
        // "a" reproduced perfectly, "b" reproduced with drift.
        let produced = vec![vec![1.0, 0.0], vec![0.9, 0.436]];
        let result = score(&fx, &produced);
        assert_eq!(result.n, 2);
        assert!((result.min_cosine - result.per_entry[1].1).abs() < 1e-9);
        assert!(result.min_cosine < result.mean_cosine);
    }

    #[test]
    fn passes_compares_min_cosine_to_the_threshold() {
        let fx = fixture(vec![("a", vec![1.0, 0.0])]);
        let result = score(&fx, &[vec![1.0, 0.0]]);
        assert!(result.passes(0.9999));
        assert!(!result.passes(1.0 + 1e-9));
    }

    #[test]
    fn worst_sorts_ascending_and_truncates() {
        let fx = fixture(vec![
            ("a", vec![1.0, 0.0]),
            ("b", vec![1.0, 0.0]),
            ("c", vec![1.0, 0.0]),
        ]);
        let produced = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7, 0.7]];
        let result = score(&fx, &produced);
        let worst = result.worst(2);
        assert_eq!(worst.len(), 2);
        assert_eq!(worst[0].0, "b"); // cosine 0.0, the worst
        assert!(worst[0].1 <= worst[1].1);
    }

    #[test]
    fn load_reads_a_fixture_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calibration.json");
        std::fs::write(
            &path,
            r#"{"schema":"sopack.calibration/1","contract":"e5-large-v1","entries":[
                {"id":"x","profile":"sop","text":"hello","vector":[0.1,0.2]}
            ]}"#,
        )
        .unwrap();
        let fx = CalibrationFixture::load(&path).unwrap();
        assert_eq!(fx.entries.len(), 1);
        assert_eq!(fx.entries[0].id, "x");
    }

    #[test]
    fn sha256_of_file_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calibration.json");
        std::fs::write(&path, b"{}").unwrap();
        let a = CalibrationFixture::sha256_of_file(&path).unwrap();
        let b = CalibrationFixture::sha256_of_file(&path).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
