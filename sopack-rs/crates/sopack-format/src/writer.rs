//! [`PackWriter`] — streams points into a `sopack/2` `.sopack`, with a
//! resumable on-disk checkpoint so a killed `pack` run can continue instead
//! of re-embedding from scratch (SOPACK-1.0-PLAN.md §3.3 "Resume").
//!
//! Design, vs. the Python `/1` writer (`sopack/format.py::PackWriter`):
//!
//! - Python spools straight to two **anonymous-ish** temp files next to the
//!   destination, unique per writer instance, deleted on any exit. That
//!   gives concurrency safety (two writers never destroy each other) but no
//!   resumability — a killed process loses everything.
//! - Here, `add()` appends to two **named, deterministic** files under
//!   `<out>.sopack.partial/` (`points.jsonl` plain, `vectors.f32` a spool) so
//!   a second process can find and continue them ([`PackWriter::resume`]).
//!   The concurrency-safety property is kept a different way:
//!   [`PackWriter::create`] refuses if that directory already exists —
//!   a second writer must explicitly `resume()`, never silently reuses or
//!   clobbers a live one's checkpoint.
//! - Every sha256 in the manifest is computed by reading the finished
//!   checkpoint files once at [`PackWriter::finish`], not accumulated
//!   incrementally. `sha2`'s hasher state cannot be serialized, so an
//!   incremental hash would need to be re-derived from disk on resume
//!   anyway (per the plan's own note); re-hashing at `finish()`
//!   unconditionally means resumed and uninterrupted runs share the exact
//!   same code path, which is what makes them byte-identical (see
//!   `tests::crash_then_resume_is_byte_identical_to_one_run`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sopack_contract::{validate_payload, Contract};
use zip::write::{FileOptions, SimpleFileOptions};
use zip::{CompressionMethod, ZipWriter};

use crate::error::{FormatError, Result};
use crate::f32io::{f32_to_le_bytes, ITEM_SIZE};
use crate::idfields::fields_from;
use crate::jsonline::to_python_json_bytes;
use crate::manifest::{
    build_manifest_v2, BookEntry, ContractRef, Counts, EmbeddingV2, ProbeEntry, ProbeV2, SelfCheck,
    TargetV2,
};

const CHECKPOINT_SCHEMA: &str = "sopack.checkpoint/1";
const POINTS: &str = "points.jsonl";
const VECTORS: &str = "vectors.f32";
const PROBE: &str = "probe.f32";
const TITLES: &str = "titles.json";
const MANIFEST: &str = "manifest.json";
const STATE: &str = "state.json";

/// Everything about how the vectors being written were produced —
/// `manifest.json["embedding"]`'s four provenance-only fields (R10;
/// SOPACK-2-FORMAT.md §2: "not compared", unlike the seven contract keys).
#[derive(Debug, Clone)]
pub struct EmbeddingProvenance {
    pub runtime: String,
    pub device: String,
    pub threads: u32,
    pub batch_tokens: u32,
}

/// One calibration-fixture entry to probe, as the caller (which did the
/// embedding) supplies it — `vector_offset` is assigned by the writer from
/// array position, matching `probe.entries` being "every fixture entry, in
/// fixture order" (SOPACK-2-FORMAT.md §2).
#[derive(Debug, Clone)]
pub struct ProbeEntryInput {
    pub id: String,
    pub profile: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointState {
    schema: String,
    profile: String,
    pack_id: String,
    created_by: String,
    id_rule: String,
    dim: u32,
    contract_id: String,
    contract_sha256: String,
    calibration_sha256: String,
    embedding: EmbeddingV2,
}

#[derive(Debug)]
struct ProbeBuild {
    entries: Vec<ProbeEntry>,
    vectors: Vec<Vec<f32>>,
    self_check: SelfCheck,
}

/// Streams points into a `sopack/2` `.sopack`. See the module docs for the
/// checkpoint/resume design.
///
/// ```no_run
/// # use sopack_contract::Contract;
/// # use sopack_format::{PackWriter, EmbeddingProvenance, ProbeEntryInput};
/// # use serde_json::json;
/// # fn main() -> sopack_format::Result<()> {
/// let contract = Contract::embedded("e5-large-v1")?;
/// let provenance = EmbeddingProvenance {
///     runtime: "sopack-rs 0.9.0".into(), device: "cpu".into(),
///     threads: 4, batch_tokens: 512,
/// };
/// let mut w = PackWriter::create("/tmp/out.sopack", &contract, "sop", "p1".into(),
///     "sopack-rs test".into(), None, provenance)?;
/// w.add("en:TT:1.1#0", json!({"lang":"en","book_code":"TT","book_pair":"TT","page":1,
///     "para":1,"para_key":"1.1","raw_text":"hi","aligned":null}).as_object().unwrap().clone(),
///     &vec![0.0; contract.embedding.dim as usize])?;
/// // w.set_books(...); w.set_probe(...)? are required before finish().
/// # Ok(()) }
/// ```
#[derive(Debug)]
pub struct PackWriter<'c> {
    out_path: PathBuf,
    checkpoint_dir: PathBuf,
    contract: &'c Contract,
    profile_name: String,
    pack_id: String,
    created_by: String,
    id_rule: String,
    id_rule_doc: String,
    dim: u32,
    contract_sha256: String,
    calibration_sha256: String,
    embedding: EmbeddingV2,
    count: u64,
    points_file: Option<BufWriter<File>>,
    vectors_file: Option<BufWriter<File>>,
    books: Vec<BookEntry>,
    titles: Option<Value>,
    probe: Option<ProbeBuild>,
    finished: bool,
}

