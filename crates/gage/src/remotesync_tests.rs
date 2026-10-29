//! Sync between a vault and a real bare remote standing in for origin,
//! with a [`gittest::Device`] playing the other machine. The network is never
//! touched: remotes are local paths, and "offline" is either a loopback
//! port refusing connections or an injected [`RemoteSyncer`] returning the
//! error that refusal classifies as.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::config::{self, Global, VaultEntry};
use crate::entry::{NewEntry, WriteError};
use crate::exitcode::{Code, HasCode};
use crate::gittest::{self, BareRemote, Device};
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::syncerr;
use crate::testsupport::fast_scrypt;
use crate::vault::{self, CreateSpec};
use crate::{identityfile, paths, vaultconfig, vaultlock};

pub(crate) const NAME: &str = "personal";
pub(crate) const DEVICE: &str = "laptop-1";
pub(crate) const PASSPHRASE: &str = "correct horse battery staple";

/// A real vault whose origin is a fresh bare repository, its first commit
/// already published, registered on a machine of its own with this
/// device's identity on disk: the state `gage init --remote` leaves.
pub(crate) struct SyncVault {
    _root: tempfile::TempDir,
    pub(crate) remote: BareRemote,
    pub(crate) vault: Vault,
}

impl SyncVault {
    pub(crate) fn new() -> SyncVault {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let remote = gittest::new_bare_remote();
        let id = vaultconfig::new_id();
        let pubkey = identityfile::create(&dirs, &id, NAME, DEVICE, &unlocks()).unwrap();
        let path = root.path().join("vault");
        let origin = remote.path().to_str().unwrap().to_string();
        let vault = Vault::create(
            &dirs,
            &CreateSpec {
                name: NAME.into(),
                id: id.clone(),
                path: path.clone(),
                vault_type: vault::TYPE_GIT.into(),
                method: vault::METHOD_PASSPHRASE.into(),
                device: DEVICE.into(),
                recipients: vec![pubkey],
                extra_recipients: Vec::new(),
                remote: Some(origin.clone()),
            },
        )
        .unwrap();
        let mut global = Global::default();
        global
            .register(
                NAME,
                VaultEntry {
                    path,
                    id,
                    vault_type: vault::TYPE_GIT.into(),
                    device: DEVICE.into(),
                    method: vault::METHOD_PASSPHRASE.into(),
                    git: config::GitMeta { origin },
                    ..Default::default()
                },
            )
            .unwrap();
        config::save(&paths::global_config_file(&dirs), &global).unwrap();
        gitrepo::push(&vault.path, &gittest::remote_options())
            .expect("publishing the first commit");
        SyncVault {
            _root: root,
            remote,
            vault,
        }
    }

    pub(crate) fn unlock(&self) -> Identity {
        self.vault.unlock(&unlocks()).unwrap()
    }

    /// Another device, freshly cloned from the remote.
    pub(crate) fn other(&self) -> Device {
        Device::clone(self.remote.path())
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.vault.path.join(rel)).unwrap()
    }

    fn write(&self, rel: &str, content: &str) {
        std::fs::write(self.vault.path.join(rel), content).unwrap();
    }

    fn head(&self) -> String {
        let repo = git2::Repository::open(&self.vault.path).unwrap();
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string()
    }

    fn commits(&self) -> usize {
        gitrepo::commit_count(&self.vault.path).unwrap()
    }

    /// The vault-relative path of its only entry, as git sees it.
    fn entry_file(&self) -> String {
        let ids = self.vault.entry_ids().unwrap();
        assert_eq!(ids.len(), 1, "want exactly one entry");
        format!("entries/{}.age", ids[0])
    }

    /// The lock file a write would take.
    fn lock_path(&self) -> std::path::PathBuf {
        let path = self.vault.lock_file_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }
}

pub(crate) fn unlocks() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

pub(crate) fn sample(title: &str) -> NewEntry {
    NewEntry {
        title: title.into(),
        value: "hunter2".into(),
        ..NewEntry::default()
    }
}

