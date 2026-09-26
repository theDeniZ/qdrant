//! `sopack commands --json` — subcommands, flags, result schemas and exit
//! codes, generated from clap's own command tree
//! (`clap::CommandFactory`/`Cli::command()`) so it cannot drift from what
//! `main.rs` actually dispatches on (`SOPACK-1.0-PLAN.md` §3.5).

use clap::CommandFactory;
use serde::Serialize;

use crate::args::Cli;
use crate::exit::{CliError, EXIT_CODE_TABLE, EXIT_OK};
use crate::output::{print_json, print_text};

/// Argument ids declared `global = true` on [`crate::args::Cli`] — excluded
/// from each subcommand's own `args` list (they are reported once, under
/// `global_args`, instead of duplicated onto every leaf).
const GLOBAL_ARG_IDS: &[&str] = &["json", "progress", "contract", "quiet"];

#[derive(Serialize)]
struct ArgInfo {
    name: String,
    positional: bool,
    required: bool,
    multiple: bool,
    default: Option<String>,
    help: Option<String>,
}

#[derive(Serialize)]
struct CommandInfo {
    name: String,
    about: Option<String>,
    args: Vec<ArgInfo>,
    result_schema: Option<String>,
}

#[derive(Serialize)]
struct ExitCodeInfo {
    code: i32,
    name: String,
    description: String,
}

#[derive(Serialize)]
struct CommandsReport {
    version: String,
    global_args: Vec<ArgInfo>,
    exit_codes: Vec<ExitCodeInfo>,
    commands: Vec<CommandInfo>,
}

fn arg_info(arg: &clap::Arg) -> ArgInfo {
    let positional = arg.is_positional();
    let name = if positional {
        arg.get_id().to_string()
    } else if let Some(long) = arg.get_long() {
        format!("--{long}")
    } else {
        arg.get_id().to_string()
    };
    let default = arg
        .get_default_values()
        .first()
        .map(|v| v.to_string_lossy().into_owned());
    let multiple = matches!(
        arg.get_num_args(),
        Some(r) if r.max_values() > 1 || r.max_values() == usize::MAX
    );
    ArgInfo {
        name,
        positional,
        required: arg.is_required_set(),
        multiple,
        default,
        help: arg.get_help().map(|h| h.to_string()),
    }
}

/// Result schema name for a leaf command's dotted path (`"model fetch"`,
/// `"contract show"`, …) — the one hand-maintained mapping this
/// introspection still needs, since a result *shape* isn't something clap
/// knows about. Kept in exactly one place (here) rather than scattered
/// across each `commands/*.rs`.
fn result_schema_for(dotted_name: &str) -> Option<&'static str> {
    Some(match dotted_name {
        "extract" => "extract",
        "inspect" => "inspect",
        "pack" => "pack",
        "calibrate" => "calibrate",
        "verify" => "verify",
        "doctor" => "doctor",
        "model fetch" | "model import" | "model verify" => "model",
        "model path" => "model-path",
        "schema" => "schema-list",
        "commands" => "commands",
        "contract show" => "contract-show",
        "contract list" => "contract-list",
        _ => return None,
    })
}

fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<CommandInfo>) {
    for sub in cmd.get_subcommands() {
        let name = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        if sub.get_subcommands().next().is_some() {
            walk(sub, &name, out);
            continue;
        }
        let args: Vec<ArgInfo> = sub
            .get_arguments()
            .filter(|a| {
                let id = a.get_id().as_str();
                id != "help" && id != "version" && !GLOBAL_ARG_IDS.contains(&id)
            })
            .map(arg_info)
            .collect();
        out.push(CommandInfo {
            about: sub.get_about().map(|a| a.to_string()),
            args,
            result_schema: result_schema_for(&name).map(str::to_string),
            name,
        });
    }
}

pub fn run(cli: &Cli) -> Result<i32, CliError> {
    let root = Cli::command();
    let global_args: Vec<ArgInfo> = root
        .get_arguments()
        .filter(|a| GLOBAL_ARG_IDS.contains(&a.get_id().as_str()))
        .map(arg_info)
        .collect();
    let mut commands = Vec::new();
    walk(&root, "", &mut commands);

    let exit_codes = EXIT_CODE_TABLE
        .iter()
        .map(|&(code, name, desc)| ExitCodeInfo {
            code,
            name: name.to_string(),
            description: desc.to_string(),
        })
        .collect();

    let report = CommandsReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        global_args,
        exit_codes,
        commands,
    };

    if cli.json {
        print_json(&serde_json::to_value(&report).unwrap());
    } else {
        for c in &report.commands {
            print_text(&c.name);
        }
    }
    Ok(EXIT_OK)
}
