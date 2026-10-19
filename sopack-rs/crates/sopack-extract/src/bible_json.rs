//! OSIS-keyed Bible JSON → [`sopack_book::Book`] (profile `bible`).
//!
//! Reads the flat `{"Gen.1.1": "In the beginning…", …}` shape of
//! `generator/data/bibles/<id>.json` — the files the live `bibles`
//! collection was built from. Keys are OSIS verse references in the
//! edition's **own** numbering (no KJV remap; see `sopack_book::osis`),
//! values the verse text.
//!
//! One block per verse, never chunked: `bible/v1` ids (`bible:{bible}:{osis}`)
//! carry no `seq`, so a split verse would overwrite itself. The embedder
//! truncates at the contract's token window, which no real verse reaches.
//! Text is kept verbatim except for trimming, so re-packing a live Bible's
//! file reproduces its points exactly (`reimport_is_normal` for bibles).
//!
//! Metadata: `book_code` (the bible id, e.g. `luther1912`) falls back to the
//! file stem — the documented naming of this source; `lang` and `title` are
//! never in the file and must be given (sidecar or flags). Nothing else is
//! accepted: a bible point carries only `bible`/`osis`/`text`.
//!
//! A key that is not an OSIS verse reference, or a value that is not a
//! string, makes the whole file invalid (exit 3) rather than being dropped:
//! a wrong key would silently mis-number a verse. An empty verse is dropped
//! and recorded in `dropped_detail`.

use std::path::Path;

use serde_json::Value;
use sopack_book::osis::parse_verse;
use sopack_book::{Block, Book, SCHEMA_BOOK};

use crate::chunk;
use crate::common::{build_book_meta, build_source, build_stats, dropped_entry, sha256_file};
use crate::error::ExtractError;
use crate::options::ExtractOptions;
use crate::progress::{ProgressSink, Stage};

/// How many offending keys an error message lists before counting the rest.
const MAX_LISTED: usize = 10;

fn listed(items: &[String]) -> String {
    let mut s = items
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > MAX_LISTED {
        s.push_str(&format!(" … and {} more", items.len() - MAX_LISTED));
    }
    s
}

/// Does *data* look like an OSIS-keyed Bible (non-empty, every key an OSIS
/// verse reference)? What `sopack extract` uses to tell a `.json` Bible
/// from a `sop_json` book without `--kind`.
pub fn looks_like_bible(data: &serde_json::Map<String, Value>) -> bool {
    !data.is_empty() && data.keys().all(|k| parse_verse(k).is_some())
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
    let data = match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(m)) => m,
        Ok(_) => {
            return Err(ExtractError::input_invalid(format!(
                "{}: top level must be a JSON object of OSIS verse -> text",
                path.display()
            )))
        }
        Err(e) => {
            return Err(ExtractError::input_invalid(format!(
                "{}: not valid JSON: {e}",
                path.display()
            )))
        }
    };
    progress.stage_end(Stage::ReadContainer);

    let bad_keys: Vec<String> = data
        .keys()
        .filter(|k| parse_verse(k).is_none())
        .map(|k| format!("{k:?}"))
        .collect();
    if !bad_keys.is_empty() {
        return Err(ExtractError::input_invalid(format!(
            "{}: {} key(s) are not OSIS verse references: {}",
            path.display(),
            bad_keys.len(),
            listed(&bad_keys)
        ))
        .with_hint(
            "keys must be Book.Chapter.Verse (optionally !a for a split verse) in the \
             edition's own numbering, with one of the 66 canonical OSIS book names",
        ));
    }
    let non_text: Vec<String> = data
        .iter()
        .filter(|(_, v)| !v.is_string())
        .map(|(k, _)| format!("{k:?}"))
        .collect();
    if !non_text.is_empty() {
        return Err(ExtractError::input_invalid(format!(
            "{}: {} verse(s) are not strings: {}",
            path.display(),
            non_text.len(),
            listed(&non_text)
        )));
    }

    progress.stage_start(Stage::Gate, Some(data.len() as u64));
    let mut verses: Vec<(_, &String, String)> = data
        .iter()
        .map(|(k, v)| {
            let verse = parse_verse(k).expect("checked above");
            (verse, k, v.as_str().unwrap_or_default().trim().to_string())
        })
        .collect();
    verses.sort_by_key(|(v, _, _)| v.sort_key());

    let mut blocks = Vec::new();
    let mut dropped_detail = Vec::new();
    for (verse, key, text) in verses {
        if text.is_empty() {
            dropped_detail.push(dropped_entry(key, "empty verse", None));
            continue;
        }
        let words = text.split_whitespace().count() as i64;
        blocks.push(Block::new(
            key.clone(),
            verse.chapter,
            verse.verse,
            0,
            1,
            text,
            words,
            Some(0),
        ));
    }
    progress.stage_end(Stage::Gate);

    let joined = blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let stats = build_stats(
        data.len() as i64,
        blocks.len() as i64,
        dropped_detail.len() as i64,
        if joined.is_empty() {
            0.0
        } else {
            chunk::damage_score(&joined)
        },
        joined.split_whitespace().count() as i64,
        dropped_detail,
    );

    let code = opts.book_code.clone().unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let book_meta = build_book_meta(
        Some(&code),
        opts.lang.as_deref(),
        None,
        opts.title.as_deref(),
        None,
        None,
        None,
        None,
        None,
    );

    let sha256 = sha256_file(path)?;
    Ok(Book {
        schema: SCHEMA_BOOK.to_string(),
        profile: "bible".to_string(),
        source: build_source(
            &path.display().to_string(),
            Some(&sha256),
            "bible_json",
            opts.acquired_from.as_deref(),
            opts.rights.as_deref(),
        ),
        book: book_meta,
        id_rule: "bible/v1".to_string(),
        alignment: None,
        stats,
        blocks,
    })
}
