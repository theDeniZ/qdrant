//! `sopack extract` / `inspect` on an OSIS-keyed Bible (`bible_json`):
//! auto-detection, the `bible` profile, metadata gates and exit codes.

mod support;

use support::{assert_valid, run};

fn bible(dir: &std::path::Path, meta: Option<&str>) -> std::path::PathBuf {
    let src = dir.join("cep.json");
    std::fs::write(
        &src,
        r#"{"Gen.1.2": "Země byla pustá", "Gen.1.1": "Na počátku stvořil Bůh nebe a zemi."}"#,
    )
    .unwrap();
    if let Some(m) = meta {
        std::fs::write(dir.join("cep.json.meta.toml"), m).unwrap();
    }
    src
}

#[test]
fn json_bible_is_detected_and_extracted_as_profile_bible() {
    let dir = tempfile::tempdir().unwrap();
    let src = bible(
        dir.path(),
        Some("lang = \"cs\"\ntitle = \"Český ekumenický překlad\"\n"),
    );
    let out = dir.path().join("cep.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("extract", &v);
    assert_eq!(v["book_code"], "cep");
    assert_eq!(v["id_rule"], "bible/v1");

    let book: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(book["profile"], "bible");
    assert_eq!(book["source"]["kind"], "bible_json");
    assert_eq!(book["blocks"][0]["para_key"], "Gen.1.1");

    let r = run(&["inspect", out.to_str().unwrap(), "--json"]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("inspect", &v);
    assert_eq!(v["bible"]["verses"], 2);
    assert_eq!(v["bible"]["missing_books"].as_array().unwrap().len(), 65);
}

#[test]
fn bible_without_title_is_exit_4_naming_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let src = bible(dir.path(), None);
    let out = dir.path().join("cep.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--lang",
        "cs",
        "--json",
    ]);
    assert_eq!(r.status, 4, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("error", &v);
    assert_eq!(v["error"]["field"], "title");
}

#[test]
fn bible_with_sop_metadata_is_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let src = bible(dir.path(), Some("lang = \"cs\"\ntitle = \"CEP\"\n"));
    let out = dir.path().join("cep.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--author",
        "x",
        "--json",
    ]);
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
}

#[test]
fn bible_with_page_kind_overlay_is_exit_3() {
    let dir = tempfile::tempdir().unwrap();
    let src = bible(dir.path(), Some("lang = \"cs\"\ntitle = \"CEP\"\n"));
    let out = dir.path().join("cep.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--page-kind",
        "print",
        "--json",
    ]);
    assert_eq!(r.status, 3, "stderr: {}", r.stderr);
    assert!(
        r.stdout
            .contains("page_kind is not used by the bible profile"),
        "{}",
        r.stdout
    );
}

#[test]
fn explicit_kind_bible_json_rejects_a_sop_json_file_with_exit_3() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("SC.json");
    std::fs::write(
        &src,
        r#"{"meta": {"en_code": "SC"}, "en": {"1.1": {"text": "x"}}}"#,
    )
    .unwrap();
    let out = dir.path().join("SC.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--kind",
        "bible_json",
        "--lang",
        "en",
        "--title",
        "x",
        "--json",
    ]);
    assert_eq!(r.status, 3, "stderr: {}", r.stderr);
}
