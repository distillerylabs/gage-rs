//! `gage recovery verify`, `enroll` and `rotate`, the `recovery` parent,
//! and the recovery label's reservation through the CLI, in-process.
//! The harness (a prompter that
//! pastes and answers the re-type from the screen, the hygiene walk) is
//! `init_recoverykey_tests`'.

use crate::exitcode::ExitCode;
use gage::prompter::{FakePrompter, Purpose};
use gage::recovery::RECOVERY_DEVICE_LABEL;

use crate::init_recoverykey_tests::{ScreenPrompter, assert_never_stored, pubkey_of, secret_keys};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, init_vault_with_recovery_key_file};

const VAULT: &str = "personal";

/// Through the real `recovery verify`: whether
/// `key` is still the vault's recovery key, the one enroll accepts.
fn assert_verifies(env: &Env, key: &str, want: bool) {
    let p = FakePrompter::new().with_value(key);
    let r = env.run_with(&["recovery", "verify"], &p);
    if want {
        assert_eq!(
            r.code,
            ExitCode::Success,
            "{key:.20}... is gone: {}",
            r.stderr
        );
        assert!(r.stdout.contains("will accept it"), "{}", r.stdout);
    } else {
        assert_eq!(
            r.code,
            ExitCode::LockedOrAuth,
            "{key:.20}... still verifies: {}",
            r.stdout
        );
    }
}

fn insert(env: &Env, title: &str, value: &str) {
    let r = env.run_stdin(
        &["insert", title, "--value-stdin"],
        &FakePrompter::new().with_passphrase(PASSPHRASE),
        format!("{value}\n").as_bytes(),
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
}

/// `recovery rotate <args>` at a terminal, confirming from the screen
///: the result, the last key shown, and the prompter.
fn run_rotate(env: &Env, args: &[&str]) -> (CliResult, Option<String>, ScreenPrompter) {
    let p = ScreenPrompter::new();
    let mut all = vec!["recovery", "rotate"];
    all.extend_from_slice(args);
    let r = p.run(env, &all);
    let shown = secret_keys(&r.stderr).pop();
    (r, shown, p)
}

// One commit; the old key retired,
// the new one shown and registered under the label; the vault still reads.
#[test]
fn recovery_rotate_replaces_the_key() {
    let env = Env::new();
    let old = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let path = env.entry(VAULT).path;
    let commits = gage::sync::gitrepo::commit_count(&path).unwrap();

    let (r, shown, p) = run_rotate(&env, &[]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let shown = shown.expect("no key shown");
    assert_ne!(shown, old);
    let requests = p.inner.unlock_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].purpose, Purpose::Unlock);

    assert_eq!(
        gage::sync::gitrepo::commit_count(&path).unwrap(),
        commits + 1
    );
    assert_verifies(&env, &shown, true);
    assert_verifies(&env, &old, false);
    let show = env.run(&["show", "api-token"]);
    assert_eq!(show.code, ExitCode::Success, "{}", show.stderr);
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

// The way back from having
// said --no-recovery-key.
#[test]
fn recovery_rotate_adds_one_to_a_vault_without() {
    let env = Env::new();
    env.init(VAULT, &["--device", "laptop-1"]);
    insert(&env, "api-token", "s3cr3t");

    let (r, shown, _) = run_rotate(&env, &[]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_verifies(&env, &shown.unwrap(), true);
    assert_eq!(env.vault_config(VAULT).recipients.len(), 2);
    assert!(r.stdout.contains("it had none"), "{}", r.stdout);
    assert!(!r.stdout.contains("retired"), "{}", r.stdout);
}

// Nowhere to
// show the key, so it doesn't even ask for the passphrase (the prompter is
// unscripted). And rotate has no opt-out flag to name: producing a key is
// the whole command.
#[test]
fn recovery_rotate_without_a_terminal_refuses_before_any_prompt() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let p = FakePrompter::new();
    let r = env.run_with(&["recovery", "rotate"], &p);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--recovery-key-out"), "{}", r.stderr);
    assert!(!r.stderr.contains("--no-"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(secret_keys(&(r.stdout + &r.stderr)).is_empty());
}

// The key destination is checked before the unlock.
// Knowable from the path alone; found after the passphrase and a
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
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("vault"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert!(!inside.exists());
}

// No terminal, no re-type
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
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

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

// Neither the retired key
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
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let shown = shown.unwrap();
    let path = env.entry(VAULT).path;
    assert_never_stored(&env, &path, &old, "retired recovery key");
    assert_never_stored(&env, &path, &shown, "new recovery key");
    let all = format!("{}{}", r.stdout, r.stderr);
    assert_eq!(all.matches(&shown).count(), 1, "{all}");
    assert!(!all.contains(&old), "the retired key was printed");
}

// As init:
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
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    let shown = secret_keys(&r.stderr);
    assert_eq!(shown.len(), 1, "{}", r.stderr);
    let published = gage::gittest::Device::clone(remote.path()).read(".age-recipients");
    assert!(published.contains(&pubkey_of(&shown[0])), "{published}");
    assert!(r.stdout.contains("new recovery key is"), "{}", r.stdout);
    assert!(r.stderr.contains("gage recovery rotate"), "{}", r.stderr);
}

// And in its parent's help too.
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

// --- recovery verify ---------------------------------------------------------

/// `recovery verify` with `pasted` as the answer to its one prompt, no
/// terminal. A passphrase is scripted so that
/// a verify that unlocked would succeed and be caught by the request log,
/// not fail for want of an answer.
fn run_verify(env: &Env, pasted: &str) -> (CliResult, FakePrompter) {
    let p = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_value(pasted);
    let r = env.run_with(&["recovery", "verify"], &p);
    (r, p)
}

/// A well-formed key no vault here lists.
fn stranger() -> String {
    gage::recovery::new_recovery_key()
        .unwrap()
        .secret
        .to_string()
}

#[test]
fn recovery_verify_accepts_the_vaults_recovery_key() {
    let env = Env::new();
    let secret = init_vault_with_recovery_key_file(&env, VAULT);
    let (r, _) = run_verify(&env, &secret);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(r.stdout.contains(VAULT), "{}", r.stdout);
}

#[test]
fn recovery_verify_tolerates_pasted_whitespace() {
    let env = Env::new();
    let secret = init_vault_with_recovery_key_file(&env, VAULT);
    let (r, _) = run_verify(&env, &format!("  {secret}\n"));
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
}

// The check a lost identity file
// sends you to can't depend on that file. No lock either; it
// reads only the plaintext recipient list. The lock file is removed first,
// so taking the lock would leave a new one behind.
#[test]
fn recovery_verify_needs_no_unlock() {
    let env = Env::new();
    let secret = init_vault_with_recovery_key_file(&env, VAULT);
    let lock =
        gage::storage::paths::lock_file_path(&env.dirs, &env.entry(VAULT).id.clone().unwrap());
    let _ = std::fs::remove_file(&lock);

    let (r, p) = run_verify(&env, &secret);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(p.unlock_requests().is_empty(), "{:?}", p.unlock_requests());
    assert_eq!(p.value_prompts().len(), 1, "{:?}", p.value_prompts());
    assert!(!lock.exists(), "verify took the vault lock");
}

// Nothing on
// stdout, which is where success goes.
#[test]
fn recovery_verify_rejects_a_key_that_is_not_a_recipient() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let (r, _) = run_verify(&env, &stranger());
    assert_eq!(r.code, ExitCode::LockedOrAuth, "{}", r.stderr);
    assert_eq!(r.stdout, "");
}

