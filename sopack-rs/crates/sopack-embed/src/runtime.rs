//! ONNX Runtime dylib resolution (SOPACK-1.0-PLAN.md §3.1 "ONNX Runtime:
//! `ort` with `load-dynamic`"). Search order: `ORT_DYLIB_PATH`;
//! `<exe_dir>/../lib/`; `<exe_dir>/`; then a clear error naming everywhere
//! that was tried. The release ships Microsoft's official ONNX Runtime
//! shared library next to the binary (M0 measured exactly 1.30.0); this
//! module reports the version actually loaded for manifest provenance
//! (SOPACK-2-FORMAT.md §2 `embedding.runtime`).

use crate::error::{EmbedError, Result};
use libloading::{Library, Symbol};
use std::ffi::CStr;
use std::path::{Path, PathBuf};

/// Search order for the ONNX Runtime shared library. Returns the resolved
/// path, or `EmbedError::OrtDylibNotFound` naming every place it looked.
pub fn resolve_ort_dylib() -> Result<PathBuf> {
    resolve_ort_dylib_from(
        std::env::var("ORT_DYLIB_PATH").ok(),
        std::env::current_exe().ok(),
    )
}

/// The same search, with the environment variable and executable path
/// injected — split out so the search order itself is unit-testable
/// without needing to fork a process or mutate `std::env` (SOPACK-1.0-PLAN.md
/// §3.1: dylib resolution order is part of "produce a clear error message
/// naming what was searched").
fn resolve_ort_dylib_from(
    env_path: Option<String>,
    current_exe: Option<PathBuf>,
) -> Result<PathBuf> {
    let mut searched = Vec::new();

    if let Some(p) = env_path {
        let path = PathBuf::from(&p);
        if path.is_file() {
            return Ok(path);
        }
        searched.push(format!("$ORT_DYLIB_PATH={p} (not a file)"));
    } else {
        searched.push("$ORT_DYLIB_PATH (unset)".to_string());
    }

    let Some(exe) = current_exe else {
        searched.push("<exe_dir> (could not resolve the current executable path)".to_string());
        return Err(EmbedError::OrtDylibNotFound { searched });
    };
    // Canonicalise first: Homebrew (and install.sh) put a SYMLINK to the
    // binary on PATH, and on macOS `current_exe()` returns that symlink's
    // path, not the real file in the Cellar. `../lib` must be taken relative
    // to the real binary, where the bundled ONNX Runtime actually lives.
    let exe = exe.canonicalize().unwrap_or(exe);
    let exe_dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let lib_dir = exe_dir.join("..").join("lib");

    for dir in [lib_dir.as_path(), exe_dir.as_path()] {
        match find_ort_lib(dir) {
            Some(found) => return Ok(found),
            None => searched.push(dir.display().to_string()),
        }
    }
    Err(EmbedError::OrtDylibNotFound { searched })
}

fn lib_file_matches(name: &str) -> bool {
    if cfg!(target_os = "macos") {
        // `libonnxruntime.dylib` / `libonnxruntime.1.30.0.dylib`, never a
        // provider plugin such as `libonnxruntime_providers_shared.dylib`.
        name.starts_with("libonnxruntime.") && name.ends_with(".dylib")
    } else {
        name.starts_with("libonnxruntime.so")
    }
}

fn find_ort_lib(dir: &Path) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(lib_file_matches)
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    entries.into_iter().next()
}

