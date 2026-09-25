//! EPUB → [`sopack_book::Book`]. Port of `sopack/extract/epub.py` (itself
//! ported from `pd-books/qdrant/build_pioneers_corpus.py`: zipfile + OPF
//! metadata, HTML→text, boilerplate/TOC stripping, the printed
//! page.paragraph citation lift).

use std::collections::{HashMap, HashSet};
use std::fs::File;
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

// A trailing "CHR 24.1" is the printed page.paragraph reference many
// archive.org digital editions carry inline. It is the real citation key, so
// it is lifted into page/para and stripped from the embedded text.
static REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*([A-Z][A-Za-z0-9]{1,9})\s+(\d{1,4})\.(\d{1,3})\s*$").expect("valid REF_RE")
});
static PAGEMARK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(\d{1,4})\]$").expect("valid PAGEMARK_RE"));

// archive.org prepends a scanner disclaimer to every EPUB it generates, and
// appends a per-page accuracy banner. It is scanner metadata, not book text.
// Project Gutenberg prepends its own transcriber credits and licence header.
pub static BOILERPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        "(?i)produced in EPUB format by the Internet Archive",
        "|relies on optical character recognition",
        "|scanned and converted to EPUB format automatically",
        "|this page is estimated to be",
        "|The Internet Archive was founded in 1996",
        r"|archive\.org/details",
        "|Online Distributed Proofreading Team",
        r"|pgdp\.net",
        "|Project Gutenberg",
    ))
    .expect("valid BOILERPLATE_RE")
});

// A line that is mostly bare numbers is a table of contents, an index or a
// page-number column, not prose.
pub static NUMERIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\d.,;:—–-]+$").expect("valid NUMERIC_RE"));

static FULLPATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"full-path="([^"]+)""#).expect("valid FULLPATH_RE"));
static ITEM_ID_HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<item\b[^>]*?id="([^"]+)"[^>]*?href="([^"]+)""#).expect("valid item id/href re")
});
static ITEM_HREF_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<item\b[^>]*?href="([^"]+)"[^>]*?id="([^"]+)""#).expect("valid item href/id re")
});
static ITEMREF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<itemref\b[^>]*?idref="([^"]+)""#).expect("valid itemref re"));
static XHTML_EXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.x?html?$").expect("valid xhtml ext re"));

static SCRIPT_STYLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<(script|style)\b.*?</\1>").expect("valid script/style re"));
static BODY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<body[^>]*>(.*)</body>").expect("valid body re"));
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<(p|h[1-6]|li|blockquote)\b[^>]*>(.*?)</\1>").expect("valid tag re")
});
static BR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid br re"));
static ANY_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid any tag re"));
static WS_NBSP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\s\u{a0}]+").expect("valid ws+nbsp re"));
static YEAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(1[5-9]\d{2}|20\d{2})").expect("valid year re"));

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

