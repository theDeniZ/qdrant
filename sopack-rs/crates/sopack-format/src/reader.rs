//! [`PackReader`] — streams a `.sopack`. Peak RAM is one batch, whatever the
//! file size (`sopack.format.PackReader` in the Python reference).
//!
//! `zip::ZipArchive<R>` holds a single reader and hands out `ZipFile<'_>`
//! streams that borrow it mutably, so — unlike Python's `zipfile.ZipFile`,
//! which can have `points.jsonl` and `vectors.f32` open at once — only one
//! entry can be open at a time from one `ZipArchive`. [`PackReader::batches`]
//! works around this without buffering whole entries into memory: it reads
//! each entry's `(data_start, compressed_size)` from the central directory
//! once, then opens **two independent `File` handles** on the same path,
//! seeks each to its entry's data, and decodes from there — `vectors.f32` is
//! `STORED` so that's a raw byte range; `points.jsonl` is `Deflated` so it
//! goes through a raw `flate2::read::DeflateDecoder`. Both are then read
//! incrementally per batch, same as the Python original.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::DeflateDecoder;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sopack_contract::Contract;
use zip::CompressionMethod;

use crate::error::{FormatError, Result};
use crate::f32io::{le_bytes_to_f32_vectors, ITEM_SIZE};
use crate::idfields::fields_from;
use crate::manifest::ManifestSummary;

const MANIFEST: &str = "manifest.json";
const POINTS: &str = "points.jsonl";
const VECTORS: &str = "vectors.f32";
const PROBE: &str = "probe.f32";
const TITLES: &str = "titles.json";

/// One parsed `points.jsonl` line.
#[derive(Debug, Clone, Deserialize)]
pub struct PointRecord {
    pub uid: String,
    pub id: String,
    pub payload: Map<String, Value>,
}

/// An open `.sopack`. Call [`PackReader::check`] before [`PackReader::batches`]
/// — reading refuses until it has passed, "the safe order is the only order
/// a caller can take" (Python's `PackReader.batches` docstring).
pub struct PackReader {
    path: PathBuf,
    zf: zip::ZipArchive<File>,
    names: std::collections::HashSet<String>,
    pub manifest: ManifestSummary,
    checked: bool,
}

