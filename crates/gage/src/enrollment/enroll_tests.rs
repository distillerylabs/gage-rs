//! E3 tasks 3 to 6: `Vault::enroll`, against a real bare origin. Device A
//! is [`SyncVault`], the owner; device B is a [`Machine`] cloned from it,
//! the way `gage clone` leaves one: registered, no identity, no recorded
//! key. The injected [`RemoteSyncer`]s exist only for the one failure a
//! local repository cannot produce.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::fixtures_tests::{DAY, open_all, seal_into, vault, write_pending};
use super::*;
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::exitcode::{Code, HasCode};
use crate::gittest;
use crate::prompter::{FakePrompter, PromptError, Purpose};
use crate::remotesync::RemoteSyncer;
use crate::remotesync::tests::{DEVICE, FakeSyncer, SyncVault};
use crate::syncerr;
use crate::twodevice_tests::{B_DEVICE, B_PASSPHRASE, Machine};
use crate::vaultlock;

fn pair() -> (SyncVault, Machine) {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);
    (a, b)
}

fn b_prompter() -> FakePrompter {
    FakePrompter::new().with_passphrase(B_PASSPHRASE)
}

fn identity_exists(m: &Machine, device: &str) -> bool {
    paths::identity_file_path(&m.dirs, &m.vault.id, device)
        .unwrap()
        .exists()
}

fn commits(v: &Vault) -> usize {
    gitrepo::commit_count(&v.path).unwrap()
}

/// What A sees of the origin after fetching it: its pending requests.
fn origin_pending(a: &SyncVault) -> Vec<PendingRequest> {
    a.vault.pull().unwrap();
    a.vault.pending_enrollments().unwrap()
}

/// A device-B-side syncer: fetches and pushes for real, with a hook run
/// inside every fetch and a queue of push failures to serve first.
struct TestSyncer {
    dir: PathBuf,
    on_fetch: Option<Box<dyn Fn() + Send + Sync>>,
    /// Skips the first fetch: this device's view predates a push that
    /// lands "between" its fetch and its push.
    stale_first_fetch: std::sync::atomic::AtomicBool,
    push_errs: Mutex<VecDeque<syncerr::Error>>,
    pushes: Mutex<usize>,
}

impl TestSyncer {
    fn new(v: &Vault) -> TestSyncer {
        TestSyncer {
            dir: v.path.clone(),
            on_fetch: None,
            stale_first_fetch: false.into(),
            push_errs: Mutex::default(),
            pushes: Mutex::default(),
        }
    }

    fn failing_push(v: &Vault, err: syncerr::Error) -> Arc<TestSyncer> {
        let s = TestSyncer::new(v);
        s.push_errs.lock().unwrap().push_back(err);
        Arc::new(s)
    }
}

impl RemoteSyncer for TestSyncer {
    fn fetch(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        if let Some(hook) = &self.on_fetch {
            hook();
        }
        if self
            .stale_first_fetch
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(false);
        }
        gitrepo::fetch(&self.dir, &gittest::remote_options())
    }

    fn push(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        *self.pushes.lock().unwrap() += 1;
        if let Some(err) = self.push_errs.lock().unwrap().pop_front() {
            return Err(err.into());
        }
        gitrepo::push(&self.dir, &gittest::remote_options())
    }
}

fn auth_refusal() -> syncerr::Error {
    syncerr::Error::new(syncerr::Kind::Auth, "403")
}

// --- task 3: the identity paths ----------------------------------------------

// Go: TestEnrollCreatesAnIdentityOnADeviceThatHoldsNone
#[test]
fn enroll_creates_an_identity_on_a_device_that_holds_none() {
    let (a, b) = pair();
    let p = b_prompter();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert!(req.published);
    assert!(!req.code.is_empty());
    assert!(identity_exists(&b, B_DEVICE));
    let asked = p.unlock_requests();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].purpose, Purpose::Create);
    let pending = origin_pending(&a);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, req.id);
}

