//! `Session` against real vaults, real identity files and real scrypt (at
//! the test work factor). Ports Go's `internal/gage/session_test.go`, less
//! the four `EntryTitleCandidates` rows (M13); each test names its Go row.
//!
//! Every vault's page locks go through a [`PageLedger`], a `Locker` that
//! keeps the set of locked pages the way the OS does (no reference counts:
//! one unlock releases a page however many keys share it). "Every key was
//! dropped" is then "the ledger is empty", and "two keys don't share a
//! page" is checkable on any platform rather than only where Windows
//! happens to complain.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{self, Global, RegistryError, VaultEntry};
use crate::entry::{NewEntry, ResolveError};
use crate::exitcode::{Code, HasCode};
use crate::locker::Locker;
use crate::memlock;
use crate::paths;
use crate::prompter::{FakePrompter, PromptError};
use crate::session::{OpenError, Session, SessionConfig, SessionError};
use crate::testsupport::fast_scrypt;
use crate::vault::{self, CreateSpec, Vault};
use crate::xdgpaths::Dirs;
use crate::{identityfile, vaultconfig};

pub(crate) const PASSPHRASE: &str = "correct horse battery staple";
pub(crate) const DEVICE: &str = "laptop-1";

/// The set of pages currently locked, and every lock call's pages in order.
#[derive(Default)]
pub(crate) struct PageLedger {
    live: Mutex<BTreeSet<usize>>,
    locks: Mutex<Vec<BTreeSet<usize>>>,
}

fn pages(buf: &[u8]) -> BTreeSet<usize> {
    let page = memlock::page_size();
    let start = buf.as_ptr() as usize / page;
    let end = (buf.as_ptr() as usize + buf.len().max(1) - 1) / page;
    (start..=end).collect()
}

impl PageLedger {
    pub(crate) fn live(&self) -> BTreeSet<usize> {
        self.live.lock().unwrap().clone()
    }

    pub(crate) fn locks(&self) -> Vec<BTreeSet<usize>> {
        self.locks.lock().unwrap().clone()
    }
}

impl Locker for PageLedger {
    fn lock(&self, buf: &[u8]) -> io::Result<()> {
        let p = pages(buf);
        self.live.lock().unwrap().extend(p.iter().copied());
        self.locks.lock().unwrap().push(p);
        Ok(())
    }

    fn unlock(&self, buf: &[u8]) -> io::Result<()> {
        let mut live = self.live.lock().unwrap();
        for p in pages(buf) {
            live.remove(&p);
        }
        Ok(())
    }
}

/// One simulated machine holding several real vaults (Go's
/// `newSessionDevice`): one set of roots, one identity per vault, and a
/// global-config registration for each.
pub(crate) struct Device {
    pub(crate) _root: tempfile::TempDir,
    pub(crate) vaults: BTreeMap<String, Vault>,
    pub(crate) ledger: Arc<PageLedger>,
}

