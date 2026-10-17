//! `gage recipient pending`, `approve` and `deny` through the real CLI,
//! in-process: Go's `approve_cli_test.go`. Each test names its Go row.
//! [`Enrolled`] is the state an approval starts from: device A owns the
//! vault, device B has published a request with `identity enroll`, and A
//! has pulled it.

use std::path::{Path, PathBuf};

use gage::exitcode::Code;
use gage::gitrepo;
use gage::prompter::FakePrompter;
use jiff::{Timestamp, ToSpan};

use crate::enroll_cli_tests::code_in;
use crate::identity_cli_tests::{TwoDevices, VAULT};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};

pub(crate) struct Enrolled {
    pub(crate) t: TwoDevices,
    pub(crate) code: String,
}

impl Enrolled {
    /// B enrolls as `device` and A catches up to its request.
    pub(crate) fn new(device: &str) -> Enrolled {
        let t = TwoDevices::new();
        let r = t.b.run(&["identity", "enroll", "--device", device]);
        assert_eq!(r.code, Code::Success, "enroll: {}", r.stderr);
        let code = code_in(&r.stdout);
        t.a.ok(&["pull"]);
        Enrolled { t, code }
    }

    pub(crate) fn a(&self) -> &Env {
        &self.t.a
    }

    /// A third machine clones, enrolls as `device`, and A catches up.
    /// Returns its code; the machine is kept alive by the caller.
    pub(crate) fn enroll_another(&self, device: &str) -> (Env, String) {
        let env = Env::new();
        let url = self.t._origin.path().to_str().unwrap();
        let r = env.run_with(
            &["clone", url, "--name", VAULT, "--device", device],
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Success, "clone: {}", r.stderr);
        let r = env.run(&["identity", "enroll", "--device", device]);
        assert_eq!(r.code, Code::Success, "enroll: {}", r.stderr);
        let code = code_in(&r.stdout);
        self.a().ok(&["pull"]);
        (env, code)
    }

    pub(crate) fn vault_path(&self) -> PathBuf {
        self.a().entry(VAULT).path
    }

    pub(crate) fn commits(&self) -> usize {
        gitrepo::commit_count(&self.vault_path()).unwrap()
    }
}

/// What an approver answers: yes to the approval, then the passphrase.
pub(crate) fn approving() -> FakePrompter {
    FakePrompter::new()
        .with_confirm(true)
        .with_passphrase(PASSPHRASE)
}

/// The request ids in a vault's `pending/`, sorted.
pub(crate) fn pending_ids(vault: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(vault.join(".gage/pending")) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = dir
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter_map(|n| n.get(..36).map(str::to_string))
        .collect();
    ids.sort();
    ids
}

/// A hex digit every id carries (Go's `commonHexDigit`).
pub(crate) fn common_hex_digit(ids: &[String]) -> String {
    "0123456789abcdef"
        .chars()
        .find(|c| ids.iter().all(|id| id.replace('-', "").contains(*c)))
        .expect("no hex digit common to every id")
        .to_string()
}

pub(crate) fn still_pending(env: &Env) -> bool {
    env.run_with(&["recipient", "pending"], &FakePrompter::new())
        .stdout
        .contains("expires")
}

fn stray_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

/// Files and commits a request whose sealed fields are exactly these,
/// live by its filename; returns its code.
pub(crate) fn file_unchecked(
    vault: &Path,
    env: &Env,
    device: &str,
    created: Timestamp,
    expires: Timestamp,
) -> String {
    let v = gage::vault::Vault::from_registration(&env.dirs, VAULT, &env.entry(VAULT)).unwrap();
    let (_, code) = gage::testsupport::file_unchecked_request(
        &v,
        device,
        &stray_key(),
        created,
        expires,
        Timestamp::now() + 1.hour(),
    );
    gitrepo::commit_all(vault, "test: a request gage did not write").unwrap();
    code
}

impl Env {
    pub(crate) fn approve(&self, args: &[&str], p: &FakePrompter) -> CliResult {
        let mut full = vec!["recipient", "approve"];
        full.extend_from_slice(args);
        self.run_with(&full, p)
    }
}