// Go: TestEnrollReusesAnExistingIdentityAndPromptsOnce
#[test]
fn enroll_reuses_an_existing_identity_and_prompts_once() {
    let (_a, b) = pair();
    let key = b.identity_add(B_DEVICE);
    let file = paths::identity_file_path(&b.dirs, &b.vault.id, B_DEVICE).unwrap();
    let before = std::fs::read(&file).unwrap();
    let p = b_prompter();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert_eq!(req.pubkey, key);
    let asked = p.unlock_requests();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].purpose, Purpose::Unlock);
    assert_eq!(std::fs::read(&file).unwrap(), before, "the file changed");
    assert_eq!(b.vault.list_identities().unwrap().len(), 1);
}

// Go: TestEnrollWithAWrongPassphraseOnTheReusePathPublishesNothing
#[test]
fn enroll_with_a_wrong_passphrase_on_the_reuse_path_publishes_nothing() {
    let (a, b) = pair();
    b.identity_add(B_DEVICE);
    let before = commits(&b.vault);
    let p = FakePrompter::new()
        .with_passphrases(&["wrong", "wrong", "wrong"])
        .with_unlock_error(PromptError::Cancelled);

    let err = b.vault.enroll(B_DEVICE, DAY, &p).unwrap_err();

    assert_eq!(err.code(), Code::LockedOrAuth, "{err}");
    assert_eq!(commits(&b.vault), before);
    assert!(origin_pending(&a).is_empty());
    assert!(!b.vault.pending_dir().exists());
}

// Go: TestEnrollSealsThePublicKeyThisDeviceActuallyHolds
#[test]
fn enroll_seals_the_public_key_this_device_actually_holds() {
    let (a, b) = pair();
    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();

    let held = b.unlock();
    assert_eq!(held.recipient(), req.pubkey);
    a.vault.pull().unwrap();
    let opened = open_all(&a.vault, &req.code).unwrap();
    assert_eq!(opened[0].pubkey, held.recipient());
    assert_eq!(opened[0].device, B_DEVICE);
}

// Go: TestEnrollHonorsTheTTL
#[test]
fn enroll_honors_the_ttl() {
    let (a, b) = pair();
    let ttl = Duration::from_secs(2 * 3600);
    let req = b.vault.enroll(B_DEVICE, ttl, &b_prompter()).unwrap();

    assert_eq!(
        req.expires.as_second() - req.created.as_second(),
        ttl.as_secs() as i64
    );
    let pending = origin_pending(&a);
    assert_eq!(pending[0].expires, req.expires, "the filename epoch");
    let opened = open_all(&a.vault, &req.code).unwrap();
    assert_eq!(opened[0].expires, req.expires, "the sealed expiry");
}

// Go: TestEnrollRefusesAnUnusableTTLBeforeAnythingHappens
#[test]
fn enroll_refuses_an_unusable_ttl_before_anything_happens() {
    let (_a, b) = pair();
    for ttl in [Duration::ZERO, MAX_ENROLLMENT_TTL + Duration::from_secs(1)] {
        let p = FakePrompter::new();
        let before = commits(&b.vault);
        let err = b.vault.enroll(B_DEVICE, ttl, &p).unwrap_err();
        assert!(matches!(err, EnrollmentError::InvalidTtl { .. }), "{err}");
        assert_eq!(err.code(), Code::Usage);
        assert!(p.unlock_requests().is_empty());
        assert!(!identity_exists(&b, B_DEVICE));
        assert_eq!(commits(&b.vault), before);
    }
}

// Go: TestEnrollRefusesADeviceNameOutsideTheAllowlist
#[test]
fn enroll_refuses_a_device_name_outside_the_allowlist() {
    let (_a, b) = pair();
    for bad in ["", "../escape", "has space", "Upper!"] {
        let p = FakePrompter::new();
        let err = b.vault.enroll(bad, DAY, &p).unwrap_err();
        assert!(
            matches!(err, EnrollmentError::InvalidDevice { .. }),
            "{bad}: {err}"
        );
        assert!(p.unlock_requests().is_empty());
    }
    assert!(b.vault.list_identities().unwrap().is_empty());
}