/// Reads `OrtGetApiBase().GetVersionString()` straight from the dylib at
/// `path`, independent of `ort`'s own (private) loader. This is the only
/// way to recover the exact ONNX Runtime version string for manifest
/// provenance: `ort::sys::OrtApi` (what `ort::api()` exposes once an
/// environment is committed) does not carry `GetVersionString` — only the
/// smaller `OrtApiBase`, returned by `OrtGetApiBase`, does.
///
/// # Safety
/// Loads and calls into an arbitrary shared library found on disk. Callers
/// pass a path from `resolve_ort_dylib`, which only returns paths that
/// matched the expected ONNX Runtime library filename pattern.
pub fn ort_dylib_version(path: &Path) -> Result<String> {
    unsafe {
        let lib = Library::new(path).map_err(|e| EmbedError::OrtDylibLoad {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let get_base: Symbol<unsafe extern "C" fn() -> *const ort::sys::OrtApiBase> = lib
            .get(b"OrtGetApiBase")
            .map_err(|e| EmbedError::OrtDylibLoad {
                path: path.to_path_buf(),
                reason: e.to_string(),
            })?;
        let base = get_base();
        if base.is_null() {
            return Err(EmbedError::OrtDylibLoad {
                path: path.to_path_buf(),
                reason: "OrtGetApiBase returned null".into(),
            });
        }
        let get_version = (*base).GetVersionString;
        let raw = get_version();
        if raw.is_null() {
            return Err(EmbedError::OrtDylibLoad {
                path: path.to_path_buf(),
                reason: "GetVersionString returned null".into(),
            });
        }
        Ok(CStr::from_ptr(raw).to_string_lossy().into_owned())
    }
}

/// What one process's `ort` initialisation resolved to.
pub struct OrtInit {
    pub dylib_path: PathBuf,
    pub version: String,
}

/// Resolves + loads the ONNX Runtime dylib and commits the global `ort`
/// environment against it. Idempotent across repeated calls in one process
/// (`EnvironmentBuilder::commit` only takes effect the first time — see
/// `ort::environment`'s docs — so building several `Engine`s, e.g. across
/// the ignored thread-scaling/batch-budget model tests, is safe); execution
/// providers are registered per-`Session` instead (`device::execution_providers`
/// passed to `SessionBuilder`), not here, so different `Engine`s in the same
/// process can use different devices despite the environment itself only
/// being configurable once.
pub fn init() -> Result<OrtInit> {
    let path = resolve_ort_dylib()?;
    let version = ort_dylib_version(&path)?;
    ort::init_from(&path)
        .map_err(|e| EmbedError::OrtInit(e.to_string()))?
        .commit();
    Ok(OrtInit {
        dylib_path: path,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub filename the CURRENT platform's `lib_file_matches` accepts:
    /// the search only picks up `.dylib`s on macOS and `.so`s elsewhere, so
    /// a test staging the wrong suffix would (correctly) find nothing.
    fn ort_lib_name(versioned: bool) -> &'static str {
        match (cfg!(target_os = "macos"), versioned) {
            (true, true) => "libonnxruntime.1.30.0.dylib",
            (true, false) => "libonnxruntime.dylib",
            (false, true) => "libonnxruntime.so.1.30.0",
            (false, false) => "libonnxruntime.so",
        }
    }

    #[test]
    fn lib_file_pattern_accepts_the_platform_names_only() {
        assert!(lib_file_matches(ort_lib_name(true)));
        assert!(lib_file_matches(ort_lib_name(false)));
        assert!(!lib_file_matches("libonnxruntime_providers_shared.dylib"));
        assert!(!lib_file_matches("libonnxruntime_providers_shared.so"));
    }

    #[test]
    fn env_var_wins_when_it_points_at_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("libonnxruntime.so.1.30.0");
        std::fs::write(&lib, b"not a real library, just needs to exist").unwrap();
        let found = resolve_ort_dylib_from(Some(lib.display().to_string()), None).unwrap();
        assert_eq!(found, lib);
    }

    #[test]
    fn falls_back_to_exe_dir_lib_subdir_when_env_var_is_unset() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        let lib_dir = root.path().join("lib");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let lib = lib_dir.join(ort_lib_name(false));
        std::fs::write(&lib, b"stub").unwrap();
        let exe = exe_dir.join("sopack");
        std::fs::write(&exe, b"stub").unwrap();

        let found = resolve_ort_dylib_from(None, Some(exe)).unwrap();
        // Not `== lib`: the resolved path goes through `<exe_dir>/../lib`
        // unnormalized (matching the plan's literal search order), so
        // compare canonicalized paths instead of the exact string.
        assert_eq!(found.canonicalize().unwrap(), lib.canonicalize().unwrap());
    }

    #[test]
    fn falls_back_to_exe_dir_itself() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let lib = exe_dir.join(ort_lib_name(true));
        std::fs::write(&lib, b"stub").unwrap();
        let exe = exe_dir.join("sopack");
        std::fs::write(&exe, b"stub").unwrap();

        let found = resolve_ort_dylib_from(None, Some(exe)).unwrap();
        // Canonical comparison: macOS temp dirs live under the /var ->
        // /private/var symlink, and the resolver canonicalises the exe path.
        assert_eq!(found.canonicalize().unwrap(), lib.canonicalize().unwrap());
    }

    /// Homebrew and install.sh put a symlink to the binary on PATH; `../lib`
    /// must resolve next to the REAL binary, not next to the symlink.
    #[cfg(unix)]
    #[test]
    fn follows_a_symlinked_exe_to_the_real_lib_dir() {
        let root = tempfile::tempdir().unwrap();
        let keg = root.path().join("Cellar/sopack/1.0.0");
        std::fs::create_dir_all(keg.join("bin")).unwrap();
        std::fs::create_dir_all(keg.join("lib")).unwrap();
        let lib = keg.join("lib").join(ort_lib_name(true));
        std::fs::write(&lib, b"stub").unwrap();
        let real_exe = keg.join("bin/sopack");
        std::fs::write(&real_exe, b"stub").unwrap();
        // The prefix bin/ has the symlink, and the prefix lib/ has NO ORT.
        std::fs::create_dir_all(root.path().join("bin")).unwrap();
        std::fs::create_dir_all(root.path().join("lib")).unwrap();
        let link = root.path().join("bin/sopack");
        std::os::unix::fs::symlink(&real_exe, &link).unwrap();

        let found = resolve_ort_dylib_from(None, Some(link)).unwrap();
        assert_eq!(found.canonicalize().unwrap(), lib.canonicalize().unwrap());
    }

    #[test]
    fn reports_every_place_it_looked_when_nothing_is_found() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("sopack");
        std::fs::write(&exe, b"stub").unwrap();

        let err = resolve_ort_dylib_from(
            Some("/nonexistent/path/libonnxruntime.so".into()),
            Some(exe),
        )
        .unwrap_err();
        let EmbedError::OrtDylibNotFound { searched } = err else {
            panic!("expected OrtDylibNotFound, got {err:?}")
        };
        assert_eq!(
            searched.len(),
            3,
            "should report the env var attempt plus both directories: {searched:?}"
        );
        assert!(searched[0].contains("ORT_DYLIB_PATH"));
    }

    #[test]
    fn a_file_that_does_not_match_the_platform_pattern_is_not_picked_up() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("not-onnxruntime.txt"), b"stub").unwrap();
        let exe = exe_dir.join("sopack");
        std::fs::write(&exe, b"stub").unwrap();

        let err = resolve_ort_dylib_from(None, Some(exe)).unwrap_err();
        assert!(matches!(err, EmbedError::OrtDylibNotFound { .. }));
    }
}
