//! Stdout writers. `SOPACK-1.0-PLAN.md` §3.5: "with `--json`, stdout carries
//! exactly one JSON document (result or error); progress goes to stderr
//! only" — every command handler in `commands/` returns a `serde_json::Value`
//! result and this module is the only place that writes it, so the
//! stdout/stderr split is enforced in one spot rather than per command.

use serde_json::Value;

/// The one JSON document `--json` prints to stdout for a successful
/// command. `sopack commands --json` and every error path use their own
/// small helpers instead (`error::CliError::to_json`, `commands` command),
/// but they all end up printed through this function so the "exactly one
/// document on stdout" property holds everywhere.
pub fn print_json(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// Plain-text stdout for a command run without `--json`. A single string —
/// callers build whatever human-readable report makes sense for that
/// command (mirrors the Python CLI's plain `print()` calls).
pub fn print_text(text: &str) {
    println!("{text}");
}