/// What a refused connection classifies as, by the real classifier: a
/// libgit2 fetch from a loopback port nobody listens on. The offline tests
/// inject exactly this, so the warn-and-proceed path is reached through
/// the same classification a real network failure goes through.
pub(crate) fn unreachable_error() -> syncerr::Error {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("seed.txt"), "seed").unwrap();
    gitrepo::init_and_commit(dir.path(), "seed").unwrap();
    gitrepo::set_remote(dir.path(), &gittest::refused_url()).unwrap();
    match gitrepo::fetch(dir.path(), &gittest::remote_options()) {
        Err(gitrepo::Error::Sync(e)) => e,
        other => panic!("a refused connection gave {other:?}"),
    }
}

/// A push rejected for divergence, by the real classifier's elimination.
pub(crate) fn diverged_error() -> syncerr::Error {
    syncerr::classify_push(&std::io::Error::other(
        "non-fast-forward update: refs/heads/main",
    ))
}

/// The injected [`RemoteSyncer`] for the one situation a local repository
/// can't produce. Every other sync test runs the real one.
#[derive(Default)]
pub(crate) struct FakeSyncer {
    pub(crate) fetch_err: Option<syncerr::Error>,
    pub(crate) push_err: Option<syncerr::Error>,
    pub(crate) fetches: AtomicUsize,
    pub(crate) pushes: AtomicUsize,
}

impl FakeSyncer {
    pub(crate) fn offline() -> Arc<FakeSyncer> {
        Arc::new(FakeSyncer {
            fetch_err: Some(unreachable_error()),
            push_err: Some(unreachable_error()),
            ..FakeSyncer::default()
        })
    }
}

impl RemoteSyncer for FakeSyncer {
    fn fetch(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.fetch_err.clone().map_or(Ok(false), |e| Err(e.into()))
    }

    fn push(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        self.pushes.fetch_add(1, Ordering::SeqCst);
        self.push_err.clone().map_or(Ok(false), |e| Err(e.into()))
    }
}

/// Fetches for real and fails each push with the next error in turn, so a
/// test can drive merge-then-fail.
struct SequencedSyncer {
    dir: std::path::PathBuf,
    push_errs: Vec<syncerr::Error>,
    pushes: AtomicUsize,
}

impl RemoteSyncer for SequencedSyncer {
    fn fetch(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        gitrepo::fetch(&self.dir, &gittest::remote_options())
    }

    fn push(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        let n = self.pushes.fetch_add(1, Ordering::SeqCst);
        Err(self.push_errs[n.min(self.push_errs.len() - 1)]
            .clone()
            .into())
    }
}

// --- the injected offline error --------------------------------------------

// Pins the
// fake's error to the real classification, so the offline tests can't
// quietly stop exercising the path they exist for.
#[test]
fn the_injected_offline_error_is_what_vault_treats_as_unreachable() {
    let err = unreachable_error();
    assert_eq!(err.kind(), syncerr::Kind::Unreachable, "{err}");
    let as_sync = SyncError::new(SyncCause::Remote(err.into()));
    assert_eq!(as_sync.remote_kind(), Some(syncerr::Kind::Unreachable));
    assert_eq!(as_sync.code(), Code::Unreachable);
    assert_eq!(offline_or_error(&as_sync), "could not reach origin");
}

// --- the report -------------------------------------------------------------

// The design doc's "origin has diverged — N local commits pending, run
// `gage sync`", in each of its three shapes, and the classification under
// it: a recipient file outranks an entry.
#[test]
fn summary_says_what_diverged_and_what_to_run() {
    let report = |conflicts: &[&str]| SyncReport {
        diverged: true,
        ahead: 2,
        conflicts: conflicts.iter().map(|c| c.to_string()).collect(),
        ..SyncReport::default()
    };
    let plain = report(&[]);
    assert_eq!(plain.conflict_kind(), ConflictKind::None);
    assert_eq!(
        plain.summary(),
        "origin has diverged — 2 local commit(s) pending; run `gage sync`"
    );

    let entries = report(&["entries/a.age", "entries/b.age"]);
    assert_eq!(entries.conflict_kind(), ConflictKind::Entries);
    assert_eq!(
        entries.summary(),
        "origin has diverged — 2 local commit(s) pending, and 2 entr(y/ies) changed on both sides; run `gage sync`"
    );

    for path in [".age-recipients", ".gage/config.toml"] {
        let both = report(&["entries/a.age", path]);
        assert_eq!(both.conflict_kind(), ConflictKind::Recipients, "{path}");
        let summary = both.summary();
        for want in [path, "who can read this vault", "never merged", "gage sync"] {
            assert!(summary.contains(want), "{want}: {summary}");
        }
    }
}

