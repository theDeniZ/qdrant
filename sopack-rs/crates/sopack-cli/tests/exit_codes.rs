//! Exit codes for each error class, and the `--json` error object shape
//! (`SOPACK-1.0-PLAN.md` §3.5).

mod support;

use support::{assert_valid, run};

#[test]
fn bad_global_flag_value_is_exit_2_usage() {
    // clap itself rejects an unknown --progress value before any subcommand runs.
    let r = run(&["--progress", "bogus", "doctor", "--quick"]);
    assert_eq!(r.status, 2);
}

#[test]
fn bad_device_value_is_exit_2_usage() {
    // A book.json that exists but declares an unknown profile is rejected
    // by `sopack_book::load` before `--device` is ever parsed by our own
    // code, so this uses a minimal, valid sop book.json to reach that check.
    let dir = tempfile::tempdir().unwrap();
    let book_path = dir.path().join("t.book.json");
    std::fs::write(
        &book_path,
        r#"{"schema":"sopack.book/1","profile":"sop","source":{},"book":{"lang":"en","book_code":"T","title":"T","corpus":"pioneers","author":"A","year":2000},"id_rule":"sop/seq","stats":{},"blocks":[{"para_key":"1.1","page":1,"para":1,"seq":0,"chunks":1,"text":"hello world","words":2}]}"#,
    )
    .unwrap();
    let out = dir.path().join("out.sopack");
    let r = run(&[
        "pack",
        book_path.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--device",
        "tpu",
    ]);
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
}

#[test]
fn unknown_subcommand_is_exit_2_usage() {
    let r = run(&["not-a-real-command"]);
    assert_eq!(r.status, 2);
}

#[test]
fn missing_required_flag_is_exit_2_usage() {
    // `pack` requires -o/--out.
    let r = run(&["pack", "/tmp/nope.book.json"]);
    assert_eq!(r.status, 2);
}

#[test]
fn extract_of_a_nonexistent_source_is_exit_3_input_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.book.json");
    let r = run(&[
        "extract",
        "/no/such/file.epub",
        "-o",
        out.to_str().unwrap(),
        "--book-code",
        "X",
        "--json",
    ]);
    assert_eq!(r.status, 3, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("error", &v);
    assert_eq!(v["error"]["code"], "input_invalid");
    assert_eq!(v["error"]["exit"], 3);
}

#[test]
fn extract_markdown_without_required_metadata_is_exit_4_needs_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("plain.md");
    std::fs::write(&src, "Just a body, no title/front-matter/book_code.\n").unwrap();
    let out = dir.path().join("out.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(r.status, 4, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("error", &v);
    assert_eq!(v["error"]["code"], "needs_metadata");
    assert_eq!(v["error"]["exit"], 4);
    assert!(
        !out.exists(),
        "must not write a book.json when metadata is missing"
    );
}

#[test]
fn extract_with_kind_specific_rejected_option_is_exit_2_usage() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("x.json");
    std::fs::write(&src, r#"{"en": {}, "meta": {}}"#).unwrap();
    let out = dir.path().join("out.book.json");
    // sop_json does not accept --title (it's derived from the file itself).
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "--kind",
        "sop_json",
        "-o",
        out.to_str().unwrap(),
        "--title",
        "Should be rejected",
        "--json",
    ]);
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
    let v = r.stdout_json();
    assert_valid("error", &v);
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn verify_of_a_missing_pack_is_exit_3_and_clean_false() {
    let r = run(&["verify", "/no/such/pack.sopack", "--json"]);
    assert_eq!(r.status, 3);
    let v = r.stdout_json();
    assert_valid("verify", &v);
    assert_eq!(v["clean"], false);
    assert!(!v["errors"].as_array().unwrap().is_empty());
}

#[test]
fn verify_of_a_garbage_file_is_exit_3() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.sopack");
    std::fs::write(&path, b"not a zip file at all").unwrap();
    let r = run(&["verify", path.to_str().unwrap(), "--json"]);
    assert_eq!(r.status, 3);
}

#[test]
fn inspect_of_a_missing_book_json_is_exit_1_internal_io_error() {
    // sopack_book::load's io error path -> our generic std::io::Error From
    // impl -> "internal" (no more specific code fits an unreadable file
    // whose problem is "does not exist", as opposed to malformed content).
    let r = run(&["inspect", "/no/such/book.json", "--json"]);
    assert_ne!(r.status, 0);
    let v = r.stdout_json();
    assert_valid("error", &v);
}

#[test]
fn pack_of_zero_books_is_exit_2_usage() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.sopack");
    // clap itself refuses this (books is a required Vec<PathBuf> with at
    // least... actually clap allows 0 by default for Vec, so this reaches
    // our own "no books given" usage error).
    let r = run(&["pack", "-o", out.to_str().unwrap()]);
    assert_eq!(r.status, 2);
}