// A public key is the likely
// wrong paste.
#[test]
fn recovery_verify_rejects_a_malformed_key() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let public = gage::recovery::new_recovery_key().unwrap().pubkey;
    for bad in ["", "not a key", &public] {
        let (r, _) = run_verify(&env, bad);
        assert_eq!(r.code, ExitCode::Usage, "{bad:?}: {}", r.stderr);
    }
}

// Not on success, not in either
// failure, on neither stream.
#[test]
fn recovery_verify_never_echoes_the_key() {
    let env = Env::new();
    let secret = init_vault_with_recovery_key_file(&env, VAULT);
    for (name, pasted) in [
        ("match", secret.clone()),
        ("non-recipient", stranger()),
        ("malformed", format!("x{secret}")),
    ] {
        let (r, p) = run_verify(&env, &pasted);
        assert_eq!(p.value_prompts().len(), 1, "{name}");
        let all = format!("{}{}", r.stdout, r.stderr);
        assert!(!all.contains(&pasted), "{name}: echoed:\n{all}");
        // Nor any part of it long enough to matter: the body after the
        // fixed prefix.
        let body = &pasted[pasted.len() - 20..];
        assert!(!all.contains(body), "{name}: echoed in part:\n{all}");
    }
}

#[test]
fn recovery_verify_is_listed_in_help() {
    let env = Env::new();
    let help = env.ok(&["help"]).stdout;
    assert!(help.contains("recovery verify"), "{help}");
}

// Verify distinguishes decrypting from enrolling.
// "Is a recipient" and "enroll will take it" are different
// claims, and verify is the check the docs say to trust, so it must not
// pass a key enroll refuses. The third case, enroll refusing that key,
// needs enroll and is below with its rows.
#[test]
fn recovery_verify_distinguishes_decrypting_from_enrolling() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    // A hand-rolled backup: an ordinary recipient, not under the label.
    let spare = gage::recovery::new_recovery_key().unwrap();
    env.ok(&[
        "recipient",
        "add",
        &spare.pubkey,
        "--device",
        "paper-backup",
    ]);

    // The recovery key says both.
    let (r, _) = run_verify(&env, &pasted);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(r.stdout.contains(RECOVERY_DEVICE_LABEL), "{}", r.stdout);
    assert!(
        r.stdout.contains("`gage recovery enroll` will accept it"),
        "{}",
        r.stdout
    );

    // Another recipient's key really does decrypt, so exit 0, but it names
    // its own label and says enroll won't take it.
    let (r, _) = run_verify(&env, &spare.secret);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(r.stdout.contains("\"paper-backup\""), "{}", r.stdout);
    assert!(
        r.stdout
            .contains("`gage recovery enroll` will not accept it"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("will accept it"), "{}", r.stdout);
}

