//! `--progress auto|tty|plain|json|none` (`SOPACK-1.0-PLAN.md` §3.4) —
//! `sopack-progress::Mode` has no `auto`/`none` of its own (`Mode::detect()`
//! is how a caller gets "auto", and "no progress at all" is simply not
//! building a `Progress` and using `NullSink` instead), so this is the CLI's
//! own thin enum resolving both.

use clap::ValueEnum;
use sopack_progress::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ProgressOpt {
    Auto,
    Tty,
    Plain,
    Json,
    None,
}

/// What a resolved `--progress` value means for one command run: either a
/// concrete rendering [`Mode`], or no progress reporting at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedProgress {
    Mode(Mode),
    Off,
}

impl ProgressOpt {
    pub fn resolve(self) -> ResolvedProgress {
        match self {
            ProgressOpt::Auto => ResolvedProgress::Mode(Mode::detect()),
            ProgressOpt::Tty => ResolvedProgress::Mode(Mode::Tty),
            ProgressOpt::Plain => ResolvedProgress::Mode(Mode::Plain),
            ProgressOpt::Json => ResolvedProgress::Mode(Mode::Json),
            ProgressOpt::None => ResolvedProgress::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_resolves_to_off() {
        assert_eq!(ProgressOpt::None.resolve(), ResolvedProgress::Off);
    }

    #[test]
    fn explicit_modes_resolve_directly() {
        assert_eq!(
            ProgressOpt::Json.resolve(),
            ResolvedProgress::Mode(Mode::Json)
        );
        assert_eq!(
            ProgressOpt::Plain.resolve(),
            ResolvedProgress::Mode(Mode::Plain)
        );
        assert_eq!(
            ProgressOpt::Tty.resolve(),
            ResolvedProgress::Mode(Mode::Tty)
        );
    }
}
