//! Shared engine-building logic for `pack` and `calibrate`: resolve the
//! model directory, resolve `--device`, and build a
//! [`sopack_embed::engine::Engine`] with device self-verification
//! (`SOPACK-1.0-PLAN.md` §3.6). One function so the two commands can never
//! disagree on how a device/model-dir override resolves.

use std::path::{Path, PathBuf};

use sopack_contract::Contract;
use sopack_embed::cache;
use sopack_embed::device::Device;
use sopack_embed::engine::{DeviceCheckOptions, DeviceReport, Engine};
use sopack_progress::ProgressSink;

use crate::args::EngineOpts;
use crate::exit::CliError;

/// The model directory a run will use: `--model-dir` if given, otherwise
/// the contract's model under the resolved platform cache dir
/// (`sopack model path` reports the same value).
pub fn resolve_model_dir(contract: &Contract, override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(p) => p.to_path_buf(),
        None => {
            let spec = sopack_embed::spec::EmbedSpec::from(contract);
            cache::model_dir_in_cache(&cache::default_cache_dir(), &spec)
        }
    }
}

/// Verifies the model is present (exit 6 with the `sopack model fetch` hint
/// if not — SOPACK-1.0-PLAN.md's `pack` order: "model present+verified …
/// else exit 6"), then builds an `Engine` for `opts.device`, self-verifying
/// every non-CPU candidate against the contract's calibration fixture.
pub fn build_engine(
    contract: &Contract,
    opts: &EngineOpts,
    progress: &dyn ProgressSink,
) -> Result<(Engine, DeviceReport, PathBuf), CliError> {
    let model_dir = resolve_model_dir(contract, opts.model_dir.as_deref());
    let spec = sopack_embed::spec::EmbedSpec::from(contract);

    sopack_embed::verify::verify_model(&model_dir, &spec, progress).map_err(|e| {
        CliError::resources(format!(
            "model at {} is missing or does not match the contract: {e}",
            model_dir.display()
        ))
        .with_hint(format!(
            "run `sopack model fetch --model-dir {}` (or without --model-dir to use the \
             default cache) to download it",
            model_dir.display()
        ))
    })?;

    let device: Device = opts
        .device
        .parse()
        .map_err(|e: String| CliError::usage(e))?;

    let fixture = contract.fixture.as_ref().ok_or_else(|| {
        CliError::resources(format!(
            "contract {:?} has no calibration fixture loaded — cannot self-verify any device",
            contract.id
        ))
    })?;
    let embed_fixture: sopack_embed::calibration::CalibrationFixture = fixture.into();

    let (engine, report) = sopack_embed::engine::build_with_device_check(
        &spec,
        &model_dir,
        device,
        progress,
        DeviceCheckOptions {
            threads: opts.threads,
            batch_tokens: opts.batch_tokens,
            calibration_fixture: &embed_fixture,
            pack_min_cosine: contract.calibration.pack_min_cosine,
        },
    )?;
    Ok((engine, report, model_dir))
}
