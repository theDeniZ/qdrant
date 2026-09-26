//! `sopack` — the CLI binary. `SOPACK-1.0-PLAN.md` §3.1/§3.5.

mod args;
mod commands;
mod contract_load;
mod engine_build;
mod exit;
mod extract_progress;
mod output;
mod progress_build;
mod progress_opt;
mod provenance;
mod schemas;

use clap::Parser;

use args::{Cli, Command};
use exit::{CliError, EXIT_OK};

fn dispatch(cli: &Cli) -> Result<i32, CliError> {
    match &cli.command {
        Command::Extract(a) => commands::extract::run(cli, a),
        Command::Inspect(a) => commands::inspect::run(cli, a),
        Command::Pack(a) => commands::pack::run(cli, a),
        Command::Calibrate(a) => commands::calibrate::run(cli, a),
        Command::Verify(a) => commands::verify::run(cli, a),
        Command::Doctor(a) => commands::doctor::run(cli, a),
        Command::Model(a) => commands::model::run(cli, a),
        Command::Schema(a) => commands::schema_cmd::run(cli, a),
        Command::Commands => commands::commands_cmd::run(cli),
        Command::Contract(a) => commands::contract_cmd::run(cli, a),
    }
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match dispatch(&cli) {
        Ok(code) => code,
        Err(err) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&err.to_json()).unwrap());
            } else {
                eprintln!("error: {err}");
            }
            err.code.exit()
        }
    };
    // Never silently report success on the sentinel `Ok(EXIT_OK)` path
    // while actually exiting nonzero, and vice versa — `std::process::exit`
    // is the single point that turns the resolved code into the real exit
    // status, matching every command's documented exit code
    // (SOPACK-1.0-PLAN.md §3.5) exactly.
    debug_assert!(exit_code == EXIT_OK || exit_code != 0);
    std::process::exit(exit_code);
}
