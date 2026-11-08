//! The per-vault MAC key that authenticates entries (ADR-0065): 32 random
//! bytes, minted once when the vault is created and stored in
//! `.gage/vault-key.age`, sealed to every recipient exactly as an entry is.
//!
//! Producing an entry needs only the public keys in `.age-recipients`, so
//! `age` alone can't say a recipient wrote it. The key can: only a holder
//! of a recipient's private key can open it, and every entry carries an
//! HMAC under it, bound to the entry's id (see `doc/format/v3.md`).
//!
//! The file also carries the keys this one replaced (ADR-0066). Only the
//! current key authenticates a live entry. The retired ones exist for the
//! revisions git still holds.

use std::path::{Path, PathBuf};

use hmac::{Hmac, KeyInit, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::crypt;
use crate::crypto::identity::Identity;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::storage::atomicfile;

/// Where the sealed key lives, relative to the vault root.
pub const VAULT_KEY_FILE: &str = ".gage/vault-key.age";

/// One key's length, in bytes: HMAC-SHA256's block-sized-or-less key, at
/// the hash's own strength.
pub const KEY_LEN: usize = 32;

/// What every entry tag's input starts with, so a tag can never be
/// mistaken for one computed over anything else under the same key.
const ENTRY_DOMAIN: &[u8] = b"gage-entry-auth-v3\n";

/// What a fingerprint's input is, for the same reason.
const FINGERPRINT_DOMAIN: &[u8] = b"gage-vault-key-fingerprint-v3\n";

/// The MAC key, opened, with the keys it replaced. Wiped on drop; not
/// `Clone`, and its `Debug` shows nothing, like every secret here.
pub struct VaultKey {
    current: Zeroizing<[u8; KEY_LEN]>,
    /// Newest first.
    retired: Zeroizing<Vec<[u8; KEY_LEN]>>,
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A v3 vault always has its key: one without is damaged, never read
    /// unauthenticated.
    #[error("{} is missing: this vault's entries can't be authenticated without it", path.display())]
    Missing { path: PathBuf },
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("writing {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: atomicfile::Error,
    },
    /// Opening or sealing it, typed as the entry errors are: "not a
    /// recipient" stays distinguishable from "damaged".
    #[error("vault key {}: {source}", path.display())]
    Crypt { path: PathBuf, source: crypt::Error },
    /// It decrypted, to something that isn't a key.
    #[error("{} holds {len} bytes, not a whole number of {KEY_LEN}-byte keys", path.display())]
    Malformed { path: PathBuf, len: usize },
    /// Neither side of a merge knows the other's key, so the file was not
    /// written by anyone holding this vault's key: anyone can seal a new
    /// key to the public recipient list.
    #[error(
        "{} on origin does not continue this vault's key, so it was not written by a holder of it",
        path.display()
    )]
    Unrelated { path: PathBuf },
    /// The key file doesn't continue the key this device last saw for the
    /// vault (ADR-0067). Nothing was read: anyone can seal a key to the
    /// public recipient list.
    #[error(
        "{} does not continue the key this device last saw for {vault:?}, so someone without the \
         vault's key may have replaced it; nothing was read or written",
        path.display()
    )]
    Discontinuous { vault: String, path: PathBuf },
    /// This device's own record of the key is damaged.
    #[error("{} is not a key-continuity record", path.display())]
    BadRecord { path: PathBuf },
}

impl HasKind for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::Missing { .. }
            | Error::Malformed { .. }
            | Error::Unrelated { .. }
            | Error::Discontinuous { .. } => ErrorKind::Conflict,
            Error::Crypt { source, .. } => source.kind(),
            Error::Io { .. } | Error::Write { .. } | Error::BadRecord { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            Error::Crypt { source, .. } => source.remedy(),
            Error::Discontinuous { .. } => Some(Remedy::KeyContinuity),
            _ => None,
        }
    }
}

impl VaultKey {
    /// A fresh key from the OS CSPRNG.
    pub(crate) fn generate() -> VaultKey {
        let mut current = Zeroizing::new([0u8; KEY_LEN]);
        OsRng.fill_bytes(current.as_mut());
        VaultKey {
            current,
            retired: Zeroizing::new(Vec::new()),
        }
    }

    /// A fresh key that replaces this one, which it keeps as its newest
    /// retired key.
    pub(crate) fn rotated(&self) -> VaultKey {
        let mut retired = Zeroizing::new(Vec::with_capacity(self.retired.len() + 1));
        retired.push(*self.current);
        retired.extend_from_slice(&self.retired);
        VaultKey {
            retired,
            ..VaultKey::generate()
        }
    }