impl PackReader {
    pub fn open(path: impl AsRef<Path>) -> Result<PackReader> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let mut zf = zip::ZipArchive::new(file)?;
        // `ZipArchive::new` already validates the central directory; a full
        // CRC pass (Python's `testzip()`) happens per-entry in `check()`
        // below, where every declared entry is read to completion anyway —
        // the `zip` crate validates each entry's CRC32 as it is read to EOF.
        let names: std::collections::HashSet<String> =
            zf.file_names().map(str::to_string).collect();
        for required in [MANIFEST, POINTS, VECTORS] {
            if !names.contains(required) {
                return Err(FormatError::invalid(format!(
                    "pack is missing {required:?}"
                )));
            }
        }
        let manifest_bytes = {
            let mut f = zf.by_name(MANIFEST)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            buf
        };
        let manifest = ManifestSummary::parse(&manifest_bytes)?;
        Ok(PackReader {
            path,
            zf,
            names,
            manifest,
            checked: false,
        })
    }

    pub fn dim(&self) -> usize {
        self.manifest.counts.dim as usize
    }

    pub fn count(&self) -> u64 {
        self.manifest.counts.points
    }

    /// Everything verifiable without embedding anything. Empty means the
    /// pack is internally consistent and — for `sopack/2` — matches
    /// *contract*. `sopack/1` packs skip the contract/target/embedding
    /// checks (their `target`/`embedding` blocks use a different, retired
    /// vocabulary this crate no longer models — see SOPACK-AUTONOMY.md); the
    /// schema-agnostic checks (sha256, byte counts, profile/id_rule
    /// validity, dim) still run and a genuine `/1` pack is expected to pass
    /// them cleanly.
    pub fn check(&mut self, contract: &Contract) -> Vec<String> {
        let major = match self.manifest.schema_major() {
            Ok(m) => m,
            Err(e) => return vec![e.to_string()],
        };
        let mut errors = Vec::new();

        let profile = match contract.get_profile(&self.manifest.profile) {
            Ok(p) => p.clone(),
            Err(e) => return vec![e.to_string()],
        };

        if major >= 2 {
            match self.manifest.embedding_v2() {
                Some(embedding) => errors.extend(sopack_contract::check_embedding(
                    contract,
                    &embedding.declared(),
                )),
                None => errors
                    .push("manifest.embedding is missing or not in the sopack/2 shape".to_string()),
            }
            match self.manifest.target_v2() {
                Some(target) => {
                    if target.profile != profile.name {
                        errors.push(format!(
                            "target.profile: pack says {:?}, manifest.profile is {:?}",
                            target.profile, profile.name
                        ));
                    }
                    if target.contract != contract.id {
                        errors.push(format!(
                            "target.contract: pack says {:?}, contract requires {:?}",
                            target.contract, contract.id
                        ));
                    }
                }
                None => errors
                    .push("manifest.target is missing or not in the sopack/2 shape".to_string()),
            }
            match self.manifest.contract_ref() {
                Some(cref) => {
                    if cref.sha256 != contract.contract_sha256() {
                        errors.push(format!(
                            "contract.sha256: pack says {}, contract.toml hashes to {}",
                            cref.sha256,
                            contract.contract_sha256()
                        ));
                    }
                    if let Some(calib_sha) = contract.calibration_sha256() {
                        if cref.calibration_sha256 != calib_sha {
                            errors.push(format!(
                                "contract.calibration_sha256: pack says {}, contract's \
                                 calibration.json hashes to {calib_sha}",
                                cref.calibration_sha256
                            ));
                        }
                    }
                }
                None => errors
                    .push("manifest.contract is missing or not in the sopack/2 shape".to_string()),
            }
        }

        if !profile.id_rules.iter().any(|r| r == &self.manifest.id_rule) {
            errors.push(format!(
                "id_rule {:?} is not valid for profile {:?} (allowed: {:?})",
                self.manifest.id_rule, profile.name, profile.id_rules
            ));
        }
        if self.manifest.counts.dim != contract.embedding.dim {
            errors.push(format!(
                "counts.dim is {}, contract requires {}",
                self.manifest.counts.dim, contract.embedding.dim
            ));
        }

        for entry in [POINTS, VECTORS, PROBE, TITLES] {
            if !self.names.contains(entry) {
                continue;
            }
            let want = self.manifest.sha256.get(entry);
            let want = match want {
                Some(w) if !w.is_empty() => w.clone(),
                _ => {
                    errors.push(format!("manifest declares no sha256 for {entry:?}"));
                    continue;
                }
            };
            match self.sha256_of_entry(entry) {
                Ok(got) if got == want => {}
                Ok(got) => errors.push(format!(
                    "{entry}: sha256 {}… != manifest {}…",
                    &got[..12.min(got.len())],
                    &want[..12.min(want.len())]
                )),
                Err(e) => errors.push(format!("{entry}: {e}")),
            }
        }

        let want_bytes =
            self.manifest.counts.points * self.manifest.counts.dim as u64 * ITEM_SIZE as u64;
        match self.zf.by_name(VECTORS) {
            Ok(f) => {
                if f.size() != want_bytes {
                    errors.push(format!(
                        "{VECTORS} is {} bytes, expected {want_bytes} for {} x {} float32",
                        f.size(),
                        self.manifest.counts.points,
                        self.manifest.counts.dim
                    ));
                }
            }
            Err(e) => errors.push(format!("{VECTORS}: {e}")),
        }

        self.checked = errors.is_empty();
        errors
    }

    fn sha256_of_entry(&mut self, name: &str) -> Result<String> {
        let mut f = self.zf.by_name(name)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    pub fn titles(&mut self) -> Result<Option<Value>> {
        if !self.names.contains(TITLES) {
            return Ok(None);
        }
        let mut f = self.zf.by_name(TITLES)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(Some(serde_json::from_slice(&buf)?))
    }

    pub fn probe_vectors(&mut self) -> Result<Vec<Vec<f32>>> {
        if !self.names.contains(PROBE) {
            return Ok(Vec::new());
        }
        let dim = self.dim();
        let mut f = self.zf.by_name(PROBE)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(le_bytes_to_f32_vectors(&buf, dim))
    }

    /// Stream `(points, vectors)` batches of at most *size* each, in vector
    /// order. With *verify_ids*, every line's `id` is recomputed from its
    /// `uid` under *contract* and a disagreement is an error — this alone is
    /// **not** an integrity check (editing `payload.raw_text` changes
    /// neither `uid` nor `id`), which is why [`Self::check`] must have
    /// passed first. *contract* is a parameter rather than something
    /// [`PackReader`] stores, so the reader itself stays contract-agnostic
    /// outside of [`Self::check`].
    pub fn batches<'a>(
        &mut self,
        contract: &'a Contract,
        size: usize,
        verify_ids: bool,
    ) -> Result<BatchIter<'a>> {
        if !self.checked {
            return Err(FormatError::invalid(
                "refusing to read points before check() has passed — call check() first \
                 (payload tampering is caught only by its sha256)",
            ));
        }
        if size == 0 {
            return Err(FormatError::invalid("batches: size must be at least 1"));
        }
        let dim = self.dim();
        let total = self.count();
        let profile_name = self.manifest.profile.clone();
        let id_rule = self.manifest.id_rule.clone();

        let (points_start, points_len, points_method) = {
            let f = self.zf.by_name(POINTS)?;
            (f.data_start(), f.compressed_size(), f.compression())
        };
        let (vectors_start, vectors_len) = {
            let f = self.zf.by_name(VECTORS)?;
            (f.data_start(), f.size())
        };

        let mut pf = File::open(&self.path)?;
        pf.seek(SeekFrom::Start(points_start))?;
        let bounded: Box<dyn Read> = match points_method {
            CompressionMethod::Deflated => Box::new(DeflateDecoder::new(pf.take(points_len))),
            CompressionMethod::Stored => Box::new(pf.take(points_len)),
            other => {
                return Err(FormatError::invalid(format!(
                    "{POINTS} uses unsupported compression {other:?}"
                )))
            }
        };
        let lines = BufReader::new(bounded);

        let mut vf = File::open(&self.path)?;
        vf.seek(SeekFrom::Start(vectors_start))?;
        let vectors_reader = BufReader::new(vf.take(vectors_len));

        Ok(BatchIter {
            lines,
            vectors_reader,
            dim,
            batch_size: size,
            seen: 0,
            total,
            verify_ids,
            contract,
            profile_name,
            id_rule,
        })
    }
}

