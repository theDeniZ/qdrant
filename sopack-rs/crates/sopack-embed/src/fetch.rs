//! Model download (SOPACK-1.0-PLAN.md §3.2 "Model download is the one
//! network action"): spawns `curl` per file into a `.part` with resume
//! (`-C -`), verifies sha256+size, then renames atomically. Deliberately
//! has no HTTP client crate in its dependency tree — CI checks `cargo tree`
//! for `sopack-cli` to enforce that (§3.2's "no HTTP client dependency at
//! all"), and shelling out to `curl` is how this module honours it while
//! still resolving.

use crate::error::{EmbedError, Result};
use crate::spec::EmbedSpec;
use sopack_progress::{NullSink, ProgressSink, Unit};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// True if a `curl` binary answers on `PATH`.
pub fn curl_available() -> bool {
    Command::new("curl")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn download_url(spec: &EmbedSpec, file: &str) -> String {
    format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        spec.repo, spec.revision, file
    )
}

/// The exact argument list `fetch_model` runs `curl` with for one file —
/// split out so it is unit-testable without a network (SOPACK-1.0-PLAN.md
/// §3.1: "curl command construction" is one of the model-free tests).
/// `-f` fails on an HTTP error status instead of writing the error page as
/// if it were the file; `-C -` resumes a partial `.part`; `-L` follows the
/// redirect Hugging Face's `resolve` URLs issue to the CDN.
pub fn curl_args(spec: &EmbedSpec, file: &str, dest_part: &Path) -> Vec<String> {
    vec![
        "-fL".to_string(),
        "-C".to_string(),
        "-".to_string(),
        "-sS".to_string(),
        "-o".to_string(),
        dest_part.display().to_string(),
        download_url(spec, file),
    ]
}

/// Downloads every file `spec` names into `dest_dir`, verifies each one,
/// and renames `<file>.part` -> `<file>` atomically. Never called from the
/// `pack` path itself — only from an explicit `model fetch` command — so
/// that "no network calls during a pack" (§3.2) holds regardless of what
/// `sopack-cli` wires this function to.
pub fn fetch_model(spec: &EmbedSpec, dest_dir: &Path, progress: &dyn ProgressSink) -> Result<()> {
    if !curl_available() {
        return Err(EmbedError::CurlNotFound);
    }
    fs::create_dir_all(dest_dir)?;

    let total = spec.total_bytes();
    progress.stage_start("download", total, Unit::Bytes);
    let result = fetch_files(spec, dest_dir, progress);
    progress.stage_end();
    result
}

fn already_fetched(final_path: &Path, want_bytes: u64) -> bool {
    fs::metadata(final_path)
        .map(|m| m.len() == want_bytes)
        .unwrap_or(false)
}

fn fetch_files(spec: &EmbedSpec, dest_dir: &Path, progress: &dyn ProgressSink) -> Result<()> {
    // Credit bytes already on disk from an earlier, interrupted run before
    // starting any new curl process, so resuming doesn't visually restart
    // the bar from zero.
    for f in &spec.files {
        let final_path = dest_dir.join(&f.name);
        if already_fetched(&final_path, f.bytes) {
            progress.advance(f.bytes);
            continue;
        }
        let part = dest_dir.join(format!("{}.part", f.name));
        let existing = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if existing > 0 {
            progress.advance(existing.min(f.bytes));
        }
    }

    for f in &spec.files {
        let final_path = dest_dir.join(&f.name);
        if already_fetched(&final_path, f.bytes) {
            continue;
        }
        fetch_one(spec, f, dest_dir, progress)?;
    }
    Ok(())
}

fn fetch_one(
    spec: &EmbedSpec,
    f: &crate::spec::ModelFile,
    dest_dir: &Path,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let part = dest_dir.join(format!("{}.part", f.name));
    let final_path = dest_dir.join(&f.name);
    let mut sent = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let args = curl_args(spec, &f.name, &part);
    let mut child = Command::new("curl")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let now = fs::metadata(&part).map(|m| m.len()).unwrap_or(sent);
        if now > sent {
            progress.advance(now - sent);
            sent = now;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let now = fs::metadata(&part).map(|m| m.len()).unwrap_or(sent);
    if now > sent {
        progress.advance(now - sent);
    }

    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut stderr);
        }
        return Err(EmbedError::DownloadFailed {
            url: download_url(spec, &f.name),
            status: status.code().unwrap_or(-1),
            detail: stderr.trim().to_string(),
        });
    }

    let got_size = fs::metadata(&part)?.len();
    if got_size != f.bytes {
        return Err(EmbedError::SizeMismatch {
            path: part,
            file: f.name.clone(),
            expected: f.bytes,
            found: got_size,
        });
    }
    let got_hash = crate::verify::sha256_file(&part, &NullSink)?;
    if got_hash != f.sha256 {
        return Err(EmbedError::HashMismatch {
            path: part,
            file: f.name.clone(),
            expected: f.sha256.clone(),
            got: got_hash,
        });
    }
    fs::rename(&part, &final_path)?;
    Ok(())
}

