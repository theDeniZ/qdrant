//! Resolves the offline book-code registry (`contracts/<id>/book_codes.json`,
//! `SOPACK-1.0-PLAN.md` §3.2) `propose`/`extract` check collisions against.
//! Best-effort: a registry that cannot be found or loaded degrades to
//! [`sopack_extract::Registry::empty`] (every candidate then reports
//! `collision: null`, "not checked" — see `propose.rs`'s doc comment) rather
//! than failing the command, since collision checking is a convenience, not
//! a correctness requirement.

use std::path::{Path, PathBuf};

use sopack_extract::Registry;

/// Search order for `book_codes.json` when `--registry` was not given:
/// 1. `--contract <dir>` was itself a directory -> `<dir>/book_codes.json`.
/// 2. Next to the running binary, matching the release tarball's layout
///    (`bin/sopack`, `contracts/<id>/book_codes.json` — see
///    `.github/workflows/release-sopack.yml`'s "Collect artifacts" step):
///    `<exe_dir>/../contracts/<id>/book_codes.json` and
///    `<exe_dir>/contracts/<id>/book_codes.json`.
/// 3. The workspace-relative path this crate was built from (devcontainer /
///    `cargo run` convenience, not meaningful in an installed binary).
fn candidate_paths(contract_id: &str, contract_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = contract_dir {
        out.push(dir.join("book_codes.json"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            out.push(
                exe_dir
                    .join("..")
                    .join("contracts")
                    .join(contract_id)
                    .join("book_codes.json"),
            );
            out.push(
                exe_dir
                    .join("contracts")
                    .join(contract_id)
                    .join("book_codes.json"),
            );
        }
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts")
            .join(contract_id)
            .join("book_codes.json"),
    );
    out
}

/// Loads the registry for *contract_id* (and, if `--contract` named a
/// directory, *contract_dir*). *override_path* (`--registry`) is tried
/// first and, unlike the best-effort search, a bad `--registry` path IS an
/// error — the operator asked for that file specifically.
pub fn load_registry(
    contract_id: &str,
    contract_dir: Option<&Path>,
    override_path: Option<&Path>,
) -> Result<Registry, crate::exit::CliError> {
    if let Some(p) = override_path {
        return Registry::load(p).map_err(crate::exit::CliError::from);
    }
    for candidate in candidate_paths(contract_id, contract_dir) {
        if candidate.is_file() {
            if let Ok(reg) = Registry::load(&candidate) {
                return Ok(reg);
            }
        }
    }
    Ok(Registry::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_empty_when_nothing_is_found() {
        let reg = load_registry("no-such-contract-id", None, None).unwrap();
        assert!(reg.is_empty());
    }

    #[test]
    fn explicit_override_path_errors_when_missing() {
        let err =
            load_registry("e5-large-v1", None, Some(Path::new("/nonexistent/x.json"))).unwrap_err();
        assert!(err.message.contains("cannot read"), "{}", err.message);
    }

    #[test]
    fn finds_the_real_workspace_registry_via_manifest_dir_fallback() {
        // Exercises the CARGO_MANIFEST_DIR fallback path directly (a `cargo
        // test` run's current_exe() is the test binary, not sopack itself,
        // so the exe-relative candidates never match in this environment).
        let reg = load_registry("e5-large-v1", None, None).unwrap();
        assert!(
            !reg.is_empty(),
            "expected the committed book_codes.json to be found"
        );
    }
}
