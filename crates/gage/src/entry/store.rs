//! Entries on disk: `entries/<uuid>.age`, encrypted to every key in
//! `.age-recipients`. Vault-generic and stateless like the rest of `Vault`:
//! the identity that reads is a parameter.

use std::path::PathBuf;

use uuid::Uuid;
use zeroize::Zeroizing;

use super::auth::{self, AUTH_KEY};
use super::{ENTRIES_DIR_NAME, ENTRY_FILE_EXT, Entry, Error};
use crate::crypto::vaultkey::VaultKey;
use crate::vault::Vault;
use crate::{crypto::crypt, crypto::identity::Identity, storage::atomicfile, trust::recipient};

impl Vault {
    /// `<vault>/entries`.
    fn entries_dir(&self) -> PathBuf {
        self.path.join(ENTRIES_DIR_NAME)
    }

    /// `<vault>/entries/<id>.age`.
    pub(crate) fn entry_path(&self, id: Uuid) -> PathBuf {
        self.entries_dir().join(format!("{id}{ENTRY_FILE_EXT}"))
    }

    /// Every key in `.age-recipients`, ready to encrypt to: the devices
    /// that can read a freshly written entry. A plugin recipient
    /// (`age1yubikey1…`) is legal there but not encryptable by this build,
    /// and fails here, before anything is written.
    pub(crate) fn encrypt_recipients(&self) -> Result<Vec<crypt::Recipient>, Error> {
        let keys = recipient::load(&self.path.join(".age-recipients"))?;
        Ok(keys
            .iter()
            .map(|k| crypt::Recipient::parse(k))
            .collect::<Result<_, _>>()?)
    }

    /// The vault's MAC key, opened with `identity` (ADR-0065).
    pub(crate) fn vault_key(&self, identity: &Identity) -> Result<VaultKey, Error> {
        Ok(VaultKey::load(&self.path, identity)?)
    }

    /// Encrypts `e` and writes it to `entries/<id>.age`, atomically, at
    /// `0600`, to every recipient currently listed in `.age-recipients`.
    /// `id` is the caller's to choose: a fresh [`super::new_id`] for an
    /// insert, or an existing entry's own id to rewrite it in place.
    /// `identity` opens the vault's MAC key, which stamps the entry's
    /// `auth` tag; it is not who the entry is encrypted to.
    ///
    /// The caller holds the vault's write lock and commits; this only
    /// writes the file.
    pub fn write_entry(&self, id: Uuid, e: &Entry, identity: &Identity) -> Result<(), Error> {
        let to = self.encrypt_recipients()?;
        self.write_entry_to(id, e, &to, &self.vault_key(identity)?)
    }

