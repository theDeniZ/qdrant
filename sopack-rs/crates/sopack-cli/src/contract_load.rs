//! Resolves the global `--contract <id|path>` flag (default `e5-large-v1`,
//! embedded) into a loaded [`sopack_contract::Contract`].

use std::path::Path;

use sopack_contract::Contract;

use crate::exit::CliError;

/// *value* is a directory (containing `contract.toml`) when it names an
/// existing path on disk; otherwise it is an embedded contract id.
pub fn load_contract(value: &str) -> Result<Contract, CliError> {
    let path = Path::new(value);
    if path.is_dir() {
        Ok(Contract::from_dir(path)?)
    } else {
        Contract::embedded(value).map_err(|e| {
            CliError::from(e).with_hint(format!(
                "{value:?} is neither a directory containing contract.toml nor a \
                 contract id this binary embeds (embedded: e5-large-v1)"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_default_embedded_contract() {
        let c = load_contract("e5-large-v1").unwrap();
        assert_eq!(c.id, "e5-large-v1");
    }

    #[test]
    fn unknown_id_is_a_clear_error() {
        let err = load_contract("no-such-contract").unwrap_err();
        assert!(err.message.contains("no-such-contract"), "{}", err.message);
    }

    #[test]
    fn loads_from_an_explicit_directory() {
        let dir = tempfile::tempdir().unwrap();
        // A directory without contract.toml is a load failure, but it must
        // be treated as a *path* attempt (not silently fall back to
        // interpreting the empty dir name as an embedded id).
        let err = load_contract(dir.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("contract.toml"), "{}", err.message);
    }
}
