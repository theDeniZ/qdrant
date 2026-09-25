//! `Engine` — one loaded model + tokenizer + ONNX Runtime session, and the
//! embedding pipeline over it (SOPACK-1.0-PLAN.md §3.1/§3.3).
//!
//! Pipeline: tokenise every text up front with `rayon` (this is the
//! "pipeline overlap" the plan asks for, done the simple/safe way — since
//! there is only ever one ORT session, the two things that *can* run
//! concurrently are tokenising many texts and running one batch through the
//! model, and front-loading every tokenisation before the batch loop starts
//! means the parallel tokenising work is never serialised behind ORT calls,
//! without needing a background-thread double-buffer), plan length-sorted
//! token-budget batches (`batch::plan_batches`), run each batch through the
//! session, pool + normalise (`pool::pool_and_normalize`), and scatter
//! results back to the caller's original order (`batch::scatter`).

use crate::batch;
use crate::calibration::{CalibrationFixture, CalibrationResult};
use crate::device::{self, Device};
use crate::error::{EmbedError, Result};
use crate::memory;
use crate::pool;
use crate::runtime;
use crate::spec::{EmbedSpec, Role};
use crate::tokenizer::load_tokenizer;
use ndarray::Array2;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use rayon::prelude::*;
use sopack_progress::{ProgressSink, Unit};
use std::path::Path;
use tokenizers::Tokenizer;

/// The `ort` crate version this workspace pins (`Cargo.toml`
/// `[workspace.dependencies]`) — a compile-time constant kept in sync by
/// hand, same as the pin itself; bumping one without the other is a lie
/// about what was measured (SOPACK-1.0-PLAN.md §6 "bumping them means
/// re-measuring").
pub const ORT_CRATE_VERSION: &str = "2.0.0-rc.13";

/// CPU default from the M0 spike's budget sweep: batch 1 (2.73 blocks/s) was
/// *faster* than batch 32 (0.99 blocks/s) on this CPU, and a 512-token
/// budget matched batch-1 throughput while still giving the batching
/// machinery something to do — see `spike/README.md` "What this changes in
/// the plan" §2. Larger budgets only help a GPU/CoreML device.
const DEFAULT_BATCH_TOKENS_CPU: usize = 512;
/// GPU/CoreML default — one of the budgets the M0 sweep measured
/// (`spike/README.md`'s `sorted, budget 8192`), well above the CPU default
/// since a GPU pays for kernel-launch/host-sync overhead per batch rather
/// than per padded token the way a CPU convolution over padding does.
const DEFAULT_BATCH_TOKENS_GPU: usize = 8192;

