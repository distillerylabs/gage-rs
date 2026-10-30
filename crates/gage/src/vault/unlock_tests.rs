//! Identity files, `Vault::unlock` and the unlocked `Identity`'s lifetime,
//! against real files under temp roots and real scrypt (at the test work
//! factor).

use std::any::Any;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::crypto::crypt::{self, Recipient};
use crate::crypto::identity::SECRET_KEY_PREFIX;
use crate::crypto::locker::{Locker, MemLocker};
use crate::error::{ErrorKind, HasKind};
use crate::prompter::{FakePrompter, Kind, PromptError, Purpose};
use crate::storage::config::{self, Global, VaultEntry};
use crate::storage::identityfile::{self, MARKER_FILE_NAME, UnlockError};
use crate::storage::paths;
use crate::storage::vaultconfig::VaultId;
use crate::storage::xdgpaths::Dirs;
use crate::testsupport::fast_scrypt;
use crate::vault::{CreateSpec, Vault};

const PASSPHRASE: &str = "correct horse battery staple";

/// One isolated machine: its own config, data and state roots.
struct Machine {
    root: tempfile::TempDir,
    dirs: Dirs,
}

impl Machine {
    fn new() -> Machine {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config").join("gage"),
            data: root.path().join("data").join("gage"),
            state: root.path().join("state").join("gage"),
        };
        Machine { root, dirs }
    }

    /// Writes the global config record `Vault::unlock` reads this device's
    /// name and method from, replacing any other.
    fn register(&self, name: &str, id: &VaultId, device: &str, method: &str) {
        let mut global = Global::default();
        global
            .register(
                name,
                VaultEntry {
                    path: self.root.path().join(name),
                    id: Some(id.clone()),
                    vault_type: crate::storage::vaultconfig::VaultType::Git,
                    device: device.to_string(),
                    method: method.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        config::save_creating_dir(&paths::global_config_file(&self.dirs), &global).unwrap();
    }

    fn vault(&self, name: &str, id: &VaultId) -> Vault {
        Vault::for_tests(&self.dirs, name, id)
    }

    fn create_identity(&self, id: &VaultId, name: &str, device: &str) -> String {
        identityfile::create(
            &self.dirs,
            id,
            name,
            device,
            &FakePrompter::new().with_passphrase(PASSPHRASE),
        )
        .unwrap()
    }

    /// A registered vault with this device's identity on disk: the vault
    /// and the identity's public key.
    fn unlockable(&self, name: &str, device: &str) -> (Vault, String) {
        let id = VaultId::new();
        self.register(name, &id, device, "passphrase");
        let pubkey = self.create_identity(&id, name, device);
        (self.vault(name, &id), pubkey)
    }

    fn identity_path(&self, id: &VaultId, device: &str) -> PathBuf {
        paths::identity_file_path(&self.dirs, id, device).unwrap()
    }

    fn identities_root(&self) -> PathBuf {
        self.dirs.data.join("identities")
    }

    /// The file names in a vault's identities directory, sorted, or `None`
    /// if the directory is gone.
    fn identities_dir_entries(&self, id: &VaultId) -> Option<Vec<String>> {
        let dir = paths::identities_dir(&self.dirs, id);
        let entries = std::fs::read_dir(dir).ok()?;
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        Some(names)
    }
}

fn prompter(passphrases: &[&str]) -> FakePrompter {
    FakePrompter::new().with_passphrases(passphrases)
}

/// A prompter answering `passphrases` in order, then refusing, as a prompter
/// that runs out of answers does.
fn prompter_then_gives_up(passphrases: &[&str]) -> FakePrompter {
    prompter(passphrases).with_unlock_error(PromptError::Cancelled)
}

fn round_trip(id: &crate::crypto::identity::Identity, pubkey: &str) {
    let payload = b"only this identity can read this";
    let ct = crypt::encrypt(payload, &[Recipient::parse(pubkey).unwrap()]).unwrap();
    assert_eq!(crypt::decrypt(&ct, id).unwrap(), payload);
}

fn truncate_identity_file(path: &std::path::Path) {
    let data = std::fs::read(path).unwrap();
    std::fs::write(path, &data[..data.len() / 3]).unwrap();
}

/// The shapes a device name must never take, since it becomes a filename
/// component. They arrive from config any git-writer can edit.
fn unsafe_device_names() -> Vec<String> {
    [
        "../../../../etc/cron.d/x",
        "..",
        ".",
        "sub/dir",
        r"sub\dir",
        "C:evil",
        "with space",
        "UPPER",
    ]
    .into_iter()
    .map(String::from)
    .chain(std::iter::once("a".repeat(200)))
    .collect()
}

// --- Locker fakes -----------------------------------------------------------

/// A locker that succeeds without asking the OS, so the `Drop` contract
/// tests hold on hosts that refuse to lock pages. The
/// real one is covered by `memlock`'s own tests and by
/// `unlock_page_locks_or_warns_but_never_both`.
struct AlwaysLocks;

impl Locker for AlwaysLocks {
    fn lock(&self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn unlock(&self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// Counts lock and unlock calls and records what each one was handed, or
/// fails every lock with `lock_err`.
#[derive(Default)]
struct CountingLocker {
    lock_err: Option<&'static str>,
    locked: Mutex<Vec<Vec<u8>>>,
    unlocked: Mutex<Vec<Vec<u8>>>,
}

impl CountingLocker {
    fn failing(reason: &'static str) -> Arc<CountingLocker> {
        Arc::new(CountingLocker {
            lock_err: Some(reason),
            ..Default::default()
        })
    }

    fn locks(&self) -> usize {
        self.locked.lock().unwrap().len()
    }

    fn unlocks(&self) -> usize {
        self.unlocked.lock().unwrap().len()
    }
}

impl Locker for CountingLocker {
    fn lock(&self, buf: &[u8]) -> io::Result<()> {
        self.locked.lock().unwrap().push(buf.to_vec());
        match self.lock_err {
            Some(reason) => Err(io::Error::other(reason)),
            None => AlwaysLocks.lock(buf),
        }
    }

    fn unlock(&self, buf: &[u8]) -> io::Result<()> {
        self.unlocked.lock().unwrap().push(buf.to_vec());
        match self.lock_err {
            // Nothing was locked; a real locker would fail here, which is
            // exactly what Drop must never reach.
            Some(_) => Err(io::Error::other(
                "asked to unlock a page that was never locked",
            )),
            None => AlwaysLocks.unlock(buf),
        }
    }
}

// --- Identity ---------------------------------------------------------------

// Drop's contract in
// one place: the key is wiped *before* the page lock comes off, so the
// locker's unlock sees nothing but zeros.
#[test]
fn identity_close_releases_the_lock_and_zeroes_the_key() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let counter = Arc::new(CountingLocker::default());
    let v = v.with_locker(counter.clone());

    let id = v.unlock(&prompter(&[PASSPHRASE])).unwrap();
    assert_eq!(counter.locks(), 1, "one lock per unlock");
    assert!(id.page_locked());
    let held = counter.locked.lock().unwrap()[0].clone();
    assert!(
        held.starts_with(SECRET_KEY_PREFIX.as_bytes()),
        "locked something other than the key line"
    );

    drop(id);

    let unlocked = counter.unlocked.lock().unwrap().clone();
    assert_eq!(unlocked.len(), 1, "one page unlock per drop");
    assert_eq!(
        unlocked[0].len(),
        held.len(),
        "unlocked a different range than was locked"
    );
    assert!(
        unlocked[0].iter().all(|&b| b == 0),
        "the key was not wiped before its page lock came off"
    );
}

// Rust runs Drop exactly once by
// construction; what remains to prove is that one drop releases the page
// exactly once.
#[test]
fn identity_close_is_idempotent() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let counter = Arc::new(CountingLocker::default());
    let id = v
        .with_locker(counter.clone())
        .unlock(&prompter(&[PASSPHRASE]))
        .unwrap();
    drop(id);
    assert_eq!(counter.unlocks(), 1);
}

#[test]
fn close_after_a_failed_page_lock_does_not_unlock_anything() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let counter = CountingLocker::failing("nope");
    let id = v
        .with_locker(counter.clone())
        .unlock(&prompter(&[PASSPHRASE]))
        .unwrap();
    drop(id);
    assert_eq!(counter.locks(), 1);
    assert_eq!(
        counter.unlocks(),
        0,
        "unlocked a page whose lock had failed"
    );
}

// A weaker guarantee beats an
// unusable tool.
#[test]
fn page_lock_failure_warns_once_and_proceeds() {
    let m = Machine::new();
    let (v, pubkey) = m.unlockable("personal", "laptop-1");
    let v = v.with_locker(CountingLocker::failing("mlock: cannot allocate memory"));
    let p = prompter(&[PASSPHRASE]);

    let id = v
        .unlock(&p)
        .expect("a page-lock failure aborted the unlock");

    round_trip(&id, &pubkey);
    assert!(!id.page_locked());
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("cannot allocate memory"),
        "{}",
        warnings[0]
    );
    assert!(warnings[0].contains("laptop-1"), "{}", warnings[0]);
}