// --- recovery enroll: refusals before the paste ------------------------------

/// Deletes every wrapped identity this machine holds, the state `recovery
/// enroll` is for. The registration stays, as
/// on a machine whose data dir was lost while its config survived.
fn lose_the_identity_file(env: &Env) {
    std::fs::remove_dir_all(env.dirs.data.join("identities")).unwrap();
}

/// Every wrapped identity file under this machine's data dir.
fn identity_files(env: &Env) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(vaults) = std::fs::read_dir(env.dirs.data.join("identities")) else {
        return found;
    };
    for dir in vaults.flatten() {
        for f in std::fs::read_dir(dir.path()).unwrap().flatten() {
            if f.path().extension().is_some_and(|x| x == "age") {
                found.push(f.path());
            }
        }
    }
    found
}

/// `recovery enroll <args>` at a terminal, pasting `pasted`, choosing
/// [`PASSPHRASE`] and confirming the replacement key from the screen
///: the result, the last key shown, and the prompter.
fn run_enroll(
    env: &Env,
    pasted: &str,
    args: &[&str],
) -> (CliResult, Option<String>, ScreenPrompter) {
    let p = ScreenPrompter::new().pasting(pasted);
    let mut all = vec!["recovery", "enroll"];
    all.extend_from_slice(args);
    let r = p.run(env, &all);
    let shown = secret_keys(&r.stderr).pop();
    (r, shown, p)
}

// The
// replacement key is shown once, and a run with nowhere to show it must
// not get as far as asking for the old one. The refusal names enroll's own
// opt-out, not init's.
#[test]
fn recovery_enroll_without_a_terminal_refuses_before_anything() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let before = env.vault_config(VAULT).recipients;

    let p = FakePrompter::new();
    let r = env.run_with(&["recovery", "enroll", "--device", "laptop-2"], &p);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--recovery-key-out"), "{}", r.stderr);
    assert!(r.stderr.contains("--no-new-recovery-key"), "{}", r.stderr);
    assert!(!r.stderr.contains("--no-recovery-key"), "{}", r.stderr);
    assert!(p.value_prompts().is_empty(), "{:?}", p.value_prompts());
    assert!(p.unlock_requests().is_empty());
    assert!(identity_files(&env).is_empty());
    assert_eq!(env.vault_config(VAULT).recipients, before);
}

// A name the
// vault lists is visible in the plaintext recipient list, so it must not
// cost a paste and a passphrase to find out. It is the recipient verbs'
// own `RecipientExists`, with the way out added.
#[test]
fn recovery_enroll_refuses_a_colliding_name_before_the_paste() {
    use crate::lifecycle_tests::Tty;
    use gage::trust::recipient::RecipientError;

    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);

    // laptop-1 is still a recipient, and --replaces doesn't name it.
    let (r, _, p) = run_enroll(&env, &pasted, &["--device", "laptop-1"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(p.asked.borrow().is_empty(), "{:?}", p.asked.borrow());
    assert!(p.inner.unlock_requests().is_empty());
    assert!(r.stderr.contains("--replaces laptop-1"), "{}", r.stderr);

    let p = ScreenPrompter::new().pasting(&pasted);
    let err = env
        .dispatch(
            &["recovery", "enroll", "--device", "laptop-1"],
            &p,
            Tty::BOTH,
        )
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RecipientError>(),
            Some(RecipientError::RecipientExists { device, .. }) if device == "laptop-1"
        ),
        "{err}"
    );
}

// The local-identity error is matchable (it was once formatted with %v,
// so nothing could match it). A local
// identity file under a name the vault doesn't list, so the identity-file
// check is what refuses rather than the name collision. Its passphrase is
// never asked for: a forgotten one is why anyone is here.
#[test]
fn recovery_enroll_local_identity_error_is_matchable() {
    use crate::commands::LocalIdentityExists;
    use crate::lifecycle_tests::Tty;

    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    env.ok(&["identity", "add", "--device", "spare-1"]);
    let spare = identity_files(&env)
        .into_iter()
        .find(|f| f.ends_with("spare-1.age"))
        .unwrap();
    let original = std::fs::read(&spare).unwrap();

    let p = ScreenPrompter::new().pasting(&pasted);
    let err = env
        .dispatch(
            &["recovery", "enroll", "--device", "spare-1"],
            &p,
            Tty::BOTH,
        )
        .unwrap_err();
    assert_eq!(err.code(), ExitCode::Usage, "{err}");
    let typed = err
        .downcast_ref::<LocalIdentityExists>()
        .unwrap_or_else(|| panic!("not LocalIdentityExists: {err}"));
    assert_eq!(typed.device, "spare-1");
    assert_eq!(typed.path, spare);
    assert!(err.to_string().contains("--replaces spare-1"), "{err}");
    assert!(
        p.inner.unlock_requests().is_empty(),
        "asked for a passphrase"
    );
    assert!(p.asked.borrow().is_empty(), "asked for the key first");
    assert_eq!(std::fs::read(&spare).unwrap(), original);

    // And through the real renderer, the same refusal and code.
    let (r, _, _) = run_enroll(&env, &pasted, &["--device", "spare-1"]);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(
        r.stderr.contains("already holds an identity"),
        "{}",
        r.stderr
    );
}