impl Device {
    pub(crate) fn new(names: &[&str]) -> Device {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let ledger = Arc::new(PageLedger::default());
        let mut global = Global::default();
        let mut vaults = BTreeMap::new();
        for name in names {
            let id = vaultconfig::new_id();
            let path = root.path().join(name);
            let pubkey = identityfile::create(
                &dirs,
                &id,
                name,
                DEVICE,
                &FakePrompter::new().with_passphrase(PASSPHRASE),
            )
            .unwrap();
            let v = Vault::create(
                &dirs,
                &CreateSpec {
                    name: name.to_string(),
                    id: id.clone(),
                    path: path.clone(),
                    vault_type: vault::TYPE_GIT.into(),
                    method: vault::METHOD_PASSPHRASE.into(),
                    device: DEVICE.into(),
                    recipients: vec![pubkey],
                    remote: None,
                },
            )
            .unwrap();
            global
                .register(
                    name,
                    VaultEntry {
                        path,
                        id,
                        vault_type: vault::TYPE_GIT.into(),
                        device: DEVICE.into(),
                        method: vault::METHOD_PASSPHRASE.into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            vaults.insert(name.to_string(), v.with_locker(ledger.clone()));
        }
        // No `current`: each test seeds the session's own.
        global.current.clear();
        config::save(&paths::global_config_file(&dirs), &global).unwrap();
        Device {
            _root: root,
            vaults,
            ledger,
        }
    }

    pub(crate) fn open(&self, name: &str) -> Result<Vault, OpenError> {
        self.vaults
            .get(name)
            .cloned()
            .ok_or_else(|| OpenError::new(RegistryError::NotRegistered(name.to_string())))
    }

    pub(crate) fn session<'a>(&'a self, prompter: &'a FakePrompter, cfg: Cfg) -> Session<'a> {
        Session::new(SessionConfig {
            open: Box::new(move |name| self.open(name)),
            prompter,
            idle_timeout: cfg.idle_timeout,
            current: cfg.current.map(str::to_string),
            now: cfg
                .clock
                .map(|c| Box::new(move || c.now()) as Box<dyn Fn() -> Instant>),
        })
    }
}

/// The parts of a `SessionConfig` a test varies.
#[derive(Default)]
pub(crate) struct Cfg {
    pub(crate) idle_timeout: Option<Duration>,
    pub(crate) current: Option<&'static str>,
    pub(crate) clock: Option<Clock>,
}

/// A clock a test moves by hand instead of waiting for.
#[derive(Clone)]
pub(crate) struct Clock(Rc<Cell<Instant>>);

impl Clock {
    pub(crate) fn new() -> Clock {
        Clock(Rc::new(Cell::new(Instant::now())))
    }

    fn now(&self) -> Instant {
        self.0.get()
    }

    pub(crate) fn advance(&self, d: Duration) {
        self.0.set(self.0.get() + d);
    }
}

/// A prompter answering `n` unlocks; a further one fails the test.
pub(crate) fn unlocks(n: usize) -> FakePrompter {
    FakePrompter::new().with_passphrases(&vec![PASSPHRASE; n])
}

/// How many times `p` was asked to unlock `vault`.
pub(crate) fn unlock_count(p: &FakePrompter, vault: &str) -> usize {
    p.unlock_requests()
        .iter()
        .filter(|r| r.vault == vault)
        .count()
}

impl Device {
    /// Inserts one entry per title into `vault` through a one-shot unlock,
    /// before any session exists, and returns their ids in order.
    pub(crate) fn seed(&self, vault: &str, titles: &[&str]) -> Vec<uuid::Uuid> {
        let v = self.open(vault).unwrap();
        let ident = v.unlock(&unlocks(1)).unwrap();
        titles
            .iter()
            .map(|title| {
                v.insert(
                    &ident,
                    NewEntry {
                        title: title.to_string(),
                        value: format!("v-{title}"),
                        ..NewEntry::default()
                    },
                    true,
                )
                .unwrap()
            })
            .collect()
    }
}

pub(crate) fn current_personal() -> Cfg {
    Cfg {
        current: Some("personal"),
        ..Cfg::default()
    }
}

// --- use and reuse ----------------------------------------------------------

// Go: TestSessionUseUnlocksOnceThenReusesTheKey. The unlock is paid once
// per vault per session, not once per command.
#[test]
fn session_use_unlocks_once_then_reuses_the_key() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, Cfg::default());

    s.use_vault("personal").unwrap();
    for _ in 0..3 {
        s.vault(None).unwrap();
    }
    s.use_vault("personal").unwrap();

    assert_eq!(unlock_count(&p, "personal"), 1);
    assert_eq!(s.current(), Some("personal"));
}

// Go: TestSessionUseWithEmptyNameSaysSo
#[test]
fn session_use_with_empty_name_says_so() {
    let d = Device::new(&["personal"]);
    let p = FakePrompter::new();
    let mut s = d.session(&p, Cfg::default());

    let err = s.use_vault("").unwrap_err();
    assert!(matches!(err, SessionError::NoCurrentVault { .. }), "{err}");
    assert_eq!(err.code(), Code::Usage);
}