// Three passphrase
// attempts, still one lock, so one possible warning, structurally.
#[test]
fn page_lock_is_attempted_exactly_once_per_unlock() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let counter = Arc::new(CountingLocker::default());
    let p = prompter(&["wrong", "wrong again", PASSPHRASE]);
    let _id = v.with_locker(counter.clone()).unlock(&p).unwrap();
    assert_eq!(counter.locks(), 1);
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

// Page locking with the real locker: holds
// whether or not this host grants the lock.
#[test]
fn unlock_page_locks_or_warns_but_never_both() {
    let m = Machine::new();
    let (v, pubkey) = m.unlockable("personal", "laptop-1");
    let p = prompter(&[PASSPHRASE]);

    let id = v
        .unlock(&p)
        .expect("unlock must succeed whether or not the page lock is granted");

    match (id.page_locked(), p.warnings().len()) {
        (true, 0) | (false, 1) => {}
        (locked, n) => panic!(
            "page_locked = {locked} with {n} warnings: {:?}",
            p.warnings()
        ),
    }
    round_trip(&id, &pubkey);
}

// What stops the injected
// test lockers from quietly becoming "gage never page-locks at all".
#[test]
fn default_locker_is_the_real_memlock_backed_one() {
    let m = Machine::new();
    let root = tempfile::tempdir().unwrap();
    let spec = CreateSpec {
        name: "personal".into(),
        id: VaultId::new(),
        path: root.path().join("personal"),
        vault_type: crate::storage::vaultconfig::VaultType::Git,
        method: crate::storage::vaultconfig::UnlockMethod::Passphrase,
        device: "laptop-1".into(),
        recipients: vec![age::x25519::Identity::generate().to_public().to_string()],
        extra_recipients: Vec::new(),
        remote: None,
    };
    let created = Vault::create(&m.dirs, &spec).unwrap();
    let entry = VaultEntry {
        path: spec.path.clone(),
        id: Some(spec.id.clone()),
        ..Default::default()
    };
    let resolved = Vault::from_registration(&m.dirs, "personal", &entry).unwrap();
    for (how, v) in [("create", created), ("from_registration", resolved)] {
        let locker = v.locker();
        assert!(
            (&*locker as &dyn Any).is::<MemLocker>(),
            "{how}: not the memlock-backed locker"
        );
    }
}