// `--replaces recovery-paper-key` is knowable
// from the flags alone, so it is refused with the paste, not after a new
// passphrase. The library refuses it only from inside the swap, which
// still backs this up.
#[test]
fn recovery_enroll_refuses_replacing_the_recovery_key_before_the_paste() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let (r, _, p) = run_enroll(
        &env,
        &pasted,
        &["--device", "laptop-2", "--replaces", RECOVERY_DEVICE_LABEL],
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(
        r.stderr.contains("whether or not it is named"),
        "{}",
        r.stderr
    );
    assert!(p.asked.borrow().is_empty(), "{:?}", p.asked.borrow());
    assert!(p.inner.unlock_requests().is_empty());
    assert!(identity_files(&env).is_empty());
}

// A wrong paste
// is the likeliest mistake, and it must not cost a passphrase entry first.
#[test]
fn recovery_enroll_checks_the_key_before_the_new_passphrase() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);

    let (r, _, p) = run_enroll(&env, &stranger(), &["--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::LockedOrAuth, "{}", r.stderr);
    assert_eq!(p.asked.borrow().len(), 1, "{:?}", p.asked.borrow());
    let requests = p.inner.unlock_requests();
    assert!(
        !requests.iter().any(|r| r.purpose == Purpose::Create),
        "a wrong paste still cost a new passphrase: {requests:?}"
    );
    assert!(
        identity_files(&env).is_empty(),
        "{:?}",
        identity_files(&env)
    );
}

#[test]
fn recovery_enroll_is_listed_in_help() {
    let env = Env::new();
    let help = env.ok(&["help"]).stdout;
    assert!(help.contains("recovery enroll"), "{help}");
}

// Its third
// case: verify's caveat has to be true, so enroll really does refuse a
// recipient's key that isn't under the recovery label.
#[test]
fn recovery_verify_distinguishes_decrypting_from_enrolling_and_enroll_agrees() {
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let spare = gage::recovery::new_recovery_key().unwrap();
    env.ok(&[
        "recipient",
        "add",
        &spare.pubkey,
        "--device",
        "paper-backup",
    ]);
    lose_the_identity_file(&env);

    let (r, _, p) = run_enroll(&env, &spare.secret, &["--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("\"paper-backup\""), "{}", r.stderr);
    assert!(p.inner.unlock_requests().is_empty());
    assert!(identity_files(&env).is_empty());
}

// --- recovery enroll: the swap and its rollback ------------------------------

/// The labels on the vault's committed recipient list, in order.
fn labels(env: &Env) -> Vec<String> {
    env.vault_config(VAULT)
        .recipients
        .into_iter()
        .map(|r| r.device)
        .collect()
}

/// Decrypts the vault's one entry with `secret` alone, as `age -d -i`
/// would, and returns the plaintext.
fn decrypt_the_entry_with(env: &Env, secret: &str) -> String {
    let identity: age::x25519::Identity = secret.parse().unwrap();
    let entries: Vec<_> = std::fs::read_dir(env.entry(VAULT).path.join("entries"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let plaintext = age::decrypt(&identity, &std::fs::read(&entries[0]).unwrap())
        .expect("that key does not decrypt the entry");
    String::from_utf8(plaintext).unwrap()
}

fn show(env: &Env, title: &str) -> CliResult {
    env.run(&["show", title])
}

// The milestone end to end.
// A vault whose only identity is gone, opened again with nothing but the
// paper key.
#[test]
fn recovery_enroll_gets_a_working_device_back() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    lose_the_identity_file(&env);
    assert_ne!(
        show(&env, "api-token").code,
        ExitCode::Success,
        "proves nothing"
    );
    let path = env.entry(VAULT).path;
    let commits = gage::sync::gitrepo::commit_count(&path).unwrap();

    let (r, shown, p) = run_enroll(&env, &pasted, &["--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

    // The paste, then the new passphrase and nothing else: never an
    // unlock of a file that isn't there.
    let requests = p.inner.unlock_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].purpose, Purpose::Create);
    assert_eq!(requests[0].device, "laptop-2");
    assert_eq!(
        gage::sync::gitrepo::commit_count(&path).unwrap(),
        commits + 1
    );

    // The vault reads again, through the ordinary unlock.
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
    assert_eq!(read.stdout.trim(), "s3cr3t");
    assert_eq!(env.entry(VAULT).device, "laptop-2");

    // Exactly the list asked for: the pasted key retired, the new one live.
    assert_eq!(
        labels(&env),
        ["laptop-1", "laptop-2", RECOVERY_DEVICE_LABEL]
    );
    let shown = shown.expect("no replacement recovery key was shown");
    assert_ne!(shown, pasted);
    assert_verifies(&env, &shown, true);
    assert_verifies(&env, &pasted, false);
    assert!(decrypt_the_entry_with(&env, &shown).contains("s3cr3t"));

    assert!(r.stdout.contains("enrolled \"laptop-2\""), "{}", r.stdout);
    assert!(
        r.stdout.contains(&format!(
            "retired the recovery key you used ({})",
            pubkey_of(&pasted)
        )),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains(&pubkey_of(&shown)), "{}", r.stdout);
    assert!(r.stdout.contains("re-encrypted 1 entry"), "{}", r.stdout);
}

// A brand-new machine with
// nothing but the clone and the paper.
#[test]
fn recovery_enroll_works_on_a_fresh_clone() {
    let owner = Env::new();
    let remote = gage::gittest::new_bare_remote();
    let out = owner.root.path().join("rk");
    owner.ok(&[
        "init",
        VAULT,
        "--device",
        "laptop-1",
        "--remote",
        remote.path().to_str().unwrap(),
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]);
    insert(&owner, "api-token", "s3cr3t");
    let pasted = std::fs::read_to_string(&out).unwrap().trim().to_string();

    let env = Env::new();
    env.ok(&["clone", remote.path().to_str().unwrap(), "--name", VAULT]);
    let (r, _, _) = run_enroll(&env, &pasted, &["--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
    assert_eq!(read.stdout.trim(), "s3cr3t");
    // Published, so the owner's machine sees the new device too.
    let published = gage::gittest::Device::clone(remote.path()).read(".age-recipients");
    assert!(
        published.contains(env.entry(VAULT).pubkey.as_deref().unwrap()),
        "{published}"
    );
}

// The
// half a blanket "refuse an existing identity file" rule would block. The
// file survives; only its passphrase is gone, so it is never asked for.
#[test]
fn recovery_enroll_rebuilds_a_device_whose_passphrase_is_forgotten() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let file = identity_files(&env).pop().unwrap();
    let original = std::fs::read(&file).unwrap();
    let old_pubkey = env.entry(VAULT).pubkey.unwrap();

    let (r, _, p) = run_enroll(
        &env,
        &pasted,
        &["--device", "laptop-1", "--replaces", "laptop-1"],
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    for req in p.inner.unlock_requests() {
        assert_eq!(req.purpose, Purpose::Create, "asked to open the old file");
    }

    let after = std::fs::read(&file).expect("the file is gone rather than replaced");
    assert_ne!(
        after, original,
        "the unusable identity file was left in place"
    );
    // One laptop-1, with the new key, and the vault reads.
    assert_eq!(labels(&env), ["laptop-1", RECOVERY_DEVICE_LABEL]);
    let recipients = env.vault_config(VAULT).recipients;
    assert_ne!(recipients[0].pubkey, old_pubkey);
    assert_eq!(
        Some(&recipients[0].pubkey),
        env.entry(VAULT).pubkey.as_ref()
    );
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
}

// The commonest rebuild, a new machine under the
// old one's name, with the old file gone rather than unopenable. Nothing to
// back up, and the name is free because --replaces names it.
#[test]
fn recovery_enroll_rebuilds_under_the_same_name_after_losing_the_file() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let old_pubkey = env.entry(VAULT).pubkey.unwrap();
    lose_the_identity_file(&env);

    let (r, _, _) = run_enroll(
        &env,
        &pasted,
        &["--device", "laptop-1", "--replaces", "laptop-1"],
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(labels(&env), ["laptop-1", RECOVERY_DEVICE_LABEL]);
    assert_ne!(env.vault_config(VAULT).recipients[0].pubkey, old_pubkey);
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
}

// A new machine
// taking over from one that is gone, under a new name, in the same commit.
#[test]
fn recovery_enroll_replaces_a_different_lost_device() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let path = env.entry(VAULT).path;
    let commits = gage::sync::gitrepo::commit_count(&path).unwrap();

    let (r, _, _) = run_enroll(
        &env,
        &pasted,
        &["--device", "laptop-2", "--replaces", "laptop-1"],
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(labels(&env), ["laptop-2", RECOVERY_DEVICE_LABEL]);
    assert_eq!(
        gage::sync::gitrepo::commit_count(&path).unwrap(),
        commits + 1
    );
    assert!(r.stdout.contains("removed \"laptop-1\""), "{}", r.stdout);
}

// --replaces
// names nobody, which the library refuses after the new identity exists:
// no orphan identity, no registration change, no commit.
#[test]
fn recovery_enroll_rolls_back_the_identity_when_the_swap_fails() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let before = env.vault_config(VAULT).recipients;
    let registration = env.entry(VAULT);
    let commits = gage::sync::gitrepo::commit_count(&registration.path).unwrap();

    let (r, shown, p) = run_enroll(
        &env,
        &pasted,
        &["--device", "laptop-2", "--replaces", "desktop-9"],
    );
    assert_eq!(r.code, ExitCode::NotFound, "{}", r.stderr);
    // It got as far as the identity, so the rollback really ran.
    assert_eq!(p.inner.unlock_requests().len(), 1);
    assert!(
        identity_files(&env).is_empty(),
        "{:?}",
        identity_files(&env)
    );
    assert_eq!(env.vault_config(VAULT).recipients, before);
    let after = env.entry(VAULT);
    assert_eq!(
        (after.device, after.pubkey),
        (registration.device, registration.pubkey)
    );
    assert_eq!(
        gage::sync::gitrepo::commit_count(&registration.path).unwrap(),
        commits
    );
    // A key made for a swap that never happened is no one's: never shown.
    assert_eq!(shown, None, "{}", r.stderr);
    assert_verifies(&env, &pasted, true);
}

// Recovery enroll restores the identity it displaced when aborted.
// The identity is kept, not deleted first: every earlier
// rollback test deleted it, so the restore never ran. The abort is at the
// new-passphrase prompt, after the old file has been cleared, and it was
// the vault's only identity, so its directory was pruned with it.
#[test]
fn recovery_enroll_restores_the_identity_it_displaced_when_aborted() {
    use gage::prompter::PromptError;
    use gage::storage::identityfile::MARKER_FILE_NAME;

    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let file = identity_files(&env).pop().unwrap();
    let dir = file.parent().unwrap().to_path_buf();
    let original = std::fs::read(&file).unwrap();
    let marker = std::fs::read(dir.join(MARKER_FILE_NAME)).unwrap();
    let before = env.vault_config(VAULT).recipients;

    let out = env.root.path().join("new.key");
    let p = FakePrompter::new()
        .with_value(&pasted)
        .with_unlock_error(PromptError::Cancelled);
    let r = env.run_with(
        &[
            "recovery",
            "enroll",
            "--device",
            "laptop-1",
            "--replaces",
            "laptop-1",
            "--recovery-key-out",
            out.to_str().unwrap(),
        ],
        &p,
    );
    assert_ne!(r.code, ExitCode::Success, "{}", r.stdout);
    let requests = p.unlock_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].purpose,
        Purpose::Create,
        "aborted somewhere else"
    );

    // Byte for byte, with the directory and marker remade, and it still
    // opens the vault with the passphrase it always had.
    assert_eq!(std::fs::read(&file).expect("destroyed"), original);
    assert_eq!(std::fs::read(dir.join(MARKER_FILE_NAME)).unwrap(), marker);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&dir), 0o700);
    }
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
    assert_eq!(env.vault_config(VAULT).recipients, before);
    assert!(
        !out.exists(),
        "a key was delivered for a swap that never happened"
    );
}

