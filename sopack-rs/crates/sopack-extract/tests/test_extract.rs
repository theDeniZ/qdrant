//! Port of `qdrant/sopack/tests/test_extract.py` — same test names, same
//! behaviour asserted. Uses the crate's public API (`sopack_extract::*`)
//! plus the epub-specific regexes it re-exports for the boilerplate/ref
//! tests.

use std::io::Write;
use std::str::FromStr;

use sopack_book::validate;
use sopack_extract::{
    chunk, extract, ExtractOptions, Kind, NoopProgress, BOILERPLATE_RE, NUMERIC_RE,
};

// ── tiny in-memory EPUB fixture builder ─────────────────────────────────

const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

fn opf(title: &str, creator: &str, date: &str, items: &[&str]) -> String {
    let manifest_items: String = items
        .iter()
        .enumerate()
        .map(|(i, h)| format!(r#"<item id="c{i}" href="{h}" media-type="application/xhtml+xml"/>"#))
        .collect::<Vec<_>>()
        .join("\n");
    let spine_items: String = (0..items.len())
        .map(|i| format!(r#"<itemref idref="c{i}"/>"#))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{creator}</dc:creator>
    <dc:date>{date}</dc:date>
    <dc:rights>Public domain</dc:rights>
  </metadata>
  <manifest>
    {manifest_items}
  </manifest>
  <spine>
    {spine_items}
  </spine>
</package>"#
    )
}

/// *chapters* are raw XHTML body fragments, one per spine document.
fn make_epub(path: &std::path::Path, chapters: &[&str], title: &str, creator: &str, date: &str) {
    let names: Vec<String> = (0..chapters.len())
        .map(|i| format!("chap{i}.xhtml"))
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("mimetype", options).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    zip.start_file("META-INF/container.xml", options).unwrap();
    zip.write_all(CONTAINER_XML.as_bytes()).unwrap();

    let href_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    zip.start_file("OEBPS/content.opf", options).unwrap();
    zip.write_all(opf(title, creator, date, &href_refs).as_bytes())
        .unwrap();

    for (name, body) in names.iter().zip(chapters.iter()) {
        zip.start_file(format!("OEBPS/{name}"), options).unwrap();
        zip.write_all(
            format!(r#"<?xml version="1.0"?><html><body>{body}</body></html>"#).as_bytes(),
        )
        .unwrap();
    }
    zip.finish().unwrap();
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::mem::forget(dir);
    path
}

fn opts() -> ExtractOptions {
    ExtractOptions::default()
}

// ── ChunkBoundaries ──────────────────────────────────────────────────────

#[test]
fn test_short_text_not_split() {
    let text = "This is a short paragraph of prose.";
    assert_eq!(
        chunk::split_long(text, &chunk::ChunkLimits::default()),
        vec![text.to_string()]
    );
}

#[test]
fn test_long_text_is_split_on_sentence_boundaries() {
    let sentence = "This is one sentence with several words in it. ";
    let text = sentence.repeat(40); // well over MAX_WORDS
    let limits = chunk::ChunkLimits::default();
    let pieces = chunk::split_long(&text, &limits);
    assert!(pieces.len() > 1);
    for p in &pieces {
        assert!(p.split_whitespace().count() <= limits.max_words);
    }
    // nothing lost
    let joined_words: usize = pieces.join(" ").split_whitespace().count();
    assert_eq!(joined_words, text.split_whitespace().count());
}

#[test]
fn test_single_sentence_longer_than_max_words_is_hard_cut() {
    let limits = chunk::ChunkLimits::default();
    let text = "word ".repeat(limits.max_words + 50);
    let pieces = chunk::split_long(text.trim(), &limits);
    assert!(pieces.len() > 1);
    for p in &pieces {
        assert!(p.split_whitespace().count() <= limits.max_words);
    }
}

#[test]
fn test_quality_gate_drops_too_short() {
    let reason = chunk::quality_gate("Too short.", &chunk::ChunkLimits::default());
    assert!(reason.is_some());
    assert!(reason.unwrap().contains("short"));
}

#[test]
fn test_quality_gate_passes_normal_prose() {
    let text = "This is a perfectly ordinary sentence of nineteenth century prose about faith.";
    assert!(chunk::quality_gate(text, &chunk::ChunkLimits::default()).is_none());
}

// ── DamageGate ───────────────────────────────────────────────────────────

#[test]
fn test_clean_prose_scores_low_damage() {
    let text = "The truth of the gospel shines through every generation of believers.";
    assert!(chunk::damage_score(text) < 0.1);
}

#[test]
fn test_scrambled_text_scores_high_damage() {
    let text = "^^i'^M - er^ aniel anb IRrt^flati oNs^ ButastoJesus xQz^^ fW~rD";
    assert!(chunk::damage_score(text) > chunk::ChunkLimits::default().max_block_damage);
}

#[test]
fn test_junk_char_ratio_flags_box_drawing_noise() {
    let text = "░▒▓".repeat(20);
    assert!(chunk::junk_char_ratio(&text) > chunk::ChunkLimits::default().max_junk_chars);
}

#[test]
fn test_junk_char_ratio_clean_for_ordinary_text() {
    let text = "Ordinary English prose, with punctuation; and a hyphen-ated word.";
    assert!(chunk::junk_char_ratio(text) < chunk::ChunkLimits::default().max_junk_chars);
}

// ── BoilerplateStripping ─────────────────────────────────────────────────

#[test]
fn test_archive_org_disclaimer_matches() {
    let text = "This book was produced in EPUB format by the Internet Archive.";
    assert!(BOILERPLATE_RE.is_match(text).unwrap());
}

#[test]
fn test_gutenberg_credit_matches() {
    let text = "Produced by the Online Distributed Proofreading Team at pgdp.net";
    assert!(BOILERPLATE_RE.is_match(text).unwrap());
}

#[test]
fn test_ordinary_prose_does_not_match() {
    let text = "In the beginning God created the heavens and the earth.";
    assert!(!BOILERPLATE_RE.is_match(text).unwrap());
}

#[test]
fn test_numeric_toc_line_detected() {
    let tokens: Vec<&str> = "1 2 3 4 5 6 7 8 9 10 Contents Page".split(' ').collect();
    let numeric_hits = tokens
        .iter()
        .filter(|t| NUMERIC_RE.is_match(t).unwrap())
        .count();
    assert!((numeric_hits as f64) > 0.35 * (tokens.len() as f64));
}

// ── RefLifting ───────────────────────────────────────────────────────────

#[test]
fn test_ref_re_lifts_trailing_page_para() {
    // REF_RE itself is private; exercised indirectly through a full epub
    // extraction below (test_inline_ref_lifts_page_para_and_strips_citation)
    // as well as directly here via the extractor's own citation-detection
    // behaviour on a single block.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>Some quoted sentence of the book. CHR 24.1</p><p>Another sentence of prose that is long enough here today.</p>"],
        "T",
        "A",
        "1900",
    );
    let mut o = opts();
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("x".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();
    assert_eq!(book.book["book_code"], serde_json::json!("CHR"));
}

#[test]
fn test_ref_re_does_not_match_plain_prose() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>This sentence has no trailing citation in it at all today.</p><p>Neither does this one, which is also long enough to pass gates.</p>"],
        "T",
        "A",
        "1900",
    );
    let book = extract(&path, Kind::Epub, &opts(), &mut NoopProgress).unwrap();
    assert!(book.book["book_code"].is_null());
}

#[test]
fn test_pagemark_re_matches_bare_bracketed_number() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>[89]</p><p>This is a real paragraph of prose that follows the page marker here.</p>"],
        "T",
        "A",
        "1900",
    );
    let book = extract(&path, Kind::Epub, &opts(), &mut NoopProgress).unwrap();
    assert!(book.blocks.iter().any(|b| b.para_key == "89.1"));
}