// --- identity files ---------------------------------------------------------

#[test]
fn create_identity_writes_the_file_with_tight_permissions() {
    let m = Machine::new();
    let id = VaultId::new();
    let p = prompter(&[PASSPHRASE]);

    let pubkey = identityfile::create(&m.dirs, &id, "personal", "laptop-1", &p).unwrap();

    let path = m.identity_path(&id, "laptop-1");
    assert_eq!(
        path,
        m.dirs
            .data
            .join("identities")
            .join(&id)
            .join("laptop-1.age")
    );
    assert!(path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "identity file");
        assert_eq!(mode(path.parent().unwrap()), 0o700, "identities directory");
    }
    Recipient::parse(&pubkey).expect("create returned something that is not a usable recipient");
    let requests = p.unlock_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].purpose,
        Purpose::Create,
        "asked to unlock rather than for a new passphrase"
    );
    assert_eq!(
        (requests[0].vault.as_str(), requests[0].device.as_str()),
        ("personal", "laptop-1")
    );
}

// The plan's "tightened if it exists looser" half of the same row: an
// identities directory left 0755 by an older build or a wide umask is not
// trusted as it stands.
#[cfg(unix)]
#[test]
fn create_identity_tightens_an_existing_looser_directory() {
    use std::os::unix::fs::PermissionsExt;
    let m = Machine::new();
    let id = VaultId::new();
    let dir = paths::identities_dir(&m.dirs, &id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    m.create_identity(&id, "personal", "laptop-1");

    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

// An identity file is passphrase-wrapped to a sole recipient, and
// the work factor read off a real identity file.
#[test]
fn create_identity_file_is_passphrase_wrapped_and_sole_recipient() {
    let m = Machine::new();
    let id = VaultId::new();
    let pubkey = m.create_identity(&id, "personal", "laptop-1");
    let data = std::fs::read(m.identity_path(&id, "laptop-1")).unwrap();
    let text = String::from_utf8_lossy(&data);

    assert!(
        !text.contains(SECRET_KEY_PREFIX),
        "the identity file holds a plaintext key line"
    );
    assert!(
        !text.contains(&pubkey),
        "the identity file leaks its public key"
    );
    let stanzas: Vec<&str> = text.lines().filter(|l| l.starts_with("-> ")).collect();
    assert_eq!(
        stanzas.len(),
        1,
        "want exactly one recipient stanza: {stanzas:?}"
    );
    assert!(stanzas[0].starts_with("-> scrypt "), "{}", stanzas[0]);
    let factor: u8 = stanzas[0]
        .split_whitespace()
        .nth(3)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(factor, crypt::scrypt_work_factor());
}

// Accepting it would write
// a real, long-lived key protected by nothing.
#[test]
fn create_identity_rejects_an_empty_passphrase() {
    let m = Machine::new();
    let id = VaultId::new();
    let err =
        identityfile::create(&m.dirs, &id, "personal", "laptop-1", &prompter(&[""])).unwrap_err();
    assert!(matches!(err, UnlockError::EmptyPassphrase), "{err}");
    assert!(
        !m.identities_root().exists(),
        "something was written despite the refusal"
    );
}

// The decrypted contents are
// what age-keygen writes, so stock `age -d -i <device>.age` can use it.
#[test]
fn identity_file_is_age_keygen_compatible() {
    let m = Machine::new();
    let id = VaultId::new();
    let pubkey = m.create_identity(&id, "personal", "laptop-1");
    let wrapped = std::fs::read(m.identity_path(&id, "laptop-1")).unwrap();

    let plaintext = age::decrypt(&crypt::scrypt_identity(PASSPHRASE), &wrapped).unwrap();
    let text = String::from_utf8(plaintext.clone()).unwrap();
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    let created = lines[0]
        .strip_prefix("# created: ")
        .expect("a # created: comment");
    assert!(
        created.len() == 20 && created.ends_with('Z') && &created[10..11] == "T",
        "not an RFC 3339 UTC timestamp: {created}"
    );
    assert_eq!(lines[1], format!("# public key: {pubkey}"));
    assert!(
        lines[2].starts_with(SECRET_KEY_PREFIX),
        "{}",
        &lines[2][..16]
    );
    assert!(text.ends_with('\n'));

    let file = age::IdentityFile::from_buffer(plaintext.as_slice()).unwrap();
    let recipients = file.to_recipients().unwrap();
    assert_eq!(recipients.len(), 1, "age's own parser sees one key");
    // And the key age derives from it (age-keygen -y's conversion) is the
    // public key gage reported and will list as this device's recipient: a
    // file whose comment and key disagreed would pass everything above.
    let mut derived = Vec::new();
    file.write_recipients_file(&mut derived).unwrap();
    assert_eq!(String::from_utf8(derived).unwrap(), format!("{pubkey}\n"));
}

// Creating an identity file reuses an existing one: never
// overwrite the only copy of a private key; reuse it, and say so.
#[test]
fn create_identity_reuses_an_existing_file() {
    let m = Machine::new();
    let id = VaultId::new();
    let first = m.create_identity(&id, "personal", "laptop-1");
    let before = std::fs::read(m.identity_path(&id, "laptop-1")).unwrap();

    let p = prompter(&[PASSPHRASE]);
    let second = identityfile::create(&m.dirs, &id, "personal", "laptop-1", &p).unwrap();

    assert_eq!(second, first);
    assert_eq!(
        p.unlock_requests()[0].purpose,
        Purpose::Unlock,
        "reuse opens the file, it doesn't choose a passphrase"
    );
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("reusing the existing local identity file"),
        "{}",
        warnings[0]
    );
    assert_eq!(
        std::fs::read(m.identity_path(&id, "laptop-1")).unwrap(),
        before
    );

    m.register("personal", &id, "laptop-1", "passphrase");
    let unlocked = m
        .vault("personal", &id)
        .unlock(&prompter(&[PASSPHRASE]))
        .unwrap();
    assert_eq!(unlocked.recipient(), first);
}

// Never mint a new key
// over, or appear to succeed against, a file it never opened.
#[test]
fn create_identity_reuse_rejects_wrong_passphrase() {
    let m = Machine::new();
    let id = VaultId::new();
    m.create_identity(&id, "personal", "laptop-1");
    let before = std::fs::read(m.identity_path(&id, "laptop-1")).unwrap();

    let err = identityfile::create(
        &m.dirs,
        &id,
        "personal",
        "laptop-1",
        &prompter_then_gives_up(&["a different passphrase"]),
    )
    .unwrap_err();

    assert!(matches!(err, UnlockError::WrongPassphrase(_)), "{err}");
    assert_eq!(
        std::fs::read(m.identity_path(&id, "laptop-1")).unwrap(),
        before
    );
}

#[test]
fn create_identity_refuses_an_unsafe_device_name() {
    let m = Machine::new();
    let err = identityfile::create(
        &m.dirs,
        &VaultId::new(),
        "personal",
        "../../escape",
        &FakePrompter::new(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            UnlockError::UnsafePath(paths::Error::InvalidDeviceName(_))
        ),
        "{err}"
    );
    assert!(!m.identities_root().exists());
}

// An unsafe device name is refused at the one builder
// every caller goes through.
#[test]
fn identity_file_path_refuses_an_unsafe_device_name() {
    let m = Machine::new();
    let id = VaultId::new();
    for device in std::iter::once(String::new()).chain(unsafe_device_names()) {
        let err = paths::identity_file_path(&m.dirs, &id, &device).unwrap_err();
        assert_eq!(err, paths::Error::InvalidDeviceName(device.clone()));
        assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    }
}

#[test]
fn remove_identity_is_the_rollback_path_and_nothing_more() {
    let m = Machine::new();
    let id = VaultId::new();
    m.create_identity(&id, "personal", "laptop-1");
    m.create_identity(&id, "personal", "desktop-2");

    identityfile::remove(&m.dirs, &id, "laptop-1").unwrap();
    assert!(!m.identity_path(&id, "laptop-1").exists());
    assert!(
        m.identity_path(&id, "desktop-2").exists(),
        "took another device's identity with it"
    );

    // Idempotent: a rollback runs where the state is already uncertain.
    identityfile::remove(&m.dirs, &id, "laptop-1").unwrap();

    let err = identityfile::remove(&m.dirs, &id, "../../escape").unwrap_err();
    assert!(matches!(err, UnlockError::UnsafePath(_)), "{err}");

    m.register("personal", &id, "desktop-2", "passphrase");
    m.vault("personal", &id)
        .unlock(&prompter(&[PASSPHRASE]))
        .expect("the surviving identity no longer unlocks");
}

#[test]
fn identities_directory_carries_a_plaintext_marker_naming_the_vault() {
    let m = Machine::new();
    let id = VaultId::new();
    m.create_identity(&id, "personal", "laptop-1");
    assert_eq!(
        m.identities_dir_entries(&id).unwrap(),
        ["laptop-1.age", MARKER_FILE_NAME]
    );
    let dir = paths::identities_dir(&m.dirs, &id);
    let marker = std::fs::read_to_string(dir.join(MARKER_FILE_NAME)).unwrap();
    assert_eq!(marker.trim(), "personal");
}

// Nothing reads the marker to make a decision, the unlock half.
// `has_identity` and `list_identities` carry the same check.
#[test]
fn nothing_reads_the_marker_to_make_a_decision() {
    let m = Machine::new();
    let (v, pubkey) = m.unlockable("personal", "laptop-1");
    let marker = paths::identities_dir(&m.dirs, &v.id).join(MARKER_FILE_NAME);
    std::fs::remove_file(&marker).unwrap();

    let unlocked = v
        .unlock(&prompter(&[PASSPHRASE]))
        .expect("unlock without the marker");
    assert_eq!(unlocked.recipient(), pubkey);
    assert!(!marker.exists(), "a read path recreated the marker");
}

#[test]
fn removing_the_last_identity_leaves_no_orphaned_directory() {
    let m = Machine::new();
    let id = VaultId::new();
    m.create_identity(&id, "personal", "laptop-1");
    identityfile::remove(&m.dirs, &id, "laptop-1").unwrap();
    assert_eq!(m.identities_dir_entries(&id), None);
}

#[test]
fn removing_a_non_last_identity_leaves_the_marker_in_place() {
    let m = Machine::new();
    let id = VaultId::new();
    for device in ["laptop-1", "desktop-2"] {
        m.create_identity(&id, "personal", device);
    }
    identityfile::remove(&m.dirs, &id, "laptop-1").unwrap();
    assert_eq!(
        m.identities_dir_entries(&id).unwrap(),
        ["desktop-2.age", MARKER_FILE_NAME]
    );
}

// --- identityfile::backup / restore -----------------------------------------

// Nothing to hold, so nothing to put back: `recovery enroll` on a machine
// whose identity file is already gone.
#[test]
fn backup_of_a_missing_file_is_none() {
    let m = Machine::new();
    let id = VaultId::new();
    assert!(
        identityfile::backup(&m.dirs, &id, "laptop-1")
            .unwrap()
            .is_none()
    );
    let err = identityfile::backup(&m.dirs, &id, "../../escape").unwrap_err();
    assert!(matches!(err, UnlockError::UnsafePath(_)), "{err}");
    assert!(!m.identities_root().exists(), "a backup wrote something");
}

// Regression, tested where the fix lives: removing the vault's only
// identity prunes the marker and the directory, so a restore that only
// wrote the file back failed, and destroyed the file it existed to keep.
#[test]
fn restore_recreates_a_pruned_identity_directory() {
    let m = Machine::new();
    let (v, pubkey) = m.unlockable("personal", "laptop-1");
    let path = m.identity_path(&v.id, "laptop-1");
    let original = std::fs::read(&path).unwrap();

    let backup = identityfile::backup(&m.dirs, &v.id, "laptop-1")
        .unwrap()
        .expect("the file is there");
    identityfile::remove(&m.dirs, &v.id, "laptop-1").unwrap();
    assert_eq!(m.identities_dir_entries(&v.id), None, "not pruned");

    backup.restore().unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        m.identities_dir_entries(&v.id).unwrap(),
        ["laptop-1.age", MARKER_FILE_NAME]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "identity file");
        assert_eq!(mode(path.parent().unwrap()), 0o700, "identities directory");
    }
    let unlocked = v
        .unlock(&prompter(&[PASSPHRASE]))
        .expect("the restored file no longer opens");
    assert_eq!(unlocked.recipient(), pubkey);
}