// The restore when the *swap* fails, not the passphrase. The
// row above aborts at step 4; this one gets the new identity made and
// written over the old one's place, then has the swap refuse, so both the
// removal of the new file and the restore of the old one have to run. The
// swap refuses a vault whose two recipient files disagree.
#[test]
fn recovery_enroll_restores_the_displaced_identity_when_the_swap_fails() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    insert(&env, "api-token", "s3cr3t");
    let file = identity_files(&env).pop().unwrap();
    let original = std::fs::read(&file).unwrap();
    let path = env.entry(VAULT).path;
    let mut listed = gage::trust::recipient::load(&path.join(".age-recipients")).unwrap();
    listed.push(gage::recovery::new_recovery_key().unwrap().pubkey);
    gage::trust::recipient::save(&path.join(".age-recipients"), &listed).unwrap();
    gage::sync::gitrepo::commit_all(&path, "diverge the recipient files").unwrap();
    let commits = gage::sync::gitrepo::commit_count(&path).unwrap();

    let out = env.root.path().join("new.key");
    let p = FakePrompter::new()
        .with_value(&pasted)
        .with_passphrase("a different passphrase");
    let r = env.run_with(
        &[
            "recovery",
            "enroll",
            "--device",
            "laptop-1",
            "--replaces",
            "laptop-1",
            "--recovery-key-out",
            out.to_str().unwrap(),
        ],
        &p,
    );
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(".age-recipients"), "{}", r.stderr);
    // Step 4 ran: a new identity was made in the old one's place.
    let requests = p.unlock_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].purpose, Purpose::Create);

    assert_eq!(std::fs::read(&file).expect("destroyed"), original);
    assert_eq!(identity_files(&env), [file]);
    assert_eq!(gage::sync::gitrepo::commit_count(&path).unwrap(), commits);
    assert!(
        !out.exists(),
        "a key was delivered for a swap that never happened"
    );
    // The old passphrase, not the one typed for the new identity.
    let read = show(&env, "api-token");
    assert_eq!(read.code, ExitCode::Success, "{}", read.stderr);
}

