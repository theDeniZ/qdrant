//! `validate` — semantic problems with a [`Book`], mirroring
//! `sopack.book.validate` message-for-message (substrings the Python test
//! suite asserts on are preserved verbatim; exact `repr()` quoting style is
//! not since nothing depends on it).

use std::collections::HashSet;

use crate::contract_bridge;
use crate::model::Book;

/// Semantic problems with *book*. Empty vec means it is valid.
///
/// Rejects: missing required metadata (`book_code`, `lang`, `title` — plus
/// `author`/`year` for non-EGW works, i.e. `book.corpus` is set to anything
/// but `"egw"`; Ellen White's works — `corpus = "egw"`, or no corpus at all,
/// the convention before 2026-10-04 — are exempt), an `id_rule` not allowed
/// by the book's profile, duplicate `(para_key, seq)` pairs, empty block
/// text, a `para_key` whose `seq` values are not dense from 0, and blocks
/// that do not partition into whole paragraphs of `chunks` pieces.
///
/// A `bible` book additionally needs a valid bible id as `book_code`, no
/// sop-only metadata (a bible point carries only `bible`/`osis`/`text`, so
/// it would be silently lost), and one unsplit block per verse keyed by its
/// OSIS reference with `page`/`para` = chapter/verse — see [`crate::osis`].
///
/// A `para_key` carrying several *paragraphs* is legitimate and is not an
/// error — see [`crate::model::Block`].
pub fn validate(book: &Book) -> Vec<String> {
    let mut errors = Vec::new();

    if book.schema != contract_bridge::SCHEMA_BOOK {
        errors.push(format!("unsupported schema {:?}", book.schema));
        return errors;
    }

    let profile = match contract_bridge::get_profile(&book.profile) {
        Ok(p) => p,
        Err(exc) => {
            errors.push(exc);
            return errors;
        }
    };

    let meta_truthy = |key: &str| -> bool {
        match book.book.get(key) {
            None => false,
            Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
            Some(serde_json::Value::Array(a)) => !a.is_empty(),
            Some(serde_json::Value::Object(o)) => !o.is_empty(),
        }
    };

    for key in ["book_code", "lang", "title"] {
        if !meta_truthy(key) {
            errors.push(format!("book.{key} is required"));
        }
    }

    if profile.name == "bible" {
        bible_problems(book, &meta_truthy, &mut errors);
    }

    let is_egw = profile.name == "bible"
        || match book.book.get("corpus") {
            None => true,
            Some(v) if v.is_null() => true,
            Some(v) => v.as_str() == Some("egw"),
        };
    if !is_egw {
        for key in ["author", "year"] {
            if !meta_truthy(key) {
                let corpus_repr = book
                    .book
                    .get("corpus")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "None".to_string());
                errors.push(format!(
                    "book.{key} is required for non-EGW works (corpus={corpus_repr})"
                ));
            }
        }
    }

    if !profile.id_rules.iter().any(|r| r == &book.id_rule) {
        errors.push(format!(
            "id_rule {:?} is not valid for profile {:?} (allowed: {})",
            book.id_rule,
            profile.name,
            profile.id_rules.join(", ")
        ));
    }

    let mut seen_pairs: HashSet<(String, i64)> = HashSet::new();
    // Insertion-ordered grouping by para_key, matching Python's
    // `defaultdict` (iterated in first-seen order).
    let mut order: Vec<String> = Vec::new();
    let mut by_para_key: std::collections::HashMap<String, Vec<&crate::model::Block>> =
        std::collections::HashMap::new();
    for b in &book.blocks {
        let pair = (b.para_key.clone(), b.seq);
        if seen_pairs.contains(&pair) {
            errors.push(format!(
                "duplicate block (para_key={:?}, seq={})",
                b.para_key, b.seq
            ));
        }
        seen_pairs.insert(pair);
        if b.text.trim().is_empty() {
            errors.push(format!(
                "block (para_key={:?}, seq={}) has empty text",
                b.para_key, b.seq
            ));
        }
        if !by_para_key.contains_key(&b.para_key) {
            order.push(b.para_key.clone());
        }
        by_para_key.entry(b.para_key.clone()).or_default().push(b);
    }

    for para_key in &order {
        let group = &by_para_key[para_key];
        let mut seqs: Vec<i64> = group.iter().map(|b| b.seq).collect();
        seqs.sort_unstable();
        let expected: Vec<i64> = (0..group.len() as i64).collect();
        if seqs != expected {
            errors.push(format!(
                "para_key {para_key:?}: seq values {seqs:?} are not 0..{} \
                 (every block sharing a para_key needs a dense, unique seq)",
                group.len() as i64 - 1
            ));
            continue;
        }

        let mut ordered = group.clone();
        ordered.sort_by_key(|b| b.seq);
        let n = ordered.len();
        let mut i = 0usize;
        while i < n {
            let chunks = ordered[i].chunks;
            if chunks < 1 {
                errors.push(format!(
                    "para_key {para_key:?}: chunks must be >= 1, got {chunks} at seq {}",
                    ordered[i].seq
                ));
                break;
            }
            let chunks_usize = chunks as usize;
            let end = (i + chunks_usize).min(n);
            let piece = &ordered[i..end];
            if piece.len() < chunks_usize {
                errors.push(format!(
                    "para_key {para_key:?}: paragraph at seq {} claims chunks={chunks} but only {} block(s) follow",
                    ordered[i].seq,
                    piece.len()
                ));
                break;
            }
            let mut bad_chunks: Vec<i64> = piece.iter().map(|b| b.chunks).collect();
            bad_chunks.sort_unstable();
            bad_chunks.dedup();
            if bad_chunks != vec![chunks] {
                errors.push(format!(
                    "para_key {para_key:?}: inconsistent 'chunks' values {bad_chunks:?} within the paragraph starting at seq {}",
                    ordered[i].seq
                ));
                break;
            }
            let got_chunk_seq: Vec<i64> = piece.iter().map(|b| b.chunk).collect();
            let want_chunk_seq: Vec<i64> = (0..chunks).collect();
            if got_chunk_seq != want_chunk_seq {
                errors.push(format!(
                    "para_key {para_key:?}: chunk values {got_chunk_seq:?} do not match chunks={chunks} (expected {want_chunk_seq:?})"
                ));
                break;
            }
            i += chunks_usize;
        }
    }

    errors
}