// --- Vault::unlock ----------------------------------------------------------

// The core property at the library level: unlock yields an identity that
// decrypts.
#[test]
fn unlock_produces_an_identity_that_decrypts() {
    let m = Machine::new();
    let (v, pubkey) = m.unlockable("personal", "laptop-1");
    let id = v.unlock(&prompter(&[PASSPHRASE])).unwrap();
    assert_eq!(id.device(), "laptop-1");
    assert_eq!(id.recipient(), pubkey);
    round_trip(&id, &pubkey);
}

#[test]
fn unlock_requests_the_passphrase_through_the_prompter() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let p = prompter(&[PASSPHRASE]);
    v.unlock(&p).unwrap();
    let requests = p.unlock_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.kind, Kind::Passphrase);
    assert_eq!(req.purpose, Purpose::Unlock);
    assert_eq!(
        (req.vault.as_str(), req.device.as_str()),
        ("personal", "laptop-1")
    );
    assert_eq!(req.attempt, 1);
}

#[test]
fn unlock_with_the_wrong_passphrase_is_distinguishable() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let err = v
        .unlock(&prompter_then_gives_up(&["not the passphrase"]))
        .unwrap_err();
    assert!(matches!(err, UnlockError::WrongPassphrase(_)), "{err}");
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
}