#[test]
fn test_pagemark_re_does_not_match_prose() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>[not a page marker]</p><p>This is a real paragraph of prose in the book today.</p>"],
        "T",
        "A",
        "1900",
    );
    let book = extract(&path, Kind::Epub, &opts(), &mut NoopProgress).unwrap();
    // "[not a page marker]" is itself extracted as ordinary (short) text,
    // not treated as a page marker, so page/para bookkeeping falls back to
    // chapter.ordinal keys starting at "1.1".
    assert!(book
        .blocks
        .iter()
        .any(|b| b.para_key == "1.1" || b.para_key == "1.2"));
}

// ── EpubExtraction ───────────────────────────────────────────────────────

fn long_para(n: usize) -> String {
    "This sentence has a handful of words in it for length. "
        .repeat(n)
        .trim()
        .to_string()
}

#[test]
fn test_basic_extraction_reads_opf_metadata_and_paragraphs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>This is the first paragraph of real prose in the book.</p><p>This is the second paragraph, also real prose, quite readable.</p>"],
        "History of Something",
        "J. N. Andrews",
        "1873",
    );
    let mut o = opts();
    o.book_code = Some("HSFD".into());
    o.corpus = Some("pioneers".into());
    o.slug = Some("hsfd".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert_eq!(book.profile, "sop");
    assert_eq!(book.id_rule, "sop/seq");
    assert_eq!(book.book["book_code"], serde_json::json!("HSFD"));
    assert_eq!(
        book.book["title"],
        serde_json::json!("History of Something")
    );
    assert_eq!(book.book["author"], serde_json::json!("J. N. Andrews"));
    assert_eq!(book.book["year"], serde_json::json!(1873));
    assert_eq!(book.book["book_pair"], serde_json::json!("HSFD"));
    assert_eq!(book.blocks.len(), 2);
    assert_eq!(book.stats["blocks_in"], serde_json::json!(2));
    assert_eq!(book.stats["blocks_out"], serde_json::json!(2));
    assert_eq!(book.stats["dropped"], serde_json::json!(0));
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_boilerplate_paragraph_is_dropped_and_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>This book was produced in EPUB format by the Internet Archive, which relies on optical character recognition software.</p><p>This is genuine prose that should survive the boilerplate filter fine.</p>"],
        "T",
        "A",
        "1900",
    );
    let mut o = opts();
    o.book_code = Some("X".into());
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("x".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert_eq!(book.blocks.len(), 1);
    assert_eq!(book.stats["dropped"], serde_json::json!(1));
    let dropped_detail = book.stats["dropped_detail"].as_array().unwrap();
    assert!(dropped_detail
        .iter()
        .any(|d| d["reason"].as_str().unwrap().contains("boilerplate")));
}

