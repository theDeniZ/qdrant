//! `manifest.json` — typed pieces plus a schema-agnostic reader.
//!
//! `sopack/1` and `sopack/2` manifests share eight top-level keys (`schema`,
//! `profile`, `pack_id`, `created_by`, `id_rule`, `id_rule_doc`, `counts`,
//! `sha256`, `books`) and disagree on the rest: `/1`'s `target` names a Qdrant
//! collection/vector, `/2`'s names a `{profile, contract}`; `/1`'s `embedding`
//! carries `library`/`library_version`, `/2`'s carries `dim`/`distance`/
//! `max_tokens`/runtime provenance instead; `/1`'s `probe` is
//! canary-ids-in-a-live-collection, `/2`'s is fixture-relative
//! (SOPACK-2-FORMAT.md §1–2). Readers "MUST reject an unknown **major**
//! schema … and MUST ignore unknown manifest keys" (§1) — the natural
//! implementation of that is: parse the shared fields strictly, and parse
//! the schema-specific blocks (`target`/`contract`/`embedding`/`probe`) only
//! on demand, tolerating their absence or a different shape. See
//! [`ManifestSummary`].

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{FormatError, Result};

pub const SCHEMA_PREFIX: &str = "sopack/";
pub const SCHEMA_V2: &str = "sopack/2";
pub const NEWEST_SUPPORTED_MAJOR: u32 = 2;

/// `manifest.json["counts"]` — identical shape in `/1` and `/2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    pub points: u64,
    pub books: u64,
    pub dim: u32,
    pub points_bytes: u64,
}

/// One `manifest.json["books"][i]` — "unchanged from `/1`"
/// (SOPACK-2-FORMAT.md §1). `lang`/`title`/`author`/`year`/`corpus`/`slug`/
/// `book_pair`/`first_id`/`book_sha256` are all legitimately absent/`null`
/// for hand-built or reduced fixtures, so every field but the two always
/// supplied by `pack.py`/the Rust writer (`book_code`, `points`, `id_rule`)
/// is optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookEntry {
    pub book_code: String,
    #[serde(default)]
    pub lang: Option<String>,
    pub points: u64,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub corpus: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub book_pair: Option<String>,
    pub id_rule: String,
    #[serde(default)]
    pub book_sha256: Option<String>,
}

/// `manifest.json["target"]` for `sopack/2` — `{profile, contract}` only;
/// SOPACK-2-FORMAT.md §2: "no `collection`, `vector_name`, `vector_size` or
/// Qdrant `distance` spelling. Those are adapter config on the importer."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetV2 {
    pub profile: String,
    pub contract: String,
}

/// `manifest.json["contract"]` for `sopack/2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractRef {
    pub id: String,
    pub sha256: String,
    pub calibration_sha256: String,
}

/// `manifest.json["embedding"]` for `sopack/2` — the seven contract-checked
/// keys (§2: "checked … on the keys `model, pooling, normalized, dim,
/// distance, max_tokens, passage_prefix`") plus the four provenance-only
/// keys ("`runtime`, `device`, `threads`, `batch_tokens` are provenance …
/// and are not compared").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingV2 {
    pub model: String,
    pub pooling: String,
    pub normalized: bool,
    pub dim: u32,
    pub distance: String,
    pub max_tokens: u32,
    pub passage_prefix: String,
    pub runtime: String,
    pub device: String,
    pub threads: u32,
    pub batch_tokens: u32,
}

impl EmbeddingV2 {
    /// The subset [`sopack_contract::check_embedding`] compares against the
    /// contract.
    pub fn declared(&self) -> sopack_contract::DeclaredEmbedding {
        sopack_contract::DeclaredEmbedding {
            model: self.model.clone(),
            pooling: self.pooling.clone(),
            normalized: self.normalized,
            dim: self.dim,
            distance: self.distance.clone(),
            max_tokens: self.max_tokens,
            passage_prefix: self.passage_prefix.clone(),
        }
    }
}