impl<'c> PackWriter<'c> {
    fn checkpoint_dir_for(out_path: &Path) -> PathBuf {
        let mut s = out_path.as_os_str().to_owned();
        s.push(".sopack.partial");
        PathBuf::from(s)
    }

    /// Start a fresh pack at *path*. Fails if a checkpoint for this exact
    /// output path already exists — resume it explicitly with
    /// [`PackWriter::resume`], or remove the directory to discard it.
    pub fn create(
        path: impl AsRef<Path>,
        contract: &'c Contract,
        profile_name: &str,
        pack_id: String,
        created_by: String,
        id_rule: Option<String>,
        provenance: EmbeddingProvenance,
    ) -> Result<PackWriter<'c>> {
        let out_path = path.as_ref().to_path_buf();
        let checkpoint_dir = Self::checkpoint_dir_for(&out_path);
        if checkpoint_dir.exists() {
            return Err(FormatError::invalid(format!(
                "a checkpoint already exists at {} — call PackWriter::resume(...) to \
                 continue it, or remove the directory to start fresh",
                checkpoint_dir.display()
            )));
        }

        let profile = contract.get_profile(profile_name)?;
        let id_rule = id_rule.unwrap_or_else(|| profile.default_id_rule.clone());
        if !profile.id_rules.contains(&id_rule) {
            return Err(FormatError::invalid(format!(
                "id_rule {id_rule:?} is not valid for profile {profile_name:?} (allowed: {:?})",
                profile.id_rules
            )));
        }
        let id_rule_doc = contract.get_id_rule(&id_rule)?.doc();

        let fixture = contract.fixture.as_ref().ok_or_else(|| {
            FormatError::invalid(format!(
                "contract {:?} has no calibration fixture loaded — sopack/2 requires one \
                 (SOPACK-2-FORMAT.md §3); run the fixture generator first",
                contract.id
            ))
        })?;
        let calibration_sha256 = fixture.sha256();

        let embedding = EmbeddingV2 {
            model: contract.embedding.model.clone(),
            pooling: contract.embedding.pooling.clone(),
            normalized: contract.embedding.normalized,
            dim: contract.embedding.dim,
            distance: contract.embedding.distance.clone(),
            max_tokens: contract.embedding.max_tokens,
            passage_prefix: contract.embedding.passage_prefix.clone(),
            runtime: provenance.runtime,
            device: provenance.device,
            threads: provenance.threads,
            batch_tokens: provenance.batch_tokens,
        };

        fs::create_dir(&checkpoint_dir).map_err(FormatError::from)?;
        let state = CheckpointState {
            schema: CHECKPOINT_SCHEMA.to_string(),
            profile: profile_name.to_string(),
            pack_id: pack_id.clone(),
            created_by: created_by.clone(),
            id_rule: id_rule.clone(),
            dim: contract.embedding.dim,
            contract_id: contract.id.clone(),
            contract_sha256: contract.contract_sha256(),
            calibration_sha256: calibration_sha256.clone(),
            embedding: embedding.clone(),
        };
        write_state(&checkpoint_dir, &state)?;