// No key made or
// printed, no recovery recipient left, and a warning that says so.
#[test]
fn recovery_enroll_no_new_recovery_key_leaves_none() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);

    let p = FakePrompter::new()
        .with_value(&pasted)
        .with_passphrase(PASSPHRASE);
    let r = env.run_with(
        &[
            "recovery",
            "enroll",
            "--device",
            "laptop-2",
            "--no-new-recovery-key",
        ],
        &p,
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(labels(&env), ["laptop-1", "laptop-2"]);
    assert!(secret_keys(&format!("{}{}", r.stdout, r.stderr)).is_empty());
    assert!(r.stderr.contains("now has no recovery key"), "{}", r.stderr);
    assert!(!r.stdout.contains("new recovery key"), "{}", r.stdout);
}

// --- recovery enroll: delivery, registration, hygiene ------------------------

/// `recovery enroll --device laptop-2 <extra>` with no terminal: the paste
/// and [`PASSPHRASE`] scripted, nothing else.
fn run_enroll_piped(env: &Env, pasted: &str, extra: &[&str]) -> (CliResult, FakePrompter) {
    let p = FakePrompter::new()
        .with_value(pasted)
        .with_passphrase(PASSPHRASE);
    let mut args = vec!["recovery", "enroll", "--device", "laptop-2"];
    args.extend_from_slice(extra);
    let r = env.run_with(&args, &p);
    (r, p)
}