// --- pull on unlock ---------------------------------------------------------

#[test]
fn unlock_fast_forwards_from_the_remote() {
    let s = SyncVault::new();
    s.other().write_commit_push(
        "notes.txt",
        "from the other device",
        "another device's commit",
    );
    let before = s.commits();

    let report = s.vault.pull().unwrap();

    assert!(report.pulled, "{report:?}");
    assert_eq!(s.commits(), before + 1);
    assert!(s.vault.path.join("notes.txt").exists());
}

// The catch-up is
// tied to Vault::unlock, which a one-shot command's implicit unlock is,
// not to a session's `use`.
#[test]
fn unlock_pulls_without_a_session_covers_the_one_shot_path() {
    let s = SyncVault::new();
    s.other().write_commit_push(
        "notes.txt",
        "from the other device",
        "another device's commit",
    );

    let p = unlocks();
    let _identity = s.vault.unlock(&p).unwrap();

    assert!(
        s.vault.path.join("notes.txt").exists(),
        "unlock did not fast-forward"
    );
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

#[test]
fn unlock_warns_once_and_proceeds_when_the_remote_is_unreachable() {
    let s = SyncVault::new();
    let offline = FakeSyncer::offline();
    let vault = s.vault.clone().with_remote_syncer(offline.clone());

    let p = unlocks();
    vault
        .unlock(&p)
        .expect("an offline unlock must warn and proceed");

    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("could not reach origin"),
        "{}",
        warnings[0]
    );
    assert!(warnings[0].contains("continuing with the local copy"));
    assert!(warnings[0].contains(NAME));
    assert_eq!(offline.fetches.load(Ordering::SeqCst), 1);
}

// Any other failure warns and proceeds too: nothing about reaching a
// remote stands between a human and their own vault.
#[test]
fn unlock_warns_and_proceeds_on_an_auth_failure() {
    let s = SyncVault::new();
    let refused = Arc::new(FakeSyncer {
        fetch_err: Some(syncerr::Error::new(syncerr::Kind::Auth, "401")),
        ..FakeSyncer::default()
    });
    let vault = s.vault.clone().with_remote_syncer(refused);

    let p = unlocks();
    vault.unlock(&p).unwrap();

    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("refused access"), "{}", warnings[0]);
    assert!(!warnings[0].starts_with("gage: gage:"), "{}", warnings[0]);
}

// And a divergence on unlock is reported, never merged: pull is
// fast-forward only.
#[test]
fn unlock_warns_on_a_divergence_and_leaves_it() {
    let s = SyncVault::new();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    s.write("local.txt", "mine");
    gitrepo::commit_all(&s.vault.path, "a local commit").unwrap();
    let head = s.head();

    let p = unlocks();
    s.vault.unlock(&p).unwrap();

    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("diverged"), "{}", warnings[0]);
    assert!(warnings[0].contains("gage sync"), "{}", warnings[0]);
    assert_eq!(s.head(), head, "a pull on unlock merged");
    assert!(!s.vault.path.join("notes.txt").exists());
}

// The doc's "a vault with no remote does nothing": no fetch, no warning,
// and no lock file either.
#[test]
fn unlock_of_a_local_only_vault_touches_no_lock() {
    let s = SyncVault::new();
    let repo = git2::Repository::open(&s.vault.path).unwrap();
    repo.remote_delete("origin").unwrap();
    let counting = Arc::new(FakeSyncer::default());
    let vault = s.vault.clone().with_remote_syncer(counting.clone());

    let p = unlocks();
    vault.unlock(&p).unwrap();

    assert_eq!(counting.fetches.load(Ordering::SeqCst), 0);
    assert!(p.warnings().is_empty());
    assert!(
        !s.vault.lock_file_path().unwrap().exists(),
        "unlocking a local-only vault created its lock file"
    );
}

#[test]
fn pull_that_brings_nothing_reports_no_change() {
    let s = SyncVault::new();
    let report = s.vault.pull().unwrap();
    assert!(!report.pulled, "{report:?}");
    assert!(!report.diverged, "{report:?}");
}