// Go: TestSessionWithNoCurrentVaultSaysSo. Reported before any prompt:
// there is no vault whose passphrase could be asked for.
#[test]
fn session_with_no_current_vault_says_so() {
    let d = Device::new(&["personal"]);
    let p = FakePrompter::new();
    let mut s = d.session(&p, Cfg::default());

    let err = s.vault(None).unwrap_err();
    assert!(matches!(err, SessionError::NoCurrentVault { .. }), "{err}");
    assert_eq!(err.code(), Code::Usage);
    assert!(p.unlock_requests().is_empty());
}

// An unknown name fails in the opener, before any prompt, keeping the
// opener's own error and code.
#[test]
fn session_open_failure_keeps_the_openers_code_and_never_prompts() {
    let d = Device::new(&["personal"]);
    let p = FakePrompter::new();
    let mut s = d.session(&p, Cfg::default());

    let err = s.use_vault("nope").unwrap_err();
    assert!(matches!(err, SessionError::Open(_)), "{err}");
    assert_eq!(err.code(), Code::NotFound);
    assert!(err.to_string().contains("nope"), "{err}");
    assert_eq!(s.current(), None, "a failed use selects nothing");
    assert!(
        s.status().is_empty(),
        "a vault that never opened isn't held"
    );
}

// Go: TestSessionAdHocVaultDoesNotChangeCurrent. The `-u/--use` path:
// another vault, unlocked and used, without moving the session.
#[test]
fn session_ad_hoc_vault_does_not_change_current() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());

    s.use_vault("personal").unwrap();
    let (v, _) = s.vault(Some("work")).unwrap();
    assert_eq!(v.name, "work");
    assert_eq!(unlock_count(&p, "work"), 1);
    assert_eq!(s.current(), Some("personal"));

    let (v, _) = s.vault(None).unwrap();
    assert_eq!(v.name, "personal", "the next bare call goes to current");
}

// A current vault seeded from config is selected, not unlocked: the first
// call that needs its key unlocks it.
#[test]
fn a_seeded_current_vault_is_unlocked_by_its_first_use() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    assert_eq!(s.current(), Some("personal"));
    assert!(p.unlock_requests().is_empty());
    s.vault(None).unwrap();
    assert_eq!(unlock_count(&p, "personal"), 1);
}

// --- the same Vault methods -------------------------------------------------

// Go: TestSessionCallsTheSameVaultMethodsAsOneShot. A round trip across
// the two modes: written through a session-held identity, read back
// through a freshly unlocked one, so a session-only path can't satisfy it.
#[test]
fn session_calls_the_same_vault_methods_as_one_shot() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let (v, ident) = s.vault(None).unwrap();
    let id = v
        .insert(
            ident,
            NewEntry {
                title: "ProtonMail".into(),
                value: "correct horse".into(),
                ..NewEntry::default()
            },
            false,
        )
        .unwrap();

    let vault = d.open("personal").unwrap();
    let one_shot = vault.unlock(&unlocks(1)).unwrap();
    let got = vault.read_entry(id, &one_shot).unwrap();
    assert_eq!(got.title, "ProtonMail");
    assert_eq!(got.value, "correct horse");
}

// Go: TestSessionAddsNoDuplicateVaultMethods. Go parses the package for
// a Session method named like a Vault CRUD verb. Here a method of the same
// name on Vault twice can't compile, so what's left is that session.rs
// re-implements none of them and adds nothing to Vault; the fn-pointer
// bindings prove each verb really is a Vault method, so the scan isn't
// checking names that no longer exist.
#[test]
fn session_adds_no_duplicate_vault_methods() {
    let _ = (
        Vault::insert,
        Vault::read_entry,
        Vault::write_entry,
        Vault::remove,
        Vault::rename,
        Vault::update,
        Vault::entry_ids,
        Vault::unlock,
    );
    let source = include_str!("session.rs");
    assert!(
        source.contains("pub struct Session"),
        "scanning the wrong file"
    );
    assert!(
        !source.contains("impl Vault"),
        "session.rs adds methods to Vault"
    );
    for verb in [
        "insert",
        "read_entry",
        "write_entry",
        "remove",
        "rename",
        "update",
        "entry_ids",
        "unlock",
    ] {
        for decl in [format!("fn {verb}("), format!("fn {verb}<")] {
            assert!(
                !source.contains(&decl),
                "session.rs declares `{decl}`: session mode must route to Vault's, not re-implement it"
            );
        }
    }
}

