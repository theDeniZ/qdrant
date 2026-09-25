//! `sopack doctor [--quick]` — environment check (`SOPACK-1.0-PLAN.md` §3.5).
//! Every check is independent and best-effort: one failing check never stops
//! the rest from running, so the report always shows the full picture
//! (mirrors `sopack/doctor.py`'s design note verbatim).

use serde::Serialize;
use sopack_embed::{cache, memory, runtime};
use sopack_progress::{StageWeight, Unit};

use crate::args::{Cli, DoctorArgs};
use crate::contract_load::load_contract;
use crate::engine_build::resolve_model_dir;
use crate::exit::{CliError, EXIT_OK, EXIT_RESOURCES};
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;
use sopack_embed::device::Device;
use sopack_embed::engine::{Engine, EngineOptions};

#[derive(Serialize, Clone)]
struct Check {
    name: &'static str,
    ok: bool,
    level: &'static str,
    detail: String,
}

#[derive(Serialize)]
struct DoctorReport {
    checks: Vec<Check>,
    ok: bool,
}

fn ok(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        level: "info",
        detail: detail.into(),
    }
}

fn warn(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        level: "warning",
        detail: detail.into(),
    }
}

fn err(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
        level: "error",
        detail: detail.into(),
    }
}

/// Full-mode-only: actually load the model and run the calibration
/// self-check, exactly like `pack` will (matching `sopack/doctor.py`'s
/// `_check_model_loads`: "empirical" on purpose, since a cache-layout
/// heuristic cannot tell a working model from a broken one — only loading
/// it can). Costs real time (loading + embedding 16 fixture entries), which
/// is exactly why `--quick` skips it and says so rather than reporting a
/// bare pass.
fn run_model_load_check(
    contract: &sopack_contract::Contract,
    spec: &sopack_embed::spec::EmbedSpec,
    model_dir: &std::path::Path,
) -> Result<String, String> {
    let mut engine = Engine::new(
        spec.clone(),
        model_dir,
        EngineOptions {
            device: Device::Cpu,
            threads: Some(1),
            batch_tokens: None,
        },
        &sopack_progress::NullSink,
    )
    .map_err(|e| e.to_string())?;
    let fixture = contract
        .fixture
        .as_ref()
        .ok_or_else(|| "contract has no calibration fixture loaded".to_string())?;
    let embed_fixture: sopack_embed::calibration::CalibrationFixture = fixture.into();
    let result = engine
        .calibrate(&embed_fixture, &sopack_progress::NullSink)
        .map_err(|e| e.to_string())?;
    let threshold = contract.calibration.pack_min_cosine;
    if result.passes(threshold) {
        Ok(format!(
            "loaded {}, embedded {} fixture entries, calibration min cosine {:.6} (threshold {threshold})",
            spec.model_id, result.n, result.min_cosine
        ))
    } else {
        Err(format!(
            "loaded {} but calibration min cosine {:.6} is below the threshold {threshold}",
            spec.model_id, result.min_cosine
        ))
    }
}

