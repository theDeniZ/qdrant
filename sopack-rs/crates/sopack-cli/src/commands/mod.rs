//! One module per subcommand. Each exposes a `run(cli: &Cli, args: &...)
//! -> Result<i32, CliError>`: `Ok(exit_code)` for a normal result (which
//! may itself carry a non-zero, *expected* exit code — `verify`/`calibrate`
//! on a found problem/failed gate — since that is a result, not a
//! `CliError`), `Err(CliError)` when the command could not produce a result
//! at all. `main.rs` is the only place that turns either into process exit
//! behaviour + stdout.

pub mod calibrate;
pub mod commands_cmd;
pub mod contract_cmd;
pub mod doctor;
pub mod extract;
pub mod inspect;
pub mod model;
pub mod pack;
pub mod schema_cmd;
pub mod verify;
