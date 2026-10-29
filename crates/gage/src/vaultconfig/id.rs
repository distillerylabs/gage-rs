//! The vault id: minting it, and validating it as untrusted input.
//!
//! It lives beside the file it is read from because it is part of that
//! file's schema. It is the structural twin of a device name: both are
//! filename components arriving from a committed, git-writable file, and
//! both are validated on the way in (reading the config) and again on the
//! way out (every path builder in [`crate::paths`]). See ADR-0006 and
//! ADR-0010.

use uuid::Uuid;

/// Mints a fresh vault id: a random UUIDv4, assigned rather than derived.
///
/// Assigned is the point (ADR-0006): hashing the origin URL can't work, because
/// one remote has many spellings, `git set-remote` legitimately changes
/// it, and a local-only vault has no origin at all.
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// Whether `id` is a vault id gage will build a path from: the canonical
/// lowercase 8-4-4-4-12 form of a non-nil UUID.
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
    fn new_id_mints_a_distinct_uuid_v4() {
        let (first, second) = (new_id(), new_id());
        assert_ne!(first, second, "every vault gets its own id");
        for id in [first, second] {
            let parsed = Uuid::try_parse(&id).unwrap();
            assert_eq!(parsed.get_version_num(), 4, "{id}");
            assert!(valid_id(&id), "{id}");
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
    #[test]
    fn valid_id_rejects_anything_that_is_not_a_canonical_uuid() {
        for (case, id) in [
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
        ] {
            assert!(!valid_id(&id), "{case}: {id:?} accepted");
        }
    }

    // The property the
    // validation exists for, asserted directly.
    #[test]
    fn valid_ids_are_always_single_path_components() {
        for _ in 0..200 {
            let id = new_id();
            assert!(
                !id.contains(['/', '\\', ':', '\0']),
                "minted an unsafe component: {id:?}"
            );
            assert_eq!(id.len(), 36);
        }
    }
}
