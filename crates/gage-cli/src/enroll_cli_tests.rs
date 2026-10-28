//! `gage identity enroll` through the real CLI, in-process: Go's
//! `enroll_cli_test.go`. Each test names its Go row. [`TwoDevices`] gives
//! device A (the owner, `init --remote`) and device B (cloned, no
//! identity), the state a joining machine starts from.

use std::time::Duration;

use gage::exitcode::Code;
use gage::gittest::{self, BareRemote};
use gage::prompter::FakePrompter;

use crate::commands::human_duration;
use crate::identity_cli_tests::{A_DEVICE, B_DEVICE, TwoDevices, VAULT, identity_files};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::repl_tests::script;
use crate::script_tests::Run;

fn enroll_args<'a>(device: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["identity", "enroll", "--device", device];
    args.extend_from_slice(extra);
    args
}

impl Env {
    /// `gage identity enroll --device DEVICE`, answering the new
    /// identity's passphrase.
    fn enroll(&self, device: &str, extra: &[&str]) -> CliResult {
        self.run(&enroll_args(device, extra))
    }
}

/// The names under `.gage/pending/` at the remote's HEAD, read from a
/// fresh clone of it.
pub(crate) fn pending_on_remote(remote: &BareRemote) -> Vec<String> {
    let clone = gittest::Device::clone(remote.path());
    let Ok(dir) = std::fs::read_dir(clone.path().join(".gage/pending")) else {
        return Vec::new();
    };
    dir.map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect()
}

fn remote_file(remote: &BareRemote, rel: &str) -> Vec<u8> {
    let clone = gittest::Device::clone(remote.path());
    std::fs::read(clone.path().join(rel)).unwrap()
}

pub(crate) fn code_in(stdout: &str) -> String {
    stdout
        .split_whitespace()
        .find(|w| w.starts_with("GAGE-"))
        .unwrap_or_else(|| panic!("no code in {stdout:?}"))
        .to_string()
}

fn stray_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

