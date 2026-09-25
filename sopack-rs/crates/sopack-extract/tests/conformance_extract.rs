//! Conformance gate (`docs/SOPACK-1.0-PLAN.md` §4): every `book.json`
//! [`sopack_extract::extract`] produces for
//! `sopack-rs/conformance/extract/manifest.json`'s fixtures and real books
//! must be byte-identical to the golden the Python reference wrote via
//! `sopack-rs/conformance/extract/make_extract_goldens.py`. See
//! `sopack-rs/conformance/README.md` for any documented intended
//! difference (there should be none).
//!
//! Real-book entries are skipped (not failed) when the referenced file is
//! not present locally — `pd-books/converted/*.epub` sources are
//! deliberately not copied into `conformance/`.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sopack_book::dump;
use sopack_extract::{extract, ExtractOptions, Kind, NoopProgress};

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    name: String,
    kind: String,
    source: String,
    #[serde(default)]
    options: std::collections::HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    fixtures: Vec<ManifestEntry>,
    real_books: Vec<ManifestEntry>,
}

fn qdrant_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../qdrant/sopack-rs/crates/sopack-extract
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("qdrant/ root should exist three levels above the crate")
}

fn opt_str(o: &std::collections::HashMap<String, Value>, key: &str) -> Option<String> {
    o.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn opt_i64(o: &std::collections::HashMap<String, Value>, key: &str) -> Option<i64> {
    o.get(key).and_then(|v| v.as_i64())
}

fn build_options(o: &std::collections::HashMap<String, Value>) -> ExtractOptions {
    ExtractOptions {
        book_code: opt_str(o, "book_code"),
        lang: opt_str(o, "lang"),
        title: opt_str(o, "title"),
        author: opt_str(o, "author"),
        year: opt_i64(o, "year"),
        corpus: opt_str(o, "corpus"),
        slug: opt_str(o, "slug"),
        book_pair: opt_str(o, "book_pair"),
        acquired_from: opt_str(o, "acquired_from"),
        rights: opt_str(o, "rights"),
        chunk_limits: Default::default(),
    }
}

/// Runs every manifest entry sequentially in a single test function (rather
/// than one `#[test]` per entry) because it needs the process's current
/// directory set to `qdrant/` — the manifest's `source` paths, and the
/// `book.json` `source.file` value both sides must agree on byte for byte,
/// are written relative to that root. `set_current_dir` is process-wide, so
/// sharing it across parallel `#[test]` threads in the same binary would
/// race; one test avoids that.
#[test]
fn conformance_book_json_matches_python_goldens() {
    let root = qdrant_root();
    let manifest_path = root.join("sopack-rs/conformance/extract/manifest.json");
    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display()));
    let manifest: Manifest = serde_json::from_str(&manifest_raw).expect("valid manifest.json");

    let original_dir = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(&root).expect("chdir to qdrant/");

    let mut checked = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for entry in manifest.fixtures.iter().chain(manifest.real_books.iter()) {
        let source_path = Path::new(&entry.source);
        if !source_path.exists() {
            eprintln!("skip {}: {} not found locally", entry.name, entry.source);
            skipped += 1;
            continue;
        }
        let kind: Kind = entry
            .kind
            .parse()
            .unwrap_or_else(|_| panic!("unknown kind {:?} for {}", entry.kind, entry.name));
        let options = build_options(&entry.options);

        let book = match extract(source_path, kind, &options, &mut NoopProgress) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{}: extract failed: {e}", entry.name));
                continue;
            }
        };

        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        dump(&book, tmp.path()).expect("dump book.json");
        let got = std::fs::read_to_string(tmp.path()).expect("read dumped book.json");

        let golden_path = root
            .join("sopack-rs/conformance/extract/goldens")
            .join(format!("{}.book.json", entry.name));
        let want = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("cannot read golden {}: {e}", golden_path.display()));

        checked += 1;
        if got != want {
            let first_diff = got
                .lines()
                .zip(want.lines())
                .enumerate()
                .find(|(_, (g, w))| g != w)
                .map(|(i, (g, w))| format!("first differing line {i}:\n  got:  {g}\n  want: {w}"))
                .unwrap_or_else(|| {
                    format!(
                        "lengths differ: got {} bytes, want {} bytes",
                        got.len(),
                        want.len()
                    )
                });
            failures.push(format!(
                "{}: book.json does not match golden {}\n{first_diff}",
                entry.name,
                golden_path.display()
            ));
        }
    }

    std::env::set_current_dir(&original_dir).expect("restore cwd");

    eprintln!("conformance: {checked} checked, {skipped} skipped (real-book fixture not present)");
    assert!(checked > 0, "no conformance entries were actually checked");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
