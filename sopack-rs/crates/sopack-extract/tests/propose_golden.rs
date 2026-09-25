//! Golden test: `propose` against real EPUBs from the 2026-09 pioneer
//! acquisition (`pd-books/converted/`), cross-checked against the batch's
//! own manifest, `_results_pioneers2026.json` (M5 exit test shape,
//! `SOPACK-1.0-PLAN.md` §3.5).
//!
//! Skips entirely if `pd-books/` is not present in this checkout (it is a
//! large, separately-acquired directory, not guaranteed to exist in every
//! clone) — same "skip if absent" the task brief asks for. Three books:
//! `ATNW` (`waggoner-jh__the-atonement-…`, the plan's own M5 example),
//! `CIS` (`haskell-sn__the-cross-and-its-shadow…`) and `SOGO`
//! (`waggoner-jh__the-spirit-of-god…`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sopack_extract::{propose_quiet, Kind, Registry};

struct ManifestEntry {
    title: String,
    author: String,
    year: i64,
}

fn pd_books_converted_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pd-books/converted")
}

fn load_manifest(dir: &Path) -> Option<HashMap<String, ManifestEntry>> {
    let path = dir.join("_results_pioneers2026.json");
    let text = std::fs::read_to_string(path).ok()?;
    let arr: Vec<Value> = serde_json::from_str(&text).ok()?;
    let mut out = HashMap::new();
    for entry in arr {
        let code = entry["book_code"].as_str()?.to_string();
        let title = entry["title"].as_str()?.to_string();
        let author = entry["author"].as_str()?.to_string();
        let year: i64 = entry["year"].as_str()?.parse().ok()?;
        out.insert(
            code,
            ManifestEntry {
                title,
                author,
                year,
            },
        );
    }
    Some(out)
}

/// (book_code, filename) for the three books this golden checks.
const TARGETS: [(&str, &str); 3] = [
    (
        "ATNW",
        "waggoner-jh__the-atonement-examination-remedial-system__1884__archive.epub",
    ),
    (
        "CIS",
        "haskell-sn__the-cross-and-its-shadow__1914__archive.epub",
    ),
    ("SOGO", "waggoner-jh__the-spirit-of-god__1877__archive.epub"),
];

#[test]
fn propose_candidates_contain_manifest_metadata_for_real_pioneer_epubs() {
    let dir = pd_books_converted_dir();
    let Some(manifest) = load_manifest(&dir) else {
        eprintln!("SKIP: pd-books/converted/_results_pioneers2026.json not found — pd-books/ not in this checkout");
        return;
    };

    let mut ran_any = false;
    let mut report: Vec<String> = Vec::new();

    for (code, filename) in TARGETS {
        let path = dir.join(filename);
        if !path.is_file() {
            eprintln!("SKIP {code}: {filename} not found under pd-books/converted/");
            continue;
        }
        let entry = manifest
            .get(code)
            .unwrap_or_else(|| panic!("{code} missing from _results_pioneers2026.json"));
        ran_any = true;

        let proposal = propose_quiet(&path, Some(Kind::Epub), None)
            .expect("propose must succeed on a real EPUB");

        // ---- title: candidates must contain the manifest's exact title.
        let title_values: Vec<&str> = proposal
            .fields
            .title
            .candidates
            .iter()
            .filter_map(|c| c.value.as_str())
            .collect();
        assert!(
            title_values.contains(&entry.title.as_str()),
            "{code}: manifest title {:?} not among title candidates {title_values:?}",
            entry.title
        );
        // nothing false: if a value was resolved, it must be the correct title.
        if !proposal.fields.title.value.is_null() {
            assert_eq!(
                proposal.fields.title.value,
                Value::from(entry.title.as_str())
            );
        }

        // ---- author: candidates must contain the manifest's exact author form.
        let author_values: Vec<&str> = proposal
            .fields
            .author
            .candidates
            .iter()
            .filter_map(|c| c.value.as_str())
            .collect();
        assert!(
            author_values.contains(&entry.author.as_str()),
            "{code}: manifest author {:?} not among author candidates {author_values:?}",
            entry.author
        );
        if !proposal.fields.author.value.is_null() {
            assert_eq!(
                proposal.fields.author.value,
                Value::from(entry.author.as_str())
            );
        }

        // ---- year: candidates must contain the manifest's exact year.
        let year_values: Vec<i64> = proposal
            .fields
            .year
            .candidates
            .iter()
            .filter_map(|c| c.value.as_i64())
            .collect();
        assert!(
            year_values.contains(&entry.year),
            "{code}: manifest year {} not among year candidates {year_values:?}",
            entry.year
        );
        if !proposal.fields.year.value.is_null() {
            assert_eq!(proposal.fields.year.value, Value::from(entry.year));
        }
        // No year-trap false alarm: the manifest year itself must carry no warning.
        for c in &proposal.fields.year.candidates {
            if c.value.as_i64() == Some(entry.year) {
                assert!(
                    c.warning.is_none(),
                    "{code}: manifest year {} incorrectly flagged: {:?}",
                    entry.year,
                    c.warning
                );
            }
        }

        // ---- book_code: candidates only ever *offered*, never asserted as
        // the historical code (see propose.rs's module doc comment — these
        // are hand-picked mnemonics, not a mechanical function of the
        // title). The one invariant that must hold: value is never set to
        // anything but the real code, and in practice — since no
        // title-page source exists for these front-matter-stripped
        // conversions — it stays null.
        if !proposal.fields.book_code.value.is_null() {
            assert_eq!(proposal.fields.book_code.value, Value::from(code));
        }

        // ---- corpus: author is not Ellen G. White, so "pioneers" must be offered.
        assert!(
            proposal
                .fields
                .corpus
                .candidates
                .iter()
                .any(|c| c.value.as_str() == Some("pioneers")),
            "{code}: expected a 'pioneers' corpus candidate for author {:?}",
            entry.author
        );

        let resolved: Vec<&str> = proposal
            .fields
            .iter()
            .filter(|(_, f)| !f.value.is_null())
            .map(|(name, _)| name)
            .collect();
        let unresolved: Vec<&str> = proposal.unresolved.iter().map(String::as_str).collect();
        report.push(format!(
            "{code} ({filename}): resolved={resolved:?} unresolved={unresolved:?} warnings={:?}",
            proposal.warnings
        ));
    }

    if !ran_any {
        eprintln!("SKIP: none of the 3 target EPUBs were found under pd-books/converted/");
        return;
    }

    eprintln!("\n--- propose golden report (resolved vs unresolved per book) ---");
    for line in &report {
        eprintln!("{line}");
    }
}

