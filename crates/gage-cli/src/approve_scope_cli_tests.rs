//! `recipient approve`'s scope, its late unlock across both invocation
//! modes, and its two questions: Go's `approve_scope_cli_test.go`. Each
//! test names its Go row.

use gage::exitcode::Code;
use gage::gitrepo;
use gage::gittest;
use gage::prompter::FakePrompter;
use jiff::tz::{TimeZone, offset};
use jiff::{Timestamp, ToSpan};

use crate::approve_cli_tests::{Enrolled, approving, common_hex_digit, pending_ids, still_pending};
use crate::enroll_cli_tests::code_in;
use crate::identity_cli_tests::VAULT;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::repl_tests::{passphrase_prompts, script};
use crate::script_tests::Run;
use crate::trustcache_cli_tests::commit_routine_change;

/// A device listed in the vault by hand, without re-encrypting, so it
/// can't read what was written before (the damage A19 left one way to
/// reach). Returns its machine, caught up.
fn partially_admitted(e: &Enrolled, device: &str) -> Env {
    let env = Env::new();
    let url = e.t._origin.path().to_str().unwrap();
    let r = env.run_with(
        &["clone", url, "--name", VAULT, "--device", device],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Success, "clone: {}", r.stderr);
    env.ok(&["identity", "add", "--device", device]);
    let key = env.entry(VAULT).pubkey;
    commit_routine_change(&e.vault_path(), device, &key);
    gitrepo::push(&e.vault_path(), &gittest::remote_options()).unwrap();
    env.ok(&["pull"]);
    env
}

/// Fills `pending/` with `n` live-looking files no code opens, and commits.
fn stuff_pending(e: &Enrolled, n: usize) {
    let dir = e.vault_path().join(".gage/pending");
    let expires = (Timestamp::now() + 2.hours()).as_second();
    for _ in 0..n {
        let name = format!("{}-{expires}.age", uuid::Uuid::new_v4());
        std::fs::write(dir.join(name), "not a seal anyone will open").unwrap();
    }
    gitrepo::commit_all(&e.vault_path(), "test: a stuffed pending directory").unwrap();
}

/// A real `--script` run with the terminal prompter and
/// `GAGE_PASSPHRASE` set, as CI runs one.
fn real_script(env: &Env, lines: &[&str]) -> crate::lifecycle_tests::CliResult {
    let file = env.script_file(lines);
    env.script_run_real_prompter(
        &["--script", &file],
        Run {
            stdin: "",
            tty: false,
            passphrase_env: Some(PASSPHRASE),
        },
    )
}