/// Yields `(points, vectors)` batches. See [`PackReader::batches_with`].
pub struct BatchIter<'a> {
    lines: BufReader<Box<dyn Read>>,
    vectors_reader: BufReader<std::io::Take<File>>,
    dim: usize,
    batch_size: usize,
    seen: u64,
    total: u64,
    verify_ids: bool,
    contract: &'a Contract,
    profile_name: String,
    id_rule: String,
}

impl Iterator for BatchIter<'_> {
    type Item = Result<(Vec<PointRecord>, Vec<Vec<f32>>)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut points = Vec::new();
        loop {
            let mut line = String::new();
            let n = match self.lines.read_line(&mut line) {
                Ok(n) => n,
                Err(e) => return Some(Err(e.into())),
            };
            if n == 0 {
                break; // EOF
            }
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if trimmed.is_empty() {
                continue;
            }
            let rec: PointRecord = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    return Some(Err(FormatError::invalid(format!(
                        "{POINTS} line {}: {e}",
                        self.seen + 1
                    ))))
                }
            };
            if self.verify_ids {
                let profile = match self.contract.get_profile(&self.profile_name) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e.into())),
                };
                let _ = profile; // profile existence already implies the id rule table is loaded
                let fields = fields_from(&rec.payload, &rec.uid);
                let want = match self.contract.point_id(&self.id_rule, &fields) {
                    Ok(w) => w,
                    Err(e) => return Some(Err(e.into())),
                };
                if rec.id != want {
                    return Some(Err(FormatError::invalid(format!(
                        "{POINTS} line {}: id {} does not match {} of uid {:?} (expected {}) \
                         — pack is corrupt or was edited by hand",
                        self.seen + 1,
                        rec.id,
                        self.id_rule,
                        rec.uid,
                        want
                    ))));
                }
            }
            self.seen += 1;
            points.push(rec);
            if points.len() >= self.batch_size {
                break;
            }
        }

        if points.is_empty() {
            return if self.seen != self.total {
                Some(Err(FormatError::invalid(format!(
                    "{POINTS} holds {} points, manifest declares {}",
                    self.seen, self.total
                ))))
            } else {
                None
            };
        }

        let stride = self.dim * ITEM_SIZE;
        let mut raw = vec![0u8; stride * points.len()];
        if let Err(e) = self.vectors_reader.read_exact(&mut raw) {
            return Some(Err(FormatError::invalid(format!(
                "{VECTORS} ran out after {} points: {e}",
                self.seen
            ))));
        }
        let vectors = le_bytes_to_f32_vectors(&raw, self.dim);
        Some(Ok((points, vectors)))
    }
}
