//! Where gage keeps per-vault local state, keyed by vault id.
//!
//! Every builder validates the components it is handed rather than
//! documenting that callers must: "no path is constructed from an
//! unvalidated device name or vault id" is only true if the construction
//! site enforces it. Both arrive from a vault's committed config, which any
//! git-writer can edit (ADR-0010, ADR-0006).
//!
//! Nothing here is keyed by the vault's local *name*: that is a label for
//! humans, chosen at clone time and tied to the vault by nothing. Two
//! unrelated vaults registered under one name in sequence would otherwise
//! share an identities directory, and two registrations of one repository
//! would take two different locks.

use std::path::PathBuf;

use crate::devicename;
use crate::exitcode::{Code, HasCode};
use crate::vaultconfig;
use crate::xdgpaths::Dirs;

/// A device name or vault id that would have become a path component
/// failed validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("vault id {0:?} is not a valid id: unsafe path component")]
    InvalidVaultId(String),
    #[error("device name {0:?} is invalid: unsafe path component")]
    InvalidDeviceName(String),
    #[error("host {host:?} cannot be used as a path component: {reason}")]
    InvalidHost { host: String, reason: &'static str },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        match self {
            // A refused path component: the vault can't be
            // unlocked on this device as configured.
            Error::InvalidVaultId(_) | Error::InvalidDeviceName(_) => Code::LockedOrAuth,
            // A host arrives from a remote URL or `--host`: the command
            // line or the config was wrong.
            Error::InvalidHost { .. } => Code::Usage,
        }
    }
}

/// `$GAGE_CONFIG/config.toml`: the global config, the vaults this device
/// knows (see `config`).
pub fn global_config_file(dirs: &Dirs) -> PathBuf {
    dirs.config.join("config.toml")
}

/// `$GAGE_DATA/vaults`: where the vaults themselves (git repos) live.
pub fn vaults_dir(dirs: &Dirs) -> PathBuf {
    dirs.data.join("vaults")
}

/// `$GAGE_DATA/identities/<vault-id>`: this device's wrapped identity files
/// for one vault. A sibling of `vaults/`, never inside a vault's own git
/// tree (see "Local identity storage" in the design doc).
pub fn identities_dir(dirs: &Dirs, vault_id: &str) -> Result<PathBuf, Error> {
    check_vault_id(vault_id)?;
    Ok(dirs.data.join("identities").join(vault_id))
}

/// `$GAGE_DATA/identities/<vault-id>/<device>.age`.
pub fn identity_file_path(dirs: &Dirs, vault_id: &str, device: &str) -> Result<PathBuf, Error> {
    if !devicename::valid(device) {
        return Err(Error::InvalidDeviceName(device.to_string()));
    }
    Ok(identities_dir(dirs, vault_id)?.join(format!("{device}.age")))
}

/// `$GAGE_STATE/locks/<vault-id>.lock`: the advisory write lock (see
/// `vaultlock`). Under the state root because a lock is ephemeral,
/// machine-local coordination: nothing to commit, sync or back up.
pub fn lock_file_path(dirs: &Dirs, vault_id: &str) -> Result<PathBuf, Error> {
    check_vault_id(vault_id)?;
    Ok(dirs.state.join("locks").join(format!("{vault_id}.lock")))
}

/// `$GAGE_STATE/<vault-id>`: this device's trust cache for the vault.
pub fn trust_cache_dir(dirs: &Dirs, vault_id: &str) -> Result<PathBuf, Error> {
    check_vault_id(vault_id)?;
    Ok(dirs.state.join(vault_id))
}

/// `$GAGE_STATE/tokens`: per-host HTTPS tokens (see `remoteauth`). Under
/// the state root because a token is disposable and machine-local:
/// re-acquirable by logging in again, unlike an identity file.
pub fn tokens_dir(dirs: &Dirs) -> PathBuf {
    dirs.state.join("tokens")
}

/// `$GAGE_STATE/tokens/<host>`. The host comes from a remote URL, which is
/// committed, git-editable config, or from `--host`, so it is validated as
/// a single path component before the path is built: `github.com/../../etc`
/// is refused, not resolved (the rule ADR-0010 applies to devices).
pub fn token_file_path(dirs: &Dirs, host: &str) -> Result<PathBuf, Error> {
    check_host(host)?;
    Ok(tokens_dir(dirs).join(host))
}

fn check_host(host: &str) -> Result<(), Error> {
    let reason = if host.is_empty() {
        "it is empty"
    } else if host == "." || host == ".." {
        "it is a relative path element"
    } else if host.contains(['/', '\\']) {
        "it contains a path separator"
    } else if host.contains('\0') {
        "it contains a NUL byte"
    } else if host.contains(':') {
        "it contains a colon"
    } else if host.trim() != host {
        "it has surrounding whitespace"
    } else {
        return Ok(());
    };
    Err(Error::InvalidHost {
        host: host.to_string(),
        reason,
    })
}

