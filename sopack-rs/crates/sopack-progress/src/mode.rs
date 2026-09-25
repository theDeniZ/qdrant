//! Output mode selection (SOPACK-1.0-PLAN.md §3.4): a TTY gets a live bar,
//! a non-TTY (CI logs) gets periodic plain lines, and `--progress json`
//! forces NDJSON on stderr regardless of what stderr is attached to.

use std::io::IsTerminal;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A live, redrawn bar (`indicatif`) — used when stderr is a terminal.
    Tty,
    /// One line per update, no carriage returns, throttled by time/percent
    /// so CI logs aren't flooded.
    Plain,
    /// NDJSON on stderr, one event per state change, un-throttled.
    Json,
}

impl Mode {
    /// `Tty` when stderr is attached to a terminal, `Plain` otherwise. The
    /// caller (`sopack-cli`) overrides this with an explicit `--progress`
    /// flag when given one.
    pub fn detect() -> Self {
        if std::io::stderr().is_terminal() {
            Mode::Tty
        } else {
            Mode::Plain
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "tty" => Ok(Mode::Tty),
            "plain" => Ok(Mode::Plain),
            "json" => Ok(Mode::Json),
            other => Err(format!(
                "unknown progress mode {other:?} (expected tty, plain or json)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_modes_case_insensitively() {
        assert_eq!("tty".parse::<Mode>().unwrap(), Mode::Tty);
        assert_eq!("PLAIN".parse::<Mode>().unwrap(), Mode::Plain);
        assert_eq!("Json".parse::<Mode>().unwrap(), Mode::Json);
    }

    #[test]
    fn rejects_unknown_mode() {
        let err = "xml".parse::<Mode>().unwrap_err();
        assert!(
            err.contains("xml"),
            "error should name the bad value: {err}"
        );
    }
}
