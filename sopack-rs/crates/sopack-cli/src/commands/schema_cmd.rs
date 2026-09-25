//! `sopack schema [<name>|--list]`.

use crate::args::{Cli, SchemaArgs};
use crate::exit::{CliError, EXIT_OK};
use crate::output::{print_json, print_text};
use crate::schemas;

pub fn run(cli: &Cli, args: &SchemaArgs) -> Result<i32, CliError> {
    if args.list || args.name.is_none() {
        let names = schemas::names();
        if cli.json {
            print_json(&serde_json::json!({"schemas": names}));
        } else {
            for n in &names {
                print_text(n);
            }
        }
        return Ok(EXIT_OK);
    }
    let name = args.name.as_deref().unwrap();
    match schemas::get(name) {
        Some(text) => {
            // The schema files are themselves JSON documents — printed
            // as-is (already pretty), not re-wrapped, whether or not
            // --json was given: there is exactly one sensible
            // representation for "print this JSON Schema".
            print_text(text.trim_end());
            Ok(EXIT_OK)
        }
        None => Err(CliError::usage(format!(
            "unknown schema {name:?} (see `sopack schema --list`)"
        ))),
    }
}