// Go: TestIdentityEnrollPublishesARequestAndPrintsTheCode
#[test]
fn identity_enroll_publishes_a_request_and_prints_the_code() {
    let t = TwoDevices::new();
    let r = t.b.enroll("phone-1", &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert!(r.stdout.contains("gage recipient approve"), "{}", r.stdout);
    assert_eq!(pending_on_remote(&t._origin).len(), 1);
}

// Go: TestIdentityEnrollRecordsThisDevicesPubkey. Without it the orphan
// check keeps the file on every enrolled device, and enroll's own
// already-a-recipient check has nothing to compare next time.
#[test]
fn identity_enroll_records_this_devices_pubkey() {
    let t = TwoDevices::new();
    let r = t.b.enroll("phone-1", &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let entry = t.b.entry(VAULT);
    assert_eq!(entry.device, "phone-1");
    assert_eq!(entry.method, "passphrase");
    assert!(!entry.pubkey.is_empty());
    assert!(r.stdout.contains(&entry.pubkey), "{}", r.stdout);
}

// Go: TestIdentityEnrollTheSecondTimeIsANoOpOnceApproved. The key is added
// by `recipient add`, since approve is E4's.
#[test]
fn identity_enroll_the_second_time_is_a_no_op_once_approved() {
    let t = TwoDevices::new();
    let key = {
        t.b.identity_add(&["--device", "phone-1"]);
        t.b.entry(VAULT).pubkey
    };
    t.a.add_recipient(&key, "phone-1");

    let r =
        t.b.run_with(&enroll_args("phone-1", &[]), &FakePrompter::new());

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(!r.stderr.contains("--device"), "{}", r.stderr);
    assert!(!r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert!(r.stdout.contains("already"), "{}", r.stdout);
    assert!(pending_on_remote(&t._origin).is_empty());
}

// Go: TestIdentityEnrollRefusesANameThatLabelsSomeoneElse
#[test]
fn identity_enroll_refuses_a_name_that_labels_someone_else() {
    let t = TwoDevices::new();
    t.a.add_recipient(&stray_key(), "phone-1");

    let r =
        t.b.run_with(&enroll_args("phone-1", &[]), &FakePrompter::new());
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("--device"), "{}", r.stderr);
    assert!(pending_on_remote(&t._origin).is_empty());

    // The recovery the message points at.
    let r = t.b.enroll("phone-2", &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(pending_on_remote(&t._origin).len(), 1);
}

// Go: TestIdentityEnrollRefusesAnUnusableTTL
#[test]
fn identity_enroll_refuses_an_unusable_ttl() {
    for ttl in ["0", "-1h", "8d", "169h", "soon"] {
        let t = TwoDevices::new();
        let r = t.b.run_with(
            &enroll_args("phone-1", &["--ttl", ttl]),
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Usage, "--ttl {ttl}: {}", r.stderr);
        assert!(pending_on_remote(&t._origin).is_empty(), "--ttl {ttl}");
        assert!(identity_files(&t.b).is_empty(), "--ttl {ttl}");
    }
}

/// The lifetime of the one request on the remote, from its filename epoch.
fn request_lifetime(remote: &BareRemote) -> i64 {
    let names = pending_on_remote(remote);
    assert_eq!(names.len(), 1, "{names:?}");
    let epoch: i64 = names[0]
        .strip_suffix(".age")
        .and_then(|n| n.rsplit('-').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no epoch in {:?}", names[0]));
    epoch - jiff::Timestamp::now().as_second()
}

// Go: TestIdentityEnrollHonorsTTLAndDefaultsTo24h
#[test]
fn identity_enroll_honors_ttl_and_defaults_to_24h() {
    for (extra, want) in [(&[][..], 24 * 3600), (&["--ttl", "2h"][..], 2 * 3600)] {
        let t = TwoDevices::new();
        let r = t.b.enroll("phone-1", extra);
        assert_eq!(r.code, Code::Success, "{}", r.stderr);
        let got = request_lifetime(&t._origin);
        assert!((want - 60..=want).contains(&got), "{extra:?}: {got}");
    }
}

// Go: TestIdentityEnrollRefusesAnInvalidDeviceName
#[test]
fn identity_enroll_refuses_an_invalid_device_name() {
    let t = TwoDevices::new();
    let r =
        t.b.run_with(&enroll_args("../etc/passwd", &[]), &FakePrompter::new());
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(pending_on_remote(&t._origin).is_empty());
    assert!(identity_files(&t.b).is_empty());
}

// Go: TestIdentityEnrollRemoteErrorsCarryTheirExitCodes
#[test]
fn identity_enroll_remote_errors_carry_their_exit_codes() {
    // No remote: its own error, naming the command that configures one.
    let local = Env::new();
    local.init("local-only", &["--device", A_DEVICE]);
    let r = local.run_with(&enroll_args("phone-1", &[]), &FakePrompter::new());
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("set-remote"), "{}", r.stderr);

    // A remote that isn't there.
    let t = TwoDevices::new();
    let gone = t.b.root.path().join("gone.git");
    t.b.ok(&["git", "set-remote", VAULT, gone.to_str().unwrap()]);
    let r =
        t.b.run_with(&enroll_args("phone-1", &[]), &FakePrompter::new());
    assert_ne!(r.code, Code::Success, "{}", r.stdout);
    assert!(identity_files(&t.b).is_empty());
    assert!(pending_on_remote(&t._origin).is_empty());
}

// Go: TestIdentityEnrollSelectsAVaultWithUse
#[test]
fn identity_enroll_selects_a_vault_with_use() {
    let t = TwoDevices::new();
    let work_origin = gittest::new_bare_remote();
    let url = work_origin.path().to_str().unwrap();
    t.a.ok(&[
        "init",
        "work",
        "--device",
        A_DEVICE,
        "--remote",
        url,
        "--no-recovery-key",
    ]);
    let r = t.b.run_with(
        &["clone", url, "--name", "work", "--device", B_DEVICE],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    let r =
        t.b.run(&["identity", "enroll", "--use", "work", "--device", "phone-1"]);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(pending_on_remote(&work_origin).len(), 1);
    assert!(
        pending_on_remote(&t._origin).is_empty(),
        "--use named the other vault"
    );
}

// Go: TestTheTwoSecretsNeverCross
#[test]
fn the_two_secrets_never_cross() {
    let t = TwoDevices::new();
    let r = t.b.enroll("phone-1", &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    for stream in [&r.stdout, &r.stderr] {
        assert!(
            !stream.contains(PASSPHRASE),
            "the passphrase reached output"
        );
    }
    let names = pending_on_remote(&t._origin);
    for name in &names {
        let blob = remote_file(&t._origin, &format!(".gage/pending/{name}"));
        let text = String::from_utf8_lossy(&blob);
        assert!(!text.contains(PASSPHRASE), "the passphrase was committed");
    }
    let code = code_in(&r.stdout);
    let id = t.b.entry(VAULT).id;
    let identity = gage::paths::identity_file_path(&t.b.dirs, &id, "phone-1").unwrap();
    let wrapped = std::fs::read(identity).unwrap();
    assert!(
        !String::from_utf8_lossy(&wrapped).contains(&code),
        "the code reached the identity file"
    );
    for name in &names {
        let blob = remote_file(&t._origin, &format!(".gage/pending/{name}"));
        assert!(
            !String::from_utf8_lossy(&blob).contains(&code),
            "the code reached committed plaintext"
        );
    }
}

// Go: TestBothSecretsAreLabelledAtTheirPointOfUse. The passphrase half is
// rendered by the real terminal prompter, so it is asserted there.
#[test]
fn both_secrets_are_labelled_at_their_point_of_use() {
    let t = TwoDevices::new();
    let r = t.b.enroll("phone-1", &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("safe to send"), "{}", r.stdout);
    assert!(r.stdout.contains("not your passphrase"), "{}", r.stdout);

    use gage::prompter::{Kind, Prompter, Purpose, UnlockRequest};
    let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
    struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let input = format!("{PASSPHRASE}\n{PASSPHRASE}\n");
    let p = crate::prompter::TerminalPrompter::new(
        std::io::Cursor::new(input.into_bytes()),
        Sink(out.clone()),
        crate::prompter::SecretInput::Lines,
    );
    p.unlock(&UnlockRequest {
        kind: Kind::Passphrase,
        purpose: Purpose::Create,
        vault: VAULT.into(),
        device: "phone-1".into(),
        attempt: 1,
    })
    .unwrap();
    let said = String::from_utf8(out.borrow().clone()).unwrap();
    assert!(said.contains("stays on this device"), "{said}");
}

// Go: TestHumanDurationReadsLikeAPersonSaidIt
#[test]
fn human_duration_reads_like_a_person_said_it() {
    let h = |h: u64, m: u64| Duration::from_secs(h * 3600 + m * 60);
    for (input, want) in [
        (h(24, 0), "24h"),
        (h(2, 30), "2h30m"),
        (h(0, 20), "20m"),
        (h(168, 0), "168h"),
        // A hair under: what is left moments after a request is sealed.
        (Duration::from_secs(24 * 3600 - 1), "24h"),
    ] {
        assert_eq!(human_duration(input), want);
    }
}

// --- session and script mode -------------------------------------------------

fn stdin_script(env: &Env, lines: &[&str], passphrase: Option<&str>, tty: bool) -> CliResult {
    env.script_run(
        &["--stdin"],
        Run {
            stdin: &script(lines),
            tty,
            passphrase_env: passphrase,
        },
        &FakePrompter::new(),
    )
}

// Go: TestIdentityEnrollWorksInASessionAgainstAVaultThatWasNeverUnlocked.
// On the reuse path, the one a script can answer.
#[test]
fn identity_enroll_works_in_a_session_against_a_vault_that_was_never_unlocked() {
    let t = TwoDevices::new();
    t.b.identity_add(&["--device", "phone-1"]);

    let r = stdin_script(
        &t.b,
        &["identity enroll -u personal --device phone-1"],
        Some(PASSPHRASE),
        false,
    );

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert_eq!(pending_on_remote(&t._origin).len(), 1);
}

// Go: TestUseStillFailsOnAVaultThisDeviceCannotDecrypt. The hint is M6's
// obligation.
#[test]
fn use_still_fails_on_a_vault_this_device_cannot_decrypt() {
    let t = TwoDevices::new();
    let r = stdin_script(&t.b, &["use personal"], Some(PASSPHRASE), false);
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("identity enroll"), "{}", r.stderr);
}

// Go: TestScriptedEnrollRefusesTheCreatePath. A new identity's passphrase
// is a Create answer, and GAGE_PASSPHRASE deliberately never answers one.
#[test]
fn scripted_enroll_refuses_the_create_path() {
    for env_var in [None, Some(PASSPHRASE)] {
        let t = TwoDevices::new();
        let r = stdin_script(
            &t.b,
            &["identity enroll -u personal --device phone-1"],
            env_var,
            false,
        );
        assert_eq!(r.code, Code::LockedOrAuth, "{env_var:?}: {}", r.stderr);
        assert!(identity_files(&t.b).is_empty());
        assert!(pending_on_remote(&t._origin).is_empty());
    }
}

// Go: TestScriptedEnrollSucceedsOnTheReusePath: the retry-after-a-failed-
// push path, and the reason enroll is scriptable at all.
#[test]
fn scripted_enroll_succeeds_on_the_reuse_path() {
    let t = TwoDevices::new();
    t.b.identity_add(&["--device", "phone-1"]);
    let r = stdin_script(
        &t.b,
        &["identity enroll -u personal --device phone-1"],
        Some(PASSPHRASE),
        false,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert_eq!(pending_on_remote(&t._origin).len(), 1);
}

// Go: TestARefusedScriptedEnrollLeavesTheFastForwardInPlace. The refusal
// lands after the catch-up, with the lock released and nothing to undo.
#[test]
fn a_refused_scripted_enroll_leaves_the_fast_forward_in_place() {
    let t = TwoDevices::new();
    gittest::Device::clone(t._origin.path()).write_commit_push(
        "README",
        "moved on",
        "another device wrote this",
    );

    let r = stdin_script(
        &t.b,
        &["identity enroll -u personal --device phone-1"],
        None,
        false,
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);

    let readme = std::fs::read_to_string(t.b.entry(VAULT).path.join("README")).unwrap();
    assert!(readme.contains("moved on"), "{readme}");
    // And the lock was released: the next command can take it.
    let r = t.b.identity_add(&["--device", "phone-1"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// Go: TestScriptFileAtATerminalTakesTheCreatePathNormally
#[test]
fn script_file_at_a_terminal_takes_the_create_path_normally() {
    let t = TwoDevices::new();
    let file =
        t.b.script_file(&["identity enroll -u personal --device phone-1"]);
    let r = t.b.script_run(
        &["--script", &file],
        Run {
            tty: true,
            ..Run::default()
        },
        &FakePrompter::new().with_passphrase(PASSPHRASE),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert_eq!(pending_on_remote(&t._origin).len(), 1);
}
