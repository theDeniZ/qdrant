//! sopack-book — the `sopack.book/1` model: `Book`, `Block`, `BookError`,
//! `load`/`dump`, `validate`, `to_payload`, `uid`, `book_sha256`, and the
//! `inspect` report. Port of `qdrant/sopack/book.py` — see
//! `qdrant/docs/SOPACK-1.0-PLAN.md` §3.1, §3.5, §4, §6.
//!
//! This is a **fixed seam**, same as the Python module: `Block`, `Book`,
//! `load`, `dump`, `validate`, `to_payload`, `uid` and `BookError` are
//! consumed by other code (`sopack-extract` today; `sopack pack` later) and
//! their names/signatures should not change lightly.
//!
//! **book.json schema.** The JSON Schema this module's `load`/`dump` shape
//! agrees with is embedded for reference (e.g. an external validator or a
//! future `sopack schema book` command) — see [`SCHEMA_JSON`].

mod contract_bridge;
mod error;
mod inspect;
mod io;
mod model;
mod payload;
mod validate;

pub use contract_bridge::{get_profile, point_id, uid_for, validate_payload, Profile, SCHEMA_BOOK};
pub use error::BookError;
pub use inspect::InspectReport;
pub use io::{book_sha256, dump, load};
pub use model::{Block, Book};
pub use payload::{to_payload, uid};
pub use validate::validate;

/// `qdrant/sopack/schemas/book.schema.json`, embedded verbatim. Structural
/// shape only — semantic rules live in [`validate`], not here, same
/// division of responsibility as the Python reference.
pub const SCHEMA_JSON: &str = include_str!("../../../../sopack/schemas/book.schema.json");