// --- the lock: process-level ------------------------------------------------
//
// The lock is held by a real, separate process (this test binary
// re-executed into `lock_holder_trampoline`): file locks belong to the open
// file description, so a thread-based test passes whether or not the lock
// is real.

const HOLDER_ENV: &str = "GAGE_REMOTESYNC_TEST_LOCK";

/// Set alongside `HOLDER_ENV`: try the lock once instead of holding it.
const PROBE_ENV: &str = "GAGE_REMOTESYNC_TEST_PROBE";

/// The child side: take the lock at `$HOLDER_ENV`, announce it on stderr,
/// and hold it until stdin closes. With `$PROBE_ENV`, try it once instead
/// and report `HELD` or `FREE`.
#[test]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
fn lock_holder_trampoline() {
    let Some(path) = std::env::var_os(HOLDER_ENV) else {
        return;
    };
    if std::env::var_os(PROBE_ENV).is_some() {
        match vaultlock::Lock::try_acquire(Path::new(&path)).unwrap() {
            Some(lock) => {
                lock.release().unwrap();
                eprintln!("FREE");
            }
            None => eprintln!("HELD"),
        }
        return;
    }
    let lock = vaultlock::Lock::acquire(Path::new(&path), Duration::from_secs(5)).unwrap();
    eprintln!("ACQUIRED");
    let _ = std::io::stdin().read(&mut [0u8]);
    lock.release().unwrap();
}

/// Whether another process finds the lock at `path` taken right now: what
/// a concurrent gage write would see first.
#[allow(clippy::disallowed_methods)]
pub(crate) fn lock_is_held_elsewhere(path: &Path) -> bool {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "remotesync::tests::lock_holder_trampoline",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HOLDER_ENV, path)
        .env(PROBE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    match (
        stderr.lines().any(|l| l == "HELD"),
        stderr.lines().any(|l| l == "FREE"),
    ) {
        (true, false) => true,
        (false, true) => false,
        _ => panic!("the lock probe reported neither HELD nor FREE:\n{stderr}"),
    }
}

pub(crate) struct Holder {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Holder {
    /// Starts a process holding the lock at `path`; returns once it has it.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn spawn(path: &Path) -> Holder {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "remotesync::tests::lock_holder_trampoline",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HOLDER_ENV, path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        if !stderr.lines().any(|l| l.unwrap() == "ACQUIRED") {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the holder exited without acquiring the lock");
        }
        let stdin = child.stdin.take();
        Holder { child, stdin }
    }

    fn release(&mut self) {
        drop(self.stdin.take());
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        self.release();
        let _ = self.child.wait();
    }
}

// A manual sync waits
// for the lock; the automatic one on unlock skips, silently, well inside
// the time a write would make it wait.
#[test]
fn unlock_does_not_queue_behind_a_concurrent_write() {
    let s = SyncVault::new();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    let _holder = Holder::spawn(&s.lock_path());

    let p = unlocks();
    let start = Instant::now();
    s.vault.unlock(&p).unwrap();
    let took = start.elapsed();

    assert!(
        took < Duration::from_secs(2) && took < WRITE_LOCK_TIMEOUT,
        "the unlock waited {took:?} on a write lock held elsewhere"
    );
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    assert!(
        !s.vault.path.join("notes.txt").exists(),
        "it pulled without the lock"
    );
}

// A sync is not a read: each verb waits for
// the lock while another process holds it, and goes ahead once it lets go.
#[test]
fn sync_takes_the_write_lock() {
    let s = SyncVault::new();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    let mut holder = Holder::spawn(&s.lock_path());

    let (tx, rx) = std::sync::mpsc::channel();
    for (verb, run) in [
        (
            "pull",
            Vault::pull as fn(&Vault) -> Result<SyncReport, SyncError>,
        ),
        ("push", Vault::push),
        ("sync", Vault::sync),
    ] {
        let (tx, vault) = (tx.clone(), s.vault.clone());
        std::thread::spawn(move || {
            let _ = tx.send((verb, run(&vault).map_err(|e| e.to_string())));
        });
    }
    drop(tx);
    if let Ok((verb, _)) = rx.recv_timeout(Duration::from_millis(300)) {
        panic!("{verb} returned while another process held the write lock");
    }

    holder.release();
    for _ in 0..3 {
        let (verb, result) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        result.unwrap_or_else(|e| panic!("{verb} after the release: {e}"));
    }
    assert!(
        s.vault.path.join("notes.txt").exists(),
        "nothing was pulled"
    );
}