    /// Whether this key descends from `other`: `other`'s current key is
    /// this one's current key or among its retired ones. Only a holder of
    /// `other` can produce such a file.
    pub(crate) fn continues(&self, other: &VaultKey) -> bool {
        *self.current == *other.current || self.retired.contains(&*other.current)
    }

    /// A one-way commitment to the current key, as lowercase hex: what a
    /// device records of the key it last saw (ADR-0067). The key is 32
    /// random bytes, so the fingerprint gives nothing away, and matching
    /// one takes holding the key it was made from.
    pub(crate) fn fingerprint(&self) -> String {
        fingerprint_of(&self.current)
    }

    /// Whether the key `fingerprint` was made from is this key or one it
    /// replaced: [`VaultKey::continues`], against what a device recorded.
    pub(crate) fn descends_from(&self, fingerprint: &str) -> bool {
        std::iter::once(&*self.current)
            .chain(self.retired.iter())
            .any(|key| fingerprint_of(key) == fingerprint)
    }

    /// The key as an age file encrypted to `to`: the current key, then the
    /// retired ones.
    pub(crate) fn seal(&self, to: &[crypt::Recipient]) -> Result<Vec<u8>, crypt::Error> {
        let mut plain = Zeroizing::new(Vec::with_capacity(KEY_LEN * (1 + self.retired.len())));
        plain.extend_from_slice(self.current.as_ref());
        for key in self.retired.iter() {
            plain.extend_from_slice(key);
        }
        crypt::encrypt(&plain, to)
    }

    /// Writes the key sealed to `to` at `<vault>/`[`VAULT_KEY_FILE`],
    /// atomically. Nothing is written if sealing fails.
    pub(crate) fn save(&self, vault: &Path, to: &[crypt::Recipient]) -> Result<(), Error> {
        let path = vault.join(VAULT_KEY_FILE);
        let sealed = self.seal(to).map_err(|source| Error::Crypt {
            path: path.clone(),
            source,
        })?;
        atomicfile::write(&path, &sealed, 0o644).map_err(|source| Error::Write { path, source })
    }

    /// Opens `<vault>/`[`VAULT_KEY_FILE`] with `identity`.
    #[cfg(test)]
    pub(crate) fn load(vault: &Path, identity: &Identity) -> Result<VaultKey, Error> {
        Ok(VaultKey::load_sealed(vault, identity)?.0)
    }

    /// [`VaultKey::load`], returning the sealed bytes too: what was opened
    /// is what a caller can compare, with no second read for the file to
    /// change between.
    pub(crate) fn load_sealed(
        vault: &Path,
        identity: &Identity,
    ) -> Result<(VaultKey, Vec<u8>), Error> {
        let path = vault.join(VAULT_KEY_FILE);
        let sealed = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Missing { path });
            }
            Err(source) => {
                return Err(Error::Io {
                    action: "reading",
                    path,
                    source,
                });
            }
        };
        let key = VaultKey::open(&sealed, identity).map_err(|e| e.at(path))?;
        Ok((key, sealed))
    }

    /// The key out of sealed bytes, wherever they came from.
    pub(crate) fn open(sealed: &[u8], identity: &Identity) -> Result<VaultKey, OpenError> {
        let plain = Zeroizing::new(crypt::decrypt(sealed, identity).map_err(OpenError::Crypt)?);
        let (keys, rest) = plain.as_chunks::<KEY_LEN>();
        let ([current, retired @ ..], []) = (keys, rest) else {
            return Err(OpenError::Malformed(plain.len()));
        };
        Ok(VaultKey {
            current: Zeroizing::new(*current),
            retired: Zeroizing::new(retired.to_vec()),
        })
    }

    /// The `auth` tag of entry `id` whose payload body is `body`, under
    /// the current key.
    pub(crate) fn entry_tag(&self, id: Uuid, body: &[u8]) -> [u8; 32] {
        entry_mac(&self.current, id, body)
            .finalize()
            .into_bytes()
            .into()
    }

    /// Whether `tag` is entry `id`'s tag over `body` under the current
    /// key, compared in constant time.
    pub(crate) fn verify_entry(&self, id: Uuid, body: &[u8], tag: &[u8]) -> bool {
        entry_mac(&self.current, id, body).verify_slice(tag).is_ok()
    }

    /// [`VaultKey::verify_entry`], also accepting a retired key: for a
    /// past revision only, never a live entry.
    pub(crate) fn verify_revision(&self, id: Uuid, body: &[u8], tag: &[u8]) -> bool {
        self.verify_entry(id, body, tag)
            || self
                .retired
                .iter()
                .any(|key| entry_mac(key, id, body).verify_slice(tag).is_ok())
    }

    /// A key with fixed bytes, for the golden fixture's generator and
    /// known-answer tests.
    #[cfg(test)]
    pub(crate) fn from_bytes_for_tests(bytes: [u8; KEY_LEN]) -> VaultKey {
        VaultKey {
            current: Zeroizing::new(bytes),
            retired: Zeroizing::new(Vec::new()),
        }
    }
}