#[test]
fn test_inline_ref_lifts_page_para_and_strips_citation() {
    let blocks_html: String = (1..6)
        .map(|i| {
            format!("<p>Sentence number {i} of real readable prose in this book. CHR {i}.1</p>")
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(&path, &[&blocks_html], "T", "A", "1900");
    let mut o = opts();
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("x".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert_eq!(book.book["book_code"], serde_json::json!("CHR"));
    assert_eq!(book.book["page_kind"], serde_json::json!("print"));
    let mut keys: Vec<String> = book.blocks.iter().map(|b| b.para_key.clone()).collect();
    keys.sort();
    assert_eq!(keys, (1..6).map(|i| format!("{i}.1")).collect::<Vec<_>>());
    for b in &book.blocks {
        assert!(!b.text.contains("CHR"));
    }
}

#[test]
fn test_uncited_heading_colliding_with_a_cited_key_still_validates() {
    // The COOH/DOCP failure: in a coded book the uncited blocks (headings)
    // are keyed by chapter ordinal, which collides with the citation keys.
    let mut blocks_html: String = (1..8)
        .map(|i| {
            format!("<p>Sentence number {i} of real readable prose in this book. CHR 1.{i}</p>")
        })
        .collect();
    blocks_html.push_str("<p>II. WHAT ARE WE TO UNDERSTAND BY THE FALL OF BABYLON</p>");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(&path, &[&blocks_html], "T", "A", "1900");
    let mut o = opts();
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("x".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert_eq!(validate(&book), Vec::<String>::new());
    let mut collided: Vec<(i64, i64)> = book
        .blocks
        .iter()
        .filter(|b| b.para_key == "1.1")
        .map(|b| (b.seq, b.chunk))
        .collect();
    collided.sort();
    assert_eq!(collided, vec![(0, 0), (1, 0)]);
}

#[test]
fn test_pagemark_drives_page_para_when_no_inline_code() {
    let html = "<p>[12]</p><p>This is the first real paragraph of prose on printed page twelve.</p><p>This is the second real paragraph of prose on the very same page.</p><p>[13]</p><p>This is a paragraph of prose that begins printed page thirteen instead.</p>";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(&path, &[html], "T", "A", "1900");
    let mut o = opts();
    o.book_code = Some("Z".into());
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("z".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    let mut keys: Vec<String> = book.blocks.iter().map(|b| b.para_key.clone()).collect();
    keys.sort();
    assert_eq!(keys, vec!["12.1", "12.2", "13.1"]);
    assert_eq!(book.book["page_kind"], serde_json::json!("print"));
}

#[test]
fn test_over_long_paragraph_is_chunked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    let p = format!("<p>{}</p>", long_para(40));
    make_epub(&path, &[&p], "T", "A", "1900");
    let mut o = opts();
    o.book_code = Some("LNG".into());
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("lng".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert!(book.blocks.len() > 1);
    let limits = chunk::ChunkLimits::default();
    for b in &book.blocks {
        assert!(b.words as usize <= limits.max_words);
    }
    let chunks_set: std::collections::HashSet<i64> = book.blocks.iter().map(|b| b.chunks).collect();
    assert_eq!(
        chunks_set,
        std::collections::HashSet::from([book.blocks.len() as i64])
    );
    let mut seqs: Vec<i64> = book.blocks.iter().map(|b| b.seq).collect();
    seqs.sort();
    assert_eq!(seqs, (0..book.blocks.len() as i64).collect::<Vec<_>>());
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_scrambled_title_page_is_dropped_by_damage_gate() {
    let html = "<p>^^i'^M - er^ aniel anb IRrt^flati oNs^ ButastoJesus xQz^^ fW~rD garbled text</p><p>This is a perfectly normal paragraph of readable nineteenth century prose text.</p>";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(&path, &[html], "T", "A", "1900");
    let mut o = opts();
    o.book_code = Some("DAR".into());
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("dar".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();

    assert_eq!(book.blocks.len(), 1);
    assert_eq!(book.stats["dropped"], serde_json::json!(1));
}

#[test]
fn test_missing_file_raises() {
    let result = extract(
        std::path::Path::new("/no/such/file.epub"),
        Kind::Epub,
        &opts(),
        &mut NoopProgress,
    );
    assert!(result.is_err());
}

#[test]
fn test_book_code_left_null_when_not_derivable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>Ordinary prose with no inline citation code at all here.</p>"],
        "T",
        "A",
        "1900",
    );
    let book = extract(&path, Kind::Epub, &opts(), &mut NoopProgress).unwrap(); // no book_code given, no inline code present
    assert!(book.book["book_code"].is_null());
    let errors = validate(&book);
    assert!(errors.iter().any(|e| e.contains("book_code")));
}

// ── SopJsonExtraction ────────────────────────────────────────────────────

#[test]
fn test_english_only_file() {
    let doc = serde_json::json!({
        "meta": {"en_code": "ABC", "en_title": "A Book", "publisher": "White Estate",
                 "year": "", "en_pages": 10},
        "en": {
            "0.1": {"text": "This is the first paragraph of the English original.", "chapter": "Preface"},
            "0.2": {"text": "This is the second paragraph, also part of the preface.", "chapter": "Preface"},
        },
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("en");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("ABC.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let book = extract(&path, Kind::SopJson, &opts(), &mut NoopProgress).unwrap();

    assert_eq!(book.profile, "sop");
    assert_eq!(book.id_rule, "sop/plain");
    assert_eq!(book.book["lang"], serde_json::json!("en"));
    assert_eq!(book.book["book_code"], serde_json::json!("ABC"));
    assert_eq!(book.book["title"], serde_json::json!("A Book"));
    assert!(book.book["corpus"].is_null());
    assert!(book.alignment.is_none());
    assert_eq!(book.blocks.len(), 2);
    for b in &book.blocks {
        assert_eq!(b.chunks, 1);
        assert_eq!(b.seq, 0);
    }
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_german_file_carries_en_reverse_into_alignment() {
    let doc = serde_json::json!({
        "meta": {"de_code": "BH", "en_code": "SL", "de_title": "Biblische Heiligung",
                 "en_title": "The Sanctified Life", "publisher": "Advent-Verlag",
                 "year": "1973", "de_pages": 61, "german_origin": false},
        "de": {
            "5.1": {"text": "Heiligung im Sinne der Bibel umfasst den ganzen Menschen.",
                    "en_ref": "7.1", "chapter": "Kapitel 1"},
            "5.2": {"text": "In der religiösen Welt herrscht eine falsche Heiligungslehre.",
                    "en_ref": "7.2", "chapter": "Kapitel 1"},
        },
        "en_reverse": {"7.1": ["5.1"], "7.2": ["5.2"]},
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("de");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("BH.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let book = extract(&path, Kind::SopJson, &opts(), &mut NoopProgress).unwrap();

    assert_eq!(book.book["lang"], serde_json::json!("de"));
    assert_eq!(book.book["book_code"], serde_json::json!("BH"));
    let alignment = book.alignment.clone().unwrap();
    assert_eq!(alignment["en_code"], serde_json::json!("SL"));
    assert_eq!(alignment["en_reverse"], doc["en_reverse"]);

    let b1 = book.blocks.iter().find(|b| b.para_key == "5.1").unwrap();
    let payload = sopack_book::to_payload(&book, b1).unwrap();
    assert_eq!(payload["aligned"], serde_json::json!(["7.1"]));
}

#[test]
fn test_empty_paragraph_text_is_dropped() {
    let doc = serde_json::json!({
        "meta": {"en_code": "ABC", "en_title": "A Book"},
        "en": {
            "0.1": {"text": "   ", "chapter": "Preface"},
            "0.2": {"text": "This paragraph actually has content in it.", "chapter": "Preface"},
        },
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("en");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("ABC.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let book = extract(&path, Kind::SopJson, &opts(), &mut NoopProgress).unwrap();

    assert_eq!(book.blocks.len(), 1);
    assert_eq!(book.stats["dropped"], serde_json::json!(1));
    assert_eq!(
        book.stats["dropped_detail"][0]["reason"],
        serde_json::json!("empty text")
    );
}

#[test]
fn test_missing_file_raises_sop_json() {
    let result = extract(
        std::path::Path::new("/no/such/file.json"),
        Kind::SopJson,
        &opts(),
        &mut NoopProgress,
    );
    assert!(result.is_err());
}

#[test]
fn test_bad_json_raises() {
    let path = tmp_path("bad.json");
    std::fs::write(&path, "{not json").unwrap();
    let result = extract(&path, Kind::SopJson, &opts(), &mut NoopProgress);
    assert!(result.is_err());
}

// ── MarkdownAndTextExtraction ────────────────────────────────────────────

#[test]
fn test_markdown_headings_bump_page_and_reset_para() {
    let md = "# Chapter One\n\nThis is the first paragraph of chapter one, with plenty of words to pass gates.\n\nThis is the second paragraph of chapter one, likewise readable prose here.\n\n# Chapter Two\n\nThis is the first paragraph of chapter two, again with enough words in it.\n";
    let path = tmp_path("book.md");
    std::fs::write(&path, md).unwrap();
    let mut o = opts();
    o.book_code = Some("MD1".into());
    o.lang = Some("en".into());
    o.title = Some("T".into());
    o.author = Some("A".into());
    o.year = Some(2020);
    o.corpus = Some("test".into());
    o.slug = Some("md1".into());
    let book = extract(&path, Kind::Markdown, &o, &mut NoopProgress).unwrap();

    let mut keys: Vec<String> = book.blocks.iter().map(|b| b.para_key.clone()).collect();
    keys.sort();
    assert_eq!(keys, vec!["2.1", "2.2", "3.1"]);
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_text_paragraphs_split_on_blank_lines() {
    let txt = "This is the first paragraph of plain text with enough words in it to pass.\n\nThis is the second paragraph of plain text, also long enough to survive.\n";
    let path = tmp_path("book.txt");
    std::fs::write(&path, txt).unwrap();
    let mut o = opts();
    o.book_code = Some("TX1".into());
    o.lang = Some("en".into());
    o.title = Some("T".into());
    o.author = Some("A".into());
    o.year = Some(2020);
    o.corpus = Some("test".into());
    o.slug = Some("tx1".into());
    let book = extract(&path, Kind::Text, &o, &mut NoopProgress).unwrap();

    assert_eq!(book.blocks.len(), 2);
    assert!(book.blocks.iter().all(|b| b.page == 1));
    assert_eq!(validate(&book), Vec::<String>::new());
}

#[test]
fn test_unknown_kind_raises() {
    assert_eq!(Kind::from_str("pdf"), Err(()));
}

// ── PayloadAndUidAgreement ───────────────────────────────────────────────
// Every extractor's output must produce payloads contract-valid and uids
// that agree with `sopack_book::uid_for`.

fn assert_book_payloads_and_uids_valid(book: &sopack_book::Book) {
    let profile = sopack_book::get_profile(&book.profile).unwrap();
    for block in &book.blocks {
        let payload = sopack_book::to_payload(book, block).unwrap();
        let errors = sopack_book::validate_payload(profile, &payload);
        assert_eq!(
            errors,
            Vec::<String>::new(),
            "{}: {:?}",
            block.para_key,
            errors
        );
        let u = sopack_book::uid(book, block).unwrap();

        let mut fields: std::collections::HashMap<&str, Option<String>> =
            std::collections::HashMap::new();
        if book.id_rule == "sop/seq" {
            fields.insert("lang", book.meta_str("lang"));
            fields.insert("book_code", book.meta_str("book_code"));
            fields.insert("para_key", Some(block.para_key.clone()));
            fields.insert("seq", Some(block.seq.to_string()));
        } else if book.id_rule == "sop/plain" {
            fields.insert("lang", book.meta_str("lang"));
            fields.insert("book_code", book.meta_str("book_code"));
            fields.insert("para_key", Some(block.para_key.clone()));
        } else {
            panic!("unexpected id_rule {:?}", book.id_rule);
        }
        assert_eq!(u, sopack_book::uid_for(&book.id_rule, &fields).unwrap());
    }
}

#[test]
fn test_epub_book() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.epub");
    make_epub(
        &path,
        &["<p>This is the first paragraph of real prose in the book.</p><p>This is the second paragraph, also real prose, quite readable.</p>"],
        "T",
        "A",
        "1900",
    );
    let mut o = opts();
    o.book_code = Some("PAY".into());
    o.corpus = Some("pioneers".into());
    o.author = Some("A".into());
    o.year = Some(1900);
    o.slug = Some("pay".into());
    let book = extract(&path, Kind::Epub, &o, &mut NoopProgress).unwrap();
    assert_book_payloads_and_uids_valid(&book);
}

#[test]
fn test_sop_json_book() {
    let doc = serde_json::json!({
        "meta": {"en_code": "PAY2", "en_title": "A Book"},
        "en": {"0.1": {"text": "This is a paragraph of real EGW prose for the test."}},
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("en");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("PAY2.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let book = extract(&path, Kind::SopJson, &opts(), &mut NoopProgress).unwrap();
    assert_book_payloads_and_uids_valid(&book);
}