// Go: TestEnrolledRequestCarriesThisVaultsID
#[test]
fn enrolled_request_carries_this_vaults_id() {
    let (a, b) = pair();
    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    a.vault.pull().unwrap();
    assert_eq!(open_all(&a.vault, &req.code).unwrap().len(), 1);

    // The same bytes filed in another vault are refused: the sealed id is
    // that vault's, not this one's.
    let other = vault();
    let pending = a.vault.pending_enrollments().unwrap();
    let bytes = std::fs::read(&pending[0].path).unwrap();
    write_pending(
        &other.vault,
        pending[0].path.file_name().unwrap().to_str().unwrap(),
        &bytes,
    );
    let err = open_all(&other.vault, &req.code).unwrap_err();
    assert!(matches!(err, EnrollmentError::WrongVault { .. }), "{err}");
}

/// A commit on `v`'s branch whose committer time is `hours` from now.
fn commit_dated(v: &Vault, hours: i64) {
    std::fs::write(v.path.join("dated.txt"), "x").unwrap();
    let repo = git2::Repository::open(&v.path).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("dated.txt")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let when = git2::Time::new(Timestamp::now().as_second() + hours * 3600, 0);
    let sig = git2::Signature::new("t", "t@example.com", &when).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "dated", &tree, &[&parent])
        .unwrap();
}

// Go: TestEnrollWarnsWhenThisMachinesClockIsBehindHead
#[test]
fn enroll_warns_when_this_machines_clock_is_behind_head() {
    let (_a, b) = pair();
    commit_dated(&b.vault, 48);
    let p = b_prompter();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert!(req.published, "a skewed clock warns and proceeds");
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("clock"), "{warnings:?}");
}