fn ort_err<T>(e: ort::Error<T>) -> EmbedError {
    EmbedError::Ort(e.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct EngineOptions {
    /// A literal `Device::Auto` here is treated exactly like `Device::Cpu`
    /// (no execution providers registered, CPU batch-token default) — full
    /// auto-resolution-with-fallback across candidate devices is
    /// `build_with_device_check`'s job, which calls `Engine::new` once per
    /// concrete candidate.
    pub device: Device,
    pub threads: Option<usize>,
    pub batch_tokens: Option<usize>,
}

/// One loaded model, ready to embed. Not `Clone` (an `ort::Session` owns
/// native resources) — build a new one per device via `Engine::new` or
/// `build_with_device_check`.
pub struct Engine {
    session: Session,
    tokenizer: Tokenizer,
    spec: EmbedSpec,
    device: Device,
    threads: usize,
    batch_tokens: usize,
    ort_dylib_path: std::path::PathBuf,
    onnxruntime_version: String,
}

impl Engine {
    /// Loads the model and tokenizer at `model_dir` and builds one ORT
    /// session for `options.device`. Order: resolve threads/batch_tokens →
    /// memory guard → resolve + init the ONNX Runtime dylib (idempotent
    /// across repeated calls in one process, see `runtime::init`) → build
    /// the session with `options.device`'s execution providers → load the
    /// tokenizer.
    pub fn new(
        spec: EmbedSpec,
        model_dir: &Path,
        options: EngineOptions,
        progress: &dyn ProgressSink,
    ) -> Result<Engine> {
        let threads = options
            .threads
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            })
            .max(1);
        let resolved_device = if options.device == Device::Auto {
            Device::Cpu
        } else {
            options.device
        };
        let batch_tokens = options.batch_tokens.unwrap_or(match resolved_device {
            Device::Cpu | Device::Auto => DEFAULT_BATCH_TOKENS_CPU,
            Device::CoreMl | Device::Cuda => DEFAULT_BATCH_TOKENS_GPU,
        });

        memory::guard(spec.total_bytes(), batch_tokens, spec.dim)?;

        progress.stage_start("load_model", 3, Unit::Items);
        let init = runtime::init()?;
        progress.advance(1);

        let eps = device::execution_providers(resolved_device)?;
        let mut builder = Session::builder()
            .map_err(ort_err)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(ort_err)?
            .with_intra_threads(threads)
            .map_err(ort_err)?
            .with_inter_threads(threads)
            .map_err(ort_err)?;
        if !eps.is_empty() {
            builder = builder.with_execution_providers(eps).map_err(ort_err)?;
        }
        let session = builder
            .commit_from_file(model_dir.join(&spec.onnx_file))
            .map_err(|e| EmbedError::SessionLoad(e.to_string()))?;
        progress.advance(1);

        let tokenizer = load_tokenizer(model_dir, &spec)?;
        progress.advance(1);
        progress.stage_end();

        Ok(Engine {
            session,
            tokenizer,
            spec,
            device: resolved_device,
            threads,
            batch_tokens,
            ort_dylib_path: init.dylib_path,
            onnxruntime_version: init.version,
        })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    pub fn batch_tokens(&self) -> usize {
        self.batch_tokens
    }

    pub fn onnxruntime_version(&self) -> &str {
        &self.onnxruntime_version
    }

    pub fn ort_dylib_path(&self) -> &Path {
        &self.ort_dylib_path
    }

    /// `"sopack-rs <sopack_version>; ort <ort_crate_version>; onnxruntime <loaded_version>"`
    /// — the `embedding.runtime` provenance string, SOPACK-2-FORMAT.md §2.
    pub fn runtime_string(&self, sopack_version: &str) -> String {
        format!(
            "sopack-rs {sopack_version}; ort {ORT_CRATE_VERSION}; onnxruntime {}",
            self.onnxruntime_version
        )
    }

    pub fn spec(&self) -> &EmbedSpec {
        &self.spec
    }

    /// Sums each text's token length after truncation, without running the
    /// model — for throughput reporting (tokens/s) without re-tokenising by
    /// hand (the `bench` example).
    pub fn count_tokens(&self, texts: &[String]) -> Result<u64> {
        let tokenizer = &self.tokenizer;
        texts
            .par_iter()
            .map(|t| {
                tokenizer
                    .encode(t.as_str(), true)
                    .map(|e| e.get_ids().len() as u64)
                    .map_err(|e| EmbedError::Tokenize(e.to_string()))
            })
            .try_reduce(|| 0u64, |a, b| Ok(a + b))
    }

    /// Embeds `texts` with `spec.prefix(role)` prepended to each, in
    /// original order. Reports progress on `"embed"` in tokens.
    pub fn embed(
        &mut self,
        texts: &[String],
        role: Role,
        progress: &dyn ProgressSink,
    ) -> Result<Vec<Vec<f32>>> {
        let prefix = self.spec.prefix(role).to_string();
        let prefixed: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
        self.embed_no_prefix(&prefixed, progress)
    }

    /// Embeds `fixture`'s entries (`passage_prefix + text`, per
    /// SOPACK-2-FORMAT.md §3) with this engine and scores them against the
    /// fixture's stored vectors.
    pub fn calibrate(
        &mut self,
        fixture: &CalibrationFixture,
        progress: &dyn ProgressSink,
    ) -> Result<CalibrationResult> {
        let prefix = self.spec.passage_prefix.clone();
        let texts: Vec<String> = fixture
            .entries
            .iter()
            .map(|e| format!("{prefix}{}", e.text))
            .collect();
        // Reported as its own "calibrate" stage: the inner embed call's
        // "embed" stage boundaries are swallowed so the caller's plan weight
        // for "embed" is left for the books (a shared stage name used to
        // consume it, freezing `pack`'s percentage for the whole embed phase).
        let total = self.count_tokens(&texts)?;
        progress.stage_start("calibrate", total, Unit::Tokens);
        let produced = self.embed_no_prefix(&texts, &AdvanceOnly(progress))?;
        progress.stage_end();
        Ok(crate::calibration::score(fixture, &produced))
    }

    /// The pipeline described in this module's doc comment, over texts that
    /// already carry whatever prefix they need (or none).
    fn embed_no_prefix(
        &mut self,
        texts: &[String],
        progress: &dyn ProgressSink,
    ) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let tokenizer = &self.tokenizer;
        let lens: Vec<usize> = texts
            .par_iter()
            .map(|t| {
                tokenizer
                    .encode(t.as_str(), true)
                    .map(|e| e.get_ids().len())
                    .map_err(|e| EmbedError::Tokenize(e.to_string()))
            })
            .collect::<std::result::Result<_, _>>()?;
        let total_tokens: u64 = lens.iter().map(|&l| l as u64).sum();
        progress.stage_start("embed", total_tokens, Unit::Tokens);

        let plan = batch::plan_batches(&lens, self.batch_tokens);
        let mut batch_results = Vec::with_capacity(plan.len());
        for idxs in plan {
            let batch_texts: Vec<&str> = idxs.iter().map(|&i| texts[i].as_str()).collect();
            let vecs = self.embed_batch(&batch_texts)?;
            let batch_tokens: u64 = idxs.iter().map(|&i| lens[i] as u64).sum();
            progress.advance(batch_tokens);
            batch_results.push((idxs, vecs));
        }
        progress.stage_end();
        Ok(batch::scatter(texts.len(), batch_results))
    }

    /// One ORT call: tokenise (pads to the batch's own longest — the whole
    /// point of length-sorted batching), run the session, mean/cls-pool and
    /// L2-normalise. Vector-identical to the M0 spike's `embed_batch`
    /// (`spike/src/main.rs`) for `Pooling::Mean`.
    fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let enc = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| EmbedError::Tokenize(e.to_string()))?;
        let (b, l) = (
            enc.len(),
            enc.first().map(|e| e.get_ids().len()).unwrap_or(0),
        );
        let mut ids = Array2::<i64>::zeros((b, l));
        let mut mask = Array2::<i64>::zeros((b, l));
        for (i, e) in enc.iter().enumerate() {
            for (j, (&t, &m)) in e.get_ids().iter().zip(e.get_attention_mask()).enumerate() {
                ids[[i, j]] = t as i64;
                mask[[i, j]] = m as i64;
            }
        }

        let want_types = self
            .session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let outputs = if want_types {
            let types = Array2::<i64>::zeros((b, l));
            self.session
                .run(ort::inputs![
                    "input_ids" => Tensor::from_array(ids).map_err(ort_err)?,
                    "attention_mask" => Tensor::from_array(mask.clone()).map_err(ort_err)?,
                    "token_type_ids" => Tensor::from_array(types).map_err(ort_err)?,
                ])
                .map_err(ort_err)?
        } else {
            self.session
                .run(ort::inputs![
                    "input_ids" => Tensor::from_array(ids).map_err(ort_err)?,
                    "attention_mask" => Tensor::from_array(mask.clone()).map_err(ort_err)?,
                ])
                .map_err(ort_err)?
        };

        let output_value = outputs.get(self.spec.output_name.as_str()).ok_or_else(|| {
            EmbedError::SessionLoad(format!(
                "session produced no output named {:?} (contract's [model].output)",
                self.spec.output_name
            ))
        })?;
        let hidden = output_value.try_extract_array::<f32>().map_err(ort_err)?; // [b, l, d]
        let (_, _, d) = (hidden.shape()[0], hidden.shape()[1], hidden.shape()[2]);
        if d != self.spec.dim {
            return Err(EmbedError::DimensionMismatch {
                expected: self.spec.dim,
                got: d,
            });
        }
        let hidden3 = hidden
            .into_dimensionality::<ndarray::Ix3>()
            .map_err(|e| EmbedError::SessionLoad(format!("expected a 3-D output tensor: {e}")))?;
        Ok(pool::pool_and_normalize(
            &hidden3.view(),
            &mask,
            self.spec.pooling,
            self.spec.normalize,
        ))
    }
}

