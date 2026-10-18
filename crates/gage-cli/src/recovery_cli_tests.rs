//! `gage recovery rotate`, the `recovery` parent, and the recovery label's
//! reservation through the CLI, in-process: the rotate rows of Go's
//! `cmd/gage/recovery_enroll_test.go`. Each test names its Go row. The
//! harness (a prompter that answers the re-type from the screen, the
//! hygiene walk) is `init_recoverykey_tests`'.

use gage::exitcode::Code;
use gage::prompter::{FakePrompter, Purpose};
use gage::recovery::{RECOVERY_DEVICE_LABEL, RecoveryError};
use gage::vault::Vault;

use crate::init_recoverykey_tests::{ScreenPrompter, assert_never_stored, pubkey_of, secret_keys};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, init_vault_with_recovery_key_file};

const VAULT: &str = "personal";

fn vault(env: &Env) -> Vault {
    Vault::from_registration(&env.dirs, VAULT, &env.entry(VAULT)).unwrap()
}

/// Go's `assertVerifies`: whether `key` is still the vault's recovery key.
/// Through the library, since `recovery verify` is RK4's.
fn assert_verifies(env: &Env, key: &str, want: bool) {
    match vault(env).verify_recovery_key(key) {
        Ok(label) => {
            assert!(want, "{key:.20}... still verifies, as {label:?}");
            assert_eq!(label, RECOVERY_DEVICE_LABEL);
        }
        Err(RecoveryError::NotARecipient { .. }) => assert!(!want, "{key:.20}... is gone"),
        Err(e) => panic!("{e}"),
    }
}

