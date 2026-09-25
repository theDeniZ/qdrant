//! sopack-contract — loads and validates an embedding contract
//! (`contract.toml` + `calibration.json`), and implements the parts of the
//! contract that are *behavior*, not just data: point-id rules
//! (`uuid5(NAMESPACE_DNS, uid)`), payload validation, and the manifest
//! `embedding` block check.
//!
//! See `qdrant/docs/SOPACK-1.0-PLAN.md` §3.1/§3.6 and
//! `qdrant/docs/SOPACK-2-FORMAT.md` for the normative shapes. The Python
//! reference is `qdrant/sopack/contract.py`; every public item here has a
//! named Python counterpart mentioned in its doc comment.
//!
//! ```
//! use sopack_contract::Contract;
//!
//! let contract = Contract::embedded("e5-large-v1")?;
//! assert_eq!(contract.embedding.dim, 1024);
//! # Ok::<(), sopack_contract::ContractError>(())
//! ```

mod calibration;
mod contract;
mod data;
mod embedding_check;
mod error;
mod idrule;
mod payload;
mod pystr;

pub use calibration::{Calibration, CalibrationEntry, CALIBRATION_SCHEMA};
pub use contract::{
    CalibrationConfig, Chunker, Contract, EmbedSpec, Embedding, Model, ModelFile, Profile,
    CONTRACT_SCHEMA,
};
pub use embedding_check::{check_embedding, DeclaredEmbedding};
pub use error::{ContractError, Result};
pub use idrule::IdRule;
pub use payload::validate_payload;
pub use pystr::{is_python_space, python_strip};
