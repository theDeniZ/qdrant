//! `check_embedding` — compares a pack manifest's declared `embedding` block
//! against the contract, per SOPACK-2-FORMAT.md §2:
//!
//! > `embedding` is checked by the importer against the contract named in
//! > `target.contract` on the keys `model, pooling, normalized, dim,
//! > distance, max_tokens, passage_prefix`. Any difference is fatal.
//! > `runtime`, `device`, `threads`, `batch_tokens` are provenance (R10) and
//! > are not compared.
//!
//! This is the store-neutral successor of `sopack.contract.check_embedding`
//! (which compared the whole `/1` embedding block, including
//! `library`/`library_version` — removed under `/2`, SOPACK-AUTONOMY.md: "no
//! `library` / `library_version` check any more").

use serde::{Deserialize, Serialize};

use crate::contract::Contract;

/// The six contract-defining keys of a pack manifest's `embedding` block —
/// deliberately excludes `runtime`/`device`/`threads`/`batch_tokens`
/// (provenance, never compared) and `query_prefix` (not carried in the
/// manifest at all; only `passage_prefix` is, since a pack only ever embeds
/// passages).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclaredEmbedding {
    pub model: String,
    pub pooling: String,
    pub normalized: bool,
    pub dim: u32,
    pub distance: String,
    pub max_tokens: u32,
    pub passage_prefix: String,
}

/// Every difference between *declared* and *contract*'s embedding space.
/// Empty means the pack was built against this exact vector space.
pub fn check_embedding(contract: &Contract, declared: &DeclaredEmbedding) -> Vec<String> {
    let want = &contract.embedding;
    let mut errors = Vec::new();

    macro_rules! check {
        ($key:literal, $got:expr, $want:expr) => {
            if $got != $want {
                errors.push(format!(
                    "embedding.{}: pack says {:?}, contract requires {:?}",
                    $key, $got, $want
                ));
            }
        };
    }

    check!("model", declared.model, want.model);
    check!("pooling", declared.pooling, want.pooling);
    check!("normalized", declared.normalized, want.normalized);
    check!("dim", declared.dim, want.dim);
    check!("distance", declared.distance, want.distance);
    check!("max_tokens", declared.max_tokens, want.max_tokens);
    check!(
        "passage_prefix",
        declared.passage_prefix,
        want.passage_prefix
    );

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matching(contract: &Contract) -> DeclaredEmbedding {
        DeclaredEmbedding {
            model: contract.embedding.model.clone(),
            pooling: contract.embedding.pooling.clone(),
            normalized: contract.embedding.normalized,
            dim: contract.embedding.dim,
            distance: contract.embedding.distance.clone(),
            max_tokens: contract.embedding.max_tokens,
            passage_prefix: contract.embedding.passage_prefix.clone(),
        }
    }

    #[test]
    fn identical_embedding_has_no_errors() {
        let c = Contract::embedded("e5-large-v1").unwrap();
        assert_eq!(check_embedding(&c, &matching(&c)), Vec::<String>::new());
    }

    #[test]
    fn dimension_mismatch_is_fatal() {
        let c = Contract::embedded("e5-large-v1").unwrap();
        let mut declared = matching(&c);
        declared.dim = 384;
        let errors = check_embedding(&c, &declared);
        assert!(errors.iter().any(|e| e.contains("dim")), "{errors:?}");
    }

    #[test]
    fn pooling_mismatch_is_fatal() {
        let c = Contract::embedded("e5-large-v1").unwrap();
        let mut declared = matching(&c);
        declared.pooling = "cls".to_string();
        let errors = check_embedding(&c, &declared);
        assert!(errors.iter().any(|e| e.contains("pooling")), "{errors:?}");
    }
}