// --- push on write ----------------------------------------------------------

#[test]
fn write_pushes_to_the_remote() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let p = FakePrompter::new();

    s.vault
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();

    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    assert!(
        s.other().exists(&s.entry_file()),
        "the entry never reached the remote"
    );
}

// Every write method pushes, not only insert.
#[test]
fn every_write_pushes() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let p = FakePrompter::new();
    let remote_head = || {
        let repo = git2::Repository::open_bare(s.remote.path()).unwrap();
        repo.refname_to_id("refs/heads/main").unwrap().to_string()
    };

    let id = s.vault.insert(&identity, sample("a"), false, &p).unwrap();
    assert_eq!(remote_head(), s.head(), "insert");
    let mut e = s.vault.read_entry(id, &identity).unwrap();
    e.value = "rotated".into();
    s.vault.update(id, &e, &p).unwrap();
    assert_eq!(remote_head(), s.head(), "update");
    s.vault.rename(&identity, "a", "b", false, &p).unwrap();
    assert_eq!(remote_head(), s.head(), "rename");
    s.vault.remove(&identity, "b", &p).unwrap();
    assert_eq!(remote_head(), s.head(), "remove");
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

#[test]
fn offline_push_keeps_the_commit_and_retries_later() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let offline = s.vault.clone().with_remote_syncer(FakeSyncer::offline());
    let before = s.commits();
    let p = FakePrompter::new();

    offline
        .insert(&identity, sample("ProtonMail"), false, &p)
        .expect("an offline write must still succeed");

    assert_eq!(s.commits(), before + 1, "the commit exists offline");
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("committed locally but not pushed"),
        "{}",
        warnings[0]
    );
    assert!(warnings[0].contains(UNPUSHED_WRITE_CLAUSE));

    // Back online, the next push carries the pending commit.
    let report = s.vault.push().unwrap();
    assert!(report.pushed, "{report:?}");
    assert!(s.other().exists(&s.entry_file()));
}

/// A recipient-list write against `vault` that fails to push: through the
/// batch form with `clause`, or `add_recipient` (which supplies none).
fn recipient_write_warnings(
    s: &SyncVault,
    syncer: Arc<FakeSyncer>,
    clause: Option<&str>,
) -> Vec<String> {
    let identity = s.unlock();
    let vault = s.vault.clone().with_remote_syncer(syncer);
    let p = FakePrompter::new();
    let key = age::x25519::Identity::generate().to_public().to_string();
    match clause {
        Some(clause) => {
            let add = vaultconfig::Recipient {
                device: "phone-1".into(),
                pubkey: key,
            };
            vault
                .with_vault_write(crate::entry::WRITE_LOCK_TIMEOUT, &p, || {
                    vault.add_recipient_locked(
                        &[add],
                        &crate::recipient::RecipientWrite {
                            message: "gage: test",
                            delete_paths: &[],
                            push_clause: Some(clause),
                        },
                        &identity,
                        &p,
                    )
                })
                .unwrap();
        }
        None => {
            vault.add_recipient("phone-1", &key, &identity, &p).unwrap();
        }
    }
    p.warnings()
}

// A supplied clause reaches the
// warning; `add_recipient`, which supplies none, says its own.
#[test]
fn failed_push_clause_is_the_callers_to_say() {
    let own = "the approval is committed locally but not pushed";
    let warnings = recipient_write_warnings(&SyncVault::new(), FakeSyncer::offline(), Some(own));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!("gage: {own}: ")),
        "{}",
        warnings[0]
    );
    assert!(!warnings[0].contains(UNPUSHED_WRITE_CLAUSE));

    let warnings = recipient_write_warnings(&SyncVault::new(), FakeSyncer::offline(), None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!("gage: {UNPUSHED_WRITE_CLAUSE}: ")),
        "{}",
        warnings[0]
    );
}

