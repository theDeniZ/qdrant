//! Stable exit codes (`SOPACK-1.0-PLAN.md` §3.5) and [`CliError`], the one
//! error type every command handler returns. `main.rs` turns a `CliError`
//! into either a `{"ok":false,"error":{...}}` JSON document (with `--json`)
//! or a plain `error: <message>` line on stderr, and exits with
//! [`CliError::exit`].

use std::fmt;

use serde::Serialize;

pub const EXIT_OK: i32 = 0;
pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_INPUT_INVALID: i32 = 3;
pub const EXIT_NEEDS_METADATA: i32 = 4;
pub const EXIT_CALIBRATION_FAILED: i32 = 5;
pub const EXIT_RESOURCES: i32 = 6;
pub const EXIT_INTERRUPTED: i32 = 7;

/// One row of the exit-code table — also what `sopack commands --json`
/// reports, so the table lives in exactly one place.
pub const EXIT_CODE_TABLE: &[(i32, &str, &str)] = &[
    (EXIT_OK, "ok", "success"),
    (EXIT_INTERNAL, "internal", "an unexpected/internal error"),
    (
        EXIT_USAGE,
        "usage",
        "bad command-line usage (also clap's own parse-error exit code)",
    ),
    (
        EXIT_INPUT_INVALID,
        "input_invalid",
        "the given source/file/pack is malformed or unreadable",
    ),
    (
        EXIT_NEEDS_METADATA,
        "needs_metadata",
        "required metadata is missing and must be supplied",
    ),
    (
        EXIT_CALIBRATION_FAILED,
        "calibration_failed",
        "the calibration self-check scored below the contract's threshold",
    ),
    (
        EXIT_RESOURCES,
        "resources",
        "a required resource is missing (model, ORT dylib, memory, disk)",
    ),
    (
        EXIT_INTERRUPTED,
        "interrupted",
        "the run was interrupted; a partial/resumable state was left behind",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliErrorCode {
    Internal,
    Usage,
    InputInvalid,
    NeedsMetadata,
    CalibrationFailed,
    Resources,
    Interrupted,
}

impl CliErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            CliErrorCode::Internal => "internal",
            CliErrorCode::Usage => "usage",
            CliErrorCode::InputInvalid => "input_invalid",
            CliErrorCode::NeedsMetadata => "needs_metadata",
            CliErrorCode::CalibrationFailed => "calibration_failed",
            CliErrorCode::Resources => "resources",
            CliErrorCode::Interrupted => "interrupted",
        }
    }

    pub fn exit(self) -> i32 {
        match self {
            CliErrorCode::Internal => EXIT_INTERNAL,
            CliErrorCode::Usage => EXIT_USAGE,
            CliErrorCode::InputInvalid => EXIT_INPUT_INVALID,
            CliErrorCode::NeedsMetadata => EXIT_NEEDS_METADATA,
            CliErrorCode::CalibrationFailed => EXIT_CALIBRATION_FAILED,
            CliErrorCode::Resources => EXIT_RESOURCES,
            CliErrorCode::Interrupted => EXIT_INTERRUPTED,
        }
    }
}

/// The one error type every command handler returns. Carries everything
/// `--json`'s error object needs (`{"ok":false,"error":{code,exit,message,
/// hint,field}}`, SOPACK-1.0-PLAN.md §3.5) plus a human `Display` for the
/// plain-text path.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: CliErrorCode,
    pub message: String,
    pub hint: Option<String>,
    pub field: Option<String>,
}

impl CliError {
    pub fn new(code: CliErrorCode, message: impl Into<String>) -> Self {
        CliError {
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

    pub fn usage(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::Usage, message)
    }

    pub fn input_invalid(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::InputInvalid, message)
    }

    pub fn needs_metadata(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::NeedsMetadata, message)
    }

    pub fn calibration_failed(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::CalibrationFailed, message)
    }

    pub fn resources(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::Resources, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        CliError::new(CliErrorCode::Internal, message)
    }

    /// The `--json` error document.
    pub fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct ErrorBody {
            code: String,
            exit: i32,
            message: String,
            hint: Option<String>,
            field: Option<String>,
        }
        #[derive(Serialize)]
        struct Doc {
            ok: bool,
            error: ErrorBody,
        }
        serde_json::to_value(Doc {
            ok: false,
            error: ErrorBody {
                code: self.code.as_str().to_string(),
                exit: self.code.exit(),
                message: self.message.clone(),
                hint: self.hint.clone(),
                field: self.field.clone(),
            },
        })
        .expect("CliError always serialises")
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " ({hint})")?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {}

impl From<sopack_extract::ExtractError> for CliError {
    fn from(e: sopack_extract::ExtractError) -> Self {
        let code = match e.code {
            sopack_extract::ErrorCode::Usage => CliErrorCode::Usage,
            sopack_extract::ErrorCode::InputInvalid => CliErrorCode::InputInvalid,
            sopack_extract::ErrorCode::NeedsMetadata => CliErrorCode::NeedsMetadata,
        };
        let mut err = CliError::new(code, e.message);
        if let Some(hint) = e.hint {
            err = err.with_hint(hint);
        }
        if let Some(field) = e.field {
            err = err.with_field(field);
        }
        err
    }
}

impl From<sopack_book::BookError> for CliError {
    fn from(e: sopack_book::BookError) -> Self {
        CliError::input_invalid(e.to_string())
    }
}

impl From<sopack_contract::ContractError> for CliError {
    fn from(e: sopack_contract::ContractError) -> Self {
        CliError::input_invalid(e.to_string())
    }
}

impl From<sopack_format::FormatError> for CliError {
    fn from(e: sopack_format::FormatError) -> Self {
        CliError::input_invalid(e.to_string())
    }
}

impl From<sopack_embed::error::EmbedError> for CliError {
    fn from(e: sopack_embed::error::EmbedError) -> Self {
        use sopack_embed::error::EmbedError as E;
        match &e {
            E::MissingFile { .. }
            | E::SizeMismatch { .. }
            | E::HashMismatch { .. }
            | E::CurlNotFound
            | E::DownloadFailed { .. }
            | E::OrtDylibNotFound { .. }
            | E::OrtDylibLoad { .. }
            | E::OrtInit(_)
            | E::InsufficientMemory { .. }
            | E::DeviceNotCompiled { .. }
            | E::DeviceNotAvailable { .. } => CliError::resources(e.to_string()),
            E::CalibrationFailed { .. } => CliError::calibration_failed(e.to_string()),
            E::SessionLoad(_)
            | E::Ort(_)
            | E::Tokenize(_)
            | E::MaxTokensMismatch { .. }
            | E::DimensionMismatch { .. } => CliError::input_invalid(e.to_string()),
            E::Io(_) | E::Json(_) => CliError::internal(e.to_string()),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::internal(e.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::internal(e.to_string())
    }
}