// The library
// keeps no count; the prompter decides when to stop.
#[test]
fn unlock_retries_through_the_prompter_and_stops_when_it_stops() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");

    // A later attempt succeeds.
    let p = prompter(&["wrong", "also wrong", PASSPHRASE]);
    v.unlock(&p).unwrap();
    let attempts: Vec<u32> = p.unlock_requests().iter().map(|r| r.attempt).collect();
    assert_eq!(attempts, [1, 2, 3]);

    // The prompter giving up ends the loop, and its reason survives.
    let p = prompter_then_gives_up(&["wrong", "still wrong"]);
    let err = v.unlock(&p).unwrap_err();
    assert!(
        matches!(err, UnlockError::WrongPassphrase(PromptError::Cancelled)),
        "{err:?}"
    );
    assert_eq!(p.unlock_requests().len(), 3, "2 answers + 1 refusal");
}

#[test]
fn unlock_does_not_retry_on_non_passphrase_failures() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    truncate_identity_file(&m.identity_path(&v.id, "laptop-1"));
    let p = prompter(&["a", "b", "c", PASSPHRASE]);
    v.unlock(&p).unwrap_err();
    assert_eq!(p.unlock_requests().len(), 1);
}

#[test]
fn unlock_with_a_corrupt_identity_file_is_distinguishable() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    truncate_identity_file(&m.identity_path(&v.id, "laptop-1"));
    let err = v.unlock(&prompter(&[PASSPHRASE])).unwrap_err();
    assert!(matches!(err, UnlockError::CorruptIdentityFile(_)), "{err}");
    assert_eq!(err.kind(), ErrorKind::Conflict);
}