        let points_file = open_append(&checkpoint_dir.join(POINTS))?;
        let vectors_file = open_append(&checkpoint_dir.join(VECTORS))?;

        Ok(PackWriter {
            out_path,
            checkpoint_dir,
            contract,
            profile_name: profile_name.to_string(),
            pack_id,
            created_by,
            id_rule,
            id_rule_doc,
            dim: contract.embedding.dim,
            contract_sha256: state.contract_sha256,
            calibration_sha256,
            embedding,
            count: 0,
            points_file: Some(BufWriter::new(points_file)),
            vectors_file: Some(BufWriter::new(vectors_file)),
            books: Vec::new(),
            titles: None,
            probe: None,
            finished: false,
        })
    }

    /// Reopen an existing `<path>.sopack.partial/` checkpoint and continue
    /// appending to it. *contract* must be the exact contract the
    /// checkpoint was started under (same id and `contract.toml` bytes) —
    /// otherwise every id already written would be silently wrong for the
    /// rest of the run.
    pub fn resume(path: impl AsRef<Path>, contract: &'c Contract) -> Result<PackWriter<'c>> {
        let out_path = path.as_ref().to_path_buf();
        let checkpoint_dir = Self::checkpoint_dir_for(&out_path);
        if !checkpoint_dir.is_dir() {
            return Err(FormatError::invalid(format!(
                "no checkpoint found at {} — nothing to resume",
                checkpoint_dir.display()
            )));
        }
        let state = read_state(&checkpoint_dir)?;
        if state.schema != CHECKPOINT_SCHEMA {
            return Err(FormatError::invalid(format!(
                "{}: unsupported checkpoint schema {:?}",
                checkpoint_dir.join(STATE).display(),
                state.schema
            )));
        }
        if state.contract_id != contract.id || state.contract_sha256 != contract.contract_sha256() {
            return Err(FormatError::invalid(format!(
                "checkpoint at {} was started against contract {:?} ({}), but contract \
                 {:?} ({}) was given — refusing to resume under a different contract",
                checkpoint_dir.display(),
                state.contract_id,
                state.contract_sha256,
                contract.id,
                contract.contract_sha256()
            )));
        }
        let profile = contract.get_profile(&state.profile)?;
        if !profile.id_rules.contains(&state.id_rule) {
            return Err(FormatError::invalid(format!(
                "checkpoint declares id_rule {:?}, not valid for profile {:?} under the \
                 given contract",
                state.id_rule, state.profile
            )));
        }

        let points_path = checkpoint_dir.join(POINTS);
        let vectors_path = checkpoint_dir.join(VECTORS);
        let vectors_len = fs::metadata(&vectors_path)
            .map_err(|e| {
                FormatError::invalid(format!(
                    "checkpoint at {} is missing {VECTORS}: {e}",
                    checkpoint_dir.display()
                ))
            })?
            .len();
        let stride = state.dim as u64 * ITEM_SIZE as u64;
        if stride == 0 || vectors_len % stride != 0 {
            return Err(FormatError::invalid(format!(
                "checkpoint corrupt: {VECTORS} is {vectors_len} bytes, not a multiple of \
                 dim*4 ({stride})"
            )));
        }
        let count_from_vectors = vectors_len / stride;
        let count_from_points = count_lines(&points_path)?;
        if count_from_points != count_from_vectors {
            return Err(FormatError::invalid(format!(
                "checkpoint corrupt: {POINTS} has {count_from_points} lines but {VECTORS} \
                 holds {count_from_vectors} vectors — points and vectors have gone out of \
                 sync, probably from a crash mid-write; this checkpoint cannot be resumed \
                 safely and should be discarded"
            )));
        }

        let points_file = open_append(&points_path)?;
        let vectors_file = open_append(&vectors_path)?;

        Ok(PackWriter {
            out_path,
            checkpoint_dir,
            contract,
            profile_name: state.profile,
            pack_id: state.pack_id,
            created_by: state.created_by,
            id_rule_doc: contract.get_id_rule(&state.id_rule)?.doc(),
            id_rule: state.id_rule,
            dim: state.dim,
            contract_sha256: state.contract_sha256,
            calibration_sha256: state.calibration_sha256,
            embedding: state.embedding,
            count: count_from_vectors,
            points_file: Some(BufWriter::new(points_file)),
            vectors_file: Some(BufWriter::new(vectors_file)),
            books: Vec::new(),
            titles: None,
            probe: None,
            finished: false,
        })
    }

    /// How many points have been durably appended so far (survives resume).
    /// The `pack_id` this writer is using — the one given to
    /// [`PackWriter::create`], or the one recorded in the checkpoint on
    /// [`PackWriter::resume`]. A caller that let `create` default the
    /// `pack_id` and wants to report what was actually chosen (or that
    /// needs it after a resume, when it wasn't the caller's to pick) reads
    /// it back here rather than re-deriving it.
    pub fn pack_id(&self) -> &str {
        &self.pack_id
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// Append one point. Validates *payload* against the contract's profile
    /// and computes the point id the same way the reader will recompute it
    /// (`_fields` + the id rule template) — mirrors
    /// `sopack.format.PackWriter.add`.
    pub fn add(
        &mut self,
        uid: &str,
        payload: Map<String, Value>,
        vector: &[f32],
    ) -> Result<String> {
        if vector.len() != self.dim as usize {
            return Err(FormatError::invalid(format!(
                "point {uid:?}: vector has {} dims, expected {}",
                vector.len(),
                self.dim
            )));
        }
        let profile = self.contract.get_profile(&self.profile_name)?;
        let problems = validate_payload(profile, &payload);
        if !problems.is_empty() {
            return Err(FormatError::invalid(format!(
                "point {uid:?}: {}",
                problems.join("; ")
            )));
        }
        let fields = fields_from(&payload, uid);
        let pid = self.contract.point_id(&self.id_rule, &fields)?;

        let mut line = to_python_json_bytes(&serde_json::json!({
            "uid": uid, "id": pid, "payload": payload
        }))?;
        line.push(b'\n');
        self.points_file
            .as_mut()
            .expect("writer used after finish()")
            .write_all(&line)?;
        self.vectors_file
            .as_mut()
            .expect("writer used after finish()")
            .write_all(&f32_to_le_bytes(vector))?;
        self.points_file.as_mut().unwrap().flush()?;
        self.vectors_file.as_mut().unwrap().flush()?;

        self.count += 1;
        Ok(pid)
    }

    /// Flush and `fsync` both checkpoint files — call after a batch so a
    /// kill leaves a checkpoint [`PackWriter::resume`] can trust.
    pub fn checkpoint(&mut self) -> Result<()> {
        if let Some(f) = self.points_file.as_mut() {
            f.flush()?;
            f.get_ref().sync_all()?;
        }
        if let Some(f) = self.vectors_file.as_mut() {
            f.flush()?;
            f.get_ref().sync_all()?;
        }
        Ok(())
    }

    pub fn set_books(&mut self, books: Vec<BookEntry>) {
        self.books = books;
    }

    pub fn set_titles(&mut self, titles: Option<Value>) {
        self.titles = titles;
    }

    /// Record the pack's own calibration self-check (SOPACK-2-FORMAT.md §3:
    /// embedded "before any book", "aborts … if any cosine is below
    /// `pack_min_cosine`"). *entries*/*vectors* must be the same length and
    /// in fixture order; *min_cosine*/*mean_cosine* are what the caller (who
    /// did the embedding and the cosine math) measured against the
    /// contract's fixture vectors.
    ///
    /// Refuses — this crate's half of R11 — when `min_cosine` is below the
    /// contract's `[calibration].pack_min_cosine`.
    pub fn set_probe(
        &mut self,
        entries: Vec<ProbeEntryInput>,
        vectors: Vec<Vec<f32>>,
        n: u64,
        min_cosine: f64,
        mean_cosine: f64,
    ) -> Result<()> {
        if entries.len() != vectors.len() {
            return Err(FormatError::invalid(format!(
                "probe: {} entries but {} vectors",
                entries.len(),
                vectors.len()
            )));
        }
        let threshold = self.contract.calibration.pack_min_cosine;
        if min_cosine < threshold {
            return Err(FormatError::invalid(format!(
                "calibration self-check failed: min_cosine {min_cosine} is below the \
                 contract's pack_min_cosine {threshold} — refusing to write a pack whose \
                 vectors may not be in contract {:?}'s space",
                self.contract.id
            )));
        }
        let entries = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| ProbeEntry {
                id: e.id,
                profile: e.profile,
                vector_offset: i,
            })
            .collect();
        self.probe = Some(ProbeBuild {
            entries,
            vectors,
            self_check: SelfCheck {
                n,
                min_cosine,
                mean_cosine,
                threshold,
            },
        });
        Ok(())
    }

    /// Assemble the `.sopack` zip from the checkpoint and publish it
    /// atomically to the output path, then remove the checkpoint directory.
    /// Requires [`PackWriter::set_probe`] to have been called — R11, the
    /// probe is not skippable.
    pub fn finish(mut self) -> Result<PathBuf> {
        let probe = self.probe.take().ok_or_else(|| {
            FormatError::invalid(
                "cannot finish a sopack/2 pack without a calibration probe — call \
                 set_probe() first (R11: nothing is written unless the vectors are proven \
                 to be in the contract's space)",
            )
        })?;

        self.checkpoint()?;
        // Drop the writers so the files below are read from a clean, fully
        // flushed state and are not held open on Windows-hostile locking
        // semantics (irrelevant on Linux/macOS, but cheap to get right).
        self.points_file = None;
        self.vectors_file = None;

        let points_path = self.checkpoint_dir.join(POINTS);
        let vectors_path = self.checkpoint_dir.join(VECTORS);

        let vectors_len = fs::metadata(&vectors_path)?.len();
        let stride = self.dim as u64 * ITEM_SIZE as u64;
        if vectors_len != self.count * stride {
            return Err(FormatError::invalid(format!(
                "{VECTORS} is {vectors_len} bytes, expected {} for {} points x {} dims x {} \
                 bytes",
                self.count * stride,
                self.count,
                self.dim,
                ITEM_SIZE
            )));
        }
        let points_lines = count_lines(&points_path)?;
        if points_lines != self.count {
            return Err(FormatError::invalid(format!(
                "{POINTS} holds {points_lines} lines, writer counted {} points",
                self.count
            )));
        }

        let points_bytes = fs::metadata(&points_path)?.len();
        let points_sha = sha256_of_file(&points_path)?;
        let vectors_sha = sha256_of_file(&vectors_path)?;

        let mut probe_bytes =
            Vec::with_capacity(probe.vectors.len() * self.dim as usize * ITEM_SIZE);
        for v in &probe.vectors {
            if v.len() != self.dim as usize {
                return Err(FormatError::invalid(format!(
                    "probe vector has {} dims, expected {}",
                    v.len(),
                    self.dim
                )));
            }
            probe_bytes.extend_from_slice(&f32_to_le_bytes(v));
        }
        let probe_sha = hex::encode(Sha256::digest(&probe_bytes));

        let titles_bytes = match &self.titles {
            Some(t) => Some(serde_json::to_vec_pretty(t)?),
            None => None,
        };
        let titles_sha = titles_bytes
            .as_ref()
            .map(|b| hex::encode(Sha256::digest(b)));

        let mut sha256 = indexmap::IndexMap::new();
        sha256.insert(POINTS.to_string(), points_sha);
        sha256.insert(VECTORS.to_string(), vectors_sha);
        sha256.insert(PROBE.to_string(), probe_sha.clone());
        if let Some(ts) = &titles_sha {
            sha256.insert(TITLES.to_string(), ts.clone());
        }

        let target = TargetV2 {
            profile: self.profile_name.clone(),
            contract: self.contract.id.clone(),
        };
        let contract_ref = ContractRef {
            id: self.contract.id.clone(),
            sha256: self.contract_sha256.clone(),
            calibration_sha256: self.calibration_sha256.clone(),
        };
        let counts = Counts {
            points: self.count,
            books: self.books.len() as u64,
            dim: self.dim,
            points_bytes,
        };
        let probe_manifest = ProbeV2 {
            kind: "calibration".to_string(),
            fixture_sha256: self.calibration_sha256.clone(),
            vectors: PROBE.to_string(),
            entries: probe.entries,
            self_check: probe.self_check,
        };
        let manifest_value = build_manifest_v2(
            &self.profile_name,
            &self.pack_id,
            &self.created_by,
            &target,
            &contract_ref,
            &self.embedding,
            &self.id_rule,
            &self.id_rule_doc,
            &counts,
            &sha256,
            &self.books,
            &probe_manifest,
        );
        let manifest_bytes = serde_json::to_vec_pretty(&manifest_value)?;

        let parent = self
            .out_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = self
            .out_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out.sopack".to_string());
        let mut tmp = tempfile::Builder::new()
            .prefix(&format!(".{file_name}."))
            .suffix(".part")
            .tempfile_in(parent)?;

        {
            let mut zw = ZipWriter::new(tmp.as_file_mut());
            let deflated: SimpleFileOptions =
                FileOptions::default().compression_method(CompressionMethod::Deflated);
            let stored: SimpleFileOptions = FileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .large_file(true);

            zw.start_file(POINTS, deflated)?;
            copy_file_into(&points_path, &mut zw)?;

            zw.start_file(VECTORS, stored)?;
            copy_file_into(&vectors_path, &mut zw)?;

            zw.start_file(PROBE, stored)?;
            zw.write_all(&probe_bytes)?;

            if let Some(bytes) = &titles_bytes {
                zw.start_file(TITLES, deflated)?;
                zw.write_all(bytes)?;
            }

            zw.start_file(MANIFEST, deflated)?;
            zw.write_all(&manifest_bytes)?;

            zw.finish()?;
        }

        tmp.persist(&self.out_path).map_err(|e| {
            FormatError::invalid(format!(
                "could not publish {}: {}",
                self.out_path.display(),
                e.error
            ))
        })?;

        fs::remove_dir_all(&self.checkpoint_dir).map_err(FormatError::from)?;
        self.finished = true;
        Ok(self.out_path.clone())
    }
}