// Go: TestApprovalPromptsOnceForTheApproversPassphrase. A git writer
// holding no key of this vault can't approve.
#[test]
fn approval_prompts_once_for_the_approvers_passphrase() {
    let e = Enrolled::new("phone-1");
    let p = approving();
    let r = e.a().approve(&["--code", &e.code], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(p.unlock_requests().len(), 1);
}

// Go: TestTheFullAccessRefusalCostsOneUnlockNotZero. The pre-flight is a
// decryption pass with no identity-free form, so its refusal comes after
// the one unlock: the documented cost of the late unlock. Zero here would
// be asserting the impossible.
#[test]
fn the_full_access_refusal_costs_one_unlock_not_zero() {
    let e = Enrolled::new("phone-1");
    e.a().insert_ok("owner only", &["Bank"]);
    e.a().ok(&["push"]);
    let partial = partially_admitted(&e, "desktop-1");
    // Its first unlock writes its trust baseline (the listing itself fails:
    // it can't read A's entry); then the list changes again, so M10's
    // question is live and "not asked" means something.
    assert_ne!(partial.cmd(&["ls"]).code, Code::Success);
    let stray = age::x25519::Identity::generate().to_public().to_string();
    e.a().ok(&["pull"]);
    commit_routine_change(&e.vault_path(), "tablet-1", &stray);
    gitrepo::push(&e.vault_path(), &gittest::remote_options()).unwrap();
    partial.ok(&["pull"]);
    let p = approving().with_recipient_answers(&[true]);

    let r = partial.approve(&["--code", &e.code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("cannot read every entry"), "{}", r.stderr);
    assert_eq!(p.unlock_requests().len(), 1);
    assert!(
        p.recipient_changes().is_empty(),
        "M10 asked before the refusal"
    );
}

// Go: TestApproveInASessionReusesTheCachedIdentity. `use` unlocked it;
// approve asks for no second passphrase.
#[test]
fn approve_in_a_session_reuses_the_cached_identity() {
    let e = Enrolled::new("phone-1");
    let approve = format!("recipient approve --code {}", e.code);

    let r = e.a().session(&script(&[
        "use personal",
        PASSPHRASE,
        &approve,
        "y",
        "exit",
    ]));

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(passphrase_prompts(&r), 1, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("approved \"phone-1\""), "{}", r.stdout);
    assert!(!still_pending(e.a()));
}

// Go: TestTheApproverAnswersTwoDistinctQuestions. "Let this device in" is
// approve's; "do you trust the list" is M10's, asked here because another
// device changed the list. Never collapsed, and declining the second
// aborts with nothing committed.
#[test]
fn the_approver_answers_two_distinct_questions() {
    let e = Enrolled::new("phone-1");
    // A's trust cache, written by its first encrypting write; the list
    // changes after it.
    e.a().insert_ok("primes the cache", &["Bank"]);
    let _partial = partially_admitted(&e, "desktop-1");
    let before = e.commits();

    let declines = approving().with_recipient_answers(&[false]);
    let r = e.a().approve(&["--code", &e.code], &declines);
    assert_ne!(r.code, Code::Success, "declined M10, and still approved");
    assert_eq!(declines.confirm_prompts().len(), 1);
    assert_eq!(declines.recipient_changes().len(), 1);
    assert_eq!(e.commits(), before);
    assert!(still_pending(e.a()));

    let accepts = approving().with_recipient_answers(&[true]);
    let r = e.a().approve(&["--code", &e.code], &accepts);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let asked = accepts.confirm_prompts();
    assert_eq!(asked.len(), 1);
    assert_eq!(accepts.recipient_changes().len(), 1);
    assert!(!asked[0].contains("trust"), "{asked:?}");
}

// Go: TestApproveRefusesATakenNameBeforeTheConfirmationAndBeforeAnyUnlock.
// It reads only the plaintext list, so it genuinely can come first; and
// --device is the way out.
#[test]
fn approve_refuses_a_taken_name_before_the_confirmation_and_before_any_unlock() {
    let e = Enrolled::new("phone-1");
    let stray = age::x25519::Identity::generate().to_public().to_string();
    e.a()
        .ok(&["recipient", "add", &stray, "--device", "phone-1"]);
    let p = FakePrompter::new();

    let r = e.a().approve(&["--code", &e.code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("--device"), "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
    assert!(p.unlock_requests().is_empty());

    let r = e
        .a()
        .approve(&["--code", &e.code, "--device", "phone-2"], &approving());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// Go: TestApproveSurfacesARequestExpiredByItsSeal. A real request with a
// one-second TTL, left to lapse, then filed under a live-looking name:
// the sealed expiry is what approval enforces, before any question.
#[test]
fn approve_surfaces_a_request_expired_by_its_seal() {
    let e = Enrolled::new("phone-1");
    let tablet = Env::new();
    let url = e.t._origin.path().to_str().unwrap();
    tablet.run_with(
        &["clone", url, "--name", VAULT, "--device", "tablet-1"],
        &FakePrompter::new(),
    );
    let r = tablet.run(&["identity", "enroll", "--device", "tablet-1", "--ttl", "1s"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let code = code_in(&r.stdout);
    e.a().ok(&["pull"]);
    std::thread::sleep(std::time::Duration::from_millis(2100));

    let dir = e.vault_path().join(".gage/pending");
    let live = (Timestamp::now() + 2.hours()).as_second();
    for f in std::fs::read_dir(&dir).unwrap() {
        let name = f.unwrap().file_name().into_string().unwrap();
        let fresh = format!("{}-{live}.age", &name[..36]);
        std::fs::rename(dir.join(&name), dir.join(fresh)).unwrap();
    }
    gitrepo::commit_all(
        &e.vault_path(),
        "test: live names over the seals as they are",
    )
    .unwrap();
    let p = FakePrompter::new();

    let r = e.a().approve(&["--code", &code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("expired"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(p.confirm_prompts().is_empty());
}

// Go: TestApproveOnAnAmbiguousIDRendersTheCandidates. Approve and deny
// resolve through one function, so they render one list.
#[test]
fn approve_on_an_ambiguous_id_renders_the_candidates() {
    let e = Enrolled::new("phone-1");
    let (_tablet, _) = e.enroll_another("tablet-1");
    let ids = pending_ids(&e.vault_path());

    let r = e.a().approve(
        &[&common_hex_digit(&ids), "--code", &e.code],
        &FakePrompter::new(),
    );

    assert_eq!(r.code, Code::Ambiguous, "{}", r.stderr);
    for id in &ids {
        assert!(r.stderr.contains(&id[..8]), "{}", r.stderr);
    }
}

// Go: TestApproveWithNoIDAgainstAStuffedPendingRefuses. Past the bound, a
// broad run refuses before any decryption or question, and says what to
// type instead.
#[test]
fn approve_with_no_id_against_a_stuffed_pending_refuses() {
    let e = Enrolled::new("phone-1");
    stuff_pending(&e, 40);
    let before = e.commits();
    let p = FakePrompter::new();

    let r = e.a().approve(&["--code", &e.code], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("recipient approve <ID>"), "{}", r.stderr);
    assert!(r.stderr.contains("recipient pending"), "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
    assert!(p.unlock_requests().is_empty());
    assert!(p.value_prompts().is_empty());
    assert_eq!(e.commits(), before);
}

// Go: TestNamingAnIDIsTheWayOutOfAStuffedPending. `pending` and `deny`
// open nothing, so a stuffed directory stays inspectable and cleanable,
// and `approve <ID>` narrows the question.
#[test]
fn naming_an_id_is_the_way_out_of_a_stuffed_pending() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();
    stuff_pending(&e, 40);
    let other = pending_ids(&e.vault_path())
        .into_iter()
        .find(|i| *i != id)
        .unwrap();

    e.a().ok(&["recipient", "pending"]);
    let r = e
        .a()
        .run_with(&["recipient", "deny", &other], &FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    let r = e.a().approve(&[&id, "--code", &e.code], &approving());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// Go: TestTheBoundFollowsTheScopeNotTheFlag. Naming every id gives the
// same oversized scope a broad run does, and the same refusal: the bound
// is a property of the scope, not of whether an ID was typed.
#[test]
fn the_bound_follows_the_scope_not_the_flag() {
    let e = Enrolled::new("phone-1");
    stuff_pending(&e, 40);
    let ids = pending_ids(&e.vault_path());
    let mut args: Vec<&str> = ids.iter().map(String::as_str).collect();
    args.extend(["--code", "GAGE-0000-0000-0000-0000"]);

    let r = e.a().approve(&args, &FakePrompter::new());

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("too many pending"), "{}", r.stderr);
}

// Go: TestApproveUnderARealScriptRunFailsCleanly. The terminal prompter in
// a script has nobody to answer approve's [y/N], so nothing is approved.
#[test]
fn approve_under_a_real_script_run_fails_cleanly() {
    let e = Enrolled::new("phone-1");
    let before = e.commits();
    let approve = format!("recipient approve --code {}", e.code);

    let r = real_script(e.a(), &["use personal", &approve]);

    assert_ne!(r.code, Code::Success, "{}", r.stdout);
    assert!(r.stderr.contains("nothing was approved"), "{}", r.stderr);
    assert_eq!(e.commits(), before);
    assert!(!e.a().ok(&["recipient", "list"]).stdout.contains("phone-1"));
}

// Go: TestPendingAndDenyUnderARealScriptRun
#[test]
fn pending_and_deny_under_a_real_script_run() {
    let e = Enrolled::new("phone-1");
    let id = pending_ids(&e.vault_path())[0].clone();
    let deny = format!("recipient deny {id}");

    let r = real_script(e.a(), &["use personal", "recipient pending", &deny]);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("removed pending request"), "{}", r.stdout);
    assert!(pending_ids(&e.vault_path()).is_empty());
}

// Go: TestPendingRendersExpiryInTheLocalTimezone. The same instant, two
// zones through `Io`, each reading its own wall clock; no process
// environment is touched (Q-PENDING-TZ).
#[test]
fn pending_renders_expiry_in_the_local_timezone() {
    let e = Enrolled::new("phone-1");
    let name = std::fs::read_dir(e.vault_path().join(".gage/pending"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let epoch: i64 = name[37..name.len() - 4].parse().unwrap();
    let expires = Timestamp::from_second(epoch).unwrap();

    let mut seen = Vec::new();
    for hours in [9, -4] {
        let tz = TimeZone::fixed(offset(hours));
        *e.a().time_zone.lock().unwrap() = tz.clone();
        let r = e.a().ok(&["recipient", "pending"]);
        let wall = expires.to_zoned(tz).strftime("%Y-%m-%d %H:%M").to_string();
        assert!(r.stdout.contains(&wall), "want {wall} in\n{}", r.stdout);
        seen.push(r.stdout);
    }
    assert_ne!(seen[0], seen[1], "one instant rendered alike in two zones");
}
