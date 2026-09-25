//! Model cache directory resolution (SOPACK-1.0-PLAN.md §3.1 / §5):
//! `$SOPACK_CACHE` override, else macOS `~/Library/Caches/sopack`, else
//! Linux `$XDG_CACHE_HOME/sopack` or `~/.cache/sopack`. Layout inside:
//! `<cache>/models/<repo with "/" replaced by "--">/<revision>/<files>`.
//!
//! `--model-dir` (a full override of the resolved path, not just the cache
//! root) is a CLI concern, not this module's — `sopack-cli` picks between
//! `--model-dir` and `model_dir_in_cache(default_cache_dir(), ...)` itself.

use crate::spec::EmbedSpec;
use std::env;
use std::path::{Path, PathBuf};

/// The cache root sopack uses when no `--model-dir` override is given.
pub fn default_cache_dir() -> PathBuf {
    if let Some(p) = non_empty_env("SOPACK_CACHE") {
        return PathBuf::from(p);
    }
    let home = home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library").join("Caches").join("sopack")
    } else if let Some(xdg) = non_empty_env("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("sopack")
    } else {
        home.join(".cache").join("sopack")
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `<cache>/models/<repo with "/" replaced by "--">/<revision>/` — where a
/// fetched or imported model's files live. `"qdrant/multilingual-e5-large-onnx"`
/// becomes `qdrant--multilingual-e5-large-onnx`, echoing the
/// `models--<org>--<name>` shape of the Hugging Face cache already on this
/// machine, so the two are recognisable side by side without colliding.
pub fn model_dir_in_cache(cache_dir: &Path, spec: &EmbedSpec) -> PathBuf {
    cache_dir
        .join("models")
        .join(spec.repo_dir_name())
        .join(&spec.revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ModelFile, Pooling};
    use std::sync::Mutex;

    // std::env is process-global; serialise every test that touches it so
    // parallel `cargo test` threads don't race each other's set_var/remove_var.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn spec() -> EmbedSpec {
        EmbedSpec {
            model_id: "m".into(),
            repo: "qdrant/multilingual-e5-large-onnx".into(),
            revision: "66076b8".into(),
            onnx_file: "model.onnx".into(),
            output_name: "last_hidden_state".into(),
            files: vec![ModelFile {
                name: "model.onnx".into(),
                sha256: "x".into(),
                bytes: 1,
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
    fn sopack_cache_env_wins_over_everything() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("SOPACK_CACHE", "/tmp/my-sopack-cache");
        }
        let got = default_cache_dir();
        unsafe {
            env::remove_var("SOPACK_CACHE");
        }
        assert_eq!(got, PathBuf::from("/tmp/my-sopack-cache"));
    }

    #[test]
    fn empty_sopack_cache_env_is_ignored() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("SOPACK_CACHE", "");
            env::remove_var("XDG_CACHE_HOME");
        }
        let got = default_cache_dir();
        unsafe {
            env::remove_var("SOPACK_CACHE");
        }
        assert_ne!(got, PathBuf::from(""));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn xdg_cache_home_used_on_linux() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            env::remove_var("SOPACK_CACHE");
            env::set_var("XDG_CACHE_HOME", "/tmp/xdg");
        }
        let got = default_cache_dir();
        unsafe {
            env::remove_var("XDG_CACHE_HOME");
        }
        assert_eq!(got, PathBuf::from("/tmp/xdg/sopack"));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn falls_back_to_dot_cache_on_linux() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            env::remove_var("SOPACK_CACHE");
            env::remove_var("XDG_CACHE_HOME");
            env::set_var("HOME", "/home/tester");
        }
        let got = default_cache_dir();
        assert_eq!(got, PathBuf::from("/home/tester/.cache/sopack"));
    }

    #[test]
    fn model_dir_in_cache_mangles_repo_and_keeps_revision() {
        let dir = model_dir_in_cache(Path::new("/cache"), &spec());
        assert_eq!(
            dir,
            PathBuf::from("/cache/models/qdrant--multilingual-e5-large-onnx/66076b8")
        );
    }
}
