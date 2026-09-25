//! SDARM SoP JSON → [`sopack_book::Book`]. Port of
//! `sopack/extract/sop_json.py`.
//!
//! Reads the `generator/data/sop/<lang>/<CODE>.json` shape already used by
//! `sdarm.tools.build_sop_vector_index` — the live `sop` collection was
//! built from exactly these files. German files are keyed by DE code and
//! carry `en_reverse` (mapping the *English* para_key an entry aligns to,
//! back to a list of this file's para_keys); English and every other
//! language file has no `en_reverse`.
//!
//! No chunking happens here: the live collection has always indexed one
//! point per JSON paragraph key with no splitting (id_rule `sop/plain`,
//! which has no `#<seq>` suffix to disambiguate a split block), so
//! re-extracting through this module must reproduce the same
//! one-block-per-paragraph shape or points would silently collide.
//!
//! `lang` is inferred from the parent directory name when not given (the
//! documented shape of this source, not a filename guess — rule #9).
//! Author is intentionally left `None` unless given explicitly: every file
//! under `data/sop/` is an EGW work and the corpus convention is that EGW
//! points carry no `corpus` key, which is exactly what makes `validate()`
//! exempt them from requiring `author`/`year`.

use std::path::Path;

use serde_json::Value;
use sopack_book::{Block, Book, SCHEMA_BOOK};

use crate::chunk;
use crate::common::{build_book_meta, build_source, build_stats, dropped_entry, sha256_file};
use crate::error::ExtractError;
use crate::options::ExtractOptions;
use crate::progress::{ProgressSink, Stage};

fn parse_page_para(para_key: &str) -> (i64, i64) {
    match para_key.split_once('.') {
        Some((page_str, para_str)) => {
            match (
                page_str.trim().parse::<i64>(),
                para_str.trim().parse::<i64>(),
            ) {
                (Ok(p), Ok(q)) => (p, q),
                _ => (0, 0),
            }
        }
        None => (0, 0),
    }
}

fn year_of(raw: Option<&Value>) -> Option<i64> {
    match raw {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => {
            let t = s.trim();
            if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
                t.parse::<i64>().ok()
            } else {
                None
            }
        }
        _ => None,
    }
}

fn meta_str<'a>(meta: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    match meta.get(key) {
        Some(Value::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

pub fn extract(
    path: &Path,
    opts: &ExtractOptions,
    progress: &mut dyn ProgressSink,
) -> Result<Book, ExtractError> {
    if !path.exists() {
        return Err(ExtractError::input_invalid(format!(
            "no such file: {}",
            path.display()
        )));
    }
    progress.stage_start(Stage::ReadContainer, None);
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {}: {e}", path.display())))?;
    let data: Value = serde_json::from_str(&raw).map_err(|e| {
        ExtractError::input_invalid(format!("{}: not valid JSON: {e}", path.display()))
    })?;
    let data = match data {
        Value::Object(m) => m,
        _ => {
            return Err(ExtractError::input_invalid(format!(
                "{}: top level must be a JSON object",
                path.display()
            )))
        }
    };
    progress.stage_end(Stage::ReadContainer);

    let resolved_lang = opts.lang.clone().unwrap_or_else(|| {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    if !data.contains_key(&resolved_lang) {
        let available: Vec<&str> = data
            .keys()
            .filter(|k| k.as_str() != "meta" && k.as_str() != "en_reverse")
            .map(String::as_str)
            .collect();
        let available_str = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        return Err(ExtractError::input_invalid(format!(
            "{}: no {resolved_lang:?} key in this file (has: {available_str})",
            path.display()
        )));
    }

    let meta = match data.get("meta") {
        Some(Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    let lang_code_key = format!("{resolved_lang}_code");
    let code = meta_str(&meta, &lang_code_key)
        .or_else(|| meta_str(&meta, "en_code"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let lang_title_key = format!("{resolved_lang}_title");
    let resolved_title = meta_str(&meta, &lang_title_key)
        .or_else(|| meta_str(&meta, "en_title"))
        .map(str::to_string);
    let resolved_year = year_of(meta.get("year"));

    let paras = match data.get(&resolved_lang) {
        Some(Value::Object(m)) => m,
        _ => {
            return Err(ExtractError::input_invalid(format!(
                "{}: {resolved_lang:?} must be an object of para_key -> entry",
                path.display()
            )))
        }
    };

    let mut out_blocks: Vec<Block> = Vec::new();
    let mut dropped_detail: Vec<Value> = Vec::new();
    let mut blocks_in: i64 = 0;

    progress.stage_start(Stage::Gate, Some(paras.len() as u64));
    for (para_key, entry) in paras.iter() {
        blocks_in += 1;
        let entry_obj = match entry {
            Value::Object(m) => m,
            _ => {
                dropped_detail.push(dropped_entry(para_key, "entry is not an object", None));
                continue;
            }
        };
        let text = match entry_obj.get("text") {
            Some(Value::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        if text.is_empty() {
            dropped_detail.push(dropped_entry(para_key, "empty text", None));
            continue;
        }
        let (page, para) = parse_page_para(para_key);
        out_blocks.push(Block::new(
            para_key.clone(),
            page,
            para,
            0,
            1,
            text.clone(),
            text.split_whitespace().count() as i64,
            Some(0),
        ));
    }
    progress.stage_end(Stage::Gate);

    // Python: `alignment = None; en_reverse = data.get("en_reverse"); if
    // en_reverse is not None: alignment = {...}` — an explicit JSON `null`
    // for `en_reverse` behaves exactly like the key being absent.
    let alignment = match data.get("en_reverse") {
        None | Some(Value::Null) => None,
        Some(rev) => {
            let mut m = serde_json::Map::new();
            m.insert(
                "en_code".into(),
                meta_str(&meta, "en_code")
                    .map(|s| Value::String(s.to_string()))
                    .unwrap_or(Value::Null),
            );
            m.insert("en_reverse".into(), rev.clone());
            Some(m)
        }
    };

    let joined = out_blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let stats = build_stats(
        blocks_in,
        out_blocks.len() as i64,
        dropped_detail.len() as i64,
        if joined.is_empty() {
            0.0
        } else {
            chunk::damage_score(&joined)
        },
        joined.split_whitespace().count() as i64,
        dropped_detail,
    );

    let book_meta = build_book_meta(
        Some(&code),
        Some(&resolved_lang),
        opts.book_pair.as_deref(),
        resolved_title.as_deref(),
        opts.author.as_deref(),
        resolved_year,
        opts.slug.as_deref(),
        opts.corpus.as_deref(),
        None,
    );

    let sha256 = sha256_file(path)?;
    Ok(Book {
        schema: SCHEMA_BOOK.to_string(),
        profile: "sop".to_string(),
        source: build_source(
            &path.display().to_string(),
            Some(&sha256),
            "sop_json",
            opts.acquired_from.as_deref(),
            opts.rights.as_deref(),
        ),
        book: book_meta,
        id_rule: "sop/plain".to_string(),
        alignment,
        stats,
        blocks: out_blocks,
    })
}