fn read_zip_text(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, ExtractError> {
    use std::io::Read;
    let mut f = archive
        .by_name(name)
        .map_err(|e| ExtractError::input_invalid(format!("EPUB has no {name}: {e}")))?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)
        .map_err(|e| ExtractError::input_invalid(format!("cannot read {name} from EPUB: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

/// Mimics `os.path.normpath(os.path.join(base, href))` for the posix-style
/// forward-slash paths a zip archive always uses, regardless of host OS.
fn posix_join_normpath(base: &str, href: &str) -> String {
    let combined = if base.is_empty() {
        href.to_string()
    } else {
        format!("{base}/{href}")
    };
    normpath_posix(&combined)
}

fn normpath_posix(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let is_abs = p.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for comp in p.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(&last) if last != "..") {
                    parts.pop();
                } else if !is_abs {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if is_abs {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Content documents in reading order, resolved through the OPF spine.
///
/// `pub(crate)`: reused by `crate::propose`, which needs the same reading
/// order and the raw OPF XML to gather metadata *candidates* (dc:language,
/// dc:source, dc:publisher, …) beyond the four fields [`opf_metadata`]
/// resolves for the deterministic extractor.
pub(crate) fn spine_docs(
    archive: &mut zip::ZipArchive<File>,
) -> Result<(Vec<String>, String, String), ExtractError> {
    let container = read_zip_text(archive, "META-INF/container.xml")?;
    let m = FULLPATH_RE
        .captures(&container)
        .ok()
        .flatten()
        .ok_or_else(|| {
            ExtractError::input_invalid("EPUB META-INF/container.xml has no rootfile full-path")
        })?;
    let opf_path = m.get(1).unwrap().as_str().to_string();
    let opf = read_zip_text(archive, &opf_path)?;
    let base = dirname(&opf_path);

    let mut ids: HashMap<String, String> = HashMap::new();
    for cap in ITEM_ID_HREF_RE.captures_iter(&opf) {
        let cap = cap.expect("regex match");
        ids.insert(cap[1].to_string(), cap[2].to_string());
    }
    for cap in ITEM_HREF_ID_RE.captures_iter(&opf) {
        let cap = cap.expect("regex match");
        ids.entry(cap[2].to_string())
            .or_insert_with(|| cap[1].to_string());
    }

    let names: HashSet<String> = archive.file_names().map(str::to_string).collect();
    let mut out = Vec::new();
    for cap in ITEMREF_RE.captures_iter(&opf) {
        let cap = cap.expect("regex match");
        let idref = &cap[1];
        let href = match ids.get(idref) {
            Some(h) => h,
            None => continue,
        };
        let href = html_escape::decode_html_entities(href);
        let href = href.split('#').next().unwrap_or("");
        let path = if base.is_empty() {
            href.to_string()
        } else {
            posix_join_normpath(&base, href)
        };
        if names.contains(&path) && XHTML_EXT_RE.is_match(&path).unwrap_or(false) {
            out.push(path);
        }
    }
    Ok((out, opf_path, opf))
}

#[derive(Debug, Default, Clone)]
struct OpfMeta {
    title: Option<String>,
    creator: Option<String>,
    date: Option<String>,
    rights: Option<String>,
}

/// `dc:title`/`dc:creator`/`dc:date`/`dc:rights`, resolved by XML namespace
/// URI (not literal prefix) like `ElementTree`'s `.find(".//dc:tag", ns)`.
/// Any parse error returns an empty [`OpfMeta`], matching Python's
/// `except ET.ParseError: return {}`.
fn opf_metadata(opf_xml: &str) -> OpfMeta {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
    let wanted = ["title", "creator", "date", "rights"];

    let mut reader = Reader::from_str(opf_xml);
    let mut ns_map: HashMap<String, String> = HashMap::new();
    let mut result = OpfMeta::default();
    let mut capture: Option<&'static str> = None;
    let mut depth_at_capture = 0i32;
    let mut depth = 0i32;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    if let Some(prefix) = key.strip_prefix("xmlns:") {
                        if let Ok(val) = attr.unescape_value() {
                            ns_map.insert(prefix.to_string(), val.into_owned());
                        }
                    }
                }
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                depth += 1;
                if let Some((prefix, local)) = name.split_once(':') {
                    if ns_map.get(prefix).map(String::as_str) == Some(DC_NS)
                        && wanted.contains(&local)
                    {
                        let slot: Option<&'static str> = match local {
                            "title" if result.title.is_none() => Some("title"),
                            "creator" if result.creator.is_none() => Some("creator"),
                            "date" if result.date.is_none() => Some("date"),
                            "rights" if result.rights.is_none() => Some("rights"),
                            _ => None,
                        };
                        if slot.is_some() {
                            capture = slot;
                            depth_at_capture = depth;
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(key) = capture {
                    let txt = t.unescape().unwrap_or_default().trim().to_string();
                    if !txt.is_empty() {
                        match key {
                            "title" => result.title.get_or_insert(txt),
                            "creator" => result.creator.get_or_insert(txt),
                            "date" => result.date.get_or_insert(txt),
                            "rights" => result.rights.get_or_insert(txt),
                            _ => unreachable!(),
                        };
                    }
                }
            }
            Ok(Event::End(_)) => {
                if capture.is_some() && depth == depth_at_capture {
                    capture = None;
                }
                depth -= 1;
            }
            Err(_) => return OpfMeta::default(),
            _ => {}
        }
    }
    result
}

/// The paragraph-like text blocks (`p`/`h1..h6`/`li`/`blockquote`) of one
/// spine document, in document order.
///
/// `pub(crate)`: `crate::propose` scans the first spine document(s) this
/// way for title-page evidence (a "BY <author>" byline, an imprint year) —
/// the same HTML→text pass the extractor uses, so a paragraph a human would
/// read as `blocks[0]` here is not a scrambled duplicate of what `extract`
/// would have produced for it.
pub(crate) fn blocks_of(
    archive: &mut zip::ZipArchive<File>,
    path: &str,
) -> Result<Vec<String>, ExtractError> {
    let raw = read_zip_text(archive, path)?;
    let raw = regex_replace_all(&SCRIPT_STYLE_RE, &raw, " ");
    let body = match BODY_RE.captures(&raw).ok().flatten() {
        Some(cap) => cap.get(1).unwrap().as_str().to_string(),
        None => raw,
    };
    let mut out = Vec::new();
    for cap in TAG_RE.captures_iter(&body) {
        let cap = cap.expect("regex match");
        let inner = cap.get(2).unwrap().as_str();
        let inner = regex_replace_all(&BR_RE, inner, " ");
        let stripped = regex_replace_all(&ANY_TAG_RE, &inner, "");
        let unescaped = html_escape::decode_html_entities(&stripped).into_owned();
        let text = regex_replace_all(&WS_NBSP_RE, &unescaped, " ")
            .trim()
            .to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    Ok(out)
}

fn year_from_date(date_str: Option<&str>) -> Option<i64> {
    let date_str = date_str?;
    let cap = YEAR_RE.captures(date_str).ok().flatten()?;
    cap.get(1)?.as_str().parse::<i64>().ok()
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
    let file = File::open(path)
        .map_err(|e| ExtractError::input_invalid(format!("cannot open {}: {e}", path.display())))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        ExtractError::input_invalid(format!("{}: not a valid EPUB/zip: {e}", path.display()))
    })?;
    let (docs, _opf_path, opf_xml) = spine_docs(&mut archive)?;
    let opf_meta = opf_metadata(&opf_xml);
    progress.stage_end(Stage::ReadContainer);

    progress.stage_start(Stage::ParseSpineItems, Some(docs.len() as u64));
    let mut raw_blocks: Vec<(i64, String)> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        for b in blocks_of(&mut archive, d)? {
            raw_blocks.push(((i + 1) as i64, b));
        }
        progress.stage_progress(
            Stage::ParseSpineItems,
            (i + 1) as u64,
            Some(docs.len() as u64),
        );
    }
    progress.stage_end(Stage::ParseSpineItems);

    // Inline citation-code detection: the Jones/Waggoner-style printed
    // reference scheme, present in >=40% of blocks. Ties (equal hit
    // counts) resolve to whichever code was seen first, matching Python's
    // `Counter.most_common(1)` (a stable sort keeps insertion order for
    // equal counts).
    let mut hit_order: Vec<String> = Vec::new();
    let mut hits: HashMap<String, i64> = HashMap::new();
    for (_, b) in &raw_blocks {
        if let Some(cap) = REF_RE.captures(b).ok().flatten() {
            let code = cap.get(1).unwrap().as_str().to_string();
            if !hits.contains_key(&code) {
                hit_order.push(code.clone());
            }
            *hits.entry(code).or_insert(0) += 1;
        }
    }
    let mut inline_code: Option<String> = None;
    let mut inline_n: i64 = 0;
    for code in &hit_order {
        let n = hits[code];
        if inline_code.is_none() || n > inline_n {
            inline_code = Some(code.clone());
            inline_n = n;
        }
    }
    let coded =
        inline_code.is_some() && (inline_n as f64) >= 0.4 * (raw_blocks.len().max(1) as f64);

    let resolved_code =
        opts.book_code
            .clone()
            .or_else(|| if coded { inline_code.clone() } else { None });
    let resolved_title = opts.title.clone().or_else(|| opf_meta.title.clone());
    let resolved_author = opts.author.clone().or_else(|| opf_meta.creator.clone());
    let resolved_year = opts
        .year
        .or_else(|| year_from_date(opf_meta.date.as_deref()));
    let resolved_rights = opts.rights.clone().or_else(|| opf_meta.rights.clone());

    let mut out_blocks: Vec<Block> = Vec::new();
    let mut dropped_detail: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashMap<String, i64> = HashMap::new();
    let mut page: i64 = 0;
    let mut para_in_page: i64 = 0;
    let mut saw_page_marker = false;
    let mut blocks_in: i64 = 0;

    progress.stage_start(Stage::Gate, Some(raw_blocks.len() as u64));
    for (doc_no, block_text) in &raw_blocks {
        if let Some(cap) = PAGEMARK_RE.captures(block_text.as_str()).ok().flatten() {
            page = cap.get(1).unwrap().as_str().parse::<i64>().unwrap_or(0);
            para_in_page = 0;
            saw_page_marker = true;
            continue;
        }
        blocks_in += 1;

        let ref_cap = REF_RE.captures(block_text.as_str()).ok().flatten();
        let (text, key_page, key_para);
        if coded
            && ref_cap
                .as_ref()
                .map(|c| c.get(1).unwrap().as_str() == inline_code.as_deref().unwrap_or(""))
                .unwrap_or(false)
        {
            let cap = ref_cap.unwrap();
            let m0 = cap.get(0).unwrap();
            text = block_text[..m0.start()].trim().to_string();
            key_page = cap.get(2).unwrap().as_str().parse::<i64>().unwrap_or(0);
            key_para = cap.get(3).unwrap().as_str().parse::<i64>().unwrap_or(0);
        } else {
            text = block_text.clone();
            if saw_page_marker {
                para_in_page += 1;
                key_page = page;
                key_para = para_in_page;
            } else {
                if *doc_no != page {
                    page = *doc_no;
                    para_in_page = 0;
                }
                para_in_page += 1;
                key_page = *doc_no;
                key_para = para_in_page;
            }
        }

        let para_key = format!("{key_page}.{key_para}");

        if BOILERPLATE_RE.is_match(&text).unwrap_or(false) {
            dropped_detail.push(dropped_entry(
                &para_key,
                "boilerplate",
                Some(&char_truncate(&text, 80)),
            ));
            continue;
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        if tokens.len() > 20 {
            let numeric_hits = tokens
                .iter()
                .filter(|t| NUMERIC_RE.is_match(t).unwrap_or(false))
                .count();
            if (numeric_hits as f64) > 0.35 * (tokens.len() as f64) {
                dropped_detail.push(dropped_entry(
                    &para_key,
                    "numeric/TOC line",
                    Some(&char_truncate(&text, 80)),
                ));
                continue;
            }
        }

        if let Some(reason) = chunk::quality_gate(&text, &opts.chunk_limits) {
            dropped_detail.push(dropped_entry(
                &para_key,
                &reason,
                Some(&char_truncate(&text, 80)),
            ));
            continue;
        }

        let pieces = chunk::split_long(&text, &opts.chunk_limits);
        // base_seq is non-zero when an earlier paragraph already claimed this
        // para_key: in a book with an inline citation scheme, a heading keyed
        // by chapter ordinal can land on a cited paragraph's key. seq keeps
        // their point ids distinct; chunk stays paragraph-local.
        let base_seq = *seen.get(&para_key).unwrap_or(&0);
        for (j, piece) in pieces.iter().enumerate() {
            out_blocks.push(Block::new(
                para_key.clone(),
                key_page,
                key_para,
                base_seq + j as i64,
                pieces.len() as i64,
                piece.clone(),
                piece.split_whitespace().count() as i64,
                Some(j as i64),
            ));
        }
        *seen.entry(para_key).or_insert(0) += pieces.len() as i64;
    }
    progress.stage_end(Stage::Gate);

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

    let book_pair = opts.book_pair.clone().or_else(|| resolved_code.clone());
    let page_kind = if coded || saw_page_marker {
        "print"
    } else {
        "chapter"
    };
    let book_meta = build_book_meta(
        resolved_code.as_deref(),
        Some(opts.lang.as_deref().unwrap_or("en")),
        book_pair.as_deref(),
        resolved_title.as_deref(),
        resolved_author.as_deref(),
        resolved_year,
        opts.slug.as_deref(),
        opts.corpus.as_deref(),
        Some(page_kind),
    );

    let sha256 = sha256_file(path)?;
    Ok(Book {
        schema: SCHEMA_BOOK.to_string(),
        profile: "sop".to_string(),
        source: build_source(
            &path.display().to_string(),
            Some(&sha256),
            "epub",
            opts.acquired_from.as_deref(),
            resolved_rights.as_deref(),
        ),
        book: book_meta,
        id_rule: "sop/seq".to_string(),
        alignment: None,
        stats,
        blocks: out_blocks,
    })
}
