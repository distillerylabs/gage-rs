//! `clone`'s offer to enroll. Interactive
//! rows drive the *real* terminal prompter over a scripted stdin, which is
//! what lets a test see the prompt as rendered rather than only the answer
//! a fake gave.

use gage::exitcode::Code;
use gage::gittest::{self, BareRemote};
use gage::prompter::FakePrompter;
use zeroize::Zeroizing;

use crate::enroll_cli_tests::{code_in, pending_on_remote};
use crate::identity_cli_tests::{A_DEVICE, VAULT, identity_files};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::repl_tests::script;
use crate::script_tests::Run;

/// An owner device and its published vault: what a joining machine clones.
fn owned_vault() -> (Env, BareRemote, String) {
    let owner = Env::new();
    let origin = gittest::new_bare_remote();
    let url = origin.path().to_str().unwrap().to_string();
    owner.ok(&[
        "init",
        VAULT,
        "--device",
        A_DEVICE,
        "--remote",
        &url,
        "--no-recovery-key",
    ]);
    (owner, origin, url)
}

/// `clone` at a terminal with the real prompter reading `stdin`.
fn clone_interactive(env: &Env, args: &[&str], stdin: &str) -> CliResult {
    env.script_run_real_prompter(
        args,
        Run {
            stdin,
            tty: true,
            ..Run::default()
        },
    )
}

/// The scripted stdin for an accepted offer: the answer to the offer, then
/// the new passphrase twice.
fn answers(offer: &str) -> String {
    format!("{offer}{PASSPHRASE}\n{PASSPHRASE}\n")
}

fn clone_args<'a>(url: &'a str, device: &'a str) -> Vec<&'a str> {
    vec!["clone", url, "--name", VAULT, "--device", device]
}

/// The message a user copies out of a declined or no-terminal clone.
fn assert_access_lines(out: &str) {
    for want in [
        "cannot decrypt",
        "gage identity enroll",
        "gage identity add",
        "gage recipient add",
        "gage recovery enroll",
    ] {
        assert!(out.contains(want), "clone said {out:?}, want {want:?}");
    }
}

// The same end
// state as `clone` then `identity enroll`.
#[test]
fn interactive_clone_offers_to_enroll_and_yes_publishes() {
    let (_owner, origin, url) = owned_vault();
    let b = Env::new();

    let r = clone_interactive(&b, &clone_args(&url, "phone-1"), &answers("y\n"));

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stderr.contains("enrollment request"), "{}", r.stderr);
    assert!(r.stdout.contains("GAGE-"), "{}", r.stdout);
    assert_eq!(pending_on_remote(&origin).len(), 1);
    assert_eq!(identity_files(&b).len(), 1);
    assert!(!b.entry(VAULT).pubkey.is_empty(), "no pubkey was recorded");
}

// A `confirm_default_yes` wired to
// `confirm` still compiles and still passes a yes-answering test, so the
// bare Enter and the `[Y/n]` are asserted together.
#[test]
fn clone_offer_defaults_to_yes() {
    let (_owner, origin, url) = owned_vault();
    let b = Env::new();

    let r = clone_interactive(&b, &clone_args(&url, "phone-1"), &answers("\n"));

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stderr.contains("[Y/n]"), "{}", r.stderr);
    assert_eq!(pending_on_remote(&origin).len(), 1, "a bare Enter enrolls");
}

// Adding a default-yes
// question must not have changed the default of the one that was already
// there: removing this device's own key still declines on Enter.
#[test]
fn confirms_default_is_unchanged_everywhere_else() {
    let (owner, _origin, _url) = owned_vault();
    owner.add_recipient(
        &age::x25519::Identity::generate().to_public().to_string(),
        "phone-1",
    );

    let r = owner.script_run_real_prompter(
        &["recipient", "remove", A_DEVICE, "--reencrypt"],
        Run {
            // The unlock first, then the confirmation under test.
            stdin: &format!("{PASSPHRASE}\n\n"),
            tty: true,
            ..Run::default()
        },
    );

    assert!(r.stderr.contains("[y/N]"), "{}", r.stderr);
    let recipients = owner.vault_config(VAULT).recipients;
    assert!(
        recipients.iter().any(|r| r.device == A_DEVICE),
        "a bare Enter removed this device's own key: {recipients:?}"
    );
}