// Recovery enroll shows the new key even if registration fails.
// Once the swap has committed and pushed, the replacement key is
// the vault's and its private half exists only in this process, so the
// registration failing after it must not be what swallows it. The config
// directory is made read-only: reads still work, so the run reaches the
// swap, and the atomic write that records the device is what fails. No
// seam: the real filesystem's real failure (decision 4). Unix only, and
// skipped as root, which ignores directory permissions.
#[cfg(unix)]
#[test]
fn recovery_enroll_shows_the_new_key_even_if_registration_fails() {
    use std::os::unix::fs::PermissionsExt;

    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let config_dir = env.dirs.config.clone();
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _restore = Restore(config_dir.clone());
    // Root ignores directory permission bits; there is nothing to prove.
    // Said out loud, as the library's tests do, so a suite run as root
    // shows the test proved nothing rather than reporting a silent pass.
    if std::fs::write(config_dir.join("probe"), b"").is_ok() {
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!(
                "skipping: running as root, which ignores the unwritable directory this test relies on"
            );
        }
        return;
    }

    let out = env.root.path().join("new.key");
    let (r, _) = run_enroll_piped(
        &env,
        &pasted,
        &["--recovery-key-out", out.to_str().unwrap()],
    );
    assert_ne!(r.code, ExitCode::Success, "{}", r.stdout);
    // The swap happened, so the key is real, and it reached the user.
    let written = std::fs::read_to_string(&out).expect("the new key was never delivered");
    assert_verifies(&env, written.trim(), true);
    assert_verifies(&env, &pasted, false);
    // And the error says how to finish the job.
    assert!(
        r.stderr.contains("`gage identity add --device laptop-2`"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("is a recipient"), "{}", r.stderr);
    assert_eq!(assert_names_real_commands(&r.stderr), ["identity add"]);
    // The summary still says what was committed.
    assert!(r.stdout.contains("enrolled \"laptop-2\""), "{}", r.stdout);
}

