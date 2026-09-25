//! `--progress json` emits NDJSON on stderr that is monotonic and ends at
//! 100 (`SOPACK-1.0-PLAN.md` §3.4), for every command that needs no
//! model/ORT.

mod support;

use support::run;

fn assert_monotonic_ndjson(stderr: &str) {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "expected at least one NDJSON progress line"
    );
    let mut last_pct = 0.0f64;
    let mut saw_done = false;
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("not valid NDJSON: {line:?}: {e}"));
        assert_eq!(v["v"], 1);
        assert!(v["event"].is_string());
        if let Some(pct) = v.get("pct").and_then(|p| p.as_f64()) {
            assert!(
                pct + 1e-9 >= last_pct,
                "pct went backwards: {pct} < {last_pct} in {line}"
            );
            last_pct = pct;
        }
        if v["event"] == "done" {
            saw_done = true;
            assert_eq!(v["pct"], 100.0, "done event must report pct 100: {line}");
        }
    }
    assert!(saw_done, "expected a final done event");
    assert!(
        (last_pct - 100.0).abs() < 1e-6,
        "progress must end at 100, ended at {last_pct}"
    );
}

#[test]
fn doctor_quick_progress_json_is_monotonic_and_ends_at_100() {
    // With --json, stdout carries only the result document; every NDJSON
    // progress line must be on stderr instead (SOPACK-1.0-PLAN.md §3.5).
    let r = run(&["doctor", "--quick", "--progress", "json", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&r.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be exactly one JSON document: {e}\n{}",
            r.stdout
        )
    });
    assert_monotonic_ndjson(&r.stderr);
}

#[test]
fn contract_show_progress_json_still_ends_clean() {
    // contract show has no long-running stage, but --progress json must
    // still not corrupt stdout even when nothing is reported.
    let r = run(&["contract", "show", "--progress", "json", "--json"]);
    assert_eq!(r.status, 0);
    let _: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
}

#[test]
fn inspect_accepts_progress_json_flag_without_corrupting_output() {
    let dir = tempfile::tempdir().unwrap();
    let book_path = dir.path().join("t.book.json");
    std::fs::write(
        &book_path,
        r#"{"schema":"sopack.book/1","profile":"sop","source":{},"book":{"lang":"en","book_code":"T"},"id_rule":"sop/seq","stats":{},"blocks":[{"para_key":"1.1","page":1,"para":1,"seq":0,"chunks":1,"text":"hello world","words":2}]}"#,
    )
    .unwrap();
    // `inspect` itself declares no stage plan (it's instantaneous), so this
    // mainly proves --progress json never corrupts a command that does
    // nothing with it — the deeper "extract"/"pack" cases are covered by
    // extract_conformance.rs and the model-gated pack tests.
    let r = run(&["inspect", book_path.to_str().unwrap(), "--progress", "json"]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
}

#[test]
fn extract_progress_json_is_monotonic_and_ends_at_100() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("t.md");
    std::fs::write(
        &src,
        "# A Title\n\nBY AN AUTHOR\n\nSome body text that is long enough to chunk cleanly.\n",
    )
    .unwrap();
    let out = dir.path().join("t.book.json");
    let r = run(&[
        "extract",
        src.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--book-code",
        "T",
        "--lang",
        "en",
        "--title",
        "A Title",
        "--progress",
        "json",
    ]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_monotonic_ndjson(&r.stderr);
}