fn insert(env: &Env, title: &str, value: &str) {
    let r = env.run_stdin(
        &["insert", title, "--value-stdin"],
        &FakePrompter::new().with_passphrase(PASSPHRASE),
        format!("{value}\n").as_bytes(),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

/// `recovery rotate <args>` at a terminal, confirming from the screen
/// (Go's `runRotate`): the result, the last key shown, and the prompter.
fn run_rotate(env: &Env, args: &[&str]) -> (CliResult, Option<String>, ScreenPrompter) {
    let p = ScreenPrompter::new();
    let mut all = vec!["recovery", "rotate"];
    all.extend_from_slice(args);
    let r = p.run(env, &all);
    let shown = secret_keys(&r.stderr).pop();
    (r, shown, p)
}

// Go: TestRecoveryRotateReplacesTheKey. One commit; the old key retired,
// the new one shown and registered under the label; the vault still reads.
#[test]
fn recovery_rotate_replaces_the_key() {
    let env = Env::new();
    let old = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let path = env.entry(VAULT).path;
    let commits = gage::gitrepo::commit_count(&path).unwrap();

    let (r, shown, p) = run_rotate(&env, &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let shown = shown.expect("no key shown");
    assert_ne!(shown, old);
    let requests = p.inner.unlock_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].purpose, Purpose::Unlock);

    assert_eq!(gage::gitrepo::commit_count(&path).unwrap(), commits + 1);
    assert_verifies(&env, &shown, true);
    assert_verifies(&env, &old, false);
    let show = env.run(&["show", "api-token"]);
    assert_eq!(show.code, Code::Success, "{}", show.stderr);
    assert_eq!(show.stdout.trim(), "s3cr3t");
    let recipients = env.vault_config(VAULT).recipients;
    let labels: Vec<&str> = recipients.iter().map(|r| r.device.as_str()).collect();
    assert_eq!(labels, ["laptop-1", RECOVERY_DEVICE_LABEL]);
    assert_eq!(recipients[1].pubkey, pubkey_of(&shown));

    assert!(r.stdout.contains(&recipients[1].pubkey), "{}", r.stdout);
    assert!(
        r.stdout.contains(&format!("retired {}", pubkey_of(&old))),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("re-encrypted 1 entry"), "{}", r.stdout);
    // Shown the same way init shows one.
    assert!(r.stderr.contains("not encrypted"), "{}", r.stderr);
    assert!(r.stderr.contains(crate::qr::QR_BOTH), "{}", r.stderr);
}

// Go: TestRecoveryRotateAddsOneToAVaultWithout: the way back from having
// said --no-recovery-key.
#[test]
fn recovery_rotate_adds_one_to_a_vault_without() {
    let env = Env::new();
    env.init(VAULT, &["--device", "laptop-1"]);
    insert(&env, "api-token", "s3cr3t");

    let (r, shown, _) = run_rotate(&env, &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_verifies(&env, &shown.unwrap(), true);
    assert_eq!(env.vault_config(VAULT).recipients.len(), 2);
    assert!(r.stdout.contains("it had none"), "{}", r.stdout);
    assert!(!r.stdout.contains("retired"), "{}", r.stdout);
}

// Go: TestRecoveryRotateWithoutATerminalRefusesBeforeAnyPrompt. Nowhere to
// show the key, so it doesn't even ask for the passphrase (the prompter is
// unscripted). And rotate has no opt-out flag to name: producing a key is
// the whole command.
#[test]
fn recovery_rotate_without_a_terminal_refuses_before_any_prompt() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let p = FakePrompter::new();
    let r = env.run_with(&["recovery", "rotate"], &p);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--recovery-key-out"), "{}", r.stderr);
    assert!(!r.stderr.contains("--no-"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(secret_keys(&(r.stdout + &r.stderr)).is_empty());
}

// Go: TestRecoveryRotateChecksTheKeyDestinationBeforeTheUnlock (R5's
// review). Knowable from the path alone; found after the passphrase and a
// sync, it charges the user twice for one mistake.
#[test]
fn recovery_rotate_checks_the_key_destination_before_the_unlock() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let inside = env.entry(VAULT).path.join("rk");
    let p = FakePrompter::new();
    let r = env.run_with(
        &[
            "recovery",
            "rotate",
            "--recovery-key-out",
            inside.to_str().unwrap(),
        ],
        &p,
    );
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("vault"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(!inside.exists());
}

// Go: TestRecoveryRotateKeyOutWritesAPrivateFile. No terminal, no re-type
// (a value request would fail the unscripted prompter).
#[test]
fn recovery_rotate_key_out_writes_a_private_file() {
    let env = Env::new();
    let old = init_vault_with_recovery_key_file(&env, VAULT);
    let out = env.root.path().join("next.key");
    let r = env.run(&[
        "recovery",
        "rotate",
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    let written = std::fs::read_to_string(&out).unwrap();
    let written = written.trim();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(!format!("{}{}", r.stdout, r.stderr).contains(written));
    assert!(r.stderr.contains("delete this file"), "{}", r.stderr);
    assert_verifies(&env, written, true);
    assert_verifies(&env, &old, false);
}

// Go: TestRecoveryRotateKeepsNoCopyOfEitherKey. Neither the retired key
// nor the new one is anywhere gage writes, git objects included, and the
// new one is shown exactly once.
#[test]
fn recovery_rotate_keeps_no_copy_of_either_key() {
    let env = Env::new();
    let old = init_vault_with_recovery_key_file(&env, VAULT);
    // The init's file is the user's own copy, outside gage's directories.
    std::fs::remove_file(env.root.path().join(format!("{VAULT}-recovery.key"))).unwrap();
    insert(&env, "api-token", "s3cr3t");

    let (r, shown, _) = run_rotate(&env, &[]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let shown = shown.unwrap();
    let path = env.entry(VAULT).path;
    assert_never_stored(&env, &path, &old, "retired recovery key");
    assert_never_stored(&env, &path, &shown, "new recovery key");
    let all = format!("{}{}", r.stdout, r.stderr);
    assert_eq!(all.matches(&shown).count(), 1, "{all}");
    assert!(!all.contains(&old), "the retired key was printed");
}

// Go: TestRecoveryRotateStillPublishesWhenTheConfirmationFails. As init:
// the re-type is the last gate, not an early exit, and the swap is pushed
// before it is asked.
#[test]
fn recovery_rotate_still_publishes_when_the_confirmation_fails() {
    let env = Env::new();
    let remote = gage::gittest::new_bare_remote();
    let out = env.root.path().join("rk");
    env.ok(&[
        "init",
        VAULT,
        "--device",
        "laptop-1",
        "--remote",
        remote.path().to_str().unwrap(),
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]);

    let p = ScreenPrompter::new().answering("NOTTHEKEY");
    let r = p.run(&env, &["recovery", "rotate"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    let shown = secret_keys(&r.stderr);
    assert_eq!(shown.len(), 1, "{}", r.stderr);
    let published = gage::gittest::Device::clone(remote.path()).read(".age-recipients");
    assert!(published.contains(&pubkey_of(&shown[0])), "{published}");
    assert!(r.stdout.contains("new recovery key is"), "{}", r.stdout);
    assert!(r.stderr.contains("gage recovery rotate"), "{}", r.stderr);
}

// Go: TestRecoveryRotateIsListedInHelp, and in its parent's help too.
#[test]
fn recovery_rotate_is_listed_in_help() {
    let env = Env::new();
    let help = env.ok(&["help"]).stdout;
    assert!(help.contains("recovery rotate"), "{help}");
    let parent = env.ok(&["recovery", "--help"]).stdout;
    assert!(parent.contains("rotate"), "{parent}");
    assert!(
        parent.contains("Work with a vault's offline recovery key"),
        "{parent}"
    );
}

// --- the reservation through the CLI ----------------------------------------

// Go: TestNoCommandLetsADeviceTakeTheRecoveryLabel. `recovery rotate`
// evicts whatever holds the label, so no command that names a device may
// let one take it. RK4 adds the `recovery enroll` case.
#[test]
fn no_command_lets_a_device_take_the_recovery_label() {
    // init, even with --no-recovery-key.
    let env = Env::new();
    let dir = env.root.path().join(VAULT);
    let r = env.run_with(
        &[
            "init",
            VAULT,
            "--dir",
            dir.to_str().unwrap(),
            "--device",
            RECOVERY_DEVICE_LABEL,
            "--no-recovery-key",
        ],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage, "init: {}", r.stderr);
    assert!(!dir.join(".gage").exists());

    // identity add.
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let r = env.run_with(
        &["identity", "add", "--device", RECOVERY_DEVICE_LABEL],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage, "identity add: {}", r.stderr);

    // recipient add.
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let spare = gage::recovery::new_recovery_key().unwrap();
    let r = env.run(&[
        "recipient",
        "add",
        &spare.pubkey,
        "--device",
        RECOVERY_DEVICE_LABEL,
    ]);
    assert_ne!(r.code, Code::Success, "recipient add: {}", r.stdout);
    assert_eq!(env.vault_config(VAULT).recipients.len(), 2);
}

/// Every `` `gage <words>` `` a message names.
fn named_commands(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter_map(|quoted| quoted.strip_prefix("gage "))
        .map(str::to_string)
        .collect()
}

// Beyond Go: every command init's and rotate's recovery messages tell the
// user to run exists in this build (Go's
// TestNoRecoveryKeyWarningNamesARealCommand guard, which is RK4's row,
// applied here). The messages are read off real runs: the two terminal
// refusals, a refused re-type, and a failed file write.
#[test]
fn every_command_init_and_rotate_messages_name_is_registered() {
    use crate::init_recoverykey_tests::ScreenPrompter;
    use crate::registry::registry;

    let mut said = Vec::new();
    let env = Env::new();
    said.push(env.run_with(&["init", VAULT], &FakePrompter::new()).stderr);
    let r = ScreenPrompter::new()
        .answering("NOTTHEKEY")
        .run(&env, &["init", VAULT, "--device", "laptop-1"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    said.push(r.stderr);
    said.push(
        env.run_with(&["recovery", "rotate"], &FakePrompter::new())
            .stderr,
    );

    let env = Env::new();
    let out = env.root.path().join("rk");
    let mut p = ScreenPrompter::new();
    let squatter = out.clone();
    p.on_unlock = Some(Box::new(move || std::fs::write(&squatter, "x").unwrap()));
    let r = p.run(
        &env,
        &[
            "init",
            VAULT,
            "--device",
            "laptop-1",
            "--recovery-key-out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Code::Internal, "{}", r.stderr);
    said.push(r.stderr);

    let reg = registry();
    let mut named = Vec::new();
    for text in &said {
        for words in named_commands(text) {
            // The longest prefix that is a command: `gage recovery rotate`
            // or `gage push`, whatever arguments follow.
            let parts: Vec<&str> = words.split_whitespace().collect();
            let found = (1..=parts.len())
                .rev()
                .find_map(|n| reg.find(&parts[..n].join(" ")));
            assert!(found.is_some(), "`gage {words}` is not a command:\n{text}");
            named.push(words);
        }
    }
    assert!(
        named.iter().filter(|w| *w == "recovery rotate").count() >= 2,
        "the messages stopped naming rotate: {named:?}"
    );
}

// --- help parity ------------------------------------------------------------

// RK3's parity check against the `tests/fixtures/go/help/` references
// (a reference, not a byte gate): init's long help and every flag's
// description, `recovery rotate`'s flags, usage, group and availability,
// and the `recovery` parent's child list as far as its children exist.
#[test]
fn p8_init_and_rotate_have_their_go_fixture_flags_and_descriptions() {
    use crate::identity_cli_tests::assert_child;
    use crate::registry::{Availability, GROUP_RECIPIENT, GROUP_VAULT, registry};

    let env = Env::new();
    let description = |fixture: &'static str| {
        fixture
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("Usage:"))
            .map(str::trim)
            .filter(|l| !l.is_empty())
    };
    // Cobra's "(default ...)" suffix is clap's to render its own way.
    let flag_texts = |fixture: &'static str| {
        fixture
            .lines()
            .filter(|l| l.trim_start().starts_with('-'))
            .map(|l| {
                let text = l.split("   ").last().unwrap().trim();
                text.split(" (default ").next().unwrap()
            })
    };

    for (name, fixture, flags, group) in [
        (
            "init",
            include_str!("../../../tests/fixtures/go/help/init.txt"),
            &["--no-recovery-key", "--recovery-key-out"][..],
            GROUP_VAULT,
        ),
        (
            "recovery rotate",
            include_str!("../../../tests/fixtures/go/help/recovery_rotate.txt"),
            &["--recovery-key-out", "-u", "--use"][..],
            GROUP_RECIPIENT,
        ),
    ] {
        let args: Vec<&str> = name.split(' ').chain(["--help"]).collect();
        let help = env.ok(&args).stdout;
        let first = fixture.lines().next().unwrap().trim_end_matches('.');
        assert!(help.starts_with(first), "{name}:\n{help}");
        for line in description(fixture) {
            assert!(help.contains(line), "{name}: {line:?}\n{help}");
        }
        for text in flag_texts(fixture) {
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
        for flag in flags {
            assert!(help.contains(flag), "{name}: {flag}\n{help}");
        }
        let command = registry().find(name).unwrap().clone();
        assert_eq!(command.group, group, "{name}");
        assert_eq!(command.availability, Availability::OneShotOnly, "{name}");
    }
    let rotate = env.ok(&["recovery", "rotate", "--help"]).stdout;
    assert!(rotate.contains("gage recovery rotate"), "{rotate}");
    assert_eq!(registry().find("recovery rotate").unwrap().usage, "");

    // The parent lists the children that exist with Go's descriptions;
    // enroll and verify are RK4's.
    let parent = env.ok(&["recovery"]).stdout;
    let fixture = include_str!("../../../tests/fixtures/go/help/recovery.txt");
    assert!(
        parent.starts_with(fixture.lines().next().unwrap()),
        "{parent}"
    );
    let children: Vec<(&str, &str)> = fixture
        .lines()
        .skip_while(|l| !l.starts_with("Available Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .map(|l| {
            let (child, short) = l.trim().split_once(' ').unwrap();
            (child, short.trim())
        })
        .collect();
    assert_eq!(children.len(), 3, "{fixture}");
    let reg = registry();
    let mut listed = 0;
    for (child, short) in children {
        if reg.find(&format!("recovery {child}")).is_some() {
            assert_child(&parent, child, short);
            listed += 1;
        }
    }
    assert_eq!(listed, 1, "only rotate exists in RK3");
}