// --- lock and status --------------------------------------------------------

// Go: TestSessionLockDropsOneVaultAndLeavesOthersAlone
#[test]
fn session_lock_drops_one_vault_and_leaves_others_alone() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(3);
    let mut s = d.session(&p, Cfg::default());

    s.use_vault("personal").unwrap();
    s.use_vault("work").unwrap();
    s.lock(Some("personal")).unwrap();

    s.vault(Some("work")).unwrap();
    assert_eq!(
        unlock_count(&p, "work"),
        1,
        "locking personal left work alone"
    );

    s.vault(Some("personal")).unwrap();
    assert_eq!(unlock_count(&p, "personal"), 2, "re-prompted after lock");
}

// Go: TestSessionLockWithNoVaultLocksAll
#[test]
fn session_lock_with_no_vault_locks_all() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(4);
    let mut s = d.session(&p, Cfg::default());

    s.use_vault("personal").unwrap();
    s.use_vault("work").unwrap();
    s.lock(None).unwrap();

    assert!(s.status().iter().all(|st| !st.unlocked));
    assert!(d.ledger.live().is_empty(), "a key is still page-locked");

    s.vault(Some("personal")).unwrap();
    s.vault(Some("work")).unwrap();
    assert_eq!(unlock_count(&p, "personal"), 2);
    assert_eq!(unlock_count(&p, "work"), 2);
}

// Go: TestSessionLockUnknownVaultReports. `lock nope` is a mistake to
// report, not a silent no-op.
#[test]
fn session_lock_unknown_vault_reports() {
    let d = Device::new(&["personal"]);
    let p = FakePrompter::new();
    let mut s = d.session(&p, Cfg::default());

    let err = s.lock(Some("nope")).unwrap_err();
    assert!(
        matches!(err, SessionError::NotHeld(ref n) if n == "nope"),
        "{err}"
    );
    assert_eq!(err.code(), Code::NotFound);
}

// Go: TestSessionStatusReportsMultipleVaults
#[test]
fn session_status_reports_multiple_vaults() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());
    assert!(s.status().is_empty(), "a fresh session holds nothing");

    s.use_vault("personal").unwrap();
    s.use_vault("work").unwrap();
    s.lock(Some("personal")).unwrap();

    let got = s.status();
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].name, "personal");
    assert!(!got[0].unlocked);
    assert_eq!(got[1].name, "work");
    assert!(got[1].unlocked);
    assert!(got[1].current, "work is current after `use work`");
    assert!(!got[0].current);
    assert!(got[1].idle.is_some(), "an unlocked vault has a last use");
}

// Go: TestSessionCloseClosesEveryHeldIdentity. Nothing holds key material
// once the session ends, and closing twice is safe.
#[test]
fn session_close_closes_every_held_identity() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());

    s.use_vault("personal").unwrap();
    s.use_vault("work").unwrap();
    assert_eq!(d.ledger.locks().len(), 2);
    assert!(!d.ledger.live().is_empty());

    s.close();
    assert!(d.ledger.live().is_empty(), "close left a key page-locked");
    s.close();
    assert!(s.status().iter().all(|st| !st.unlocked));
}

// Dropping a session drops its keys too: close is for saying so
// explicitly, not the only way out.
#[test]
fn dropping_a_session_drops_its_keys() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, Cfg::default());
    s.use_vault("personal").unwrap();
    assert!(!d.ledger.live().is_empty());
    drop(s);
    assert!(d.ledger.live().is_empty());
}

// --- idle timeout -----------------------------------------------------------

fn timed(clock: &Clock, timeout: Duration) -> Cfg {
    Cfg {
        idle_timeout: Some(timeout),
        clock: Some(clock.clone()),
        ..Cfg::default()
    }
}

