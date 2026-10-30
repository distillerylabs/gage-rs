//! The vault id: minting it, and validating it as untrusted input.
//!
//! It lives beside the file it is read from because it is part of that
//! file's schema. It is the structural twin of a device name: both are
//! filename components arriving from a committed, git-writable file. A
//! device name is checked on the way in and again at each path builder; a
//! vault id is checked once, by [`VaultId`]'s constructors, and every path
//! builder takes a `VaultId`, so an unchecked one has no way to reach a path
//! (ADR-0052, ADR-0006, ADR-0010).

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A vault id gage will build a path from: the canonical lowercase
/// 8-4-4-4-12 form of a non-nil UUID (see [`valid_id`]).
///
/// The only ways to get one are [`VaultId::new`] and the `TryFrom`
/// conversions, deserialization included, so holding one is proof it was
/// validated (ADR-0052).
///
/// `Ord` is the canonical string's order. Two vaults are locked in vault-id
/// order (ADR-0047), and that order must not depend on which type the id
/// is carried in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VaultId(String);

/// A string that is not a vault id: the input, so the refusal can show it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("vault id {0:?} is not a valid id")]
pub struct InvalidVaultId(pub String);

impl VaultId {
    /// Mints a fresh vault id: a random UUIDv4, assigned rather than derived.
    ///
    /// Assigned is the point (ADR-0006): hashing the origin URL can't work,
    /// because one remote has many spellings, `git set-remote` legitimately
    /// changes it, and a local-only vault has no origin at all.
    // No `Default`: a vault id has no zero value (the nil UUID is refused),
    // and a default that minted a random one would hide that a value was
    // made up.
    #[allow(clippy::new_without_default)]
    pub fn new() -> VaultId {
        VaultId(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for VaultId {
    type Error = InvalidVaultId;

    fn try_from(id: String) -> Result<VaultId, InvalidVaultId> {
        if valid_id(&id) {
            Ok(VaultId(id))
        } else {
            Err(InvalidVaultId(id))
        }
    }
}

impl TryFrom<&str> for VaultId {
    type Error = InvalidVaultId;

    fn try_from(id: &str) -> Result<VaultId, InvalidVaultId> {
        VaultId::try_from(id.to_string())
    }
}

impl From<VaultId> for String {
    fn from(id: VaultId) -> String {
        id.0
    }
}

impl AsRef<str> for VaultId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for VaultId {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for VaultId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for VaultId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Whether `id` is the canonical lowercase 8-4-4-4-12 form of a non-nil
/// UUID: what [`VaultId`] accepts, and what an enrollment request id must
/// be.
///
/// Deliberately stricter than UUID parsing, which also accepts the
/// `urn:uuid:` prefix, braces, uppercase and the undashed form. Those all
/// denote the same UUID but spell it differently, and a vault with several
/// legal spellings of its id has several identities directories: the
/// collision the id exists to make unreachable. The strictness also makes
/// the safety property trivial: everything accepted is 36 characters of
/// hex and hyphens, which can't traverse, contain a separator, or be empty.
///
/// The nil UUID is refused because a zero value and a half-written config
/// both produce it, and neither is a vault that was ever assigned an id.
pub fn valid_id(id: &str) -> bool {
    match Uuid::try_parse(id) {
        // Round-trip rather than a regex: the crate's own formatting is the
        // definition of the canonical form, so this can't drift from it.
        Ok(parsed) => !parsed.is_nil() && parsed.hyphenated().to_string() == id,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497";

    #[test]
    fn new_mints_a_distinct_uuid_v4() {
        let (first, second) = (VaultId::new(), VaultId::new());
        assert_ne!(first, second, "every vault gets its own id");
        for id in [first, second] {
            let parsed = Uuid::try_parse(id.as_str()).unwrap();
            assert_eq!(parsed.get_version_num(), 4, "{id}");
            assert!(valid_id(id.as_str()), "{id}");
        }
    }

    #[test]
    fn valid_id_accepts_the_canonical_form() {
        assert!(valid_id(CANONICAL));
    }

    // The id arrives
    // from a committed file any git-writer can edit and becomes a path
    // component, so traversal, separators and absolute paths are refused
    // rather than coerced, and so are non-canonical spellings of a real
    // UUID, which would give one vault several directories.
    /// Every spelling a vault id must not have.
    fn not_ids() -> Vec<(&'static str, String)> {
        vec![
            ("empty", String::new()),
            ("traversal", "../../../../etc/cron.d/x".into()),
            ("traversal in a uuid", format!("../{CANONICAL}")),
            ("absolute path", "/etc/passwd".into()),
            ("windows traversal", r"..\..\Windows\x".into()),
            ("dot", ".".into()),
            ("dotdot", "..".into()),
            ("separator", format!("{CANONICAL}/x")),
            ("colon", format!("C:{CANONICAL}")),
            ("a plain vault name", "personal".into()),
            ("nil uuid", "00000000-0000-0000-0000-000000000000".into()),
            ("uppercase", CANONICAL.to_uppercase()),
            ("urn form", format!("urn:uuid:{CANONICAL}")),
            ("braced form", format!("{{{CANONICAL}}}")),
            ("undashed form", CANONICAL.replace('-', "")),
            ("trailing newline", format!("{CANONICAL}\n")),
            ("leading space", format!(" {CANONICAL}")),
            ("nul byte", format!("{}\0", &CANONICAL[..35])),
        ]
    }

    #[test]
    fn valid_id_rejects_anything_that_is_not_a_canonical_uuid() {
        for (case, id) in not_ids() {
            assert!(!valid_id(&id), "{case}: {id:?} accepted");
        }
    }

    // The type accepts exactly what the validator does: holding a VaultId
    // has to mean the check ran.
    #[test]
    fn vault_id_accepts_exactly_what_valid_id_accepts() {
        let id = VaultId::try_from(CANONICAL).unwrap();
        assert_eq!(id.as_str(), CANONICAL);
        assert_eq!(id.to_string(), CANONICAL);
        assert_eq!(VaultId::try_from(CANONICAL.to_string()).unwrap(), id);
        assert_eq!(String::from(id), CANONICAL);

        for (case, bad) in not_ids() {
            assert_eq!(
                VaultId::try_from(bad.as_str()),
                Err(InvalidVaultId(bad.clone())),
                "{case}"
            );
            assert_eq!(
                VaultId::try_from(bad.clone()),
                Err(InvalidVaultId(bad)),
                "{case}"
            );
        }
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    struct Holder {
        id: VaultId,
    }

    // Deserializing is a constructor too: a bad id in a file never becomes
    // a VaultId, and a good one is written back as the same string.
    #[test]
    fn serde_refuses_a_bad_id_and_round_trips_a_good_one() {
        for bad in ["", "personal", "../x", &CANONICAL.to_uppercase()] {
            let err = toml::from_str::<Holder>(&format!("id = {bad:?}")).unwrap_err();
            assert!(
                err.to_string().contains("is not a valid id"),
                "{bad:?}: {err}"
            );
        }
        let text = format!("id = \"{CANONICAL}\"\n");
        let holder: Holder = toml::from_str(&text).unwrap();
        assert_eq!(holder.id, CANONICAL);
        assert_eq!(toml::to_string(&holder).unwrap(), text);
    }

    // ADR-0047 locks two vaults in vault-id order; the type must not move
    // that order.
    #[test]
    fn ord_is_the_strings_order() {
        let mut strings: Vec<String> = (0..64).map(|_| VaultId::new().to_string()).collect();
        strings.extend([
            "00000000-0000-0000-0000-000000000001".to_string(),
            "ffffffff-ffff-ffff-ffff-ffffffffffff".to_string(),
            "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497".to_string(),
            "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d498".to_string(),
            "a0000000-0000-0000-0000-000000000000".to_string(),
        ]);
        let mut ids: Vec<VaultId> = strings
            .iter()
            .map(|s| VaultId::try_from(s.as_str()).unwrap())
            .collect();
        strings.sort();
        ids.sort();
        let ids: Vec<String> = ids.into_iter().map(String::from).collect();
        assert_eq!(ids, strings);
    }

    // The property the
    // validation exists for, asserted directly.
    #[test]
    fn valid_ids_are_always_single_path_components() {
        for _ in 0..200 {
            let id = VaultId::new().to_string();
            assert!(
                !id.contains(['/', '\\', ':', '\0']),
                "minted an unsafe component: {id:?}"
            );
            assert_eq!(id.len(), 36);
        }
    }
}
