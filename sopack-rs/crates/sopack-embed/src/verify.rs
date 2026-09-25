//! `verify_model` (SOPACK-1.0-PLAN.md §3.1): every file `EmbedSpec` names
//! must be present with the right size and sha256, streamed with progress
//! in bytes since a whole-model verify walks ~2.2 GB (`model.onnx_data`).

use crate::error::{EmbedError, Result};
use crate::spec::EmbedSpec;
use sha2::{Digest, Sha256};
use sopack_progress::{ProgressSink, Unit};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const CHUNK: usize = 1 << 20; // 1 MiB streaming read

/// Checks every file `spec` names is present in `dir` with the right size
/// and sha256. Reports byte progress on `progress` as `"verify"`.
pub fn verify_model(dir: &Path, spec: &EmbedSpec, progress: &dyn ProgressSink) -> Result<()> {
    progress.stage_start("verify", spec.total_bytes(), Unit::Bytes);
    let result = verify_files(dir, spec, progress);
    progress.stage_end();
    result
}

fn verify_files(dir: &Path, spec: &EmbedSpec, progress: &dyn ProgressSink) -> Result<()> {
    for f in &spec.files {
        let path = dir.join(&f.name);
        let meta = std::fs::metadata(&path).map_err(|_| EmbedError::MissingFile {
            dir: dir.to_path_buf(),
            file: f.name.clone(),
        })?;
        if meta.len() != f.bytes {
            return Err(EmbedError::SizeMismatch {
                path,
                file: f.name.clone(),
                expected: f.bytes,
                found: meta.len(),
            });
        }
        let got = sha256_file(&path, progress)?;
        if got != f.sha256 {
            return Err(EmbedError::HashMismatch {
                path,
                file: f.name.clone(),
                expected: f.sha256.clone(),
                got,
            });
        }
    }
    Ok(())
}

/// Streaming sha256 of one file, advancing `progress` (bytes) per chunk
/// read. Used both by `verify_model` and, with a `NullSink`, wherever a
/// caller just wants the digest (`calibration::CalibrationFixture::sha256_of_file`).
pub fn sha256_file(path: &Path, progress: &dyn ProgressSink) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        progress.advance(n as u64);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ModelFile, Pooling};
    use sopack_progress::NullSink;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &[u8]) -> ModelFile {
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        let sha256 = sha256_file(&path, &NullSink).unwrap();
        ModelFile {
            name: name.to_string(),
            sha256,
            bytes: content.len() as u64,
        }
    }

    fn spec_with(files: Vec<ModelFile>) -> EmbedSpec {
        EmbedSpec {
            model_id: "m".into(),
            repo: "r/m".into(),
            revision: "rev".into(),
            onnx_file: "model.onnx".into(),
            output_name: "last_hidden_state".into(),
            files,
            pooling: Pooling::Mean,
            normalize: true,
            dim: 4,
            max_tokens: 512,
            passage_prefix: "passage: ".into(),
            query_prefix: "query: ".into(),
        }
    }

    #[test]
    fn sha256_file_matches_a_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello world").unwrap();
        // The well-known sha256 of the literal bytes "hello world".
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(sha256_file(&path, &NullSink).unwrap(), expected);
    }

    #[test]
    fn sha256_file_streams_across_chunk_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let content = vec![7u8; CHUNK * 2 + 12345]; // spans multiple 1 MiB reads
        std::fs::write(&path, &content).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let expected = hex::encode(hasher.finalize());
        assert_eq!(sha256_file(&path, &NullSink).unwrap(), expected);
    }

    #[test]
    fn verify_model_passes_for_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = write_file(dir.path(), "model.onnx", b"fake onnx bytes");
        let spec = spec_with(vec![f]);
        verify_model(dir.path(), &spec, &NullSink).unwrap();
    }

    #[test]
    fn verify_model_reports_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_with(vec![ModelFile {
            name: "model.onnx".into(),
            sha256: "x".into(),
            bytes: 1,
        }]);
        let err = verify_model(dir.path(), &spec, &NullSink).unwrap_err();
        assert!(matches!(err, EmbedError::MissingFile { .. }));
    }

    #[test]
    fn verify_model_reports_size_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut real = write_file(dir.path(), "model.onnx", b"12345");
        real.bytes = 999; // contract disagrees with what's on disk
        let spec = spec_with(vec![real]);
        let err = verify_model(dir.path(), &spec, &NullSink).unwrap_err();
        assert!(matches!(
            err,
            EmbedError::SizeMismatch {
                expected: 999,
                found: 5,
                ..
            }
        ));
    }

    #[test]
    fn verify_model_reports_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut real = write_file(dir.path(), "model.onnx", b"12345");
        real.sha256 = "0".repeat(64); // right size, wrong content
        let spec = spec_with(vec![real]);
        let err = verify_model(dir.path(), &spec, &NullSink).unwrap_err();
        assert!(matches!(err, EmbedError::HashMismatch { .. }));
    }

    #[test]
    fn verify_model_reports_byte_progress() {
        struct Counter(std::sync::atomic::AtomicU64);
        impl ProgressSink for Counter {
            fn stage_start(&self, _: &str, _: u64, _: Unit) {}
            fn advance(&self, n: u64) {
                self.0.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
            }
            fn warn(&self, _: &str) {}
            fn stage_end(&self) {}
            fn done(&self) {}
        }
        let dir = tempfile::tempdir().unwrap();
        let f = write_file(dir.path(), "model.onnx", &vec![1u8; 5000]);
        let spec = spec_with(vec![f]);
        let counter = Counter(std::sync::atomic::AtomicU64::new(0));
        verify_model(dir.path(), &spec, &counter).unwrap();
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 5000);
    }
}