/// One `manifest.json["probe"]["entries"][i]` for `sopack/2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeEntry {
    pub id: String,
    pub profile: String,
    pub vector_offset: usize,
}

/// `manifest.json["probe"]["self_check"]` for `sopack/2` — what the packer
/// measured cosining its own fresh embeddings of the calibration fixture
/// against the fixture's stored vectors, *before* embedding any book
/// (SOPACK-2-FORMAT.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SelfCheck {
    pub n: u64,
    pub min_cosine: f64,
    pub mean_cosine: f64,
    pub threshold: f64,
}

/// `manifest.json["probe"]` for `sopack/2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeV2 {
    pub kind: String,
    pub fixture_sha256: String,
    pub vectors: String,
    pub entries: Vec<ProbeEntry>,
    pub self_check: SelfCheck,
}

/// The eight fields every schema major shares, deserialized strictly;
/// everything schema-specific is read from [`ManifestSummary::raw`] on
/// demand. `#[serde(default)]`/no `deny_unknown_fields` means extra keys in
/// the source JSON (e.g. `/1`'s `target`/`embedding`/`probe`, or a future
/// unknown key) are silently ignored here, matching §1's "ignore unknown
/// manifest keys".
#[derive(Debug, Clone, Deserialize)]
struct PartialManifest {
    schema: String,
    profile: String,
    pack_id: String,
    created_by: String,
    id_rule: String,
    #[serde(default)]
    id_rule_doc: Option<String>,
    counts: Counts,
    sha256: IndexMap<String, String>,
    books: Vec<BookEntry>,
}

/// A parsed `manifest.json`: the shared fields typed and validated, plus the
/// full document for schema-specific access.
#[derive(Debug, Clone)]
pub struct ManifestSummary {
    pub schema: String,
    pub profile: String,
    pub pack_id: String,
    pub created_by: String,
    pub id_rule: String,
    pub id_rule_doc: Option<String>,
    pub counts: Counts,
    pub sha256: IndexMap<String, String>,
    pub books: Vec<BookEntry>,
    pub raw: serde_json::Value,
}

impl ManifestSummary {
    pub fn parse(bytes: &[u8]) -> Result<ManifestSummary> {
        let raw: serde_json::Value = serde_json::from_slice(bytes)?;
        let partial: PartialManifest = serde_json::from_value(raw.clone())?;
        Ok(ManifestSummary {
            schema: partial.schema,
            profile: partial.profile,
            pack_id: partial.pack_id,
            created_by: partial.created_by,
            id_rule: partial.id_rule,
            id_rule_doc: partial.id_rule_doc,
            counts: partial.counts,
            sha256: partial.sha256,
            books: partial.books,
            raw,
        })
    }

    /// The numeral after `"sopack/"` (`"sopack/2"` → `2`). Errors on any
    /// schema string that isn't `sopack/<non-negative integer>`, and on a
    /// major newer than this crate supports — "readers MUST reject an
    /// unknown **major** schema (`sopack/3`)" (SOPACK-2-FORMAT.md §1).
    pub fn schema_major(&self) -> Result<u32> {
        let digits = self.schema.strip_prefix(SCHEMA_PREFIX).ok_or_else(|| {
            FormatError::invalid(format!(
                "unsupported schema {:?} (expected \"sopack/<n>\")",
                self.schema
            ))
        })?;
        let major: u32 = digits.parse().map_err(|_| {
            FormatError::invalid(format!(
                "unsupported schema {:?} (expected \"sopack/<n>\")",
                self.schema
            ))
        })?;
        if major > NEWEST_SUPPORTED_MAJOR {
            return Err(FormatError::invalid(format!(
                "unsupported schema {:?} (this crate reads up to sopack/{})",
                self.schema, NEWEST_SUPPORTED_MAJOR
            )));
        }
        Ok(major)
    }