// Go: TestRecipientPendingNeedsNoUnlockAndShowsNoDeviceNames
#[test]
fn recipient_pending_needs_no_unlock_and_shows_no_device_names() {
    let e = Enrolled::new("phone-1");
    let r = e
        .a()
        .run_with(&["recipient", "pending"], &FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(!r.stdout.contains("phone-1"), "{}", r.stdout);
    assert!(r.stdout.contains("expires"), "{}", r.stdout);
    let id = &pending_ids(&e.vault_path())[0];
    assert!(r.stdout.contains(&id[..8]), "{}", r.stdout);
}

// Go: TestRecipientPendingWithNothingPendingExitsZero
#[test]
fn recipient_pending_with_nothing_pending_exits_zero() {
    let env = Env::new();
    env.init(VAULT, &[]);
    let r = env.run_with(&["recipient", "pending"], &FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("no pending"), "{}", r.stdout);
}

// Go: TestApproveEndToEndLetsTheDeviceRead. Enrolled and approved, the new
// device reads an entry written before it was approved.
#[test]
fn approve_end_to_end_lets_the_device_read() {
    let e = Enrolled::new("phone-1");
    e.a().insert_ok("s3cret", &["Bank"]);

    let r = e.a().approve(&["--code", &e.code], &approving());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("phone-1"), "{}", r.stdout);

    e.t.b.cmd_ok(&["sync"]);
    let cat = e.t.b.cmd_ok(&["cat", "Bank"]);
    assert!(cat.stdout.contains("s3cret"), "{}", cat.stdout);
}

// Go: TestApproveCostsNoUnlockOnAWrongCode. The same holds for a request
// expired by its seal: refused before any passphrase is asked for.
#[test]
fn approve_costs_no_unlock_on_a_wrong_code() {
    let e = Enrolled::new("phone-1");
    let p = FakePrompter::new();
    let r = e.a().approve(&["--code", "GAGE-0000-0000-0000-0000"], &p);
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());

    let expired = file_unchecked(
        &e.vault_path(),
        e.a(),
        "tablet-1",
        Timestamp::now() - 48.hours(),
        Timestamp::now() - 24.hours(),
    );
    let p = FakePrompter::new();
    let r = e.a().approve(&["--code", &expired], &p);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("expired"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
}

