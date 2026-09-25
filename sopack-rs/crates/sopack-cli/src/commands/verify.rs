//! `sopack verify` — offline `.sopack` integrity check (reads `/1` and
//! `/2`; exit 0 clean, exit 3 when problems, byte progress).

use serde::Serialize;
use sopack_progress::{StageWeight, Unit};

use crate::args::{Cli, VerifyArgs};
use crate::contract_load::load_contract;
use crate::exit::{CliError, EXIT_INPUT_INVALID, EXIT_OK};
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;

#[derive(Serialize)]
struct VerifyResult {
    pack: String,
    clean: bool,
    errors: Vec<String>,
}

pub fn run(cli: &Cli, args: &VerifyArgs) -> Result<i32, CliError> {
    let contract = load_contract(&cli.contract)?;
    let total_bytes = std::fs::metadata(&args.pack).map(|m| m.len()).unwrap_or(0);
    let progress = build_progress(cli.progress, vec![StageWeight::new("verify", 1.0)]);
    progress.stage_start("verify", total_bytes, Unit::Bytes);

    let mut last_done = 0u64;
    let report = sopack_format::verify_with_progress(&args.pack, &contract, |done, total| {
        let scaled = if total > 0 {
            (done as u128 * total_bytes as u128 / total as u128) as u64
        } else {
            total_bytes
        };
        if scaled > last_done {
            progress.advance(scaled - last_done);
            last_done = scaled;
        }
    });
    progress.stage_end();
    progress.done();

    let result = VerifyResult {
        pack: args.pack.display().to_string(),
        clean: report.is_clean(),
        errors: report.errors.clone(),
    };

    if cli.json {
        print_json(&serde_json::to_value(&result).unwrap());
    } else if report.is_clean() {
        if !cli.quiet {
            print_text(&format!("{}: clean", args.pack.display()));
        }
    } else {
        print_text(&format!("{} problem(s) found:", report.errors.len()));
        for e in &report.errors {
            print_text(&format!("  - {e}"));
        }
    }

    Ok(if report.is_clean() {
        EXIT_OK
    } else {
        EXIT_INPUT_INVALID
    })
}