    /// [`Vault::write_entry`] to an explicit recipient list and key rather
    /// than the ones on disk: what a re-encryption needs, since it writes
    /// every entry to the new list *before* the recipient files change
    /// (that order is its crash safety).
    pub(crate) fn write_entry_to(
        &self,
        id: Uuid,
        e: &Entry,
        to: &[crypt::Recipient],
        key: &VaultKey,
    ) -> Result<(), Error> {
        let ciphertext = seal_entry(id, e, to, key)?;

        let dir = self.entries_dir();
        std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
            action: "creating",
            path: dir.clone(),
            source,
        })?;
        // create_dir_all leaves an already-existing directory's mode alone,
        // so one created before this rule (or widened by umask) is
        // corrected here rather than trusted.
        restrict_dir(&dir).map_err(|source| Error::Io {
            action: "setting permissions on",
            path: dir.clone(),
            source,
        })?;
        atomicfile::write(&self.entry_path(id), &ciphertext, 0o600)
            .map_err(|source| Error::Write { id, source })
    }

    /// `e`, as entry `id`, encrypted to every recipient currently listed in
    /// `.age-recipients`, without writing it anywhere: what
    /// [`Vault::write_entry`] writes. Stamps only the `auth` tag: the
    /// entry's own fields are encrypted exactly as given.
    #[cfg(test)]
    pub(crate) fn encrypt_entry(
        &self,
        id: Uuid,
        e: &Entry,
        identity: &Identity,
    ) -> Result<Vec<u8>, Error> {
        let to = self.encrypt_recipients()?;
        seal_entry(id, e, &to, &self.vault_key(identity)?)
    }

    /// Decrypts entry `id`'s ciphertext and authenticates it under `key`
    /// rather than the key on disk: what a merge needs, since each side of
    /// it may be tagged under its own side's key.
    pub(crate) fn decrypt_entry_under(
        &self,
        id: Uuid,
        ciphertext: &[u8],
        identity: &Identity,
        key: &VaultKey,
    ) -> Result<Entry, Error> {
        self.count_decrypt();
        let plaintext = Zeroizing::new(crypt::decrypt(ciphertext, identity)?);
        authenticated(id, key, &plaintext, auth::verify)
    }

    /// Reads and decrypts `entries/<id>.age` with `identity`, checks its
    /// `auth` tag against this vault's key and `id`, and parses the result.
    /// Ciphertext `identity` isn't a recipient of fails with
    /// `crypt::Error::NotARecipient`, unwrapped, so a caller can tell
    /// "wrong key" from "damaged file" from "no such entry", and all three
    /// from [`Error::Unauthenticated`].
    pub fn read_entry(&self, id: Uuid, identity: &Identity) -> Result<Entry, Error> {
        self.entry_reader(identity).read(id)
    }

    /// A reader for many entries that opens the MAC key once, on the
    /// first entry that decrypts.
    pub(crate) fn entry_reader<'a>(&'a self, identity: &'a Identity) -> EntryReader<'a> {
        EntryReader {
            vault: self,
            identity,
            key: None,
        }
    }

    /// The ids of every entry file in `entries/`, sorted. It reads only
    /// filenames (no decryption, no ciphertext access), so it needs no
    /// identity. A vault with no `entries/` directory yet has no entries,
    /// not an error. Subdirectories and files that aren't a canonical
    /// `<uuid>.age` are skipped rather than failing the whole listing.
    pub fn entry_ids(&self) -> Result<Vec<Uuid>, Error> {
        let dir = self.entries_dir();
        let listing = match std::fs::read_dir(&dir) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    action: "listing",
                    path: dir,
                    source,
                });
            }
        };
        let mut ids = Vec::new();
        for item in listing {
            let item = item.map_err(|source| Error::Io {
                action: "listing",
                path: dir.clone(),
                source,
            })?;
            let is_file = item.file_type().is_ok_and(|t| !t.is_dir());
            let name = item.file_name();
            let Some(stem) = name
                .to_str()
                .and_then(|n| n.strip_suffix(ENTRY_FILE_EXT))
                .filter(|_| is_file)
            else {
                continue;
            };
            // Canonical spelling only: `entry_path` builds the lower-case
            // hyphenated form, so any other spelling names a file this
            // vault could never read back by id.
            match Uuid::try_parse(stem) {
                Ok(id) if id.hyphenated().to_string() == stem => ids.push(id),
                _ => {}
            }
        }
        ids.sort();
        Ok(ids)
    }
}

/// Reads entries with one identity, opening the vault's MAC key once.
/// The key is opened only after an entry has decrypted, so a wrong
/// identity still fails as `NotARecipient` on the entry itself.
pub(crate) struct EntryReader<'a> {
    vault: &'a Vault,
    identity: &'a Identity,
    key: Option<VaultKey>,
}