#[test]
fn clone_offer_declined_leaves_the_vault_cloned() {
    let (_owner, origin, url) = owned_vault();
    let b = Env::new();

    let r = clone_interactive(&b, &clone_args(&url, "phone-1"), "n\n");

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    b.entry(VAULT);
    assert!(identity_files(&b).is_empty());
    assert!(pending_on_remote(&origin).is_empty());
    assert_access_lines(&r.stdout);

    // And the route the message points at still works.
    let r = b.run(&["identity", "enroll", "--device", "phone-1"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(pending_on_remote(&origin).len(), 1);
}

// A sole owner holding a
// paper key was told only to publish an enrollment request, which needs
// somebody else to approve it: for them, a dead end. Both places a device
// with no identity lands say so: clone's message, and the session's `use`.
#[test]
fn clone_and_session_point_at_recovery_enroll() {
    let owner = Env::new();
    let origin = gittest::new_bare_remote();
    let url = origin.path().to_str().unwrap().to_string();
    let rk = owner.root.path().join("rk");
    owner.ok(&[
        "init",
        VAULT,
        "--device",
        A_DEVICE,
        "--remote",
        &url,
        "--recovery-key-out",
        rk.to_str().unwrap(),
    ]);

    let b = Env::new();
    let r = b.run_with(&clone_args(&url, "phone-1"), &FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout
            .contains("`gage recovery enroll` — it admits this device and retires the key it used"),
        "{}",
        r.stdout
    );
    let named = crate::recovery_cli_tests::assert_names_real_commands(&r.stdout);
    assert!(named.iter().any(|c| c == "recovery enroll"), "{named:?}");

    let r = b.script_run(
        &["--stdin"],
        Run {
            stdin: &script(&[&format!("use {VAULT}")]),
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("gage identity enroll"), "{}", r.stderr);
    assert!(
        r.stderr
            .contains("or `gage recovery enroll` if you hold its recovery key"),
        "{}",
        r.stderr
    );
    let named = crate::recovery_cli_tests::assert_names_real_commands(&r.stderr);
    assert!(named.iter().any(|c| c == "recovery enroll"), "{named:?}");
}

// The message is the
// reworded one, naming `identity enroll` beside the manual path.
#[test]
fn non_interactive_clone_never_offers_and_exits_zero() {
    let (_owner, origin, url) = owned_vault();
    let b = Env::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);

    let r = b.run_with(&clone_args(&url, "phone-1"), &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(p.confirm_default_yes_prompts().is_empty(), "it offered");
    assert!(identity_files(&b).is_empty());
    assert!(pending_on_remote(&origin).is_empty());
    assert_access_lines(&r.stdout);
}

// `clone` is one-shot
// only here, so `--stdin` and `--script` cannot reach it at all: the rule
// is the terminal, and a script's clone is refused before it could offer.
#[test]
fn the_offer_tracks_where_a_create_can_be_answered() {
    let (_owner, origin, url) = owned_vault();

    // A terminal: offered (and answered, here with no).
    let b = Env::new();
    let p = FakePrompter::new().with_confirm_default_yes(false);
    let r = b.script_run(
        &clone_args(&url, "phone-1"),
        Run {
            tty: true,
            ..Run::default()
        },
        &p,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(
        p.confirm_default_yes_prompts().len(),
        1,
        "a terminal offers"
    );

    // No terminal: never offered (the row above, via a fake that would
    // panic on an unscripted offer).
    let b = Env::new();
    let r = b.run_with(&clone_args(&url, "phone-1"), &FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    // `--stdin`: the command stream is stdin, and clone is not a session
    // command, so nothing is cloned, offered or published.
    let b = Env::new();
    let line = format!("clone {url} --name {VAULT} --device phone-1");
    let r = b.script_run(
        &["--stdin"],
        Run {
            stdin: &script(&[&line]),
            tty: true,
            ..Run::default()
        },
        &FakePrompter::new(),
    );
    assert_ne!(r.code, Code::Success, "{}", r.stdout);
    assert!(pending_on_remote(&origin).is_empty());
}

// A local
// identity was set up deliberately, and clone can't cheaply tell whether
// it is already a recipient, so it is left alone and no unlock is tried.
#[test]
fn clone_does_not_offer_when_this_device_already_holds_an_identity() {
    let (_owner, origin, url) = owned_vault();
    let b = Env::new();
    let r = clone_interactive(&b, &clone_args(&url, "phone-1"), "n\n");
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    b.identity_add(&["--device", "phone-1"]);

    // The same vault again under a second local name: the identity file
    // is keyed by the vault's id, so this device holds one for it.
    let again = b.root.path().join("again");
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let r = b.script_run(
        &[
            "clone",
            &url,
            "--name",
            "personal-again",
            "--dir",
            again.to_str().unwrap(),
            "--device",
            "phone-1",
        ],
        Run {
            tty: true,
            ..Run::default()
        },
        &p,
    );

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(p.unlock_requests().is_empty(), "prompted during a clone");
    assert!(p.confirm_default_yes_prompts().is_empty(), "it offered");
    assert!(!r.stdout.contains("cannot decrypt"), "{}", r.stdout);
    assert!(pending_on_remote(&origin).is_empty());
}

// The offer uses the
// name clone resolved, not a second one derived from the hostname, and
// the sealed request agrees with the printed line.
#[test]
fn clone_device_flag_carries_through_to_the_offer() {
    let (_owner, _origin, url) = owned_vault();
    let b = Env::new();

    let r = clone_interactive(&b, &clone_args(&url, "deliberately-named"), &answers("y\n"));

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("deliberately-named"), "{}", r.stdout);
    let entry = b.entry(VAULT);
    assert_eq!(entry.device, "deliberately-named");

    let code = code_in(&r.stdout);
    let vault = gage::vault::Vault::from_registration(&b.dirs, VAULT, &entry).unwrap();
    let pending = vault.pending_enrollments().unwrap();
    let opened = vault
        .open_enrollment(&pending, &[Zeroizing::new(code)])
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].device, "deliberately-named");
}