fn registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/e5-large-v1/book_codes.json")
}

/// Golden re-import test against the *real, just-regenerated* registry
/// (`contracts/e5-large-v1/book_codes.json`) — the exact incident
/// `Registry::find_by_title` exists to catch. The conversion manifest
/// (`_results_pioneers2026.json`) proposes `TATS` as this book's code, but
/// the live store already holds the identical work as `BP3`; before this
/// fix, `propose` only checked its own acronym candidate for a collision
/// (`TATS` isn't a registry row) and reported none.
///
/// Skips entirely if either the registry or `pd-books/` is absent from
/// this checkout — same convention as the test above.
#[test]
fn propose_finds_the_real_bp3_reimport_for_the_bates_sanctuary_epub() {
    let reg_path = registry_path();
    let Ok(registry) = Registry::load(&reg_path) else {
        eprintln!(
            "SKIP: registry not found/loadable at {} — contracts/ not in this checkout",
            reg_path.display()
        );
        return;
    };

    let path = pd_books_converted_dir().join(
        "bates-joseph__explanation-of-the-typical-and-anti-typical-sanctuary__1850__archive.epub",
    );
    if !path.is_file() {
        eprintln!(
            "SKIP: {} not found under pd-books/converted/",
            path.display()
        );
        return;
    }

    let proposal = propose_quiet(&path, Some(Kind::Epub), Some(&registry))
        .expect("propose must succeed on a real EPUB");

    let hit = proposal
        .fields
        .book_code
        .candidates
        .iter()
        .find(|c| c.from == "registry:title_match")
        .unwrap_or_else(|| {
            panic!(
                "expected a registry:title_match book_code candidate; got {:?}",
                proposal.fields.book_code.candidates
            )
        });
    assert_eq!(hit.value, Value::from("BP3"));
    assert_eq!(hit.collision, Some(true));
    // This is a full-title match (the OPF dc:title is byte-identical to
    // the registry row's), so no "short-title match only" warning.
    assert!(hit.warning.is_none(), "{:?}", hit.warning);

    // book_pair mirrors book_code, and must carry the same collision flag
    // — it is exactly as true of book_pair, which defaults to book_code.
    let pair_hit = proposal
        .fields
        .book_pair
        .candidates
        .iter()
        .find(|c| c.value == "BP3")
        .unwrap_or_else(|| {
            panic!(
                "expected a BP3 book_pair candidate; got {:?}",
                proposal.fields.book_pair.candidates
            )
        });
    assert_eq!(pair_hit.collision, Some(true));

    // ...and a proposal-level warning, not just a per-candidate one.
    assert!(
        proposal
            .warnings
            .iter()
            .any(|w| w.contains("BP3") && w.contains("RE-IMPORT")),
        "expected a re-import warning mentioning BP3; got {:?}",
        proposal.warnings
    );

    // The manifest's own proposed code for this book is TATS, never
    // BP3's collision partner — book_code must never silently commit to
    // BP3 either (no title-page-sourced code candidate agrees with it).
    assert_ne!(proposal.fields.book_code.value, Value::from("BP3"));
}

/// The negative case: an EPUB whose title is genuinely *not* yet in the
/// registry must not manufacture a `registry:title_match` candidate.
///
/// `jones__what-is-the-church__1913__archive.epub` (dc:title "What is the
/// Church?") was picked by scanning every `pd-books/converted/*.epub`'s
/// `dc:title` against the committed registry (both full- and
/// short-title-normalised) and confirming zero rows match — unlike, say,
/// `ATNW`'s or `SOGO`'s own titles, which (though neither code itself is a
/// registry row) turned out to already be in the store under other codes
/// (`AERS`, `SGOM`) and so are re-imports too, not useful negatives here.
#[test]
fn propose_reports_no_registry_title_match_for_a_book_not_yet_in_the_store() {
    let reg_path = registry_path();
    let Ok(registry) = Registry::load(&reg_path) else {
        eprintln!(
            "SKIP: registry not found/loadable at {} — contracts/ not in this checkout",
            reg_path.display()
        );
        return;
    };

    let path = pd_books_converted_dir().join("jones__what-is-the-church__1913__archive.epub");
    if !path.is_file() {
        eprintln!(
            "SKIP: {} not found under pd-books/converted/",
            path.display()
        );
        return;
    }

    let proposal = propose_quiet(&path, Some(Kind::Epub), Some(&registry))
        .expect("propose must succeed on a real EPUB");
    assert!(
        !proposal
            .fields
            .book_code
            .candidates
            .iter()
            .any(|c| c.from == "registry:title_match"),
        "unexpected registry:title_match candidate(s): {:?}",
        proposal.fields.book_code.candidates
    );
    assert!(
        !proposal.warnings.iter().any(|w| w.contains("RE-IMPORT")),
        "unexpected re-import warning: {:?}",
        proposal.warnings
    );
}
