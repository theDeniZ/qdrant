//! `sopack inspect` — counts, damage, codes for a book.json.

use crate::args::{Cli, InspectArgs};
use crate::exit::{CliError, EXIT_OK};
use crate::output::{print_json, print_text};

pub fn run(cli: &Cli, args: &InspectArgs) -> Result<i32, CliError> {
    let book = sopack_book::load(&args.book_json)?;
    let report = sopack_book::InspectReport::from_book(args.book_json.display().to_string(), &book);
    if cli.json {
        print_json(&serde_json::to_value(&report).unwrap());
    } else {
        print_text(report.to_text().trim_end());
    }
    Ok(EXIT_OK)
}