// Go: TestEnrollIsSilentWhenTheClockLooksFine
#[test]
fn enroll_is_silent_when_the_clock_looks_fine() {
    let (_a, b) = pair();
    let p = b_prompter();
    b.vault.enroll(B_DEVICE, DAY, &p).unwrap();
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

// --- task 4: collisions --------------------------------------------------------

// Go: TestEnrollOnADeviceThatIsAlreadyARecipientIsASuccessThatDoesNoWork.
// Built the way it happens: identity add, then another device runs
// recipient add of that key, then enroll.
#[test]
fn enroll_on_a_device_that_is_already_a_recipient_is_a_success_that_does_no_work() {
    let (a, b) = pair();
    let key = b.identity_add(B_DEVICE);
    a.vault
        .add_recipient(B_DEVICE, &key, &a.unlock(), &FakePrompter::new())
        .unwrap();
    let p = FakePrompter::new();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert!(!req.published);
    assert!(req.code.is_empty());
    assert!(p.unlock_requests().is_empty(), "nothing is prompted");
    assert!(origin_pending(&a).is_empty());
    assert!(!b.vault.pending_dir().exists());
    assert_eq!(commits(&b.vault), commits(&a.vault));
}

// Go: TestTheAlreadyARecipientCheckIsByKeyNotByName
#[test]
fn the_already_a_recipient_check_is_by_key_not_by_name() {
    let (a, b) = pair();
    let key = b.identity_add(B_DEVICE);
    a.vault
        .add_recipient("some-other-label", &key, &a.unlock(), &FakePrompter::new())
        .unwrap();

    let req = b.vault.enroll(B_DEVICE, DAY, &FakePrompter::new()).unwrap();
    assert!(!req.published);
}

// Go: TestWithNoRecordedPubkeyTheKeyCheckIsSkipped
#[test]
fn with_no_recorded_pubkey_the_key_check_is_skipped() {
    let (a, b) = pair();
    // A key B holds but never recorded, as after `clone`: listed under the
    // name B is about to ask for.
    let p = b_prompter();
    let key = identityfile::create(&b.dirs, &b.vault.id, "personal", B_DEVICE, &p).unwrap();
    a.vault
        .add_recipient(B_DEVICE, &key, &a.unlock(), &FakePrompter::new())
        .unwrap();

    let err = b
        .vault
        .enroll(B_DEVICE, DAY, &FakePrompter::new())
        .unwrap_err();
    assert!(
        matches!(err, EnrollmentError::DeviceNameTaken { .. }),
        "without a recorded key the check falls through to the name: {err}"
    );
}

// Go: TestEnrollRefusesANameThatLabelsADifferentRecipient
#[test]
fn enroll_refuses_a_name_that_labels_a_different_recipient() {
    let (_a, b) = pair();
    let p = FakePrompter::new();

    let err = b.vault.enroll(DEVICE, DAY, &p).unwrap_err();

    assert!(
        matches!(err, EnrollmentError::DeviceNameTaken { .. }),
        "{err}"
    );
    assert_eq!(err.code(), Code::Conflict);
    assert!(err.to_string().contains("--device"), "{err}");
    assert!(!identity_exists(&b, DEVICE));
    assert!(p.unlock_requests().is_empty());
}

// Go: TestEnrollUnderADifferentDeviceNameSucceedsWhereTheDefaultCollided
#[test]
fn enroll_under_a_different_device_name_succeeds_where_the_default_collided() {
    let (_a, b) = pair();
    b.vault
        .enroll(DEVICE, DAY, &FakePrompter::new())
        .unwrap_err();
    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    assert!(req.published);
    assert_eq!(req.device, B_DEVICE);
}

// --- task 5: the remote --------------------------------------------------------

// Go: TestEnrollFetchesBeforeItCommits
#[test]
fn enroll_fetches_before_it_commits() {
    let (a, b) = pair();
    a.other()
        .write_commit_push("notes.txt", "moved ahead", "another device's commit");

    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();

    assert!(req.published);
    assert!(
        b.vault.path.join("notes.txt").exists(),
        "fast-forwarded first"
    );
    // The push was a fast-forward over the remote, not a merge.
    a.vault.pull().unwrap();
    assert_eq!(commits(&a.vault), commits(&b.vault));
    assert_eq!(a.vault.pending_enrollments().unwrap().len(), 1);
}

// Go: TestEnrollWithAnUnreachableRemoteFailsBeforeAnythingIsCreated
#[test]
fn enroll_with_an_unreachable_remote_fails_before_anything_is_created() {
    let (_a, b) = pair();
    let offline = b.vault.clone().with_remote_syncer(FakeSyncer::offline());
    let before = commits(&b.vault);
    let p = FakePrompter::new();

    let err = offline.enroll(B_DEVICE, DAY, &p).unwrap_err();

    assert!(
        matches!(err, EnrollmentError::RemoteUnreachable(_)),
        "{err}"
    );
    assert_eq!(err.code(), Code::Unreachable);
    assert!(p.unlock_requests().is_empty());
    assert!(!identity_exists(&b, B_DEVICE));
    assert!(!b.vault.pending_dir().exists());
    assert_eq!(commits(&b.vault), before);
}

// Go: TestEnrollWithNoRemoteFailsWithItsOwnError
#[test]
fn enroll_with_no_remote_fails_with_its_own_error() {
    let f = vault();
    let p = FakePrompter::new();
    let err = f.vault.enroll("phone", DAY, &p).unwrap_err();
    assert!(matches!(err, EnrollmentError::NoRemote { .. }), "{err}");
    assert_eq!(err.code(), Code::Usage);
    assert!(p.unlock_requests().is_empty());
}

const HOLDS_THE_LOCK_ENV: &str = "GAGE_TEST_ENROLL_LOCK_FILE";

#[test]
fn lock_is_held_child() {
    if !super::seal_tests::in_child() {
        return;
    }
    let path = PathBuf::from(std::env::var(HOLDS_THE_LOCK_ENV).unwrap());
    let got = vaultlock::Lock::try_acquire(&path).unwrap();
    assert!(got.is_none(), "another process could take the write lock");
}

// Go: TestEnrollHoldsTheWriteLockAcrossThePull. A child process, because
// file-lock semantics are per open file description: a thread would pass
// whether or not the lock was held.
#[test]
fn enroll_holds_the_write_lock_across_the_pull() {
    let (_a, b) = pair();
    let lock_file = b.vault.prepare_lock_file().unwrap();
    let mut syncer = TestSyncer::new(&b.vault);
    syncer.on_fetch = Some(Box::new(move || {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "enrollment::enroll_tests::lock_is_held_child",
                "--exact",
                "--nocapture",
            ])
            .env(super::seal_tests::CHILD_ENV, "1")
            .env(HOLDS_THE_LOCK_ENV, &lock_file)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "the child failed:\n{stderr}");
        assert!(
            stderr.contains("CHILD RAN"),
            "the child never ran:\n{stderr}"
        );
    }));
    let vault = b.vault.clone().with_remote_syncer(Arc::new(syncer));

    vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
}