impl EntryReader<'_> {
    /// [`Vault::read_entry`].
    pub(crate) fn read(&mut self, id: Uuid) -> Result<Entry, Error> {
        let path = self.vault.entry_path(id);
        let ciphertext = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::EntryNotFound { id });
            }
            Err(source) => {
                return Err(Error::Io {
                    action: "reading",
                    path,
                    source,
                });
            }
        };
        self.decrypt(id, &ciphertext)
    }

    /// [`Vault::decrypt_entry`]. Authenticated before anything is parsed.
    pub(crate) fn decrypt(&mut self, id: Uuid, ciphertext: &[u8]) -> Result<Entry, Error> {
        self.open(id, ciphertext, auth::verify)
    }

    /// [`EntryReader::decrypt`] for a past revision out of git history,
    /// which may be tagged under a retired key (ADR-0066). Never for a
    /// live entry.
    pub(crate) fn decrypt_revision(&mut self, id: Uuid, ciphertext: &[u8]) -> Result<Entry, Error> {
        self.open(id, ciphertext, auth::verify_revision)
    }

    fn open(&mut self, id: Uuid, ciphertext: &[u8], check: Check) -> Result<Entry, Error> {
        self.vault.count_decrypt();
        let plaintext = Zeroizing::new(crypt::decrypt(ciphertext, self.identity)?);
        let key = match &mut self.key {
            Some(key) => key,
            empty => empty.insert(self.vault.vault_key(self.identity)?),
        };
        authenticated(id, key, &plaintext, check)
    }
}

/// How a payload is checked against a key: [`auth::verify`] or
/// [`auth::verify_revision`].
type Check = for<'p> fn(&VaultKey, Uuid, &'p [u8]) -> Result<&'p [u8], auth::AuthFailure>;

/// The entry in `plaintext`, once `check` has authenticated it.
fn authenticated(id: Uuid, key: &VaultKey, plaintext: &[u8], check: Check) -> Result<Entry, Error> {
    let body =
        check(key, id, plaintext).map_err(|failure| Error::Unauthenticated { id, failure })?;
    let entry = Entry::from_yaml(body)?;
    if entry.extra.contains_key(AUTH_KEY) {
        return Err(Error::Parse(format!(
            "{AUTH_KEY:?} appears in the entry body as well as its last line"
        )));
    }
    Ok(entry)
}

/// `e` as entry `id`'s ciphertext: its YAML, stamped with the `auth` tag
/// under `key`, encrypted to `to`.
pub(crate) fn seal_entry(
    id: Uuid,
    e: &Entry,
    to: &[crypt::Recipient],
    key: &VaultKey,
) -> Result<Vec<u8>, Error> {
    if e.extra.contains_key(AUTH_KEY) {
        return Err(Error::ReservedKey(AUTH_KEY));
    }
    let payload = auth::stamp(key, id, &e.to_yaml()?);
    Ok(crypt::encrypt(&payload, to)?)
}