#[test]
fn diverged_push_ignores_the_supplied_clause() {
    let own = "the approval is committed locally but not pushed";
    let diverged = || {
        Arc::new(FakeSyncer {
            push_err: Some(diverged_error()),
            ..FakeSyncer::default()
        })
    };
    let with = recipient_write_warnings(&SyncVault::new(), diverged(), Some(own));
    let without = recipient_write_warnings(&SyncVault::new(), diverged(), None);

    assert_eq!(with.len(), 1, "{with:?}");
    assert!(with[0].contains("diverged"), "{}", with[0]);
    assert!(!with[0].contains(own), "{}", with[0]);
    assert_eq!(with, without, "the clause changed a divergence's warning");
}

// The automatic push runs
// inside the write's own lock; taking it again would leave the write
// waiting on itself until WRITE_LOCK_TIMEOUT.
#[test]
fn write_does_not_deadlock_on_its_own_push() {
    let s = SyncVault::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let vault = s.vault.clone();
    std::thread::spawn(move || {
        let p = unlocks();
        let identity = vault.unlock(&p).unwrap();
        let r = vault.insert(&identity, sample("x"), false, &p);
        let _ = tx.send((r.map_err(|e| e.to_string()), p.warnings()));
    });
    let (result, warnings) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the write's push is waiting on the lock the write holds");
    result.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

// --- divergence -------------------------------------------------------------

// Two devices, two different
// files: no prompt, no unlock, both sides' work everywhere.
#[test]
fn disjoint_divergence_merges_and_pushes() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.other().write_commit_push(
        "notes.txt",
        "the other device's file",
        "another device's commit",
    );

    let p = FakePrompter::new();
    s.vault
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();

    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    assert!(s.vault.path.join("notes.txt").exists());
    let fresh = s.other();
    assert!(fresh.exists("notes.txt"));
    assert!(fresh.exists(&s.entry_file()));
}

// The sync contract in
// one test: noticed, classified, reported, and not resolved.
#[test]
fn conflicting_divergence_is_detected_and_left_alone() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let p = FakePrompter::new();
    let id = s
        .vault
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();
    let entry_path = s.entry_file();

    s.other().write_commit_push(
        &entry_path,
        "ciphertext written by the other device",
        "the other device's edit",
    );
    let mut e = s.vault.read_entry(id, &identity).unwrap();
    e.value = "rotated-locally".into();
    s.vault.update(id, &e, &p).unwrap();
    let head = s.head();

    let err = s.vault.push().unwrap_err();

    assert_eq!(err.remote_kind(), Some(syncerr::Kind::Diverged), "{err}");
    assert_eq!(err.code(), Code::Conflict);
    assert_eq!(err.report.conflict_kind(), ConflictKind::Entries);
    assert_eq!(err.report.conflicts, vec![entry_path]);
    assert!(err.report.diverged);
    assert_eq!(err.to_string(), format!("gage: {}", err.report.summary()));
    assert_eq!(s.head(), head, "HEAD moved on a conflicting divergence");
    let got = s.vault.read_entry(id, &identity).unwrap();
    assert_eq!(got.value, "rotated-locally", "the local edit was touched");
}

/// A write whose push is rejected for a real conflict: what the human was
/// told.
fn warning_from_conflicting_write() -> String {
    let s = SyncVault::new();
    let identity = s.unlock();
    let id = s
        .vault
        .insert(&identity, sample("ProtonMail"), false, &FakePrompter::new())
        .unwrap();
    s.other().write_commit_push(
        &s.entry_file(),
        "the other device's ciphertext",
        "conflicting edit",
    );

    let p = FakePrompter::new();
    let mut e = s.vault.read_entry(id, &identity).unwrap();
    e.value = "rotated-locally".into();
    s.vault.update(id, &e, &p).unwrap();
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    warnings[0].clone()
}

/// A write whose push can't reach the remote: what the human was told.
fn warning_from_offline_write() -> String {
    let s = SyncVault::new();
    let identity = s.unlock();
    let offline = s.vault.clone().with_remote_syncer(FakeSyncer::offline());
    let p = FakePrompter::new();
    offline
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    warnings[0].clone()
}

// Same failed
// push, two situations, two messages.
#[test]
fn divergence_is_reported_differently_from_being_offline() {
    let diverged = warning_from_conflicting_write();
    let offline = warning_from_offline_write();

    assert_ne!(diverged, offline);
    assert!(diverged.contains("diverged"), "{diverged}");
    assert!(diverged.contains("gage sync"), "{diverged}");
    assert!(!offline.contains("diverged"), "{offline}");
    assert!(offline.contains("could not reach origin"), "{offline}");
    for w in [&diverged, &offline] {
        assert!(
            w.starts_with("gage: ") && !w.starts_with("gage: gage:"),
            "{w}"
        );
    }
}

