use thiserror::Error;

/// A pack is malformed, truncated, internally inconsistent, or a caller
/// asked for something the format/contract forbids. The Rust counterpart of
/// Python's `sopack.format.PackError`, widened with the structured
/// `io`/`zip`/`json`/`contract` sources instead of Python's single
/// stringly-typed exception.
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Contract(#[from] sopack_contract::ContractError),

    /// Everything Python's `PackError(f"...")` covers ad hoc: point/pack
    /// structural problems that are not one of the typed sources above.
    #[error("{0}")]
    Invalid(String),
}

impl FormatError {
    pub fn invalid(msg: impl Into<String>) -> FormatError {
        FormatError::Invalid(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, FormatError>;
