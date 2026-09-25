//! `sopack model fetch|import <dir>|verify|path`.

use serde::Serialize;
use sopack_embed::{cache, verify};
use sopack_progress::StageWeight;

use crate::args::{Cli, ModelAction, ModelArgs, ModelDirArgs, ModelImportArgs};
use crate::contract_load::load_contract;
use crate::engine_build::resolve_model_dir;
use crate::exit::{CliError, EXIT_OK};
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;

#[derive(Serialize)]
struct ModelActionResult {
    action: &'static str,
    model_dir: String,
    files: usize,
    bytes: u64,
}

pub fn run(cli: &Cli, args: &ModelArgs) -> Result<i32, CliError> {
    let contract = load_contract(&cli.contract)?;
    let spec = sopack_embed::spec::EmbedSpec::from(&contract);

    match &args.action {
        ModelAction::Fetch(dir_args) => fetch(cli, &spec, dir_args),
        ModelAction::Import(import_args) => import(cli, &spec, import_args),
        ModelAction::Verify(dir_args) => verify_action(cli, &spec, dir_args),
        ModelAction::Path(dir_args) => path(cli, &contract, dir_args),
    }
}

fn fetch(
    cli: &Cli,
    spec: &sopack_embed::spec::EmbedSpec,
    dir_args: &ModelDirArgs,
) -> Result<i32, CliError> {
    let model_dir = dir_args
        .model_dir
        .clone()
        .unwrap_or_else(|| cache::model_dir_in_cache(&cache::default_cache_dir(), spec));
    let progress = build_progress(cli.progress, vec![StageWeight::new("download", 1.0)]);
    sopack_embed::fetch::fetch_model(spec, &model_dir, progress.as_ref())?;
    progress.done();
    report(cli, "fetch", &model_dir, spec)
}

fn import(
    cli: &Cli,
    spec: &sopack_embed::spec::EmbedSpec,
    args: &ModelImportArgs,
) -> Result<i32, CliError> {
    let model_dir = args
        .model_dir
        .model_dir
        .clone()
        .unwrap_or_else(|| cache::model_dir_in_cache(&cache::default_cache_dir(), spec));
    let progress = build_progress(cli.progress, vec![StageWeight::new("import", 1.0)]);
    sopack_embed::fetch::import_model(spec, &args.dir, &model_dir, progress.as_ref())?;
    progress.done();
    report(cli, "import", &model_dir, spec)
}

fn verify_action(
    cli: &Cli,
    spec: &sopack_embed::spec::EmbedSpec,
    dir_args: &ModelDirArgs,
) -> Result<i32, CliError> {
    let model_dir = dir_args
        .model_dir
        .clone()
        .unwrap_or_else(|| cache::model_dir_in_cache(&cache::default_cache_dir(), spec));
    let progress = build_progress(cli.progress, vec![StageWeight::new("verify", 1.0)]);
    verify::verify_model(&model_dir, spec, progress.as_ref())?;
    progress.done();
    report(cli, "verify", &model_dir, spec)
}

fn path(
    cli: &Cli,
    contract: &sopack_contract::Contract,
    dir_args: &ModelDirArgs,
) -> Result<i32, CliError> {
    let model_dir = resolve_model_dir(contract, dir_args.model_dir.as_deref());
    let exists = model_dir.is_dir();
    if cli.json {
        print_json(&serde_json::json!({"path": model_dir.display().to_string(), "exists": exists}));
    } else {
        print_text(&model_dir.display().to_string());
    }
    Ok(EXIT_OK)
}

fn report(
    cli: &Cli,
    action: &'static str,
    model_dir: &std::path::Path,
    spec: &sopack_embed::spec::EmbedSpec,
) -> Result<i32, CliError> {
    let result = ModelActionResult {
        action,
        model_dir: model_dir.display().to_string(),
        files: spec.files.len(),
        bytes: spec.total_bytes(),
    };
    if cli.json {
        print_json(&serde_json::to_value(&result).unwrap());
    } else if !cli.quiet {
        print_text(&format!(
            "{action}: {} ({} files, {} bytes) -> {}",
            spec.model_id,
            result.files,
            result.bytes,
            model_dir.display()
        ));
    }
    Ok(EXIT_OK)
}