// A valid age file wrapped to a public key fails exactly like a bad
// passphrase inside age, so gage must look at the header to avoid
// re-prompting forever for a passphrase that cannot work.
#[test]
fn unlock_with_a_well_formed_but_wrong_kind_of_file_is_corrupt_not_wrong_passphrase() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let stranger = age::x25519::Identity::generate().to_public().to_string();
    let ct = crypt::encrypt(
        b"a valid age file wrapped to a public key, not a passphrase",
        &[Recipient::parse(&stranger).unwrap()],
    )
    .unwrap();
    std::fs::write(m.identity_path(&v.id, "laptop-1"), ct).unwrap();

    let p = prompter(&[PASSPHRASE, PASSPHRASE, PASSPHRASE]);
    let err = v.unlock(&p).unwrap_err();
    assert!(matches!(err, UnlockError::CorruptIdentityFile(_)), "{err}");
    assert_eq!(p.unlock_requests().len(), 1);
}

// A file claiming more scrypt work than gage will do is corrupt, not a
// wrong passphrase (the ADR-0022 classification): re-prompting can't
// lower the claimed factor, so it fails after one prompt, at once. This is
// the behavioral half of the cap the source scans check; age refuses the
// stanza on its logN alone, before any key derivation, so a hostile file
// claiming 2^30 costs nothing.
#[test]
fn unlock_of_a_file_claiming_excessive_work_is_corrupt_not_wrong_passphrase() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let path = m.identity_path(&v.id, "laptop-1");
    let original = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    let excessive = crypt::SCRYPT_MAX_WORK_FACTOR + 1;

    // Only the stanza's logN changes; the salt, body and MAC stay as
    // written.
    let mut rewrote = false;
    let damaged: Vec<u8> = std::fs::read(&path)
        .unwrap()
        .split_inclusive(|&b| b == b'\n')
        .flat_map(|line| match std::str::from_utf8(line) {
            Ok(text) if text.starts_with("-> scrypt ") => {
                let mut fields: Vec<&str> = text.trim_end().split(' ').collect();
                let factor = excessive.to_string();
                *fields.last_mut().unwrap() = &factor;
                rewrote = true;
                format!("{}\n", fields.join(" ")).into_bytes()
            }
            _ => line.to_vec(),
        })
        .collect();
    assert!(rewrote, "no scrypt stanza in:\n{original}");
    std::fs::write(&path, &damaged).unwrap();

    let p = prompter(&[PASSPHRASE, PASSPHRASE, PASSPHRASE]);
    let start = std::time::Instant::now();
    let err = v.unlock(&p).unwrap_err();

    assert!(
        matches!(err, UnlockError::CorruptIdentityFile(_)),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(p.unlock_requests().len(), 1, "re-prompted for a passphrase");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "the claimed factor was worked rather than refused"
    );
}