// Divergence in `.age-recipients` conflicts rather than unions: the security row. Two devices each adding a different
// line would union under a line merge, leaving an access list neither
// wrote. It must conflict.
#[test]
fn recipient_file_divergence_conflicts_rather_than_unioning() {
    assert_recipient_file_conflicts(".age-recipients");
}

// The sibling case: `.gage/config.toml`.
#[test]
fn vault_config_divergence_conflicts_rather_than_unioning() {
    assert_recipient_file_conflicts(".gage/config.toml");
}

fn assert_recipient_file_conflicts(path: &str) {
    let s = SyncVault::new();
    let original = s.read(path);
    s.other().write_commit_push(
        path,
        &format!("{original}\n# added by the other device\n"),
        "the other device's recipient change",
    );
    s.write(path, &format!("{original}\n# added by this device\n"));
    gitrepo::commit_all(&s.vault.path, "this device's recipient change").unwrap();

    let err = s.vault.push().unwrap_err();

    assert_eq!(err.remote_kind(), Some(syncerr::Kind::Diverged), "{err}");
    assert_eq!(err.report.conflict_kind(), ConflictKind::Recipients);
    assert_eq!(err.report.conflicts, vec![path.to_string()]);
    let after = s.read(path);
    assert!(
        !after.contains("added by the other device"),
        "{path} gained the other device's line unreviewed:\n{after}"
    );
    assert!(after.contains("added by this device"), "{after}");

    // `gage sync` refuses this too, so the summary must not send anyone there. It says what to do
    // instead, and what reviews the result.
    let summary = err.report.summary();
    assert!(!summary.contains("run `gage sync`"), "{summary}");
    assert!(summary.contains("reconcile"), "{summary}");
    assert!(summary.contains("gage recipient verify"), "{summary}");
}

// Init writes `.gitattributes` for both recipient files, checked as content. gage's
// merge enforces the rule on its own; this file binds a real `git` a
// human runs inside the vault.
#[test]
fn init_writes_gitattributes_for_both_recipient_files() {
    let s = SyncVault::new();
    let content = s.read(".gitattributes");
    for want in [".age-recipients -merge", ".gage/config.toml -merge"] {
        assert!(
            content.lines().any(|l| l == want),
            "missing {want:?}:\n{content}"
        );
    }
}

#[test]
fn sync_on_a_vault_with_no_remote_does_nothing() {
    let s = SyncVault::new();
    git2::Repository::open(&s.vault.path)
        .unwrap()
        .remote_delete("origin")
        .unwrap();
    for run in [Vault::pull, Vault::push, Vault::sync] {
        assert_eq!(run(&s.vault).unwrap(), SyncReport::default());
    }
}

#[test]
fn sync_reports_a_dirty_work_tree_as_a_conflict_not_an_internal_error() {
    let s = SyncVault::new();
    let identity = s.unlock();
    // Offline for the write, so its commit is still pending afterwards.
    let offline = s.vault.clone().with_remote_syncer(FakeSyncer::offline());
    offline
        .insert(&identity, sample("ProtonMail"), false, &FakePrompter::new())
        .unwrap();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    // Written after the last gage write: gage's own commit sweeps the tree.
    s.write("hand-edited.txt", "half-typed note");

    let err = s.vault.sync().unwrap_err();

    assert!(err.is_dirty_work_tree(), "{err:?}");
    assert_eq!(err.code(), Code::Conflict, "{err}");
    let msg = err.to_string();
    assert!(msg.contains("hand-edited.txt"), "{msg}");
    assert!(
        msg.starts_with("gage: ") && !msg.contains("gitrepo:"),
        "{msg}"
    );
    assert_eq!(s.read("hand-edited.txt"), "half-typed note");
    assert!(!s.other().exists("hand-edited.txt"), "published by a merge");
}

