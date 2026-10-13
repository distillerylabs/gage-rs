//! Where gage keeps per-vault local state, keyed by vault id.
//!
//! Every builder validates the components it is handed rather than
//! documenting that callers must: "no path is constructed from an
//! unvalidated device name or vault id" is only true if the construction
//! site enforces it. Both arrive from a vault's committed config, which any
//! git-writer can edit (Q-DEVICE-NAME, A20).
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
}

impl HasCode for Error {
    // Go's code for a refused path component: the vault can't be unlocked
    // on this device as configured.
    fn code(&self) -> Code {
        Code::LockedOrAuth
    }
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

    // Go: TestIdentityFilePathShape
    #[test]
    fn identity_file_path_shape() {
        let got = identity_file_path(&dirs(), ID, "laptop-1").unwrap();
        assert_eq!(
            got,
            dirs().data.join("identities").join(ID).join("laptop-1.age")
        );
    }

    // Go: TestIdentityFilePathIsSiblingOfVaultsDir
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

    // Go: TestLockFilePathShape
    #[test]
    fn lock_file_path_shape() {
        assert_eq!(
            lock_file_path(&dirs(), ID).unwrap(),
            dirs().state.join("locks").join(format!("{ID}.lock"))
        );
    }

    // Go: TestTrustCacheDirShape
    #[test]
    fn trust_cache_dir_shape() {
        assert_eq!(trust_cache_dir(&dirs(), ID).unwrap(), dirs().state.join(ID));
    }

    // Go: TestPerVaultPathsRefuseAnythingButAValidID. A20's untrusted-input
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

    // Go: TestNoPerVaultLocalStateIsKeyedByTheLocalName. A property of the
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