// A file that decrypts but holds no key line is corrupt too, and so is one whose key line doesn't
// parse.
#[test]
fn unlock_of_a_file_without_a_usable_key_line_is_corrupt() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    for (what, plaintext) in [
        (
            "no key line",
            "# created: 2026-01-01T00:00:00Z\n# just comments\n".to_string(),
        ),
        ("bad key line", format!("{SECRET_KEY_PREFIX}NOTBECH32\n")),
    ] {
        let wrapped =
            crypt::encrypt(plaintext.as_bytes(), &[Recipient::passphrase(PASSPHRASE)]).unwrap();
        std::fs::write(m.identity_path(&v.id, "laptop-1"), wrapped).unwrap();
        let err = v.unlock(&prompter(&[PASSPHRASE])).unwrap_err();
        assert!(
            matches!(err, UnlockError::CorruptIdentityFile(_)),
            "{what}: {err}"
        );
    }
}

// Refused outright, not retried,
// since an empty answer means the prompter has nothing to give.
#[test]
fn unlock_rejects_an_empty_passphrase() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let p = prompter(&[""]);
    let err = v.unlock(&p).unwrap_err();
    assert!(matches!(err, UnlockError::EmptyPassphrase), "{err}");
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    assert_eq!(p.unlock_requests().len(), 1);
}