// Go: TestSessionIdleTimeoutRelocksAndRePrompts
#[test]
fn session_idle_timeout_relocks_and_re_prompts() {
    let d = Device::new(&["personal"]);
    let p = unlocks(2);
    let clock = Clock::new();
    let mut s = d.session(&p, timed(&clock, Duration::from_secs(600)));

    s.use_vault("personal").unwrap();

    clock.advance(Duration::from_secs(9 * 60));
    s.vault(None).unwrap();
    assert_eq!(unlock_count(&p, "personal"), 1, "still inside the window");

    // Measured from the last use, not the first: 9 + 11 minutes since the
    // unlock, but only 11 since the call above.
    clock.advance(Duration::from_secs(11 * 60));
    s.vault(None).unwrap();
    assert_eq!(unlock_count(&p, "personal"), 2, "re-prompted after idling");
}

// Go: TestIdleTimeoutClosesTheIdentityLikeAnExplicitLock. The same
// teardown as `lock`: the key's pages are released.
#[test]
fn idle_timeout_closes_the_identity_like_an_explicit_lock() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let clock = Clock::new();
    let mut s = d.session(&p, timed(&clock, Duration::from_secs(60)));

    s.vault(Some("personal")).unwrap();
    assert!(!d.ledger.live().is_empty());

    clock.advance(Duration::from_secs(120));
    s.status();
    assert!(
        d.ledger.live().is_empty(),
        "the idle timeout dropped the vault without releasing its key"
    );
}

// Go: TestIdleTimeoutDisabledWhenZero. Unset means "never re-lock on
// time", and so does an explicit zero.
#[test]
fn idle_timeout_disabled_when_zero() {
    for timeout in [None, Some(Duration::ZERO)] {
        let d = Device::new(&["personal"]);
        let p = unlocks(1);
        let clock = Clock::new();
        let mut s = d.session(
            &p,
            Cfg {
                idle_timeout: timeout,
                clock: Some(clock.clone()),
                ..Cfg::default()
            },
        );
        s.use_vault("personal").unwrap();
        clock.advance(Duration::from_secs(30 * 24 * 3600));
        s.vault(None).unwrap();
        assert_eq!(unlock_count(&p, "personal"), 1, "{timeout:?}");
    }
}

// Go: TestIdleTimeoutIsVisibleToStatusWithoutACommand. Status applies the
// timeout itself, so the prompt's lock glyph goes stale when the clock
// says so, not after the next command.
#[test]
fn idle_timeout_is_visible_to_status_without_a_command() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let clock = Clock::new();
    let mut s = d.session(&p, timed(&clock, Duration::from_secs(60)));

    s.use_vault("personal").unwrap();
    assert!(s.status()[0].unlocked);
    clock.advance(Duration::from_secs(120));
    assert!(!s.status()[0].unlocked);
}

// Go: TestIdleTimeoutNeverInterruptsAnInFlightCommand. A command made of
// several session calls can't have its key dropped between them, however
// long it runs; the timeout runs again once the command ends.
#[test]
fn idle_timeout_never_interrupts_an_in_flight_command() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let clock = Clock::new();
    let mut s = d.session(&p, timed(&clock, Duration::from_secs(60)));
    s.use_vault("personal").unwrap();

    {
        let mut cmd = s.begin_command();
        let first = cmd.vault(None).unwrap().1 as *const _;
        clock.advance(Duration::from_secs(5 * 60));
        let again = cmd.vault(None).unwrap().1 as *const _;
        assert_eq!(first, again, "the second call got a different identity");
        assert!(cmd.status()[0].unlocked, "re-locked mid-command");
        assert!(!d.ledger.live().is_empty());
    }
    assert_eq!(unlock_count(&p, "personal"), 1);

    // Deferred, not disabled: the vault was last used at the end of that
    // command, and idling from there still re-locks it.
    clock.advance(Duration::from_secs(120));
    assert!(!s.status()[0].unlocked);
}

// --- resolve ----------------------------------------------------------------

// Go: TestSessionResolveAsksThePrompterOnAnAmbiguousQuery. The resolver
// returns the same candidate list in both modes; one-shot fails with it
// and a session asks.
#[test]
fn session_resolve_asks_the_prompter_on_an_ambiguous_query() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS root account", "AWS IAM backup admin"]);
    // Sorted by title, the second candidate is the root account.
    let p = unlocks(1).with_choice(1);
    let mut s = d.session(&p, current_personal());

    let (id, e) = s.resolve(None, "AWS").unwrap();

    let lists = p.choose_lists();
    assert_eq!(lists.len(), 1, "asked exactly once");
    assert_eq!(lists[0].candidates.len(), 2);
    let want = &lists[0].candidates[1];
    assert_eq!(e.title, "AWS root account");
    assert_eq!(e.title, want.title);
    assert_eq!(id, want.id);
    assert_eq!(
        e.value, "v-AWS root account",
        "the chosen entry is decrypted"
    );
}