// Go: TestUncontendedEnrollNeverReportsContention
#[test]
fn uncontended_enroll_never_reports_contention() {
    let (_a, b) = pair();
    let started = Instant::now();
    let res = b.vault.enroll(B_DEVICE, DAY, &b_prompter());
    assert!(res.is_ok(), "{res:?}");
    assert!(started.elapsed() < WRITE_LOCK_TIMEOUT);
}

// Go: TestEnrollPushRejectedForWriteAccessKeepsWhatItMade
#[test]
fn enroll_push_rejected_for_write_access_keeps_what_it_made() {
    let (_a, b) = pair();
    let before = commits(&b.vault);
    let vault = b
        .vault
        .clone()
        .with_remote_syncer(TestSyncer::failing_push(&b.vault, auth_refusal()));

    let err = vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap_err();

    assert!(matches!(err, EnrollmentError::Push(_)), "{err}");
    assert_eq!(err.code(), Code::LockedOrAuth);
    assert!(err.to_string().contains("gage auth login"), "{err}");
    assert!(identity_exists(&b, B_DEVICE), "the identity is kept");
    assert_eq!(commits(&b.vault), before + 1, "the local commit is kept");
    assert_eq!(b.vault.pending_enrollments().unwrap().len(), 1);
}

// Go: TestEnrollAfterAFailedPushReusesTheIdentityAndSucceeds
#[test]
fn enroll_after_a_failed_push_reuses_the_identity_and_succeeds() {
    let (a, b) = pair();
    let failing = b
        .vault
        .clone()
        .with_remote_syncer(TestSyncer::failing_push(&b.vault, auth_refusal()));
    failing.enroll(B_DEVICE, DAY, &b_prompter()).unwrap_err();
    let first = b.vault.pending_enrollments().unwrap();

    let p = b_prompter();
    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert_eq!(p.unlock_requests()[0].purpose, Purpose::Unlock);
    assert!(req.published);
    assert_ne!(req.id, first[0].id, "a fresh request and code");
    assert_eq!(origin_pending(&a).len(), 2, "the first request rode along");
}

/// B has a failed push behind it and the remote then moves: the state a
/// diverged pull is found in.
fn diverged_pair() -> (SyncVault, Machine) {
    let (a, b) = pair();
    let failing = b
        .vault
        .clone()
        .with_remote_syncer(TestSyncer::failing_push(&b.vault, auth_refusal()));
    failing.enroll(B_DEVICE, DAY, &b_prompter()).unwrap_err();
    a.other()
        .write_commit_push("notes.txt", "moved", "the remote moved");
    (a, b)
}