// Go: TestDecliningTheApprovalCostsNoUnlock
#[test]
fn declining_the_approval_costs_no_unlock() {
    let e = Enrolled::new("phone-1");
    let before = e.commits();
    let p = FakePrompter::new().with_confirm(false);

    let r = e.a().approve(&["--code", &e.code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("nothing was approved"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert_eq!(e.commits(), before);
    assert!(still_pending(e.a()));
}

// Go: TestApproveRefusesAWrongCodeWithoutRetryingWhenItCameFromAFlag. A
// code that opens nothing fails the whole run, before any confirmation.
#[test]
fn approve_refuses_a_wrong_code_without_retrying_when_it_came_from_a_flag() {
    let e = Enrolled::new("phone-1");
    let before = e.commits();
    let p = FakePrompter::new();

    let r = e.a().approve(
        &["--code", &e.code, "--code", "GAGE-0000-0000-0000-0000"],
        &p,
    );

    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
    assert!(p.value_prompts().is_empty(), "a flagged code was retried");
    assert_eq!(e.commits(), before);
    assert!(still_pending(e.a()));
}

// Go: TestApproveReadsTheCodeThroughThePrompterWhenNoFlagIsGiven. The CLI,
// not the library, owns the retry around a wrong one.
#[test]
fn approve_reads_the_code_through_the_prompter_when_no_flag_is_given() {
    let e = Enrolled::new("phone-1");
    let p = approving()
        .with_value("GAGE-0000-0000-0000-0000")
        .with_value(&e.code);

    let r = e.a().approve(&[], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let asked = p.value_prompts();
    assert_eq!(asked.len(), 2, "a wrong code must be retried");
    assert!(asked[0].to_lowercase().contains("code"), "{asked:?}");
}

// Go: TestApproveWithAnEmptyPromptedCodeApprovesNothing. A bare Enter
// cancels.
#[test]
fn approve_with_an_empty_prompted_code_approves_nothing() {
    let e = Enrolled::new("phone-1");
    let p = FakePrompter::new().with_value("");

    let r = e.a().approve(&[], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("nothing was approved"), "{}", r.stderr);
    assert_eq!(p.value_prompts().len(), 1);
    assert!(still_pending(e.a()));
}

// Go: TestApproveDeduplicatesARequestOpenedByTwoCodes
#[test]
fn approve_deduplicates_a_request_opened_by_two_codes() {
    let e = Enrolled::new("phone-1");
    let p = approving();

    let r = e.a().approve(&["--code", &e.code, "--code", &e.code], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("opened 1 pending request."),
        "{}",
        r.stdout
    );
    assert_eq!(p.confirm_prompts().len(), 1);
    let list = e.a().ok(&["recipient", "list"]).stdout;
    assert_eq!(list.matches("phone-1").count(), 1, "{list}");
    assert!(!still_pending(e.a()));
}

// Go: TestApproveNarrowsToAPositionalID
#[test]
fn approve_narrows_to_a_positional_id() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();
    let (_tablet, _) = e.enroll_another("tablet-1");
    assert_eq!(pending_ids(&e.vault_path()).len(), 2);

    let r = e.a().approve(&[&id, "--code", &e.code], &approving());

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(pending_ids(&e.vault_path()).len(), 1);
    assert!(!pending_ids(&e.vault_path()).contains(&id));
}

// Beyond Go's rows, found in E4's review: one request named twice (two
// spellings of its id) is one device on the prompted path too, where no
// per-code dedupe runs.
#[test]
fn approve_naming_one_request_twice_is_one_device() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();
    let p = approving().with_value(&e.code);

    let r = e.a().approve(&[&id, &id[..8]], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("opened 1 pending request."),
        "{}",
        r.stdout
    );
    assert!(!still_pending(e.a()));
}

// Go: TestApproveWithADeviceFlagRelabels
#[test]
fn approve_with_a_device_flag_relabels() {
    let e = Enrolled::new("phone-1");

    let r = e
        .a()
        .approve(&["--code", &e.code, "--device", "phone-2"], &approving());

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("recorded as: phone-2"), "{}", r.stdout);
    let list = e.a().ok(&["recipient", "list"]).stdout;
    assert!(list.contains("phone-2"), "{list}");
    assert!(!list.contains("phone-1"), "{list}");
}

// Go: TestApproveRejectsADeviceFlagAgainstSeveralRequests. One flag can't
// say which request it relabels; the way out is the ID form.
#[test]
fn approve_rejects_a_device_flag_against_several_requests() {
    let e = Enrolled::new("phone-1");
    let (_tablet, second) = e.enroll_another("tablet-1");
    let p = FakePrompter::new();

    let r = e.a().approve(
        &["--code", &e.code, "--code", &second, "--device", "renamed"],
        &p,
    );

    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("ID"), "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
}

// Go: TestApproveRejectsAnInvalidDeviceNameBeforeAnythingElse. A rejection
// of the command line costs neither a confirmation nor a passphrase.
#[test]
fn approve_rejects_an_invalid_device_name_before_anything_else() {
    let e = Enrolled::new("phone-1");
    let p = FakePrompter::new();

    let r = e
        .a()
        .approve(&["--code", &e.code, "--device", "../escape"], &p);

    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
    assert!(p.unlock_requests().is_empty());
}

// Go: TestApproveSurfacesAWrongVaultRequestWithoutRetrying. A request for
// another vault names the vault it is for, renders nothing, and never
// enters the wrong-code loop: on the prompted path, one value is asked
// for and not a second.
#[test]
fn approve_surfaces_a_wrong_vault_request_without_retrying() {
    let e = Enrolled::new("phone-1");
    let work = e.a().init("work", &["--device", "laptop-1"]);
    let from = e.vault_path().join(".gage/pending");
    std::fs::create_dir_all(work.join(".gage/pending")).unwrap();
    for f in std::fs::read_dir(&from).unwrap() {
        let f = f.unwrap();
        std::fs::copy(f.path(), work.join(".gage/pending").join(f.file_name())).unwrap();
    }
    gitrepo::commit_all(&work, "test: a request for another vault").unwrap();

    for args in [
        vec!["--use", "work", "--code", &e.code],
        vec!["--use", "work"],
    ] {
        let p = FakePrompter::new().with_value(&e.code);
        let r = e.a().approve(&args, &p);
        assert_eq!(r.code, Code::Conflict, "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains("personal"), "{}", r.stderr);
        let prompted = if args.len() == 2 { 1 } else { 0 };
        assert_eq!(p.value_prompts().len(), prompted, "{args:?}");
        assert!(!r.stdout.contains("phone-1"), "{}", r.stdout);
        assert!(!r.stdout.contains("age1"), "{}", r.stdout);
        assert!(p.unlock_requests().is_empty());
    }
}

// Go: TestApproveUnderScriptStaysARealQuestion. --yes answers only M10's
// question; approve's own [y/N] is still asked, and still defaults to no.
#[test]
fn approve_under_script_stays_a_real_question() {
    let e = Enrolled::new("phone-1");
    let before = e.commits();
    let p = FakePrompter::new().with_confirm(false);

    let r = e
        .a()
        .run_with(&["--yes", "recipient", "approve", "--code", &e.code], &p);

    assert_ne!(
        r.code,
        Code::Success,
        "--yes answered approve's own question"
    );
    assert_eq!(p.confirm_prompts().len(), 1);
    assert_eq!(e.commits(), before);
    assert!(!r.stderr.is_empty());
}

// Go: TestDenyRemovesARequestAndNeedsNothing. No code and no unlock, by the
// eight-character prefix the listing shows.
#[test]
fn deny_removes_a_request_and_needs_nothing() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();

    let r = e
        .a()
        .run_with(&["recipient", "deny", &id[..8]], &FakePrompter::new());

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains(&id[..8]), "{}", r.stdout);
    assert!(!still_pending(e.a()));
    assert!(!e.a().ok(&["recipient", "list"]).stdout.contains("phone-1"));
}