// Precedence: delivery and
// registration both fail: the lost key is the worse of the two, so it sets
// the exit code, and the registration failure is still reported beside it
// with its remedy. The destination is taken between the up-front check and
// the write, at the new-passphrase prompt; the config dir is read-only as
// above.
#[cfg(unix)]
#[test]
fn recovery_enroll_reports_both_when_delivery_and_registration_fail() {
    use std::os::unix::fs::PermissionsExt;

    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let config_dir = env.dirs.config.clone();
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _restore = Restore(config_dir.clone());
    if std::fs::write(config_dir.join("probe"), b"").is_ok() {
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!(
                "skipping: running as root, which ignores the unwritable directory this test relies on"
            );
        }
        return;
    }

    let out = env.root.path().join("new.key");
    let mut p = ScreenPrompter::new().pasting(&pasted);
    let squatter = out.clone();
    p.on_unlock = Some(Box::new(move || std::fs::write(&squatter, "x").unwrap()));
    let r = p.run(
        &env,
        &[
            "recovery",
            "enroll",
            "--device",
            "laptop-2",
            "--recovery-key-out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, ExitCode::Internal, "{}", r.stderr);
    assert!(r.stderr.contains("that key is lost"), "{}", r.stderr);
    assert!(
        r.stderr.contains("`gage identity add --device laptop-2`"),
        "the registration failure went unreported:\n{}",
        r.stderr
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "x");
}

// As init's:
// the re-type is the last gate, not an early exit. The swap reached the
// remote; the key is shown once and not reprinted; the device is
// registered anyway, since a failed re-type is the key's problem, not the
// device's.
#[test]
fn recovery_enroll_still_publishes_when_the_confirmation_fails() {
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
    let pasted = std::fs::read_to_string(&out).unwrap().trim().to_string();
    lose_the_identity_file(&env);

    let p = ScreenPrompter::new()
        .pasting(&pasted)
        .answering("NOTTHEKEY");
    let r = p.run(&env, &["recovery", "enroll", "--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    let shown = secret_keys(&r.stderr);
    assert_eq!(shown.len(), 1, "{}", r.stderr);
    let published = gage::gittest::Device::clone(remote.path()).read(".age-recipients");
    assert!(published.contains(&pubkey_of(&shown[0])), "{published}");
    assert!(!published.contains(&pubkey_of(&pasted)), "{published}");
    assert!(r.stdout.contains("new recovery key is"), "{}", r.stdout);
    assert_eq!(env.entry(VAULT).device, "laptop-2");
}

#[test]
fn recovery_enroll_key_out_writes_a_private_file() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);

    let out = env.root.path().join("new-recovery.key");
    let (r, _) = run_enroll_piped(
        &env,
        &pasted,
        &["--recovery-key-out", out.to_str().unwrap()],
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let written = std::fs::read_to_string(&out).unwrap();
    let written = written.trim();
    assert_eq!(secret_keys(written).len(), 1, "{written:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(!format!("{}{}", r.stdout, r.stderr).contains(written));
    assert!(r.stderr.contains("delete this file"), "{}", r.stderr);
    assert_verifies(&env, written, true);
    assert_verifies(&env, &pasted, false);
}

// Neither the pasted key
// nor its replacement is anywhere gage writes, git objects included; the
// pasted one is never echoed and the new one is shown exactly once.
#[test]
fn recovery_enroll_keeps_no_copy_of_either_key() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    // init's file is the user's own copy, outside gage's directories.
    std::fs::remove_file(env.root.path().join(format!("{VAULT}-recovery.key"))).unwrap();
    insert(&env, "api-token", "s3cr3t");
    lose_the_identity_file(&env);

    let (r, shown, _) = run_enroll(&env, &pasted, &["--device", "laptop-2"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let shown = shown.expect("no replacement key was shown");
    let path = env.entry(VAULT).path;
    assert_never_stored(&env, &path, &pasted, "pasted recovery key");
    assert_never_stored(&env, &path, &shown, "new recovery key");
    let all = format!("{}{}", r.stdout, r.stderr);
    assert_eq!(all.matches(&shown).count(), 1, "{all}");
    assert_eq!(all.matches(&pasted).count(), 0, "the pasted key was echoed");
}

// Pointing at a command
// that doesn't exist is worse than saying nothing: the suggested fix
// prints usage and exits 0, and the user believes the vault has a key.
#[test]
fn no_recovery_key_warning_names_a_real_command() {
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let (r, _) = run_enroll_piped(&env, &pasted, &["--no-new-recovery-key"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

    let warning: String = r
        .stderr
        .lines()
        .skip_while(|l| !l.contains("now has no recovery key"))
        .take(3)
        .collect::<Vec<_>>()
        .join("\n");
    let named = assert_names_real_commands(&warning);
    assert_eq!(named, ["recovery rotate"], "{warning}");
}

// --- the reservation through the CLI ----------------------------------------

// `recovery rotate`
// evicts whatever holds the label, so no command that names a device may
// let one take it, `recovery enroll` included.
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
    assert_eq!(r.code, ExitCode::Usage, "init: {}", r.stderr);
    assert!(!dir.join(".gage").exists());

    // identity add.
    let env = Env::new();
    init_vault_with_recovery_key_file(&env, VAULT);
    let r = env.run_with(
        &["identity", "add", "--device", RECOVERY_DEVICE_LABEL],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage, "identity add: {}", r.stderr);

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
    assert_ne!(r.code, ExitCode::Success, "recipient add: {}", r.stdout);
    assert_eq!(env.vault_config(VAULT).recipients.len(), 2);

    // recovery enroll, with the reservation as the reason given: the name
    // collision would otherwise answer, and suggest `--replaces
    // recovery-paper-key`, which the library then refuses. Before the
    // paste, like every other refusal that needs no secret.
    let env = Env::new();
    let pasted = init_vault_with_recovery_key_file(&env, VAULT);
    lose_the_identity_file(&env);
    let (r, _, p) = run_enroll(&env, &pasted, &["--device", RECOVERY_DEVICE_LABEL]);
    assert_eq!(r.code, ExitCode::Usage, "recovery enroll: {}", r.stderr);
    assert!(r.stderr.contains("reserved"), "{}", r.stderr);
    assert!(!r.stderr.contains("--replaces"), "{}", r.stderr);
    assert!(p.asked.borrow().is_empty());
    assert!(p.inner.unlock_requests().is_empty());
}

/// Requires every `` `gage <words>` `` in `text` to resolve to a
/// registered command, by its longest prefix that is one (`gage identity
/// add --device x` is `identity add`), and returns the commands found.
pub(crate) fn assert_names_real_commands(text: &str) -> Vec<String> {
    let reg = crate::registry::registry();
    named_commands(text)
        .iter()
        .map(|words| {
            let parts: Vec<&str> = words.split_whitespace().collect();
            (1..=parts.len())
                .rev()
                .map(|n| parts[..n].join(" "))
                .find(|name| reg.find(name).is_some())
                .unwrap_or_else(|| panic!("`gage {words}` is not a command:\n{text}"))
        })
        .collect()
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

// Every command init's and rotate's recovery messages tell the
// user to run exists in this build.
// The messages are read off real runs: the two terminal
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
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
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
    assert_eq!(r.code, ExitCode::Internal, "{}", r.stderr);
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

// --- help ---------------------------------------------------------------------
