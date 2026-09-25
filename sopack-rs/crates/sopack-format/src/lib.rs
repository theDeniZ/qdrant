//! sopack-format — the `.sopack` container: streaming writer/reader plus
//! offline `verify`. See `qdrant/docs/SOPACK-2-FORMAT.md` (normative) and
//! `qdrant/docs/SOPACK-1.0-PLAN.md` §3.1. The Python reference is
//! `qdrant/sopack/{format,verify}.py`; every public item mentions its
//! Python counterpart in its doc comment.
//!
//! - [`PackWriter`] streams points into a `sopack/2` pack with a resumable
//!   on-disk checkpoint (see the module docs on [`writer`]).
//! - [`PackReader`] streams one back out, `/1` and `/2` alike.
//! - [`verify`]/[`verify_with_progress`] run every offline check
//!   (`sopack.verify.verify` plus the `/2` probe checks) in one call.

mod error;
mod f32io;
mod idfields;
mod jsonline;
mod manifest;
mod reader;
mod verify;
mod writer;

pub use error::{FormatError, Result};
pub use f32io::{f32_to_le_bytes, le_bytes_to_f32_vectors, ITEM_SIZE};
pub use jsonline::to_python_json_bytes;
pub use manifest::{
    build_manifest_v2, BookEntry, ContractRef, Counts, EmbeddingV2, ManifestSummary, ProbeEntry,
    ProbeV2, SelfCheck, TargetV2, NEWEST_SUPPORTED_MAJOR, SCHEMA_V2,
};
pub use reader::{BatchIter, PackReader, PointRecord};
pub use verify::{verify, verify_with_progress, VerifyReport};
pub use writer::{EmbeddingProvenance, PackWriter, ProbeEntryInput};