impl Drop for PackWriter<'_> {
    fn drop(&mut self) {
        // Best-effort: leave whatever was durably appended flushed, so a
        // process kill right after `drop` still leaves a resumable
        // checkpoint. The checkpoint directory itself is deliberately never
        // touched here — see the module docs.
        if let Some(f) = self.points_file.as_mut() {
            let _ = f.flush();
        }
        if let Some(f) = self.vectors_file.as_mut() {
            let _ = f.flush();
        }
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn write_state(dir: &Path, state: &CheckpointState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    let tmp = dir.join(format!(".{STATE}.tmp"));
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, dir.join(STATE))?;
    Ok(())
}

fn read_state(dir: &Path) -> Result<CheckpointState> {
    let bytes = fs::read(dir.join(STATE)).map_err(|e| {
        FormatError::invalid(format!(
            "{}: cannot read checkpoint state: {e}",
            dir.display()
        ))
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn count_lines(path: &Path) -> Result<u64> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut count = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
    }
    Ok(count)
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn copy_file_into<W: Write>(path: &Path, mut into: W) -> Result<()> {
    let mut file = BufReader::new(File::open(path)?);
    io::copy(&mut file, &mut into)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::PackReader;
    use sopack_contract::Contract;

    fn test_contract_dir() -> tempfile::TempDir {
        test_contract_dir_variant("")
    }

    /// *tag* perturbs the fixture text (and therefore `calibration.json`'s
    /// sha256, and therefore `contract.toml`'s bytes) so two variants
    /// produce genuinely different `Contract::contract_sha256()` values —
    /// needed by `resume_refuses_a_different_contract`.
    fn test_contract_dir_variant(tag: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let calib_bytes = make_calibration_bytes(tag);
        let sha = hex::encode(Sha256::digest(&calib_bytes));
        let toml = e5_toml_with_calibration_sha(&sha);
        fs::write(dir.path().join("contract.toml"), toml).unwrap();
        let mut f = File::create(dir.path().join("calibration.json")).unwrap();
        f.write_all(&calib_bytes).unwrap();
        dir
    }

    /// Rewrite the `[calibration]` section's `sha256 = "..."` line only —
    /// not a literal-string sentinel, since the real `calibration.json` (and
    /// therefore the real declared sha256) is being generated concurrently
    /// by another agent and may already be present in this workspace's
    /// committed `contract.toml` by the time this runs (see `common.md`).
    /// A naive whole-file substring search would also risk matching one of
    /// the six unrelated `model.files` `sha256` entries.
    fn e5_toml_with_calibration_sha(sha: &str) -> String {
        const TOML: &str = include_str!("../../../contracts/e5-large-v1/contract.toml");
        let mut out = String::new();
        let mut in_calibration = false;
        let mut replaced = false;
        for line in TOML.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('[') {
                in_calibration = trimmed.starts_with("[calibration]");
            }
            if in_calibration && !replaced && trimmed.starts_with("sha256") {
                out.push_str(&format!("sha256 = \"{sha}\"\n"));
                replaced = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        assert!(
            replaced,
            "no [calibration] sha256 line found in contract.toml"
        );
        out
    }

    fn make_calibration_bytes(tag: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": "sopack.calibration/1",
            "contract": "e5-large-v1",
            "created_at": "2026-09-24T00:00:00Z",
            "entries": [
                {"id": "fx-1", "profile": "sop", "lang": "en", "text": format!("hello{tag}"),
                 "vector": vec![0.1f32; 1024]},
                {"id": "fx-2", "profile": "sop", "lang": "de", "text": format!("hallo{tag}"),
                 "vector": vec![0.2f32; 1024]},
            ]
        }))
        .unwrap()
    }

    fn provenance() -> EmbeddingProvenance {
        EmbeddingProvenance {
            runtime: "test-runtime".into(),
            device: "cpu".into(),
            threads: 1,
            batch_tokens: 512,
        }
    }

    fn payload(i: usize) -> Map<String, Value> {
        serde_json::json!({
            "lang": "en", "book_code": "TT", "book_pair": "TT", "page": 1,
            "para": i + 1, "para_key": format!("1.{}", i + 1),
            "raw_text": format!("paragraph {i}"), "aligned": null
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn vector(dim: usize, seed: u32) -> Vec<f32> {
        (0..dim)
            .map(|i| ((seed as usize + i) % 97) as f32 / 97.0)
            .collect()
    }

    fn finish_with_books_and_probe(w: &mut PackWriter, dim: usize) {
        w.set_books(vec![BookEntry {
            book_code: "TT".into(),
            lang: Some("en".into()),
            points: w.count(),
            first_id: None,
            title: Some("Test Title".into()),
            author: None,
            year: None,
            corpus: None,
            slug: None,
            book_pair: Some("TT".into()),
            id_rule: w.id_rule.clone(),
            book_sha256: None,
        }]);
        w.set_probe(
            vec![
                ProbeEntryInput {
                    id: "fx-1".into(),
                    profile: "sop".into(),
                },
                ProbeEntryInput {
                    id: "fx-2".into(),
                    profile: "sop".into(),
                },
            ],
            vec![vector(dim, 1), vector(dim, 2)],
            2,
            0.9999999,
            0.9999999,
        )
        .unwrap();
    }

    #[test]
    fn write_then_read_round_trips() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let dim = contract.embedding.dim as usize;
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("t.sopack");

        let mut w = PackWriter::create(
            &out_path,
            &contract,
            "sop",
            "p1".into(),
            "test".into(),
            None,
            provenance(),
        )
        .unwrap();
        for i in 0..10 {
            w.add(
                &format!("en:TT:1.{}#0", i + 1),
                payload(i),
                &vector(dim, i as u32),
            )
            .unwrap();
        }
        finish_with_books_and_probe(&mut w, dim);
        let path = w.finish().unwrap();
        assert!(path.exists());
        assert!(!Path::new(&format!("{}.sopack.partial", path.display())).exists());

        let mut r = PackReader::open(&path).unwrap();
        let report = r.check(&contract);
        assert_eq!(report, Vec::<String>::new(), "{report:?}");
        assert_eq!(r.manifest.counts.points, 10);
        let mut seen = 0;
        for batch in r.batches(&contract, 4, true).unwrap() {
            let (points, vectors) = batch.unwrap();
            seen += points.len();
            assert_eq!(points.len(), vectors.len());
        }
        assert_eq!(seen, 10);
    }

    #[test]
    fn refuses_probe_below_pack_min_cosine() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let dim = contract.embedding.dim as usize;
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("t.sopack");
        let mut w = PackWriter::create(
            &out_path,
            &contract,
            "sop",
            "p1".into(),
            "test".into(),
            None,
            provenance(),
        )
        .unwrap();
        w.add("en:TT:1.1#0", payload(0), &vector(dim, 0)).unwrap();
        let err = w
            .set_probe(
                vec![ProbeEntryInput {
                    id: "fx-1".into(),
                    profile: "sop".into(),
                }],
                vec![vector(dim, 1)],
                1,
                0.5, // well below pack_min_cosine (0.9999)
                0.5,
            )
            .unwrap_err();
        assert!(err.to_string().contains("pack_min_cosine"), "{err}");
    }

    #[test]
    fn finish_without_probe_is_refused() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let dim = contract.embedding.dim as usize;
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("t.sopack");
        let mut w = PackWriter::create(
            &out_path,
            &contract,
            "sop",
            "p1".into(),
            "test".into(),
            None,
            provenance(),
        )
        .unwrap();
        w.add("en:TT:1.1#0", payload(0), &vector(dim, 0)).unwrap();
        w.set_books(vec![]);
        let err = w.finish().unwrap_err();
        assert!(err.to_string().contains("probe"), "{err}");
    }

    #[test]
    fn create_refuses_when_a_checkpoint_already_exists() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("t.sopack");
        let _w1 = PackWriter::create(
            &out_path,
            &contract,
            "sop",
            "p1".into(),
            "test".into(),
            None,
            provenance(),
        )
        .unwrap();
        let err = PackWriter::create(
            &out_path,
            &contract,
            "sop",
            "p2".into(),
            "test".into(),
            None,
            provenance(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("checkpoint already exists"),
            "{err}"
        );
    }

    #[test]
    fn crash_then_resume_is_byte_identical_to_one_run() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let dim = contract.embedding.dim as usize;

        // One continuous run.
        let dir_a = tempfile::tempdir().unwrap();
        let path_a = dir_a.path().join("t.sopack");
        {
            let mut w = PackWriter::create(
                &path_a,
                &contract,
                "sop",
                "same-id".into(),
                "same-created-by".into(),
                None,
                provenance(),
            )
            .unwrap();
            for i in 0..20 {
                w.add(
                    &format!("en:TT:1.{}#0", i + 1),
                    payload(i),
                    &vector(dim, i as u32),
                )
                .unwrap();
            }
            finish_with_books_and_probe(&mut w, dim);
            w.finish().unwrap();
        }

        // Crash-then-resume: writer dropped after 7 of 20 points, no finish().
        let dir_b = tempfile::tempdir().unwrap();
        let path_b = dir_b.path().join("t.sopack");
        {
            let mut w = PackWriter::create(
                &path_b,
                &contract,
                "sop",
                "same-id".into(),
                "same-created-by".into(),
                None,
                provenance(),
            )
            .unwrap();
            for i in 0..7 {
                w.add(
                    &format!("en:TT:1.{}#0", i + 1),
                    payload(i),
                    &vector(dim, i as u32),
                )
                .unwrap();
            }
            w.checkpoint().unwrap();
            drop(w); // simulated crash — no finish(), checkpoint stays on disk
        }
        assert!(Path::new(&format!("{}.sopack.partial", path_b.display())).exists());
        {
            let mut w = PackWriter::resume(&path_b, &contract).unwrap();
            assert_eq!(w.count(), 7);
            for i in 7..20 {
                w.add(
                    &format!("en:TT:1.{}#0", i + 1),
                    payload(i),
                    &vector(dim, i as u32),
                )
                .unwrap();
            }
            finish_with_books_and_probe(&mut w, dim);
            w.finish().unwrap();
        }

        let bytes_a = fs::read(&path_a).unwrap();
        let bytes_b = fs::read(&path_b).unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "resumed pack differs byte-for-byte from an uninterrupted run"
        );
    }

    #[test]
    fn resume_refuses_a_different_contract() {
        let cdir = test_contract_dir();
        let contract = Contract::from_dir(cdir.path()).unwrap();
        let dim = contract.embedding.dim as usize;
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("t.sopack");
        {
            let mut w = PackWriter::create(
                &out_path,
                &contract,
                "sop",
                "p1".into(),
                "test".into(),
                None,
                provenance(),
            )
            .unwrap();
            w.add("en:TT:1.1#0", payload(0), &vector(dim, 0)).unwrap();
            w.checkpoint().unwrap();
        }

        let other_cdir = test_contract_dir_variant("-different"); // different toml bytes
        let other_contract = Contract::from_dir(other_cdir.path()).unwrap();
        assert_ne!(other_contract.contract_sha256(), contract.contract_sha256());
        let err = PackWriter::resume(&out_path, &other_contract).unwrap_err();
        assert!(err.to_string().contains("different contract"), "{err}");
    }
}
