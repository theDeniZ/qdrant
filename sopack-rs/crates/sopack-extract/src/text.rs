//! Plain text → [`sopack_book::Book`]. Port of `sopack/extract/text.py`.
//!
//! No page/paragraph structure to recover, so the whole file is split on
//! blank lines into paragraphs and numbered sequentially: `page` is always
//! `1`, `para` counts up. Long paragraphs are chunked the same way as the
//! EPUB extractor ([`crate::chunk`]); short/damaged/junk paragraphs are
//! dropped and recorded in `stats["dropped_detail"]` (R8).

use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Regex;
use sopack_book::{Block, Book, SCHEMA_BOOK};

use crate::chunk;
use crate::common::{
    build_book_meta, build_source, build_stats, char_truncate, dropped_entry, sha256_file,
};
use crate::error::ExtractError;
use crate::options::ExtractOptions;
use crate::progress::{ProgressSink, Stage};

static BLANK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n\s*\n+").expect("valid BLANK_RE"));
static SPACE_RUN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ \t]+").expect("valid space run"));

fn regex_split(re: &Regex, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0usize;
    for m in re.find_iter(text) {
        let m = m.expect("regex match");
        out.push(text[last..m.start()].to_string());
        last = m.end();
    }
    out.push(text[last..].to_string());
    out
}

fn regex_replace_all(re: &Regex, text: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for m in re.find_iter(text) {
        let m = m.expect("regex match");
        out.push_str(&text[last..m.start()]);
        out.push_str(replacement);
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
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
    let raw_bytes = std::fs::read(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {}: {e}", path.display())))?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    progress.stage_end(Stage::ReadContainer);

    let mut out_blocks: Vec<Block> = Vec::new();
    let mut dropped_detail: Vec<serde_json::Value> = Vec::new();
    let mut blocks_in: i64 = 0;
    let mut para_no: i64 = 0;

    progress.stage_start(Stage::Chunk, None);
    for raw_para in regex_split(&BLANK_RE, &raw) {
        let text = regex_replace_all(&SPACE_RUN_RE, raw_para.trim(), " ");
        if text.is_empty() {
            continue;
        }
        blocks_in += 1;
        para_no += 1;
        let para_key = format!("1.{para_no}");

        progress.stage_start(Stage::Gate, None);
        let reason = chunk::quality_gate(&text, &opts.chunk_limits);
        progress.stage_end(Stage::Gate);
        if let Some(reason) = reason {
            dropped_detail.push(dropped_entry(
                &para_key,
                &reason,
                Some(&char_truncate(&text, 80)),
            ));
            continue;
        }

        let pieces = chunk::split_long(&text, &opts.chunk_limits);
        for (j, piece) in pieces.iter().enumerate() {
            out_blocks.push(Block::new(
                para_key.clone(),
                1,
                para_no,
                j as i64,
                pieces.len() as i64,
                piece.clone(),
                piece.split_whitespace().count() as i64,
                Some(j as i64),
            ));
        }
    }
    progress.stage_end(Stage::Chunk);

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

    let book_pair = opts.book_pair.as_deref().or(opts.book_code.as_deref());
    let book_meta = build_book_meta(
        opts.book_code.as_deref(),
        opts.lang.as_deref(),
        book_pair,
        opts.title.as_deref(),
        opts.author.as_deref(),
        opts.year,
        opts.slug.as_deref(),
        opts.corpus.as_deref(),
        Some("chapter"),
    );

    let sha256 = sha256_file(path)?;
    Ok(Book {
        schema: SCHEMA_BOOK.to_string(),
        profile: "sop".to_string(),
        source: build_source(
            &path.display().to_string(),
            Some(&sha256),
            "text",
            opts.acquired_from.as_deref(),
            opts.rights.as_deref(),
        ),
        book: book_meta,
        id_rule: "sop/seq".to_string(),
        alignment: None,
        stats,
        blocks: out_blocks,
    })
}