/// Book metadata a `bible` point does not carry (its payload is
/// `bible`/`osis`/`text` only) — given on a bible book, it would be dropped.
const NOT_FOR_BIBLES: [&str; 6] = ["book_pair", "author", "year", "slug", "corpus", "page_kind"];

/// At most this many per-block problems of one kind are listed; the rest are
/// counted, so a wholly wrong source does not print 31 000 lines.
const MAX_LISTED: usize = 10;

fn bible_problems(book: &Book, meta_truthy: &dyn Fn(&str) -> bool, errors: &mut Vec<String>) {
    if let Some(code) = book.book.get("book_code").and_then(|v| v.as_str()) {
        if !code.is_empty() && !crate::osis::is_bible_id(code) {
            errors.push(format!(
                "book.book_code {code:?} is not a valid bible id \
                 (lowercase letters and digits, '-'/'_' after the first, e.g. \"luther1912\")"
            ));
        }
    }
    for key in NOT_FOR_BIBLES {
        if meta_truthy(key) {
            errors.push(format!(
                "book.{key} is not used by the bible profile (a bible point carries only \
                 bible/osis/text) — remove it"
            ));
        }
    }

    let mut bad_key = Vec::new();
    let mut bad_pos = Vec::new();
    let mut split = Vec::new();
    for b in &book.blocks {
        match crate::osis::parse_verse(&b.para_key) {
            None => bad_key.push(format!("{:?}", b.para_key)),
            Some(v) if (b.page, b.para) != (v.chapter, v.verse) => bad_pos.push(format!(
                "{:?} (page={}, para={})",
                b.para_key, b.page, b.para
            )),
            Some(_) => {}
        }
        if b.seq != 0 || b.chunks != 1 {
            split.push(format!(
                "{:?} (seq={}, chunks={})",
                b.para_key, b.seq, b.chunks
            ));
        }
    }
    let mut report = |items: Vec<String>, what: &str| {
        if items.is_empty() {
            return;
        }
        let more = items.len().saturating_sub(MAX_LISTED);
        let mut listed = items
            .into_iter()
            .take(MAX_LISTED)
            .collect::<Vec<_>>()
            .join(", ");
        if more > 0 {
            listed.push_str(&format!(" … and {more} more"));
        }
        errors.push(format!("{what}: {listed}"));
    };
    report(
        bad_key,
        "para_key is not an OSIS verse reference (Book.Chapter.Verse[!grain], one of the 66 \
         canonical OSIS books)",
    );
    report(bad_pos, "page/para do not match the OSIS chapter/verse");
    report(
        split,
        "a verse must be one block (seq=0, chunks=1): bible/v1 ids carry no seq, so a split \
         verse would overwrite itself",
    );
}