fn assert_no_local_identity(err: UnlockError) {
    assert!(
        matches!(
            err,
            UnlockError::NoLocalIdentity(_) | UnlockError::NoIdentityFile(_)
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
}

#[test]
fn unlock_with_no_local_identity_is_distinguishable() {
    // No identity file.
    let m = Machine::new();
    let id = VaultId::new();
    m.register("personal", &id, "laptop-1", "passphrase");
    assert_no_local_identity(
        m.vault("personal", &id)
            .unlock(&prompter(&[PASSPHRASE]))
            .unwrap_err(),
    );

    // The vault isn't registered on this machine.
    let other = VaultId::new();
    assert_no_local_identity(
        m.vault("some-other-vault", &other)
            .unlock(&prompter(&[PASSPHRASE]))
            .unwrap_err(),
    );

    // No global config at all.
    let bare = Machine::new();
    assert_no_local_identity(
        bare.vault("personal", &id)
            .unlock(&prompter(&[PASSPHRASE]))
            .unwrap_err(),
    );
}

// (ADR-0011): the vault's [method].default is a suggestion for
// devices joining it; this device's own record decides.
#[test]
fn unlock_dispatches_on_the_local_method_not_the_vault_default() {
    let m = Machine::new();
    let root = tempfile::tempdir().unwrap();
    let spec = CreateSpec {
        name: "personal".into(),
        id: VaultId::new(),
        path: root.path().join("personal"),
        vault_type: crate::storage::vaultconfig::VaultType::Git,
        method: crate::storage::vaultconfig::UnlockMethod::Passphrase,
        device: "laptop-1".into(),
        recipients: vec![age::x25519::Identity::generate().to_public().to_string()],
        extra_recipients: Vec::new(),
        remote: None,
    };
    let v = Vault::create(&m.dirs, &spec).unwrap();
    m.register("personal", &spec.id, "laptop-1", "yubikey");

    let p = FakePrompter::new();
    let err = v.unlock(&p).unwrap_err();

    assert!(
        matches!(&err, UnlockError::UnsupportedMethod { method, .. } if method == "yubikey"),
        "{err}"
    );
    assert!(err.to_string().contains("yubikey"), "{err}");
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    assert!(
        p.unlock_requests().is_empty(),
        "prompted despite the method not being passphrase"
    );
}

// Unlock never builds a path from an unvalidated device name (ADR-0010).
// The assertion is the refusal variant, not merely failure: with
// validation removed, the traversed path wouldn't exist and the unlock
// would fail as NoLocalIdentity, so a weaker test would pass while gage
// opened a path outside identities/.
#[test]
fn unlock_never_builds_a_path_from_an_unvalidated_device_name() {
    for device in unsafe_device_names() {
        let m = Machine::new();
        let id = VaultId::new();
        m.register("personal", &id, &device, "passphrase");

        let err = m
            .vault("personal", &id)
            .unlock(&prompter(&[PASSPHRASE]))
            .unwrap_err();

        assert!(
            matches!(&err, UnlockError::UnsafePath(paths::Error::InvalidDeviceName(d)) if *d == device),
            "{device:?}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
        assert!(
            !m.identities_root().exists(),
            "{device:?}: unlock created something under identities/"
        );
    }
}

// An
// empty device never reaches the path builder.
#[test]
fn unlock_with_no_recorded_device_is_a_missing_identity_not_an_unsafe_name() {
    let m = Machine::new();
    let id = VaultId::new();
    m.register("personal", &id, "", "passphrase");
    assert_no_local_identity(
        m.vault("personal", &id)
            .unlock(&prompter(&[PASSPHRASE]))
            .unwrap_err(),
    );
}

// A frontend shows a warning as it is (the terminal prompter
// prints it verbatim), so each one names gage itself, exactly once.
#[test]
fn warnings_leave_gages_name_to_the_frontend() {
    let m = Machine::new();
    let (v, _) = m.unlockable("personal", "laptop-1");
    let vault_id = v.id.clone();
    let p = prompter(&[PASSPHRASE, PASSPHRASE]);
    let _id = v
        .with_locker(CountingLocker::failing("mlock: cannot allocate memory"))
        .unlock(&p)
        .unwrap();
    // The reuse warning: create finds this device's file already there.
    identityfile::create(&m.dirs, &vault_id, "personal", "laptop-1", &p).unwrap();

    let warnings = p.warnings();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    for w in &warnings {
        assert!(!w.contains("gage: "), "names gage itself: {w}");
    }
}
