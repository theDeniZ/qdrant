//! `verify(path, contract)` — offline verification of a `.sopack`: no
//! network, no embedding model. Mirrors `sopack/verify.py`'s streaming
//! per-book count cross-check, plus the `sopack/2` probe checks
//! SOPACK-2-FORMAT.md §4 describes for the *importer* side (fixture sha
//! match, entry coverage, self-check threshold, recomputed cosines) — this
//! crate runs those here too since they need nothing model-free verification
//! doesn't already have on hand (the contract's loaded fixture vectors).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use sopack_contract::{validate_payload, CalibrationEntry, Contract, Profile};

use crate::reader::PackReader;

/// Every problem found. Empty ([`VerifyReport::is_clean`]) means the pack is
/// clean.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub errors: Vec<String>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Verify *path* against *contract*, calling *progress* as
/// `(points_done, points_total)` after each streamed batch. Never panics or
/// propagates an `Err` for a bad pack — every problem, including "file does
/// not exist", becomes an entry in the returned report, matching Python's
/// `verify()` (`except (PackError, OSError)`).
pub fn verify_with_progress(
    path: impl AsRef<Path>,
    contract: &Contract,
    mut progress: impl FnMut(u64, u64),
) -> VerifyReport {
    let mut errors = Vec::new();

    let mut reader = match PackReader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            errors.push(e.to_string());
            return VerifyReport { errors };
        }
    };

    errors.extend(reader.check(contract));
    if !errors.is_empty() {
        // check() failed — points.jsonl/vectors.f32 cannot be trusted, and
        // batches() would refuse to stream anyway.
        return VerifyReport { errors };
    }

    let profile = match contract.get_profile(&reader.manifest.profile) {
        Ok(p) => p.clone(),
        Err(e) => {
            errors.push(e.to_string());
            return VerifyReport { errors };
        }
    };

    let manifest_books: HashMap<String, u64> = reader
        .manifest
        .books
        .iter()
        .map(|b| (b.book_code.clone(), b.points))
        .collect();
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    let total_points = reader.count();

    let batches = match reader.batches(contract, 128, true) {
        Ok(b) => b,
        Err(e) => {
            errors.push(e.to_string());
            return VerifyReport { errors };
        }
    };
    let mut done = 0u64;
    for batch in batches {
        let (points, vectors) = match batch {
            Ok(v) => v,
            Err(e) => {
                errors.push(e.to_string());
                return VerifyReport { errors };
            }
        };
        if points.len() != vectors.len() {
            errors.push(format!(
                "batch mismatch: {} points, {} vectors",
                points.len(),
                vectors.len()
            ));
        }
        for rec in &points {
            let problems = payload_problems(&profile, &rec.payload);
            if !problems.is_empty() {
                errors.push(format!("{}: {}", rec.uid, problems.join("; ")));
            }
            let code = rec
                .payload
                .get(&profile.identity)
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            *counts.entry(code).or_insert(0) += 1;
            total += 1;
        }
        done += points.len() as u64;
        progress(done, total_points);
    }

    if total != reader.count() {
        errors.push(format!(
            "streamed {total} points, manifest counts.points says {}",
            reader.count()
        ));
    }

    for (code, want) in &manifest_books {
        let got = counts.get(code).copied().unwrap_or(0);
        if got != *want {
            errors.push(format!(
                "book {code:?}: manifest declares {want} points, stream has {got}"
            ));
        }
    }
    for code in counts.keys() {
        if !manifest_books.contains_key(code) {
            errors.push(format!(
                "book {code:?} has {} points but no entry in manifest.books",
                counts[code]
            ));
        }
    }

    let probe_vectors = match reader.probe_vectors() {
        Ok(v) => v,
        Err(e) => {
            errors.push(e.to_string());
            Vec::new()
        }
    };
    for v in &probe_vectors {
        if v.len() != reader.dim() {
            errors.push(format!(
                "probe vector has {} dims, expected {}",
                v.len(),
                reader.dim()
            ));
            break;
        }
    }

    match reader.manifest.schema_major() {
        Ok(major) if major >= 2 => {
            verify_probe_v2(&reader, contract, &probe_vectors, &mut errors);
        }
        Ok(_) => {
            // sopack/1: only the shape check above applies — the live-canary
            // identity of a /1 probe cannot be verified offline (it needs
            // the store), matching Python's verify.py, which never checked
            // /1 canary identity either (only import-time did, over HTTP).
        }
        Err(e) => errors.push(e.to_string()),
    }

    VerifyReport { errors }
}

