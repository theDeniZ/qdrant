//! Tokenizer setup — behaviour is unchanged from the M0 spike's
//! `load_tokenizer` (`spike/src/main.rs`), which measured 289/289 token
//! sequences identical to Python `tokenizers` (including truncation of a
//! text over 512 tokens long). Generalised only to take `max_tokens` from the
//! `EmbedSpec` instead of blindly trusting `tokenizer_config.json`, erroring
//! if the two disagree — a mismatched tokenizer_config.json for a swapped-in
//! model is exactly the kind of drift the contract exists to catch before it
//! silently changes truncation behaviour.

use crate::error::{EmbedError, Result};
use crate::spec::EmbedSpec;
use std::path::Path;
use tokenizers::{
    AddedToken, PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams,
};

fn tok_err(e: impl std::fmt::Display) -> EmbedError {
    EmbedError::Tokenize(e.to_string())
}

pub fn load_tokenizer(dir: &Path, spec: &EmbedSpec) -> Result<Tokenizer> {
    let mut tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(tok_err)?;

    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer_config.json"))?)?;
    let model_cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;

    let file_max_len = cfg["model_max_length"].as_u64().ok_or_else(|| {
        EmbedError::Tokenize("tokenizer_config.json has no model_max_length".into())
    })? as usize;
    if file_max_len != spec.max_tokens {
        return Err(EmbedError::MaxTokensMismatch {
            expected: spec.max_tokens,
            found: file_max_len,
        });
    }

    tok.with_truncation(Some(TruncationParams {
        max_length: spec.max_tokens,
        ..Default::default()
    }))
    .map_err(tok_err)?;

    if tok.get_padding().is_none() {
        let pad_token = cfg["pad_token"]
            .as_str()
            .ok_or_else(|| EmbedError::Tokenize("tokenizer_config.json has no pad_token".into()))?;
        tok.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Right,
            pad_id: model_cfg["pad_token_id"].as_u64().unwrap_or(0) as u32,
            pad_token: pad_token.to_string(),
            ..Default::default()
        }));
    }

    // fastembed re-adds every entry of special_tokens_map.json as a special
    // token; matched here so the vocabulary (and therefore token ids) is
    // identical to what produced the live collections' vectors.
    let stm: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
        dir.join("special_tokens_map.json"),
    )?)?;
    let mut added = vec![];
    for v in stm
        .as_object()
        .ok_or_else(|| EmbedError::Tokenize("special_tokens_map.json is not an object".into()))?
        .values()
    {
        match v {
            serde_json::Value::String(s) => added.push(AddedToken::from(s.clone(), true)),
            serde_json::Value::Object(o) => added.push(
                AddedToken::from(
                    o.get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    true,
                )
                .lstrip(o.get("lstrip").and_then(|v| v.as_bool()).unwrap_or(false))
                .rstrip(o.get("rstrip").and_then(|v| v.as_bool()).unwrap_or(false))
                .single_word(
                    o.get("single_word")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                )
                .normalized(
                    o.get("normalized")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                ),
            ),
            _ => {}
        }
    }
    tok.add_special_tokens(added).map_err(tok_err)?;
    Ok(tok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_dir_is_an_io_error_not_a_panic() {
        let spec = crate::spec::EmbedSpec {
            model_id: "m".into(),
            repo: "r/m".into(),
            revision: "rev".into(),
            onnx_file: "model.onnx".into(),
            output_name: "last_hidden_state".into(),
            files: vec![],
            pooling: crate::spec::Pooling::Mean,
            normalize: true,
            dim: 4,
            max_tokens: 512,
            passage_prefix: "passage: ".into(),
            query_prefix: "query: ".into(),
        };
        let err = load_tokenizer(Path::new("/nonexistent/model/dir"), &spec).unwrap_err();
        assert!(matches!(err, EmbedError::Tokenize(_)));
    }
}