/// What one device-selection attempt resolved to — recorded in the pack
/// manifest as provenance (R10 / SOPACK-2-FORMAT.md §2 `embedding.device`).
#[derive(Debug, Clone)]
pub struct DeviceReport {
    pub requested: Device,
    pub used: Device,
    pub fell_back: bool,
    /// `None` when `used == Cpu` and no self-verification was needed to get
    /// there (SOPACK-1.0-PLAN.md §3.6: the gate applies to non-CPU runs).
    pub calibration: Option<CalibrationResult>,
}

/// Parameters for `build_with_device_check` that aren't the "what to build
/// against" triple (`spec`, `model_dir`, `requested`) — bundled into one
/// struct rather than piled onto the function signature.
#[derive(Debug, Clone, Copy)]
pub struct DeviceCheckOptions<'a> {
    pub threads: Option<usize>,
    pub batch_tokens: Option<usize>,
    pub calibration_fixture: &'a CalibrationFixture,
    pub pack_min_cosine: f64,
}

/// Builds an `Engine`, resolving `Device::Auto` to a concrete device and
/// self-verifying every non-CPU attempt against `options.calibration_fixture`
/// before accepting it (SOPACK-1.0-PLAN.md §3.6): below
/// `options.pack_min_cosine`, `Device::Auto` tries the next candidate
/// (falling back to `Cpu`, which is always tried last and never itself
/// gated); an explicit non-CPU device refuses outright with
/// `EmbedError::CalibrationFailed`.
pub fn build_with_device_check(
    spec: &EmbedSpec,
    model_dir: &Path,
    requested: Device,
    progress: &dyn ProgressSink,
    options: DeviceCheckOptions<'_>,
) -> Result<(Engine, DeviceReport)> {
    let candidates: Vec<Device> = if requested == Device::Auto {
        device::auto_candidates()
    } else {
        vec![requested]
    };
    let mut last_err: Option<EmbedError> = None;

    for candidate in candidates {
        let engine_options = EngineOptions {
            device: candidate,
            threads: options.threads,
            batch_tokens: options.batch_tokens,
        };
        let mut engine = match Engine::new(spec.clone(), model_dir, engine_options, progress) {
            Ok(e) => e,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };

        if candidate == Device::Cpu {
            return Ok((
                engine,
                DeviceReport {
                    requested,
                    used: candidate,
                    fell_back: requested != candidate,
                    calibration: None,
                },
            ));
        }

        let result = engine.calibrate(options.calibration_fixture, progress)?;
        if result.passes(options.pack_min_cosine) {
            return Ok((
                engine,
                DeviceReport {
                    requested,
                    used: candidate,
                    fell_back: requested != candidate,
                    calibration: Some(result),
                },
            ));
        }
        if requested != Device::Auto {
            return Err(EmbedError::CalibrationFailed {
                min: result.min_cosine,
                threshold: options.pack_min_cosine,
                n: result.n,
            });
        }
        progress.warn(&format!("device {candidate} scored {:.6} against the calibration fixture (below {:.6}) — falling back", result.min_cosine, options.pack_min_cosine));
    }

    Err(last_err.unwrap_or(EmbedError::DeviceNotAvailable {
        device: requested.to_string(),
    }))
}

/// Forwards advances and warnings to an already-open stage of the wrapped
/// sink and drops the stage boundaries of the call it is handed to.
struct AdvanceOnly<'a>(&'a dyn ProgressSink);

impl ProgressSink for AdvanceOnly<'_> {
    fn stage_start(&self, _stage: &str, _total: u64, _unit: Unit) {}
    fn advance(&self, n: u64) {
        self.0.advance(n);
    }
    fn warn(&self, msg: &str) {
        self.0.warn(msg);
    }
    fn stage_end(&self) {}
    fn done(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_options_default_device_is_cpu() {
        assert_eq!(EngineOptions::default().device, Device::Cpu);
    }
}