/// [`verify_with_progress`] with a no-op progress callback.
pub fn verify(path: impl AsRef<Path>, contract: &Contract) -> VerifyReport {
    verify_with_progress(path, contract, |_, _| {})
}

fn verify_probe_v2(
    reader: &PackReader,
    contract: &Contract,
    probe_vectors: &[Vec<f32>],
    errors: &mut Vec<String>,
) {
    let Some(probe_meta) = reader.manifest.probe_v2() else {
        errors.push("manifest declares schema sopack/2 but has no valid probe block".to_string());
        return;
    };
    if probe_meta.entries.len() != probe_vectors.len() {
        errors.push(format!(
            "probe: manifest declares {} entries, probe.f32 holds {} vectors",
            probe_meta.entries.len(),
            probe_vectors.len()
        ));
    }
    if probe_meta.self_check.min_cosine < contract.calibration.pack_min_cosine {
        errors.push(format!(
            "probe.self_check.min_cosine {} is below the contract's pack_min_cosine {} — \
             this pack should never have been written",
            probe_meta.self_check.min_cosine, contract.calibration.pack_min_cosine
        ));
    }

    let Some(fixture) = &contract.fixture else {
        errors.push(
            "cannot verify a sopack/2 probe: the contract has no calibration fixture loaded"
                .to_string(),
        );
        return;
    };
    if probe_meta.fixture_sha256 != fixture.sha256() {
        errors.push(format!(
            "probe.fixture_sha256 {} does not match the contract's calibration.json ({}) — \
             pack was calibrated against a different fixture",
            probe_meta.fixture_sha256,
            fixture.sha256()
        ));
    }

    let fixture_by_id: HashMap<&str, &CalibrationEntry> = fixture
        .entries_for_profile(&reader.manifest.profile)
        .map(|e| (e.id.as_str(), e))
        .collect();
    let probe_ids: HashSet<&str> = probe_meta.entries.iter().map(|e| e.id.as_str()).collect();
    for id in fixture_by_id.keys() {
        if !probe_ids.contains(id) {
            errors.push(format!(
                "probe entries do not cover fixture id {id:?} for profile {:?}",
                reader.manifest.profile
            ));
        }
    }

    for pe in &probe_meta.entries {
        let (Some(fixture_entry), Some(vector)) = (
            fixture_by_id.get(pe.id.as_str()),
            probe_vectors.get(pe.vector_offset),
        ) else {
            continue;
        };
        let c = cosine(vector, &fixture_entry.vector);
        if c < contract.calibration.probe_min_cosine {
            errors.push(format!(
                "probe entry {:?}: cosine {c:.6} with the fixture vector is below \
                 probe_min_cosine {}",
                pe.id, contract.calibration.probe_min_cosine
            ));
        }
    }
}

/// Plain cosine similarity — mirrors `sopack.contract.cosine` (this crate's
/// analogue lives here rather than in `sopack-contract` since it is only
/// needed for the `/2` probe recompute, not for anything id/payload-shaped).
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let (x, y) = (x as f64, y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na > 0.0 && nb > 0.0 {
        dot / (na.sqrt() * nb.sqrt())
    } else {
        0.0
    }
}

/// `sopack.verify._payload_problems` — narrower than
/// [`sopack_contract::validate_payload`]: only required-key presence and a
/// non-empty text field, **not** the unknown-key check (verify.py never
/// flagged unknown keys; that stays [`validate_payload`]'s job at write
/// time).
fn payload_problems(
    profile: &Profile,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Vec<String> {
    // `validate_payload` is a strict superset except for the unknown-key
    // check, so filter that one class of message out rather than
    // duplicating the required/text-field logic.
    validate_payload(profile, payload)
        .into_iter()
        .filter(|e| !e.starts_with("unknown payload key"))
        .collect()
}