// Go: TestDenyOnAnAmbiguousIDRendersTheCandidates
#[test]
fn deny_on_an_ambiguous_id_renders_the_candidates() {
    let e = Enrolled::new("phone-1");
    let (_tablet, _) = e.enroll_another("tablet-1");
    let ids = pending_ids(&e.vault_path());
    assert_eq!(ids.len(), 2);

    let r = e.a().run_with(
        &["recipient", "deny", &common_hex_digit(&ids)],
        &FakePrompter::new(),
    );

    assert_eq!(r.code, Code::Ambiguous, "{}", r.stderr);
    for id in &ids {
        assert!(r.stderr.contains(&id[..8]), "{}", r.stderr);
    }
    assert_eq!(pending_ids(&e.vault_path()).len(), 2);
}

// Go: TestDenyOnAnUnmatchedIDIsNotFound
#[test]
fn deny_on_an_unmatched_id_is_not_found() {
    let e = Enrolled::new("phone-1");
    let r = e
        .a()
        .run_with(&["recipient", "deny", "ffffffff"], &FakePrompter::new());
    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    assert!(still_pending(e.a()));
}

// Go: TestPendingAndDenyWorkUnderScript. Neither asks anything a script
// can't answer.
#[test]
fn pending_and_deny_work_under_script() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();
    let p = FakePrompter::new();
    let r = e.a().run_with(&["--yes", "recipient", "pending"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let r = e.a().run_with(&["--yes", "recipient", "deny", &id], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// Beyond Go's rows: Go E4's bullet with no row of its own, and the point of
// E2's validation. A sealed device name carrying escape sequences is
// refused before anything is rendered: the approve screen is the one
// whose correctness depends on a human reading it, and the name sits
// directly above its question.
#[test]
fn approve_never_renders_an_unvalidated_device_name() {
    let e = Enrolled::new("phone-1");
    let hostile = "evil\x1b[2K\x1b[1GApprove? [Y/n] ";
    let code = file_unchecked(
        &e.vault_path(),
        e.a(),
        hostile,
        Timestamp::now(),
        Timestamp::now() + 24.hours(),
    );
    let p = FakePrompter::new();

    let r = e.a().approve(&["--code", &code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("malformed"), "{}", r.stderr);
    for out in [&r.stdout, &r.stderr] {
        assert!(!out.contains('\x1b'), "{out:?}");
        assert!(!out.contains("evil"), "{out:?}");
    }
    assert!(p.confirm_prompts().is_empty());
    assert!(p.unlock_requests().is_empty());
}

// P7 · E4's parity check against the `tests/fixtures/go/help/` references:
// each command's name, group, usage, description and flags, and the
// `recipient` parent's whole child list, read from its fixture. A
// reference, not a byte gate.
#[test]
fn p7_recipient_enrollment_commands_have_their_go_fixture_flags_and_descriptions() {
    use crate::identity_cli_tests::assert_child;
    use crate::registry::{Availability, GROUP_RECIPIENT, registry};

    let env = Env::new();
    for (name, usage, fixture, flags) in [
        (
            "recipient pending",
            ("", "gage recipient pending"),
            include_str!("../../../tests/fixtures/go/help/recipient_pending.txt"),
            &["-u", "--use"][..],
        ),
        (
            "recipient approve",
            ("[ID...]", "[ID]..."),
            include_str!("../../../tests/fixtures/go/help/recipient_approve.txt"),
            &["--code", "--device", "-u", "--use"][..],
        ),
        (
            "recipient deny",
            ("<ID>", "<ID>"),
            include_str!("../../../tests/fixtures/go/help/recipient_deny.txt"),
            &["-u", "--use"][..],
        ),
    ] {
        let args: Vec<&str> = name.split(' ').chain(["--help"]).collect();
        let help = env.ok(&args).stdout;
        let first = fixture.lines().next().unwrap().trim_end_matches('.');
        assert!(help.starts_with(first), "{name}:\n{help}");
        for line in fixture
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("Usage:"))
        {
            assert!(help.contains(line.trim()), "{name}: {line:?}\n{help}");
        }
        // The registry carries Go's usage; clap renders it its own way.
        let (registered, rendered) = usage;
        assert!(help.contains(rendered), "{name}: {rendered}\n{help}");
        for line in fixture.lines().filter(|l| l.trim_start().starts_with('-')) {
            let text = line.split("   ").last().unwrap().trim();
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
        for flag in flags {
            assert!(help.contains(flag), "{name}: {flag}\n{help}");
        }
        let command = registry().find(name).unwrap().clone();
        assert_eq!(command.usage, registered, "{name}");
        assert_eq!(command.group, GROUP_RECIPIENT, "{name}");
        assert_eq!(command.availability, Availability::Both, "{name}");
    }

    let parent = env.ok(&["recipient"]).stdout;
    let fixture = include_str!("../../../tests/fixtures/go/help/recipient.txt");
    let children: Vec<(&str, &str)> = fixture
        .lines()
        .skip_while(|l| !l.starts_with("Available Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .map(|l| {
            let l = l.trim();
            let (child, short) = l.split_once(' ').unwrap();
            (child, short.trim())
        })
        .collect();
    assert_eq!(children.len(), 7, "{fixture}");
    for (child, short) in children {
        assert_child(&parent, child, short);
    }

    // `--code` repeats rather than taking the last value.
    let e = Enrolled::new("phone-1");
    let r = e.a().approve(
        &["--code", "GAGE-0000-0000-0000-0000", "--code", &e.code],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::LockedOrAuth, "both codes tried: {}", r.stderr);
}
