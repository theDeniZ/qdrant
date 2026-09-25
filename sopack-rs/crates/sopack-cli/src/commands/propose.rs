//! `sopack propose <source> [--write-meta]` — metadata *draft* engine
//! (`SOPACK-1.0-PLAN.md` §3.5).

use sopack_extract::{meta, propose::propose};

use crate::args::{Cli, ProposeArgs};
use crate::exit::{CliError, EXIT_OK};
use crate::extract_progress::ExtractProgressAdapter;
use crate::output::{print_json, print_text};
use crate::progress_build::build_progress;
use crate::registry_load::load_registry;

pub fn run(cli: &Cli, args: &ProposeArgs) -> Result<i32, CliError> {
    let progress = build_progress(
        cli.progress,
        vec![sopack_progress::StageWeight::new("propose", 1.0)],
    );

    let contract_dir = {
        let p = std::path::Path::new(&cli.contract);
        if p.is_dir() {
            Some(p)
        } else {
            None
        }
    };
    let registry = load_registry(&cli.contract, contract_dir, args.registry.as_deref())?;

    let kind = args.kind.map(|k| k.to_extract_kind());
    let mut adapter = ExtractProgressAdapter::new(progress.as_ref());
    let proposal = propose(&args.source, kind, Some(&registry), &mut adapter)?;
    progress.done();

    if args.write_meta {
        let mut sidecar_path = args.source.as_os_str().to_owned();
        sidecar_path.push(".meta.toml");
        let sidecar_path = std::path::PathBuf::from(sidecar_path);
        meta::write_template(&proposal, &sidecar_path)?;
        if cli.json {
            print_json(&serde_json::json!({
                "wrote": sidecar_path.display().to_string(),
                "unresolved": proposal.unresolved,
            }));
        } else if !cli.quiet {
            print_text(&format!("wrote {}", sidecar_path.display()));
            if !proposal.unresolved.is_empty() {
                print_text(&format!(
                    "  still unresolved: {}",
                    proposal.unresolved.join(", ")
                ));
            }
        }
        return Ok(EXIT_OK);
    }

    if cli.json {
        print_json(&serde_json::to_value(&proposal).unwrap());
    } else {
        print_text(&format!("source: {}", proposal.source.path));
        for (name, field) in proposal.fields.iter() {
            let resolved = if field.value.is_null() {
                "(unresolved)".to_string()
            } else {
                field.value.to_string()
            };
            print_text(&format!(
                "  {name}: {resolved}  ({} candidate(s))",
                field.candidates.len()
            ));
        }
        if !proposal.unresolved.is_empty() {
            print_text(&format!("unresolved: {}", proposal.unresolved.join(", ")));
        }
        for w in &proposal.warnings {
            print_text(&format!("warning: {w}"));
        }
    }
    Ok(EXIT_OK)
}
