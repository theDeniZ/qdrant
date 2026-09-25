//! Every way `sopack-embed` can fail. Kept as one flat enum (rather than
//! per-module errors) because callers up the stack (`sopack-cli`) need to
//! map these onto the plan's stable exit codes (§3.5: 3 input invalid, 5
//! calibration failed, 6 resources, …) — one `match` here is easier to keep
//! in sync with that table than several.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("model directory {dir} is missing required file {file:?}")]
    MissingFile { dir: PathBuf, file: String },

    #[error("{file}: expected {expected} bytes, found {found} at {path}")]
    SizeMismatch {
        path: PathBuf,
        file: String,
        expected: u64,
        found: u64,
    },

    #[error("{file}: sha256 mismatch at {path} (expected {expected}, got {got})")]
    HashMismatch {
        path: PathBuf,
        file: String,
        expected: String,
        got: String,
    },

    #[error(
        "curl was not found on PATH — sopack downloads models with curl (present on macOS and \
         Linux by default); install curl, or fetch the model another way and use `model import <dir>`"
    )]
    CurlNotFound,

    #[error("curl exited with status {status} while fetching {url}{}", if detail.is_empty() { String::new() } else { format!(": {detail}") })]
    DownloadFailed {
        url: String,
        status: i32,
        detail: String,
    },

    #[error("no ONNX Runtime dylib found (searched: {})", .searched.join("; "))]
    OrtDylibNotFound { searched: Vec<String> },

    #[error("failed to read the ONNX Runtime dylib at {path}: {reason}")]
    OrtDylibLoad { path: PathBuf, reason: String },

    #[error("failed to initialise ONNX Runtime: {0}")]
    OrtInit(String),

    #[error("failed to load the model session: {0}")]
    SessionLoad(String),

    #[error("ONNX Runtime error: {0}")]
    Ort(String),

    #[error("tokenizer error: {0}")]
    Tokenize(String),

    #[error(
        "not enough memory to load the model: {available} bytes available, need at least \
         {needed} bytes ({detail})"
    )]
    InsufficientMemory {
        available: u64,
        needed: u64,
        detail: String,
    },

    #[error("calibration failed: min cosine {min:.6} is below the contract's threshold {threshold:.6} ({n} entries checked)")]
    CalibrationFailed { min: f64, threshold: f64, n: usize },

    #[error(
        "tokenizer_config.json model_max_length ({found}) does not match the contract's \
         max_tokens ({expected}) — the contract and the model files disagree"
    )]
    MaxTokensMismatch { expected: usize, found: usize },

    #[error("device {device:?} requires the {feature:?} cargo feature, which was not compiled in")]
    DeviceNotCompiled {
        device: String,
        feature: &'static str,
    },

    #[error("device {device:?} is not available on this platform")]
    DeviceNotAvailable { device: String },

    #[error("embedding produced dimension {got}, contract requires {expected}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;