// Go: TestEnrollRefusesADivergedPull
#[test]
fn enroll_refuses_a_diverged_pull() {
    let (_a, b) = diverged_pair();
    let before = commits(&b.vault);
    let pending_before = b.vault.pending_enrollments().unwrap().len();
    let p = FakePrompter::new();

    let err = b.vault.enroll(B_DEVICE, DAY, &p).unwrap_err();

    assert_eq!(err.code(), Code::Conflict);
    assert!(!err.to_string().contains("gage sync"), "{err}");
    assert_eq!(commits(&b.vault), before);
    assert_eq!(b.vault.pending_enrollments().unwrap().len(), pending_before);
    assert!(p.unlock_requests().is_empty());
}

// Go: TestDivergenceIsFoundByTheReportRatherThanAnError. Asserts the
// refusal, not "some error": a pull that cannot fast-forward succeeds, and
// only the report says so.
#[test]
fn divergence_is_found_by_the_report_rather_than_an_error() {
    let (_a, b) = diverged_pair();
    let err = b
        .vault
        .enroll(B_DEVICE, DAY, &FakePrompter::new())
        .unwrap_err();
    assert!(matches!(err, EnrollmentError::Diverged { .. }), "{err:?}");
}

// Go: TestEnrollTakesTheDirtyTreeResetBeforeItPulls
#[test]
fn enroll_takes_the_dirty_tree_reset_before_it_pulls() {
    let (a, b) = pair();
    a.other()
        .write_commit_push("notes.txt", "moved", "the remote moved");
    // Tracked and modified: a fast-forward would refuse it as a dirty tree
    // if the reset did not run first.
    let recipients = b.vault.path.join(".age-recipients");
    let mut text = std::fs::read_to_string(&recipients).unwrap();
    text.push_str("# stray edit\n");
    std::fs::write(&recipients, text).unwrap();
    let p = b_prompter();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    assert!(req.published);
    assert!(
        p.warnings()
            .iter()
            .any(|w| w.contains("discarded uncommitted changes")),
        "{:?}",
        p.warnings()
    );
}

// --- task 6: crash-safety and two devices ----------------------------------

// Go: TestAStrayPendingFileIsInert
#[test]
fn a_stray_pending_file_is_inert() {
    let (_a, b) = pair();
    let before = b.vault.recipients().unwrap();
    seal_into(&b.vault, "phone", &super::fixtures_tests::new_pubkey(), DAY);
    assert_eq!(b.vault.recipients().unwrap(), before);
    assert!(b.vault.verify_recipients().unwrap().in_sync);
}

// Go: TestTheNextWriteDiscardsAStrayPendingFile
#[test]
fn the_next_write_discards_a_stray_pending_file() {
    let (_a, b) = pair();
    let stray = seal_into(&b.vault, "phone", &super::fixtures_tests::new_pubkey(), DAY);
    let p = b_prompter();

    let req = b.vault.enroll(B_DEVICE, DAY, &p).unwrap();

    let warnings = p.warnings();
    assert!(
        warnings.iter().any(|w| w.contains(&stray.id)),
        "the warning names it: {warnings:?}"
    );
    let ids: Vec<String> = b
        .vault
        .pending_enrollments()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![req.id], "only this enroll's request is committed");
}

// Go: TestPendingEnrollmentsListsAStrayLocallyAndNowhereElse
#[test]
fn pending_enrollments_lists_a_stray_locally_and_nowhere_else() {
    let (a, b) = pair();
    let stray = seal_into(&b.vault, "phone", &super::fixtures_tests::new_pubkey(), DAY);
    assert_eq!(b.vault.pending_enrollments().unwrap()[0].id, stray.id);
    assert!(origin_pending(&a).is_empty());
}

// Go: TestAStrayDoesNotWedgeTheJoiningDevice
#[test]
fn a_stray_does_not_wedge_the_joining_device() {
    let (a, b) = pair();
    seal_into(&b.vault, "phone", &super::fixtures_tests::new_pubkey(), DAY);
    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    assert!(req.published);
    assert_eq!(origin_pending(&a).len(), 1);
}