fn check_vault_id(id: &str) -> Result<(), Error> {
    if vaultconfig::valid_id(id) {
        Ok(())
    } else {
        Err(Error::InvalidVaultId(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ID: &str = "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497";

    fn dirs() -> Dirs {
        Dirs {
            config: "/x/config/gage".into(),
            data: "/x/data/gage".into(),
            state: "/x/state/gage".into(),
        }
    }

    type Builder = fn(&Dirs, &str) -> Result<PathBuf, Error>;

    /// Every per-vault builder, so a new one can't be left out of the
    /// cross-cutting tests below.
    fn builders() -> [(&'static str, Builder); 4] {
        [
            ("identities_dir", identities_dir),
            ("identity_file_path", |d, id| {
                identity_file_path(d, id, "laptop-1")
            }),
            ("lock_file_path", lock_file_path),
            ("trust_cache_dir", trust_cache_dir),
        ]
    }

    #[test]
    fn global_config_file_shape() {
        assert_eq!(
            global_config_file(&dirs()),
            dirs().config.join("config.toml")
        );
    }

    #[test]
    fn identity_file_path_shape() {
        let got = identity_file_path(&dirs(), ID, "laptop-1").unwrap();
        assert_eq!(
            got,
            dirs().data.join("identities").join(ID).join("laptop-1.age")
        );
    }

    #[test]
    fn identity_file_path_is_sibling_of_vaults_dir() {
        let identity = identity_file_path(&dirs(), ID, "laptop-1").unwrap();
        let vaults = vaults_dir(&dirs());
        assert!(!identity.starts_with(&vaults), "nested under vaults/");
        let gage_data = vaults.parent().unwrap();
        assert_eq!(
            identity.parent().unwrap().parent().unwrap(),
            gage_data.join("identities")
        );
    }

    #[test]
    fn lock_file_path_shape() {
        assert_eq!(
            lock_file_path(&dirs(), ID).unwrap(),
            dirs().state.join("locks").join(format!("{ID}.lock"))
        );
    }

    #[test]
    fn trust_cache_dir_shape() {
        assert_eq!(trust_cache_dir(&dirs(), ID).unwrap(), dirs().state.join(ID));
    }

    // The untrusted-input
    // half, asserted at the construction sites. The empty id is included:
    // a vault built without one must fail loudly, not write to "".
    #[test]
    fn per_vault_paths_refuse_anything_but_a_valid_id() {
        let bad = [
            String::new(),
            "personal".into(),
            "../../../../etc/cron.d/x".into(),
            "/etc/passwd".into(),
            r"..\..\Windows\x".into(),
            format!("{ID}/x"),
            ID.to_uppercase(),
        ];
        for (name, build) in builders() {
            for id in &bad {
                let err = build(&dirs(), id).unwrap_err();
                assert_eq!(err, Error::InvalidVaultId(id.clone()), "{name}({id:?})");
                assert_eq!(err.code(), Code::LockedOrAuth);
                assert!(err.to_string().contains("vault id"), "{err}");
            }
        }
    }

    #[test]
    fn identity_file_path_refuses_an_unsafe_device_name() {
        for device in ["", "..", "../x", "a/b", r"a\b", "UPPER"] {
            let err = identity_file_path(&dirs(), ID, device).unwrap_err();
            assert_eq!(err, Error::InvalidDeviceName(device.to_string()));
            assert_eq!(err.code(), Code::LockedOrAuth);
        }
    }

    #[test]
    fn token_file_path_shape() {
        assert_eq!(
            token_file_path(&dirs(), "github.com").unwrap(),
            dirs().state.join("tokens").join("github.com")
        );
    }

    // A host cannot escape the tokens directory, at the construction
    // site; remoteauth's copy goes through store/load.
    #[test]
    fn token_file_path_refuses_a_host_that_is_not_one_component() {
        for host in [
            "../../etc/cron.d/x",
            "..",
            "",
            "a/b",
            ".",
            r"a\b",
            "c:",
            "a\0b",
            " github.com",
        ] {
            let err = token_file_path(&dirs(), host).unwrap_err();
            assert!(matches!(err, Error::InvalidHost { .. }), "{host:?}");
            assert_eq!(err.code(), Code::Usage);
        }
    }

    // A property of the
    // builders together: the failure it rules out is one being left behind.
    #[test]
    fn no_per_vault_local_state_is_keyed_by_the_local_name() {
        let (first, second) = (vaultconfig::new_id(), vaultconfig::new_id());
        for (name, build) in builders() {
            let a = build(&dirs(), &first).unwrap();
            let b = build(&dirs(), &second).unwrap();
            assert_ne!(a, b, "{name} gives two vaults the same path");
            assert!(
                a.components()
                    .any(|c| Path::new(&c).to_string_lossy().contains(&first)),
                "{name} is not keyed by the id: {}",
                a.display()
            );
        }
    }
}
