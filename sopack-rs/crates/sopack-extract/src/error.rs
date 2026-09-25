//! `ExtractError` — structured errors an extractor (or the option gate)
//! raises, designed for the CLI to map onto sopack's stable exit codes
//! (`docs/SOPACK-1.0-PLAN.md` §3.5): 2 usage, 3 input invalid, 4 needs
//! metadata. `sopack.extract`'s Python reference just raises `BookError`;
//! this crate keeps that behaviour (`From<BookError>`) but adds the
//! `code`/`hint`/`field` structure the plan's agent-facing contract wants.

use std::fmt;

use sopack_book::BookError;

/// What kind of problem this is, matching the plan's exit-code table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Exit 2 — a caller asked for an option this source kind does not
    /// accept (mirrors `sopack.cli._cmd_extract`'s rejection of unknown
    /// `_EXTRACT_OPTS`).
    Usage,
    /// Exit 3 — the source file itself is missing, unreadable, not a valid
    /// container for its kind (bad zip/OPF/JSON), or otherwise malformed.
    InputInvalid,
    /// Exit 4 — the source was read fine, but a field required downstream
    /// (by `sopack_book::validate`) could not be resolved and needs a
    /// human/agent to supply it (a `--meta`/sidecar field, in the plan's
    /// `propose` design).
    NeedsMetadata,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::Usage => "usage",
            ErrorCode::InputInvalid => "input_invalid",
            ErrorCode::NeedsMetadata => "needs_metadata",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,
    pub field: Option<String>,
}

impl ExtractError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ExtractError {
            code,
            message: message.into(),
            hint: None,
            field: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn input_invalid(message: impl Into<String>) -> Self {
        ExtractError::new(ErrorCode::InputInvalid, message)
    }
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " ({hint})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ExtractError {}

impl From<BookError> for ExtractError {
    fn from(exc: BookError) -> Self {
        ExtractError::input_invalid(exc.to_string())
    }
}