fn entry_mac(key: &[u8; KEY_LEN], id: Uuid, body: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any size");
    mac.update(ENTRY_DOMAIN);
    mac.update(id.hyphenated().to_string().as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac
}

fn fingerprint_of(key: &[u8; KEY_LEN]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any size");
    mac.update(FINGERPRINT_DOMAIN);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why [`VaultKey::open`] produced no key.
#[derive(Debug)]
pub(crate) enum OpenError {
    Crypt(crypt::Error),
    Malformed(usize),
}

impl OpenError {
    /// This failure, as the error for the key file at `path`.
    pub(crate) fn at(self, path: PathBuf) -> Error {
        match self {
            OpenError::Crypt(source) => Error::Crypt { path, source },
            OpenError::Malformed(len) => Error::Malformed { path, len },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipient(key: &age::x25519::Identity) -> crypt::Recipient {
        crypt::Recipient::parse(&key.to_public().to_string()).unwrap()
    }

    #[test]
    fn a_generated_key_round_trips_through_seal_and_open() {
        let alice = age::x25519::Identity::generate();
        let bob = age::x25519::Identity::generate();
        let key = VaultKey::generate();
        let sealed = key.seal(&[recipient(&alice), recipient(&bob)]).unwrap();
        for who in [&alice, &bob] {
            let opened = VaultKey::open(&sealed, &Identity::for_tests(who)).unwrap();
            assert_eq!(*opened.current, *key.current);
        }
    }

    #[test]
    fn a_non_recipient_cannot_open_the_key() {
        let alice = age::x25519::Identity::generate();
        let sealed = VaultKey::generate().seal(&[recipient(&alice)]).unwrap();
        let stranger = Identity::for_tests(&age::x25519::Identity::generate());
        assert!(matches!(
            VaultKey::open(&sealed, &stranger),
            Err(OpenError::Crypt(crypt::Error::NotARecipient))
        ));
    }

    #[test]
    fn generated_keys_differ() {
        assert_ne!(*VaultKey::generate().current, *VaultKey::generate().current);
    }

    #[test]
    fn a_sealed_file_of_the_wrong_length_is_malformed() {
        let alice = age::x25519::Identity::generate();
        let sealed = crypt::encrypt(&[7u8; 31], &[recipient(&alice)]).unwrap();
        assert!(matches!(
            VaultKey::open(&sealed, &Identity::for_tests(&alice)),
            Err(OpenError::Malformed(31))
        ));
    }

    #[test]
    fn a_rotated_key_keeps_the_old_one_newest_first_through_a_seal() {
        let alice = age::x25519::Identity::generate();
        let first = VaultKey::generate();
        let second = first.rotated();
        let third = second.rotated();
        assert_ne!(*third.current, *second.current);
        let sealed = third.seal(&[recipient(&alice)]).unwrap();
        let opened = VaultKey::open(&sealed, &Identity::for_tests(&alice)).unwrap();
        assert_eq!(*opened.current, *third.current);
        assert_eq!(*opened.retired, vec![*second.current, *first.current]);
    }

    #[test]
    fn a_sealed_file_between_whole_keys_is_malformed() {
        let alice = age::x25519::Identity::generate();
        for len in [0, 33, 63] {
            let sealed = crypt::encrypt(&vec![7u8; len], &[recipient(&alice)]).unwrap();
            assert!(
                matches!(
                    VaultKey::open(&sealed, &Identity::for_tests(&alice)),
                    Err(OpenError::Malformed(n)) if n == len
                ),
                "{len} bytes"
            );
        }
    }

    // A live entry takes the current key alone; a past revision may also
    // take a retired one.
    #[test]
    fn only_a_revision_verifies_under_a_retired_key() {
        let old = VaultKey::generate();
        let id = Uuid::new_v4();
        let tag = old.entry_tag(id, b"body");
        let new = old.rotated();
        assert!(!new.verify_entry(id, b"body", &tag), "live entry");
        assert!(new.verify_revision(id, b"body", &tag), "past revision");
        let tag = new.entry_tag(id, b"body");
        assert!(new.verify_entry(id, b"body", &tag));
        assert!(new.verify_revision(id, b"body", &tag));
        let unrelated = VaultKey::generate().entry_tag(id, b"body");
        assert!(!new.verify_revision(id, b"body", &unrelated));
    }

    #[test]
    fn continues_means_knowing_the_others_current_key() {
        let old = VaultKey::generate();
        let new = old.rotated();
        assert!(new.continues(&old));
        assert!(!old.continues(&new), "the old key has never seen the new");
        assert!(old.continues(&old), "a re-seal changes nothing");
        assert!(!VaultKey::generate().continues(&old));
        assert!(!new.continues(&VaultKey::generate()));
    }

    // The input spelled out, as the entry tag's is: a known construction,
    // not a round trip.
    #[test]
    fn the_fingerprint_is_hmac_of_its_domain_under_the_key() {
        let key = VaultKey::from_bytes_for_tests([0x42; KEY_LEN]);
        let mut mac = Hmac::<Sha256>::new_from_slice(&[0x42; KEY_LEN]).unwrap();
        mac.update(b"gage-vault-key-fingerprint-v3\n");
        let want: [u8; 32] = mac.finalize().into_bytes().into();
        assert_eq!(key.fingerprint(), hex(&want));
        assert_eq!(key.fingerprint().len(), 64);
    }

    #[test]
    fn a_key_descends_from_its_own_fingerprint_and_every_retired_one() {
        let first = VaultKey::generate();
        let second = first.rotated();
        let third = second.rotated();
        for ancestor in [&first, &second, &third] {
            assert!(third.descends_from(&ancestor.fingerprint()));
        }
        assert!(!first.descends_from(&third.fingerprint()), "a rollback");
        assert!(!third.descends_from(&VaultKey::generate().fingerprint()));
    }

    #[test]
    fn save_then_load_round_trips_and_absence_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".gage")).unwrap();
        let alice = age::x25519::Identity::generate();
        let id = Identity::for_tests(&alice);

        let err = VaultKey::load(dir.path(), &id).unwrap_err();
        assert!(matches!(err, Error::Missing { .. }), "{err}");
        assert_eq!(err.kind(), ErrorKind::Conflict);

        let key = VaultKey::generate();
        key.save(dir.path(), &[recipient(&alice)]).unwrap();
        assert_eq!(
            *VaultKey::load(dir.path(), &id).unwrap().current,
            *key.current
        );

        let stranger = Identity::for_tests(&age::x25519::Identity::generate());
        let err = VaultKey::load(dir.path(), &stranger).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Crypt {
                    source: crypt::Error::NotARecipient,
                    ..
                }
            ),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    }

    // HMAC-SHA256 itself, against RFC 4231 test case 2, through the same
    // crate calls `entry_mac` makes: a known answer, not a round trip.
    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        let got: [u8; 32] = mac.finalize().into_bytes().into();
        assert_eq!(
            hex(&got),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // The tag's input, spelled out: domain, canonical id, newline, body.
    #[test]
    fn the_entry_tag_is_hmac_over_domain_id_and_body() {
        let key = VaultKey::from_bytes_for_tests([0x42; KEY_LEN]);
        let id = Uuid::parse_str("0b6a5c1e-2f4d-4b8a-9c3e-7d1f0a2b3c4d").unwrap();
        let body = b"title: x\nvalue: y\n";
        let mut mac = Hmac::<Sha256>::new_from_slice(&[0x42; KEY_LEN]).unwrap();
        mac.update(
            b"gage-entry-auth-v3\n0b6a5c1e-2f4d-4b8a-9c3e-7d1f0a2b3c4d\ntitle: x\nvalue: y\n",
        );
        let want: [u8; 32] = mac.finalize().into_bytes().into();
        assert_eq!(key.entry_tag(id, body), want);
    }

    #[test]
    fn verify_entry_binds_key_id_and_body() {
        let key = VaultKey::generate();
        let id = Uuid::new_v4();
        let tag = key.entry_tag(id, b"body");
        assert!(key.verify_entry(id, b"body", &tag));
        assert!(!key.verify_entry(Uuid::new_v4(), b"body", &tag), "other id");
        assert!(!key.verify_entry(id, b"bodY", &tag), "other body");
        assert!(
            !VaultKey::generate().verify_entry(id, b"body", &tag),
            "other key"
        );
        assert!(!key.verify_entry(id, b"body", &tag[..31]), "truncated tag");
    }

    #[test]
    fn debug_never_shows_the_key() {
        let key = VaultKey::from_bytes_for_tests([0xab; KEY_LEN]);
        let shown = format!("{key:?}");
        assert!(!shown.contains("ab") && !shown.contains("171"), "{shown}");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