/// A, B and a third machine C, all cloned before anyone enrolls.
fn trio() -> (SyncVault, Machine, Machine) {
    let (a, b) = pair();
    let c = Machine::cloned_from(&a, "phone");
    (a, b, c)
}

fn c_prompter() -> FakePrompter {
    FakePrompter::new().with_passphrase("a third passphrase for device c")
}

// Go: TestTwoDevicesEnrollAgainstTheSameRemoteAndBothRequestsSurvive
#[test]
fn two_devices_enroll_against_the_same_remote_and_both_requests_survive() {
    let (a, b, c) = trio();
    let rb = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    // C fetched nothing since cloning, but enroll fetches first, so make
    // C's push the rejected one by racing it past the fetch.
    let rc = c.vault.enroll("phone", DAY, &c_prompter()).unwrap();

    let pending = origin_pending(&a);
    assert_eq!(pending.len(), 2);
    assert_eq!(open_all(&a.vault, &rb.code).unwrap()[0].device, B_DEVICE);
    assert_eq!(open_all(&a.vault, &rc.code).unwrap()[0].device, "phone");
}

// Go: TestConcurrentEnrollsMergeIntoAUnion. The rejected push is real:
// C's first fetch is skipped, as if B published between C's fetch and
// C's push, so the origin rejects C's push and gage merges.
#[test]
fn concurrent_enrolls_merge_into_a_union() {
    let (a, b, c) = trio();
    b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    let racing = TestSyncer::new(&c.vault);
    racing
        .stale_first_fetch
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let c_vault = c.vault.clone().with_remote_syncer(Arc::new(racing));

    let rc = c_vault.enroll("phone", DAY, &c_prompter()).unwrap();

    assert!(rc.published);
    assert_eq!(c.vault.pending_enrollments().unwrap().len(), 2, "the union");
    assert_eq!(origin_pending(&a).len(), 2);
}

// Go: TestMergedRequestsStillGrantNothing
#[test]
fn merged_requests_still_grant_nothing() {
    let (a, b, c) = trio();
    let before = a.vault.recipients().unwrap();
    b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    c.vault.enroll("phone", DAY, &c_prompter()).unwrap();
    a.vault.pull().unwrap();
    assert_eq!(a.vault.recipients().unwrap(), before);
    assert!(a.vault.verify_recipients().unwrap().in_sync);
}

// Go: TestADivergenceInvolvingPendingIsReportedHonestly. Two devices that
// resolve the same request (one approves, one denies) both delete the same
// path, which git merges without a conflict: the only way `pending/` can
// appear in a divergence at all. A conflicting `pending/` path would be
// classed as an entry conflict, which the resolver cannot resolve, and
// this is where that would stop being a surprise.
#[test]
fn a_divergence_involving_pending_is_reported_honestly() {
    use crate::remotesync::{ConflictKind, SyncReport};

    let (a, b) = pair();
    let req = b.vault.enroll(B_DEVICE, DAY, &b_prompter()).unwrap();
    let name = format!(
        "{PENDING_DIR}/{}",
        name::pending_file_name(&req.id, req.expires)
    );
    // Clones before the approver publishes, so the sides truly diverge.
    let denier = Machine::cloned_from(&a, "phone");

    let approver = a.other();
    approver.remove(&name);
    approver.commit("gage: approve");
    approver.push();

    std::fs::remove_file(denier.vault.path.join(&name)).unwrap();
    std::fs::write(denier.vault.path.join("NOTES"), "denied").unwrap();
    gitrepo::commit_all(&denier.vault.path, "gage: deny").unwrap();

    let report = denier.vault.push().unwrap();

    assert!(report.diverged, "the fixture proves nothing otherwise");
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert_eq!(report.conflict_kind(), ConflictKind::None);
    assert!(report.merged && report.pushed, "{report:?}");

    let hypothetical = SyncReport {
        conflicts: vec![name],
        ..SyncReport::default()
    };
    assert_eq!(hypothetical.conflict_kind(), ConflictKind::Entries);
}
