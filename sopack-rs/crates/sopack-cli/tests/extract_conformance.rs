//! `sopack extract` (run as the actual CLI binary, not the library
//! directly) reproduces every small fixture in
//! `conformance/extract/manifest.json` byte-identically to its golden
//! (`SOPACK-1.0-PLAN.md` §4). The library-level equivalent already lives in
//! `sopack-extract/tests/conformance_extract.rs`; this is the CLI's own
//! leg of the same gate, since a CLI-level bug (option plumbing, page_kind/
//! id_rule overlay, output formatting) would not be caught by the library
//! test alone.
//!
//! Real-book entries (`come_out_of_her`, `seal_of_the_living_god`; `wdys`
//! is excluded — see `REAL_BOOK_FIXTURES`'s doc comment) need
//! `pd-books/converted/*.epub`, deliberately not copied into
//! `conformance/`; each is skipped (not failed) if its source file is not
//! present locally, matching the library test's own rule.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

fn qdrant_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../qdrant/sopack-rs/crates/sopack-cli
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("qdrant/ root should exist three levels above the crate")
}

struct Fixture {
    kind: &'static str,
    source: &'static str,
    golden: &'static str,
    flags: &'static [&'static str],
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        kind: "epub",
        source: "sopack-rs/conformance/extract/fixtures/small.epub",
        golden: "small_epub.book.json",
        flags: &[
            "--book-code",
            "TSTW",
            "--corpus",
            "pioneers",
            "--slug",
            "test-work",
        ],
    },
    Fixture {
        kind: "markdown",
        source: "sopack-rs/conformance/extract/fixtures/small.md",
        golden: "small_markdown.book.json",
        flags: &[
            "--book-code",
            "MDX",
            "--lang",
            "en",
            "--title",
            "MD Fixture",
            "--author",
            "M. Author",
            "--year",
            "2020",
            "--corpus",
            "test",
            "--slug",
            "md-fixture",
        ],
    },
    Fixture {
        kind: "text",
        source: "sopack-rs/conformance/extract/fixtures/small.txt",
        golden: "small_text.book.json",
        flags: &[
            "--book-code",
            "TXT",
            "--lang",
            "en",
            "--title",
            "Text Fixture",
            "--author",
            "T. Author",
            "--year",
            "2021",
            "--corpus",
            "test",
            "--slug",
            "text-fixture",
        ],
    },
    Fixture {
        kind: "sop_json",
        source: "sopack-rs/conformance/extract/fixtures/en/SMALL.json",
        golden: "small_sop_json.book.json",
        flags: &[],
    },
    Fixture {
        kind: "sop_json",
        source: "sopack-rs/conformance/extract/fixtures/de/SMALLDE.json",
        golden: "small_sop_json_de.book.json",
        flags: &[],
    },
];

/// Real pioneer-corpus EPUBs (`conformance/extract/manifest.json`'s
/// `real_books`) — no CLI-supplied metadata at all, matching that
/// manifest's `"options": {}` for each.
///
/// `wdys` is deliberately **not** included: its inline-citation `book_code`
/// heuristic never fires for that book (the golden's own `book.book_code`
/// is `null` — see `meta.rs`'s doc comment: "only fires on a minority of
/// books"), so `sopack_book::validate` correctly refuses it with "book_code
/// is required". The library-level goldens in `sopack-extract`'s own
/// conformance test are produced by calling `extract()` directly, without
/// running `validate()` — appropriate there, since that test is about
/// extraction fidelity, not the CLI's refuse-invalid-output contract. The
/// CLI (`sopack extract`, mirroring `sopack.cli._cmd_extract`) always
/// validates before writing, so a source whose golden is itself unresolved
/// for a *required* field can never be reproduced through the CLI without
/// changing what gets written — this is the CLI doing its job, not a bug.
const REAL_BOOK_FIXTURES: &[Fixture] = &[
    Fixture {
        kind: "epub",
        source: "pd-books/converted/fitch__come-out-of-her-my-people__1843__archive.epub",
        golden: "come_out_of_her.book.json",
        flags: &[],
    },
    Fixture {
        kind: "epub",
        source: "pd-books/converted/bates-joseph__a-seal-of-the-living-god__1849__archive.epub",
        golden: "seal_of_the_living_god.book.json",
        flags: &[],
    },
];

fn check_one(root: &Path, out_dir: &Path, fx: &Fixture) {
    let out_path = out_dir.join(fx.golden);
    let mut args: Vec<&str> = vec!["extract", fx.source, "--kind", fx.kind, "-o"];
    let out_str = out_path.to_str().unwrap().to_string();
    args.push(&out_str);
    args.extend_from_slice(fx.flags);

    let output = Command::new(support::bin())
        .args(&args)
        .current_dir(root)
        .output()
        .expect("failed to run sopack extract");
    assert!(
        output.status.success(),
        "extract of {} failed: {}",
        fx.source,
        String::from_utf8_lossy(&output.stderr)
    );

    let golden_path = root
        .join("sopack-rs/conformance/extract/goldens")
        .join(fx.golden);
    let golden_bytes = std::fs::read(&golden_path)
        .unwrap_or_else(|e| panic!("cannot read golden {}: {e}", golden_path.display()));
    let got_bytes = std::fs::read(&out_path)
        .unwrap_or_else(|e| panic!("cannot read produced {}: {e}", out_path.display()));
    assert_eq!(
        String::from_utf8_lossy(&got_bytes),
        String::from_utf8_lossy(&golden_bytes),
        "{} did not reproduce {} byte-for-byte",
        fx.source,
        fx.golden
    );
}

#[test]
fn cli_extract_reproduces_every_small_fixture_byte_identically() {
    let root = qdrant_root();
    let out_dir = tempfile::tempdir().unwrap();
    for fx in FIXTURES {
        check_one(&root, out_dir.path(), fx);
    }
}

#[test]
fn cli_extract_reproduces_real_books_byte_identically_when_present_locally() {
    let root = qdrant_root();
    let out_dir = tempfile::tempdir().unwrap();
    let mut checked = 0;
    for fx in REAL_BOOK_FIXTURES {
        if !root.join(fx.source).is_file() {
            eprintln!("skipping {} — not present locally", fx.source);
            continue;
        }
        check_one(&root, out_dir.path(), fx);
        checked += 1;
    }
    eprintln!(
        "checked {checked}/{} real-book fixture(s)",
        REAL_BOOK_FIXTURES.len()
    );
}