#[cfg(unix)]
fn restrict_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{AuthFailure, Extra, Timestamp, new_id};
    use crate::error::{ErrorKind, HasKind};
    use crate::storage::xdgpaths::Dirs;
    use crate::testsupport::fast_scrypt;

    /// A vault directory with a recipients file and the identities that
    /// can read it.
    struct Fixture {
        _root: tempfile::TempDir,
        vault: Vault,
        keys: Vec<age::x25519::Identity>,
    }

    impl Fixture {
        fn new(recipient_count: usize) -> Fixture {
            fast_scrypt();
            let root = tempfile::tempdir().unwrap();
            let dirs = Dirs {
                config: root.path().join("config"),
                data: root.path().join("data"),
                state: root.path().join("state"),
            };
            let mut vault =
                Vault::for_tests(&dirs, "work", &crate::storage::vaultconfig::VaultId::new());
            vault.path = root.path().join("vault");
            std::fs::create_dir_all(&vault.path).unwrap();
            let keys: Vec<_> = (0..recipient_count)
                .map(|_| age::x25519::Identity::generate())
                .collect();
            let public: Vec<String> = keys.iter().map(|k| k.to_public().to_string()).collect();
            recipient::save(&vault.path.join(".age-recipients"), &public).unwrap();
            std::fs::create_dir_all(vault.path.join(".gage")).unwrap();
            VaultKey::generate()
                .save(&vault.path, &vault.encrypt_recipients().unwrap())
                .unwrap();
            Fixture {
                _root: root,
                vault,
                keys,
            }
        }

        fn identity(&self, i: usize) -> Identity {
            Identity::for_tests(&self.keys[i])
        }

        fn entry_file(&self, id: Uuid) -> PathBuf {
            self.vault.entry_path(id)
        }

        fn write(&self, id: Uuid, e: &Entry) -> Result<(), Error> {
            self.vault.write_entry(id, e, &self.identity(0))
        }

        /// The decrypted payload of entry `id`'s file.
        fn payload(&self, id: Uuid) -> Vec<u8> {
            let ciphertext = std::fs::read(self.entry_file(id)).unwrap();
            crypt::decrypt(&ciphertext, &self.identity(0)).unwrap()
        }

        /// Encrypts `payload` to every recipient, as anyone holding only
        /// the public keys can, and files it as entry `id`.
        fn plant(&self, id: Uuid, payload: &[u8]) {
            let ciphertext =
                crypt::encrypt(payload, &self.vault.encrypt_recipients().unwrap()).unwrap();
            std::fs::create_dir_all(self.vault.entries_dir()).unwrap();
            std::fs::write(self.entry_file(id), ciphertext).unwrap();
        }
    }

    fn sample() -> Entry {
        Entry {
            title: "ProtonMail".into(),
            description: "Personal email".into(),
            created: Timestamp::from_unix(1_768_386_720),
            updated: Timestamp::from_unix(1_787_000_000),
            updated_by: "yubikey-5c-nfc-1".into(),
            value: "correct horse\nbattery staple\t!".into(),
            fields: [("username".to_string(), "me@proton.me".to_string())].into(),
            extra: [("future".to_string(), Extra::Plain("0x1F".into()))].into(),
        }
    }

    #[test]
    fn write_entry_then_read_entry_round_trips() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        assert_eq!(f.vault.read_entry(id, &f.identity(0)).unwrap(), sample());
        // Rewriting in place under the same id replaces it.
        let mut changed = sample();
        changed.value = "rotated".into();
        f.write(id, &changed).unwrap();
        assert_eq!(f.vault.read_entry(id, &f.identity(0)).unwrap(), changed);
        assert_eq!(f.vault.entry_ids().unwrap(), vec![id]);
    }

    // Every string
    // gage owns survives encrypt and decrypt unchanged.
    #[test]
    fn entry_survives_encryption_byte_for_byte() {
        let f = Fixture::new(1);
        let nasty = "\nlead\r\n\ttabs \u{2028} \"q\" \\ nul\0 \u{1f510} \n\n";
        let e = Entry {
            title: nasty.into(),
            description: nasty.into(),
            updated_by: nasty.into(),
            value: nasty.into(),
            fields: [(nasty.to_string(), nasty.to_string())].into(),
            ..Entry::default()
        };
        let id = new_id();
        f.write(id, &e).unwrap();
        let back = f.vault.read_entry(id, &f.identity(0)).unwrap();
        assert_eq!(back.value.as_bytes(), nasty.as_bytes());
        assert_eq!(back, e);
    }

    #[test]
    fn entry_is_decryptable_by_every_recipient() {
        let f = Fixture::new(3);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        for i in 0..3 {
            assert_eq!(
                f.vault.read_entry(id, &f.identity(i)).unwrap(),
                sample(),
                "recipient {i}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_entry_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(f.entry_file(id)), 0o600);
        assert_eq!(mode(f.vault.entries_dir()), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn write_entry_tightens_an_existing_loose_entries_dir() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new(1);
        let dir = f.vault.entries_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        f.write(new_id(), &sample()).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    // The write goes through atomicfile (whose
    // own tests prove the temp-and-rename); what is checked here is that
    // nothing but the finished file is left behind, and that a failed
    // write leaves the previous ciphertext untouched.
    #[test]
    fn write_entry_is_atomic() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let before = std::fs::read(f.entry_file(id)).unwrap();

        let names = || -> Vec<String> {
            std::fs::read_dir(f.vault.entries_dir())
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        };
        assert_eq!(names(), vec![format!("{id}.age")]);

        // An entry that cannot be marshaled fails before anything is
        // written, so the old ciphertext is still there, byte for byte.
        let bad = Entry {
            extra: [("k".to_string(), Extra::Plain("a\nb: c".into()))].into(),
            ..sample()
        };
        assert!(matches!(f.write(id, &bad), Err(Error::Unfaithful(_))));
        assert_eq!(std::fs::read(f.entry_file(id)).unwrap(), before);
        assert_eq!(names(), vec![format!("{id}.age")]);
    }

    #[test]
    fn entry_file_reveals_nothing_in_plaintext() {
        let f = Fixture::new(1);
        let e = Entry {
            title: "UNIQUE-TITLE-MARKER".into(),
            value: "UNIQUE-VALUE-MARKER".into(),
            fields: [(
                "UNIQUE-NAME-MARKER".to_string(),
                "UNIQUE-FIELD-MARKER".to_string(),
            )]
            .into(),
            ..Entry::default()
        };
        let id = new_id();
        f.write(id, &e).unwrap();
        let file = std::fs::read(f.entry_file(id)).unwrap();
        assert!(file.starts_with(b"age-encryption.org/v1\n"));
        for marker in ["MARKER", "title", "value", "fields"] {
            assert!(
                !file.windows(marker.len()).any(|w| w == marker.as_bytes()),
                "{marker} appears in the file"
            );
        }
    }

    #[test]
    fn write_entry_rejects_a_recipient_it_cannot_encrypt_to() {
        let f = Fixture::new(1);
        let mut keys = recipient::load(&f.vault.path.join(".age-recipients")).unwrap();
        keys.push("age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0".into());
        recipient::save(&f.vault.path.join(".age-recipients"), &keys).unwrap();
        let err = f.write(new_id(), &sample()).unwrap_err();
        assert!(
            matches!(&err, Error::Crypt(crypt::Error::InvalidRecipient { .. })),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::Usage);
        assert!(!f.vault.entries_dir().exists(), "nothing is written");
    }

    #[test]
    fn write_entry_without_a_recipients_file_fails() {
        let f = Fixture::new(1);
        std::fs::remove_file(f.vault.path.join(".age-recipients")).unwrap();
        let err = f.write(new_id(), &sample()).unwrap_err();
        assert!(matches!(err, Error::Recipients(_)), "{err}");
        assert!(!f.vault.entries_dir().exists());
        // An empty file is refused too: a file nobody can read is never made.
        std::fs::write(f.vault.path.join(".age-recipients"), "").unwrap();
        let err = f.write(new_id(), &sample()).unwrap_err();
        assert!(
            matches!(err, Error::Crypt(crypt::Error::NoRecipients)),
            "{err}"
        );
    }

    #[test]
    fn read_entry_unknown_id_fails_not_found() {
        let f = Fixture::new(1);
        let id = new_id();
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(&err, Error::EntryNotFound { id: got } if *got == id),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.to_string().contains(&id.to_string()));
    }

    #[test]
    fn read_entry_with_a_non_recipient_identity_fails_typed() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let stranger = Identity::for_tests(&age::x25519::Identity::generate());
        let err = f.vault.read_entry(id, &stranger).unwrap_err();
        assert!(
            matches!(err, Error::Crypt(crypt::Error::NotARecipient)),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    }

    #[test]
    fn read_entry_on_corrupt_ciphertext_fails() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let path = f.entry_file(id);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(err, Error::Crypt(crypt::Error::CorruptCiphertext(_))),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::Conflict);
        std::fs::write(&path, b"not an age file").unwrap();
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(err, Error::Crypt(crypt::Error::CorruptCiphertext(_))),
            "{err}"
        );
    }

    // Authenticated first, parsed after: a payload that verifies but
    // isn't an entry is a parse error, not an authentication failure.
    #[test]
    fn read_entry_of_an_authenticated_payload_that_is_not_an_entry_is_a_parse_error() {
        let f = Fixture::new(1);
        let id = new_id();
        let key = f.vault.vault_key(&f.identity(0)).unwrap();
        f.plant(id, &auth::stamp(&key, id, b"- just\n- a list\n"));
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err}");
        assert_eq!(err.kind(), ErrorKind::Conflict);
    }

    // Stamp on write: the payload is the entry's YAML and an auth line
    // whose tag verifies under the vault's key for this id.
    #[test]
    fn write_entry_stamps_an_auth_tag_that_verifies() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let payload = f.payload(id);
        let key = f.vault.vault_key(&f.identity(0)).unwrap();
        let body = auth::verify(&key, id, &payload).unwrap();
        assert_eq!(body, &sample().to_yaml().unwrap()[..]);
        let text = String::from_utf8(payload.clone()).unwrap();
        let last = text.trim_end_matches('\n').rsplit('\n').next().unwrap();
        assert!(last.starts_with("auth: hmac-sha256:"), "{text}");
    }

    #[test]
    fn read_entry_rejects_a_wrong_auth_tag() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let mut payload = f.payload(id);
        // Flip one hex digit of the tag.
        let at = payload.len() - 2;
        payload[at] = if payload[at] == b'0' { b'1' } else { b'0' };
        f.plant(id, &payload);
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(&err, Error::Unauthenticated { id: got, failure: AuthFailure::Mismatch } if *got == id),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert!(err.to_string().contains(&id.to_string()), "{err}");
    }

    #[test]
    fn read_entry_rejects_an_entry_without_an_auth_tag() {
        let f = Fixture::new(1);
        let id = new_id();
        f.plant(id, &sample().to_yaml().unwrap());
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Unauthenticated {
                    failure: AuthFailure::Missing,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn read_entry_rejects_a_tag_computed_for_a_different_id() {
        let f = Fixture::new(1);
        let (id, other) = (new_id(), new_id());
        let key = f.vault.vault_key(&f.identity(0)).unwrap();
        f.plant(id, &auth::stamp(&key, other, &sample().to_yaml().unwrap()));
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Unauthenticated {
                    failure: AuthFailure::Mismatch,
                    ..
                }
            ),
            "{err}"
        );
    }

    // The forgery check: someone with only the public keys in
    // `.age-recipients` encrypts a well-formed entry to every recipient,
    // with a tag under a key of their own. Every recipient rejects it.
    #[test]
    fn an_entry_produced_without_the_vault_key_is_rejected() {
        let f = Fixture::new(2);
        let id = new_id();
        let forged_key = VaultKey::generate();
        f.plant(
            id,
            &auth::stamp(&forged_key, id, &sample().to_yaml().unwrap()),
        );
        for i in 0..2 {
            let err = f.vault.read_entry(id, &f.identity(i)).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::Unauthenticated {
                        failure: AuthFailure::Mismatch,
                        ..
                    }
                ),
                "recipient {i}: {err}"
            );
        }
        // And a listing doesn't quietly include it.
        assert!(matches!(
            f.vault.list(&f.identity(0)),
            Err(Error::Unauthenticated { .. })
        ));
    }

    // A genuine entry's file, copied under another id (over an existing
    // entry or as a new one), is rejected: the tag names the id.
    #[test]
    fn an_entry_file_placed_under_a_different_id_is_rejected() {
        let f = Fixture::new(1);
        let (id, victim) = (new_id(), new_id());
        f.write(id, &sample()).unwrap();
        f.write(
            victim,
            &Entry {
                title: "victim".into(),
                ..sample()
            },
        )
        .unwrap();
        std::fs::copy(f.entry_file(id), f.entry_file(victim)).unwrap();
        let err = f.vault.read_entry(victim, &f.identity(0)).unwrap_err();
        assert!(
            matches!(&err, Error::Unauthenticated { id: got, failure: AuthFailure::Mismatch } if *got == victim),
            "{err}"
        );
        // The original, under its own id, still reads.
        assert_eq!(f.vault.read_entry(id, &f.identity(0)).unwrap(), sample());
    }

    // Without its key file a v3 vault reads nothing, rather than reading
    // unauthenticated.
    #[test]
    fn read_and_write_without_the_vault_key_fail_typed() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        std::fs::remove_file(f.vault.path.join(crate::crypto::vaultkey::VAULT_KEY_FILE)).unwrap();
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::VaultKey(e) if matches!(**e, crate::crypto::vaultkey::Error::Missing { .. })
            ),
            "{err}"
        );
        assert_eq!(err.kind(), ErrorKind::Conflict);
        let err = f.write(new_id(), &sample()).unwrap_err();
        assert!(matches!(err, Error::VaultKey(_)), "{err}");
    }

    #[test]
    fn auth_is_a_reserved_field_name() {
        let f = Fixture::new(1);
        let e = Entry {
            extra: [("auth".to_string(), Extra::Plain("x".into()))].into(),
            ..sample()
        };
        let err = f.write(new_id(), &e).unwrap_err();
        assert!(matches!(err, Error::ReservedKey("auth")), "{err}");
        assert_eq!(err.kind(), ErrorKind::Usage);
        // A body that carries one, even under a valid tag, is refused too.
        let id = new_id();
        let key = f.vault.vault_key(&f.identity(0)).unwrap();
        f.plant(id, &auth::stamp(&key, id, b"title: t\nauth: x\n"));
        let err = f.vault.read_entry(id, &f.identity(0)).unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err}");
    }

    // A wrong identity still fails on the entry, before the key is
    // looked at.
    #[test]
    fn a_non_recipient_fails_on_the_entry_not_the_key() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        std::fs::remove_file(f.vault.path.join(crate::crypto::vaultkey::VAULT_KEY_FILE)).unwrap();
        let stranger = Identity::for_tests(&age::x25519::Identity::generate());
        assert!(matches!(
            f.vault.read_entry(id, &stranger),
            Err(Error::Crypt(crypt::Error::NotARecipient))
        ));
    }

    #[test]
    fn entry_ids_lists_files_in_the_entries_dir_sorted() {
        let f = Fixture::new(1);
        let mut ids: Vec<Uuid> = (0..5).map(|_| new_id()).collect();
        for id in &ids {
            f.write(*id, &sample()).unwrap();
        }
        ids.sort();
        assert_eq!(f.vault.entry_ids().unwrap(), ids);
    }

    #[test]
    fn entry_ids_skips_subdirectories_and_stray_files() {
        let f = Fixture::new(1);
        let id = new_id();
        f.write(id, &sample()).unwrap();
        let dir = f.vault.entries_dir();
        let other = new_id();
        std::fs::create_dir(dir.join(format!("{other}.age"))).unwrap(); // a directory named like an entry
        std::fs::create_dir(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("README"), "x").unwrap();
        std::fs::write(dir.join("not-a-uuid.age"), "x").unwrap();
        std::fs::write(dir.join(format!("{other}.age.bak")), "x").unwrap();
        std::fs::write(dir.join(format!("{other}.txt")), "x").unwrap();
        // Valid UUIDs in a spelling entry_path would never produce.
        std::fs::write(
            dir.join(format!("{}.age", new_id().to_string().to_uppercase())),
            "x",
        )
        .unwrap();
        std::fs::write(dir.join(format!("{}.age", new_id().simple())), "x").unwrap();
        assert_eq!(f.vault.entry_ids().unwrap(), vec![id]);
    }

    #[test]
    fn entry_ids_without_an_entries_dir_is_an_empty_list() {
        let f = Fixture::new(1);
        assert_eq!(f.vault.entry_ids().unwrap(), Vec::<Uuid>::new());
    }
}