/// Copies (hardlinking when possible) an already-downloaded model snapshot
/// — e.g. an existing fastembed/Hugging Face cache dir — into `dest_dir`
/// after verifying every file's sha256, for `model import <dir>`
/// (SOPACK-1.0-PLAN.md §3.1: "Also accept an existing fastembed/HF cache
/// snapshot dir"). Fails on the first file whose content doesn't match the
/// contract, before touching `dest_dir`, so a bad import never leaves a
/// half-populated cache entry behind.
pub fn import_model(
    spec: &EmbedSpec,
    src_dir: &Path,
    dest_dir: &Path,
    progress: &dyn ProgressSink,
) -> Result<()> {
    crate::verify::verify_model(src_dir, spec, progress)?;
    fs::create_dir_all(dest_dir)?;
    progress.stage_start("import", spec.total_bytes(), Unit::Bytes);
    for f in &spec.files {
        let src = src_dir.join(&f.name);
        let dst = dest_dir.join(&f.name);
        if fs::hard_link(&src, &dst).is_err() {
            fs::copy(&src, &dst)?;
        }
        progress.advance(f.bytes);
    }
    progress.stage_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ModelFile, Pooling};
    use sopack_progress::NullSink;
    use std::path::PathBuf;

    fn spec() -> EmbedSpec {
        EmbedSpec {
            model_id: "m".into(),
            repo: "qdrant/multilingual-e5-large-onnx".into(),
            revision: "abc123".into(),
            onnx_file: "model.onnx".into(),
            output_name: "last_hidden_state".into(),
            files: vec![ModelFile {
                name: "model.onnx".into(),
                sha256: "deadbeef".into(),
                bytes: 42,
            }],
            pooling: Pooling::Mean,
            normalize: true,
            dim: 1024,
            max_tokens: 512,
            passage_prefix: "passage: ".into(),
            query_prefix: "query: ".into(),
        }
    }

    #[test]
    fn download_url_matches_the_huggingface_resolve_pattern() {
        let url = download_url(&spec(), "model.onnx");
        assert_eq!(
            url,
            "https://huggingface.co/qdrant/multilingual-e5-large-onnx/resolve/abc123/model.onnx"
        );
    }

    #[test]
    fn curl_args_resume_and_write_to_the_part_path() {
        let part = PathBuf::from("/tmp/model.onnx.part");
        let args = curl_args(&spec(), "model.onnx", &part);
        assert!(args.contains(&"-fL".to_string()));
        assert!(args
            .windows(2)
            .any(|w| w == ["-C".to_string(), "-".to_string()]));
        let o_idx = args
            .iter()
            .position(|a| a == "-o")
            .expect("-o flag present");
        assert_eq!(args[o_idx + 1], part.display().to_string());
        assert_eq!(*args.last().unwrap(), download_url(&spec(), "model.onnx"));
    }

    #[test]
    fn curl_available_does_not_panic() {
        let _ = curl_available();
    }

    #[test]
    fn import_model_hardlinks_or_copies_and_verifies_first() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let content = b"fake onnx bytes";
        std::fs::write(src.path().join("model.onnx"), content).unwrap();
        let sha = crate::verify::sha256_file(&src.path().join("model.onnx"), &NullSink).unwrap();
        let mut s = spec();
        s.files[0].bytes = content.len() as u64;
        s.files[0].sha256 = sha;

        import_model(&s, src.path(), dst.path(), &NullSink).unwrap();
        let copied = std::fs::read(dst.path().join("model.onnx")).unwrap();
        assert_eq!(copied, content);
    }

    #[test]
    fn import_model_refuses_a_snapshot_that_fails_verification() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("model.onnx"), b"wrong content").unwrap();
        let err = import_model(&spec(), src.path(), dst.path(), &NullSink).unwrap_err();
        assert!(matches!(
            err,
            EmbedError::HashMismatch { .. } | EmbedError::SizeMismatch { .. }
        ));
        assert!(
            !dst.path().join("model.onnx").exists(),
            "a failed import must not leave a partial file behind"
        );
    }
}