// A merge whose push then
// fails leaves two unpublished commits (the write and the merge);
// reporting 0 would say the work is published when it isn't.
#[test]
fn a_pending_merge_still_reports_what_is_pending() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let offline = s.vault.clone().with_remote_syncer(FakeSyncer::offline());
    offline
        .insert(&identity, sample("ProtonMail"), false, &FakePrompter::new())
        .unwrap();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");

    let sequenced = s
        .vault
        .clone()
        .with_remote_syncer(Arc::new(SequencedSyncer {
            dir: s.vault.path.clone(),
            push_errs: vec![diverged_error(), unreachable_error()],
            pushes: AtomicUsize::new(0),
        }));
    let err = sequenced.push().unwrap_err();

    assert_eq!(err.remote_kind(), Some(syncerr::Kind::Unreachable), "{err}");
    assert!(
        err.report.merged,
        "not the case under test: {:?}",
        err.report
    );
    assert_eq!(err.report.ahead, 2, "{:?}", err.report);
}

// --- the pull observer ------------------------------------------------------

/// A vault clone with a fresh observer flag, and the flag.
fn observed(s: &SyncVault) -> (Vault, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    (s.vault.clone().with_pull_observer(flag.clone()), flag)
}

// A merge raises the observer (the other device's writes
// arrived), and so does a fast-forward inside `sync`. A pull that brings
// nothing does not.
#[test]
fn a_merge_and_a_fast_forward_inside_sync_raise_the_observer() {
    let s = SyncVault::new();
    let (vault, flag) = observed(&s);

    vault.sync().unwrap();
    assert!(!flag.load(Ordering::SeqCst), "nothing moved");

    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    let report = vault.sync().unwrap();
    assert!(report.pulled, "{report:?}");
    assert!(
        flag.swap(false, Ordering::SeqCst),
        "a fast-forward inside sync"
    );

    s.other()
        .write_commit_push("more.txt", "theirs", "their second commit");
    s.write("local.txt", "mine");
    gitrepo::commit_all(&s.vault.path, "a local commit").unwrap();
    let report = vault.push().unwrap();
    assert!(report.merged && report.pushed, "{report:?}");
    assert!(flag.load(Ordering::SeqCst), "a merge");
}

// --- the report through the verbs --------------------------------------------

// ADR-0017: a push with nothing to send says so.
#[test]
fn a_push_with_nothing_to_send_reports_no_push() {
    let s = SyncVault::new();
    assert_eq!(s.vault.push().unwrap(), SyncReport::default());
    assert_eq!(s.vault.sync().unwrap(), SyncReport::default());
}

// A pull is fast-forward only: a divergence is in the report, and nothing
// local moves.
#[test]
fn pull_reports_a_divergence_rather_than_merging_it() {
    let s = SyncVault::new();
    s.other()
        .write_commit_push("notes.txt", "theirs", "their commit");
    s.write("local.txt", "mine");
    gitrepo::commit_all(&s.vault.path, "a local commit").unwrap();
    let head = s.head();

    let report = s.vault.pull().unwrap();

    assert!(report.diverged && !report.pulled, "{report:?}");
    assert_eq!(report.ahead, 1);
    assert_eq!(s.head(), head);
}

// An offline manual verb is an error, unlike the automatic paths, with
// its own exit code.
#[test]
fn an_offline_manual_verb_fails_as_unreachable() {
    let s = SyncVault::new();
    let offline = s.vault.clone().with_remote_syncer(FakeSyncer::offline());
    for run in [Vault::pull, Vault::push, Vault::sync] {
        let err = run(&offline).unwrap_err();
        assert_eq!(err.remote_kind(), Some(syncerr::Kind::Unreachable));
        assert_eq!(err.code(), Code::Unreachable);
    }
}

// A write that fails before its commit pushes nothing.
#[test]
fn a_refused_write_does_not_push() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let counting = Arc::new(FakeSyncer::default());
    let vault = s.vault.clone().with_remote_syncer(counting.clone());
    let p = FakePrompter::new();
    vault.insert(&identity, sample("dup"), false, &p).unwrap();
    assert_eq!(counting.pushes.load(Ordering::SeqCst), 1);

    let err = vault
        .insert(&identity, sample("dup"), false, &p)
        .unwrap_err();
    assert!(matches!(err, WriteError::DuplicateTitle { .. }), "{err}");
    assert_eq!(counting.pushes.load(Ordering::SeqCst), 1);
}
