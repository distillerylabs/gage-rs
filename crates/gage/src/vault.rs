//! One vault's on-disk state: config, recipients, entries. It stays a
//! stateless, identity-agnostic operator over ciphertext in both
//! invocation modes; `Session`, not `Vault`, owns any "how long does this
//! stay unlocked" bookkeeping. See "Library architecture" in the design doc.
//!
//! M0b fixes the shape later milestones build against: M1 opens and creates
//! vaults, M2 implements [`Vault::unlock`].

use std::path::PathBuf;

use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;
use crate::prompter::Prompter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    /// The name this device registered the vault under: a label for
    /// humans, never a path component.
    pub name: String,
    pub path: PathBuf,
    /// The vault's committed `[vault].id`, which keys every piece of
    /// per-vault local state (see `paths`).
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnlockError {
    /// Unlocking arrives with identities in M2.
    #[error("unlocking a vault is not implemented yet")]
    NotImplemented,
}

impl HasCode for UnlockError {
    fn code(&self) -> Code {
        Code::LockedOrAuth
    }
}

impl Vault {
    /// Proves this device can decrypt the vault, asking `prompter` for
    /// whatever its method needs, and returns the unlocked identity. The
    /// only producer of an [`Identity`].
    pub fn unlock(&self, _prompter: &dyn Prompter) -> Result<Identity, UnlockError> {
        Err(UnlockError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompter::FakePrompter;

    // The contract M2 fills in: the signature takes any Prompter, and until
    // then the typed failure maps to LockedOrAuth without asking anything.
    #[test]
    fn unlock_is_typed_and_maps_to_locked_or_auth() {
        let vault = Vault {
            name: "personal".into(),
            path: "/data/vaults/personal".into(),
            id: "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497".into(),
        };
        let prompter = FakePrompter::new();

        let result: Result<Identity, UnlockError> = vault.unlock(&prompter);

        let err = result.unwrap_err();
        assert!(matches!(err, UnlockError::NotImplemented));
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert!(prompter.unlock_requests().is_empty());
    }
}