pub fn run(cli: &Cli, args: &DoctorArgs) -> Result<i32, CliError> {
    let progress = build_progress(cli.progress, vec![StageWeight::new("doctor", 1.0)]);
    let mut checks = Vec::new();
    let total: u64 = 9;
    progress.stage_start("doctor", total, Unit::Checks);

    checks.push(ok(
        "version",
        format!("sopack {}", env!("CARGO_PKG_VERSION")),
    ));
    progress.advance(1);

    let contract = match load_contract(&cli.contract) {
        Ok(c) => {
            checks.push(ok(
                "contract",
                format!(
                    "{} (sha256 {}…, calibration {})",
                    c.id,
                    &c.contract_sha256()[..12],
                    c.calibration_sha256()
                        .map(|s| format!("{}…", &s[..12]))
                        .unwrap_or_else(|| "not loaded".to_string())
                ),
            ));
            Some(c)
        }
        Err(e) => {
            checks.push(err("contract", e.message.clone()));
            None
        }
    };
    progress.advance(1);

    let spec = contract.as_ref().map(sopack_embed::spec::EmbedSpec::from);
    let model_dir = contract
        .as_ref()
        .map(|c| resolve_model_dir(c, args.model_dir.as_deref()));
    checks.push(ok(
        "cache_dir",
        cache::default_cache_dir().display().to_string(),
    ));
    progress.advance(1);

    // ---- model
    if let (Some(spec), Some(model_dir)) = (&spec, &model_dir) {
        if args.quick {
            match sopack_embed::verify::verify_model(model_dir, spec, &sopack_progress::NullSink) {
                Ok(()) => checks.push(ok(
                    "model",
                    format!("present at {} (sizes only — NOT sha256-verified)", model_dir.display()),
                )),
                Err(e) => checks.push(warn(
                    "model",
                    format!(
                        "not verified ({e}) — run `sopack model fetch` or a non-quick `sopack doctor`"
                    ),
                )),
            }
        } else {
            match sopack_embed::verify::verify_model(model_dir, spec, &sopack_progress::NullSink) {
                Ok(()) => checks.push(ok(
                    "model",
                    format!("verified (sha256) at {}", model_dir.display()),
                )),
                Err(e) => checks.push(err("model", e.to_string())),
            }
        }
    } else {
        checks.push(err(
            "model",
            "no contract loaded, cannot resolve a model directory",
        ));
    }
    progress.advance(1);

    // ---- ONNX Runtime dylib
    match runtime::resolve_ort_dylib() {
        Ok(path) => match runtime::ort_dylib_version(&path) {
            Ok(version) => checks.push(ok(
                "onnxruntime",
                format!("{version} at {}", path.display()),
            )),
            Err(e) => checks.push(err(
                "onnxruntime",
                format!(
                    "found {} but could not read its version: {e}",
                    path.display()
                ),
            )),
        },
        Err(e) => {
            if args.quick {
                checks.push(warn(
                    "onnxruntime",
                    format!("not found (quick mode, not required): {e}"),
                ));
            } else {
                checks.push(err("onnxruntime", e.to_string()));
            }
        }
    }
    progress.advance(1);

    // ---- model loads + calibration (empirical; full mode only)
    if args.quick {
        checks.push(warn("model_loads", "skipped (--quick) — NOT VERIFIED"));
    } else if let (Some(c), Some(spec), Some(model_dir)) = (&contract, &spec, &model_dir) {
        match run_model_load_check(c, spec, model_dir) {
            Ok(detail) => checks.push(ok("model_loads", detail)),
            Err(detail) => checks.push(err("model_loads", detail)),
        }
    } else {
        checks.push(err(
            "model_loads",
            "no contract loaded, cannot run this check",
        ));
    }
    progress.advance(1);

    // ---- CPU / threads
    let cpus = std::thread::available_parallelism().map(|n| n.get());
    match cpus {
        Ok(n) => checks.push(ok(
            "cpu_count",
            format!("{n} (ORT intra/inter threads default)"),
        )),
        Err(e) => checks.push(err("cpu_count", e.to_string())),
    }
    progress.advance(1);

    // ---- memory vs guard need
    match memory::available_bytes() {
        Ok(available) => {
            let model_bytes = spec.as_ref().map(|s| s.total_bytes()).unwrap_or(0);
            let needed = memory::estimate_peak_bytes(model_bytes, 512, 1024);
            let detail = format!(
                "{:.2} GiB available, ~{:.2} GiB estimated need (model + batch activations + margin)",
                available as f64 / (1 << 30) as f64,
                needed as f64 / (1 << 30) as f64
            );
            if available >= needed {
                checks.push(ok("memory", detail));
            } else {
                checks.push(err("memory", detail));
            }
        }
        Err(e) => checks.push(err("memory", e.to_string())),
    }
    progress.advance(1);

    // ---- temp dir writable
    match tempfile::NamedTempFile::new_in(std::env::temp_dir()) {
        Ok(_) => checks.push(ok(
            "tmp_writable",
            std::env::temp_dir().display().to_string(),
        )),
        Err(e) => checks.push(err("tmp_writable", e.to_string())),
    }
    progress.advance(1);

    progress.stage_end();
    progress.done();

    let overall_ok = checks.iter().all(|c| c.ok);
    let report = DoctorReport {
        checks: checks.clone(),
        ok: overall_ok,
    };

    if cli.json {
        print_json(&serde_json::to_value(&report).unwrap());
    } else {
        let width = checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
        for c in &checks {
            let mark = if c.ok {
                if c.level == "warning" {
                    "WARN"
                } else {
                    "PASS"
                }
            } else {
                "FAIL"
            };
            print_text(&format!("[{mark}] {:width$}  {}", c.name, c.detail));
        }
        print_text(if overall_ok {
            "all checks passed"
        } else {
            "one or more checks FAILED"
        });
    }

    Ok(if overall_ok { EXIT_OK } else { EXIT_RESOURCES })
}