// An unambiguous query never reaches the picker.
#[test]
fn session_resolve_of_a_unique_query_asks_nobody() {
    let d = Device::new(&["personal"]);
    let ids = d.seed("personal", &["ProtonMail"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    assert_eq!(s.resolve(None, "proton").unwrap().0, ids[0]);
    assert!(p.choose_lists().is_empty());
}

// Go: TestSessionResolveRefusesAnIDThatWasNeverOffered. An answer outside
// the list is a frontend bug and must not select an arbitrary entry.
#[test]
fn session_resolve_refuses_an_id_that_was_never_offered() {
    let d = Device::new(&["personal"]);
    // The last is a real entry, but not one "AWS" offers.
    let ids = d.seed(
        "personal",
        &["AWS root account", "AWS IAM backup admin", "Unrelated"],
    );
    let p = unlocks(1).with_chosen_id(ids[2]);
    let mut s = d.session(&p, current_personal());

    let err = s.resolve(None, "AWS").unwrap_err();
    assert!(
        matches!(err, SessionError::NotACandidate(id) if id == ids[2]),
        "{err}"
    );
    assert_eq!(err.code(), Code::Internal);
}

// Go: TestResolveAmbiguousWithNoPrompterReturnsCandidates. Go's case is a
// nil prompter; here it is one that declines to pick. Either way nobody
// chose, and the candidate list is the answer, as in one-shot mode.
#[test]
fn resolve_ambiguous_with_no_prompter_returns_candidates() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["duplicate", "duplicate"]);
    let p = unlocks(1).with_choose_error(PromptError::Cancelled);
    let mut s = d.session(&p, current_personal());

    let err = s.resolve(Some("personal"), "duplicate").unwrap_err();
    let SessionError::Resolve(ResolveError::Ambiguous(list)) = &err else {
        panic!("want the candidate list, got {err}");
    };
    assert_eq!(list.candidates.len(), 2);
    assert_eq!(err.code(), Code::Ambiguous);
}

// A picker that gives up is reported as its own failure, coded Ambiguous,
// not as a list it was already shown.
#[test]
fn a_picker_that_gives_up_is_ambiguous() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["duplicate", "duplicate"]);
    let p = unlocks(1).with_choose_error(PromptError::NoChoice(3));
    let mut s = d.session(&p, current_personal());

    let err = s.resolve(None, "duplicate").unwrap_err();
    assert!(
        matches!(err, SessionError::Choose(PromptError::NoChoice(3))),
        "{err}"
    );
    assert_eq!(err.code(), Code::Ambiguous);
}

// --- page locks -------------------------------------------------------------

// Go: TestConcurrentlyHeldIdentitiesDoNotSharePages. A session is where
// two keys are first held at once. Page locks aren't reference counted, so
// two keys on one page share one lock, and dropping either silently
// unprotects the other on Linux and macOS (Windows fails the second unlock
// instead, which is how Go found it). The ledger behaves like the OS on
// every platform, so this is checked everywhere.
#[test]
fn concurrently_held_identities_do_not_share_pages() {
    let d = Device::new(&["personal", "work"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());

    s.vault(Some("personal")).unwrap();
    s.vault(Some("work")).unwrap();
    let locks = d.ledger.locks();
    assert_eq!(locks.len(), 2, "two keys, two locks");
    let (personal, work) = (&locks[0], &locks[1]);
    assert!(!personal.is_empty() && !work.is_empty());
    assert!(
        personal.is_disjoint(work),
        "two held keys share a page: dropping either would unlock the other"
    );

    s.lock(Some("personal")).unwrap();
    assert!(
        work.is_subset(&d.ledger.live()),
        "dropping personal released work's page lock"
    );
    assert!(personal.is_disjoint(&d.ledger.live()));
}
