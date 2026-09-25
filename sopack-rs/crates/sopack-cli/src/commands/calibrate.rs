//! `sopack calibrate` — embeds the fixture, prints per-entry cosines,
//! min/mean, pass/fail (exit 5 on fail). Used by CI.

use serde::Serialize;
use sopack_progress::StageWeight;

use crate::args::{CalibrateArgs, Cli};
use crate::contract_load::load_contract;
use crate::engine_build::build_engine;
use crate::exit::{CliError, EXIT_CALIBRATION_FAILED, EXIT_OK};
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;

#[derive(Serialize)]
struct CalibrateResult {
    device: String,
    n: usize,
    min_cosine: f64,
    mean_cosine: f64,
    threshold: f64,
    pass: bool,
    per_entry: Vec<(String, f64)>,
}

pub fn run(cli: &Cli, args: &CalibrateArgs) -> Result<i32, CliError> {
    let contract = load_contract(&cli.contract)?;
    let progress = build_progress(
        cli.progress,
        vec![
            StageWeight::new("verify", 1.0),
            StageWeight::new("load_model", 1.0),
            StageWeight::new("calibrate", 3.0),
        ],
    );

    let (mut engine, device_report, _model_dir) =
        build_engine(&contract, &args.engine, progress.as_ref())?;

    let result = if let Some(calib) = device_report.calibration {
        calib
    } else {
        let fixture = contract.fixture.as_ref().ok_or_else(|| {
            CliError::resources(format!(
                "contract {:?} has no calibration fixture loaded",
                contract.id
            ))
        })?;
        let embed_fixture: sopack_embed::calibration::CalibrationFixture = fixture.into();
        engine.calibrate(&embed_fixture, progress.as_ref())?
    };
    progress.done();

    let threshold = contract.calibration.pack_min_cosine;
    let pass = result.passes(threshold);
    let out = CalibrateResult {
        device: engine.device().to_string(),
        n: result.n,
        min_cosine: result.min_cosine,
        mean_cosine: result.mean_cosine,
        threshold,
        pass,
        per_entry: result.per_entry.clone(),
    };

    if cli.json {
        print_json(&serde_json::to_value(&out).unwrap());
    } else {
        print_text(&format!("device: {}", out.device));
        for (id, cos) in &out.per_entry {
            print_text(&format!("  {id}: {cos:.8}"));
        }
        print_text(&format!(
            "min={:.8} mean={:.8} threshold={} -> {}",
            out.min_cosine,
            out.mean_cosine,
            out.threshold,
            if out.pass { "PASS" } else { "FAIL" }
        ));
    }

    // The full per-entry result (pass or fail) is the single JSON/text
    // document this command prints — a failure is a *result*, not a
    // `CliError`, so it is never wrapped a second time in an error object;
    // only the exit code changes (SOPACK-1.0-PLAN.md §3.5: exit 5).
    Ok(if pass {
        EXIT_OK
    } else {
        EXIT_CALIBRATION_FAILED
    })
}
