//! The loaded calibration fixture (`sopack.calibration/1`,
//! SOPACK-2-FORMAT.md §3): the exact bytes (for sha256) plus the parsed
//! entries a writer embeds before any book and a reader/verifier cosines
//! against a pack's `probe.f32`.

use sha2::{Digest, Sha256};

use crate::data::CalibrationDoc;
use crate::error::{ContractError, Result};

pub const CALIBRATION_SCHEMA: &str = "sopack.calibration/1";

/// One `entries[]` row: real corpus text plus the vector stored in the
/// collection when the fixture was made.
#[derive(Debug, Clone)]
pub struct CalibrationEntry {
    pub id: String,
    pub profile: String,
    pub uid: Option<String>,
    /// `None` for entries whose language lives in `note` instead (real
    /// Bible-translation fixture rows do this — `note: "bible kjv"`).
    pub lang: Option<String>,
    pub note: String,
    pub text: String,
    pub vector: Vec<f32>,
}

/// A loaded, schema-checked `calibration.json`.
#[derive(Debug, Clone)]
pub struct Calibration {
    pub contract_id: String,
    pub created_at: String,
    pub entries: Vec<CalibrationEntry>,
    /// The exact file bytes, kept for [`Calibration::sha256`] — must be the
    /// literal bytes on disk, not a re-serialization, since a
    /// re-serialization is not guaranteed to round-trip byte-for-byte.
    bytes: Vec<u8>,
}

impl Calibration {
    /// Parse and schema-check *bytes* (already known non-empty — an empty
    /// fixture is handled one layer up as "not yet generated", not as a
    /// parse error here).
    pub(crate) fn parse(bytes: Vec<u8>, path_for_errors: &str) -> Result<Calibration> {
        let doc: CalibrationDoc =
            serde_json::from_slice(&bytes).map_err(|source| ContractError::CalibrationJson {
                path: path_for_errors.to_string(),
                source,
            })?;
        if doc.schema != CALIBRATION_SCHEMA {
            return Err(ContractError::CalibrationUnsupportedSchema {
                path: path_for_errors.to_string(),
                got: doc.schema,
            });
        }
        let entries = doc
            .entries
            .into_iter()
            .map(|e| CalibrationEntry {
                id: e.id,
                profile: e.profile,
                uid: e.uid,
                lang: e.lang,
                note: e.note,
                text: e.text,
                vector: e.vector,
            })
            .collect();
        Ok(Calibration {
            contract_id: doc.contract,
            created_at: doc.created_at,
            entries,
            bytes,
        })
    }

    /// sha256 of the exact `calibration.json` bytes this was loaded from —
    /// compared against `[calibration].sha256` in `contract.toml` and
    /// recorded in a pack's `manifest.contract.calibration_sha256`.
    pub fn sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.bytes))
    }

    /// The raw file bytes, exactly as loaded.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Entries whose `profile` matches *profile*, in fixture order — what a
    /// writer embeds and a probe records for one profile's pack.
    pub fn entries_for_profile<'a>(
        &'a self,
        profile: &'a str,
    ) -> impl Iterator<Item = &'a CalibrationEntry> {
        self.entries.iter().filter(move |e| e.profile == profile)
    }
}