    /// `manifest.json["target"]` parsed as `/2`'s shape, if present and
    /// well-formed. `/1` manifests have a differently-shaped `target` and
    /// return `None` here (not an error — callers branch on
    /// [`Self::schema_major`] first).
    pub fn target_v2(&self) -> Option<TargetV2> {
        self.raw
            .get("target")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn contract_ref(&self) -> Option<ContractRef> {
        self.raw
            .get("contract")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn embedding_v2(&self) -> Option<EmbeddingV2> {
        self.raw
            .get("embedding")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn probe_v2(&self) -> Option<ProbeV2> {
        self.raw
            .get("probe")
            .filter(|v| !v.is_null())
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
    }
}

/// Build a `sopack/2` `manifest.json` document (as a JSON [`serde_json::Value`],
/// so pretty-printing preserves exactly this key order —
/// `serde_json::Map`/`Value` preserve insertion order with the workspace's
/// `preserve_order` feature) matching SOPACK-2-FORMAT.md §2 verbatim,
/// including nested key order.
#[allow(clippy::too_many_arguments)]
pub fn build_manifest_v2(
    profile: &str,
    pack_id: &str,
    created_by: &str,
    target: &TargetV2,
    contract: &ContractRef,
    embedding: &EmbeddingV2,
    id_rule: &str,
    id_rule_doc: &str,
    counts: &Counts,
    sha256: &IndexMap<String, String>,
    books: &[BookEntry],
    probe: &ProbeV2,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("schema".into(), serde_json::json!(SCHEMA_V2));
    map.insert("profile".into(), serde_json::json!(profile));
    map.insert("pack_id".into(), serde_json::json!(pack_id));
    map.insert("created_by".into(), serde_json::json!(created_by));
    map.insert("target".into(), serde_json::to_value(target).unwrap());
    map.insert("contract".into(), serde_json::to_value(contract).unwrap());
    map.insert("embedding".into(), serde_json::to_value(embedding).unwrap());
    map.insert("id_rule".into(), serde_json::json!(id_rule));
    map.insert("id_rule_doc".into(), serde_json::json!(id_rule_doc));
    map.insert("counts".into(), serde_json::to_value(counts).unwrap());
    map.insert("sha256".into(), serde_json::to_value(sha256).unwrap());
    map.insert("books".into(), serde_json::to_value(books).unwrap());
    map.insert("probe".into(), serde_json::to_value(probe).unwrap());
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_v2_key_order_matches_spec_example() {
        let target = TargetV2 {
            profile: "sop".into(),
            contract: "e5-large-v1".into(),
        };
        let contract = ContractRef {
            id: "e5-large-v1".into(),
            sha256: "aaa".into(),
            calibration_sha256: "bbb".into(),
        };
        let embedding = EmbeddingV2 {
            model: "intfloat/multilingual-e5-large".into(),
            pooling: "mean".into(),
            normalized: true,
            dim: 1024,
            distance: "cosine".into(),
            max_tokens: 512,
            passage_prefix: "passage: ".into(),
            runtime: "sopack-rs 0.9.0".into(),
            device: "cpu".into(),
            threads: 4,
            batch_tokens: 512,
        };
        let counts = Counts {
            points: 23,
            books: 1,
            dim: 1024,
            points_bytes: 16026,
        };
        let mut sha256 = IndexMap::new();
        sha256.insert("points.jsonl".to_string(), "p".to_string());
        sha256.insert("vectors.f32".to_string(), "v".to_string());
        sha256.insert("probe.f32".to_string(), "pr".to_string());
        let books = vec![BookEntry {
            book_code: "WDYS".into(),
            lang: Some("en".into()),
            points: 23,
            first_id: Some("id0".into()),
            title: Some("t".into()),
            author: Some("a".into()),
            year: Some(1861),
            corpus: Some("pioneers".into()),
            slug: Some("s".into()),
            book_pair: Some("WDYS".into()),
            id_rule: "sop/seq".into(),
            book_sha256: Some("bsha".into()),
        }];
        let probe = ProbeV2 {
            kind: "calibration".into(),
            fixture_sha256: "bbb".into(),
            vectors: "probe.f32".into(),
            entries: vec![ProbeEntry {
                id: "fx1".into(),
                profile: "sop".into(),
                vector_offset: 0,
            }],
            self_check: SelfCheck {
                n: 16,
                min_cosine: 0.99999999,
                mean_cosine: 0.99999999,
                threshold: 0.9999,
            },
        };

        let manifest = build_manifest_v2(
            "sop",
            "sop-2026-09-24-f0a3",
            "sopack 0.9.0 (rust) on Linux 5.15 aarch64",
            &target,
            &contract,
            &embedding,
            "sop/seq",
            "uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')",
            &counts,
            &sha256,
            &books,
            &probe,
        );

        let keys: Vec<&str> = manifest
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "schema",
                "profile",
                "pack_id",
                "created_by",
                "target",
                "contract",
                "embedding",
                "id_rule",
                "id_rule_doc",
                "counts",
                "sha256",
                "books",
                "probe"
            ]
        );

        let embedding_keys: Vec<&str> = manifest["embedding"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            embedding_keys,
            vec![
                "model",
                "pooling",
                "normalized",
                "dim",
                "distance",
                "max_tokens",
                "passage_prefix",
                "runtime",
                "device",
                "threads",
                "batch_tokens"
            ]
        );

        let sha_keys: Vec<&str> = manifest["sha256"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(sha_keys, vec!["points.jsonl", "vectors.f32", "probe.f32"]);
    }

    #[test]
    fn partial_manifest_reads_v1_shape_ignoring_v1_only_fields() {
        // A trimmed real sopack/1 manifest shape (target/embedding/probe are
        // v1-flavoured and must simply be ignored by ManifestSummary::parse).
        let raw = serde_json::json!({
            "schema": "sopack/1",
            "profile": "sop",
            "pack_id": "p",
            "created_by": "t",
            "target": {"collection": "sop", "vector_name": "fast-multilingual-e5-large",
                       "vector_size": 1024, "distance": "Cosine"},
            "embedding": {"model": "intfloat/multilingual-e5-large", "library": "fastembed",
                          "library_version": "0.8.0", "pooling": "mean", "normalized": true,
                          "passage_prefix": "passage: "},
            "id_rule": "sop/seq",
            "id_rule_doc": "uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')",
            "counts": {"points": 1, "books": 1, "dim": 1024, "points_bytes": 10},
            "sha256": {"points.jsonl": "x", "vectors.f32": "y"},
            "books": [{"book_code": "TT", "lang": "en", "points": 1, "id_rule": "sop/seq"}],
            "probe": {"canaries": [{"id": "c1", "collection": "sop", "vector_offset": 0,
                                     "cosine_expected_min": 0.95}], "vectors": "probe.f32"}
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let m = ManifestSummary::parse(&bytes).unwrap();
        assert_eq!(m.schema_major().unwrap(), 1);
        assert_eq!(m.books.len(), 1);
        assert!(
            m.target_v2().is_none(),
            "v1 target has no profile/contract keys"
        );
        assert!(
            m.embedding_v2().is_none(),
            "v1 embedding lacks dim/distance/max_tokens"
        );
        assert!(
            m.probe_v2().is_none(),
            "v1 probe has no kind/fixture_sha256/self_check"
        );
    }

    #[test]
    fn unknown_major_is_rejected() {
        let raw = serde_json::json!({
            "schema": "sopack/3", "profile": "sop", "pack_id": "p", "created_by": "t",
            "id_rule": "sop/seq", "counts": {"points": 0, "books": 0, "dim": 1024, "points_bytes": 0},
            "sha256": {}, "books": []
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let m = ManifestSummary::parse(&bytes).unwrap();
        assert!(m.schema_major().is_err());
    }
}
