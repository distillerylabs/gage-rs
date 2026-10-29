//! `gage init` generating this device's identity, through the real CLI
//! in-process, with the vault-id keying rows (their registration halves stay in
//! `lifecycle_tests`).

use std::path::{Path, PathBuf};

use gage::config::VaultEntry;
use gage::crypt::{self, Recipient};
use gage::exitcode::{Code, HasCode};
use gage::prompter::{FakePrompter, Kind, PromptError, Prompter, Purpose};
use gage::vault::{UnlockError, Vault};
use gage::vaultconfig::VaultId;
use gage::{paths, recipients};

use crate::error::CliError;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::prompter::{SecretInput, TerminalPrompter};

const RECIPIENT_1: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";

fn identities_root(env: &Env) -> PathBuf {
    env.dirs.data.join("identities")
}

/// Every wrapped identity file under `root`.
fn wrapped_identity_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "age") {
                found.push(path);
            }
        }
    }
    found
}

fn non_empty_dir(env: &Env) -> PathBuf {
    let dir = env.root.path().join("mine");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();
    dir
}

// The
// milestone's definition of done, end to end. A vault created with a real
// identity; that identity unlocked from what init recorded; and used to
// decrypt something encrypted to the key in the vault's own recipient
// file. There are still no entries: the payload is just bytes.
#[test]
fn init_then_unlock_decrypts_something_encrypted_to_the_vault() {
    let env = Env::new();
    env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);

    let entry = env.entry("personal");
    let from_config = env.vault_config("personal").recipients[0].pubkey.clone();
    let from_file = recipients::read(&entry.path.join(".age-recipients")).unwrap();
    assert_eq!(from_file, std::slice::from_ref(&from_config));

    let vault = Vault::from_registration(&env.dirs, "personal", &entry).unwrap();
    let id = vault
        .unlock(&FakePrompter::new().with_passphrase(PASSPHRASE))
        .expect("unlock after init");
    assert_eq!(id.device(), "laptop-1");
    assert_eq!(id.recipient(), from_config);

    let payload = b"a secret, encrypted to the vault's recipient file";
    let ct = crypt::encrypt(payload, &[Recipient::parse(&from_file[0]).unwrap()]).unwrap();
    assert_eq!(crypt::decrypt(&ct, &id).unwrap(), payload);
}

#[test]
fn init_asks_for_the_new_passphrase_through_the_prompter() {
    let env = Env::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ],
        &p,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    let requests = p.unlock_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.kind, Kind::Passphrase);
    assert_eq!(
        req.purpose,
        Purpose::Create,
        "init establishes a passphrase, it doesn't verify one"
    );
    assert_eq!(
        (req.vault.as_str(), req.device.as_str()),
        ("personal", "laptop-1")
    );
}

// The key is what a user hands
// another device's `recipient add`, so init prints it.
#[test]
fn init_reports_the_device_and_public_key() {
    let env = Env::new();
    let r = env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);
    let pubkey = env.vault_config("personal").recipients[0].pubkey.clone();
    assert!(
        r.stdout.contains(&format!(
            "gage: this device is \"laptop-1\", public key {pubkey}"
        )),
        "{}",
        r.stdout
    );
}

// The library's typed
// error reaches the exit status a script branches on.
#[test]
fn unlock_typed_errors_carry_the_right_exit_code() {
    let env = Env::new();
    let entry = VaultEntry {
        path: env.root.path().join("nowhere"),
        id: Some(VaultId::try_from("9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497").unwrap()),
        ..Default::default()
    };
    let vault = Vault::from_registration(&env.dirs, "never-registered", &entry).unwrap();
    let err = vault
        .unlock(&FakePrompter::new().with_passphrase(PASSPHRASE))
        .unwrap_err();
    assert!(matches!(err, UnlockError::NoLocalIdentity(_)), "{err}");
    assert_eq!(err.code(), Code::LockedOrAuth);
    assert_eq!(CliError::from(err).code(), Code::LockedOrAuth);
}

// Beyond the row above, which takes one error to the exit status: every
// way a real unlock of a real vault fails keeps its library code through
// the CLI's error type, so no conversion on the way out (added context, a
// wrapped source) can turn an authentication failure into an Internal
// one, or a damaged file into a retryable auth failure.
#[test]
fn every_unlock_failure_keeps_its_exit_code_through_the_cli() {
    type Setup = fn(&Env, &VaultEntry);
    let cases: [(&str, Setup, FakePrompter, Code); 5] = [
        (
            "wrong passphrase",
            |_, _| {},
            FakePrompter::new()
                .with_passphrase("not it")
                .with_unlock_error(PromptError::Cancelled),
            Code::LockedOrAuth,
        ),
        (
            "empty passphrase",
            |_, _| {},
            FakePrompter::new().with_passphrase(""),
            Code::LockedOrAuth,
        ),
        (
            "corrupt identity file",
            |env, entry| {
                let path =
                    paths::identity_file_path(&env.dirs, entry.id.as_ref().unwrap(), &entry.device)
                        .unwrap();
                let data = std::fs::read(&path).unwrap();
                std::fs::write(&path, &data[..data.len() / 3]).unwrap();
            },
            FakePrompter::new().with_passphrase(PASSPHRASE),
            Code::Conflict,
        ),
        (
            "unsupported local method",
            |env, _| {
                let mut global = env.global();
                global.vaults.get_mut("personal").unwrap().method = "yubikey".into();
                env.save_global(&global);
            },
            FakePrompter::new(),
            Code::LockedOrAuth,
        ),
        (
            "unsafe recorded device name",
            |env, _| {
                let mut global = env.global();
                global.vaults.get_mut("personal").unwrap().device = "../escape".into();
                env.save_global(&global);
            },
            FakePrompter::new(),
            Code::LockedOrAuth,
        ),
    ];
    for (name, setup, p, want) in cases {
        let env = Env::new();
        env.ok(&[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ]);
        let entry = env.entry("personal");
        setup(&env, &entry);

        let vault = Vault::from_registration(&env.dirs, "personal", &entry).unwrap();
        let err = vault.unlock(&p).unwrap_err();

        assert_eq!(err.code(), want, "{name}: library code for {err:?}");
        let cli = CliError::from(err);
        assert_eq!(cli.code(), want, "{name}: code changed in the CLI");
    }
}

// The target directory is
// checked after the identity exists, so the rollback is what
// stands between a fixable failure and an orphaned private key.
#[test]
fn init_leaves_no_vault_behind_when_create_fails() {
    let env = Env::new();
    let dir = non_empty_dir(&env);

    let r = env.run(&[
        "init",
        "personal",
        "--dir",
        dir.to_str().unwrap(),
        "--no-recovery-key",
    ]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        env.global().vaults.is_empty(),
        "a failed init registered the vault"
    );
    assert!(
        !dir.join(".gage").exists(),
        "a failed init wrote into the target"
    );
    assert_eq!(
        wrapped_identity_files(&identities_root(&env)),
        Vec::<PathBuf>::new()
    );

    // And the retry, once the obstacle is gone, works.
    std::fs::remove_file(dir.join("keep.txt")).unwrap();
    env.ok(&[
        "init",
        "personal",
        "--dir",
        dir.to_str().unwrap(),
        "--no-recovery-key",
    ]);
}

#[test]
fn init_with_a_rejected_device_name_writes_no_identity_file() {
    let env = Env::new();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--device",
            "../../escape",
            "--no-recovery-key",
        ],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage);
    assert!(!identities_root(&env).exists());
}

// Every attempt mints a
// new id, so a rollback that did nothing would leave one directory per
// attempt, each holding a key for a vault that never existed.
#[test]
fn two_failed_inits_leave_no_orphaned_identities() {
    let env = Env::new();
    let dir = non_empty_dir(&env);
    for attempt in 1..=2 {
        let r = env.run(&[
            "init",
            "personal",
            "--dir",
            dir.to_str().unwrap(),
            "--no-recovery-key",
        ]);
        // The target directory refusal, after the identity exists: the
        // rollback path, not an earlier rejection that never generated a key.
        assert_eq!(r.code, Code::Conflict, "attempt {attempt}: {}", r.stderr);
    }
    assert_eq!(
        wrapped_identity_files(&identities_root(&env)),
        Vec::<PathBuf>::new()
    );
    // The directories go too, not just the keys: a tree of empty UUID
    // directories is the visible half of the same leak.
    let left: Vec<_> = std::fs::read_dir(identities_root(&env))
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

// (ADR-0006): a new vault under an old local name gets its own id and keypair,
// and the first vault's wrapped key is never touched.
#[test]
fn init_after_vault_remove_generates_a_fresh_key_rather_than_reusing_the_old_one() {
    let env = Env::new();
    env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);
    let first_id = env.entry("personal").id.unwrap();
    let first_path = paths::identity_file_path(&env.dirs, &first_id, "laptop-1").unwrap();
    let first_file = std::fs::read(&first_path).unwrap();
    let first_pubkey = env.vault_config("personal").recipients[0].pubkey.clone();

    // The registration goes; the vault's files and the identity stay.
    env.ok(&["vault", "remove", "personal"]);
    assert!(first_path.is_file());

    let again = env.root.path().join("again");
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--device",
            "laptop-1",
            "--dir",
            again.to_str().unwrap(),
        ],
        &p,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        p.warnings().is_empty(),
        "reused the first vault's keypair: {:?}",
        p.warnings()
    );
    assert_eq!(p.unlock_requests()[0].purpose, Purpose::Create);

    assert_ne!(
        env.entry("personal").id,
        Some(first_id),
        "a new vault gets a new id"
    );
    assert_ne!(
        env.vault_config("personal").recipients[0].pubkey,
        first_pubkey
    );
    assert_eq!(
        std::fs::read(&first_path).unwrap(),
        first_file,
        "the first vault's key was overwritten"
    );
}

// The identity file is 0700/0600, and not
// inside the vault, so never in the git tree that syncs.
#[test]
fn init_writes_the_wrapped_identity_file() {
    let env = Env::new();
    env.ok(&["init", "personal", "--no-recovery-key"]);
    let entry = env.entry("personal");
    let path = identities_root(&env)
        .join(entry.id.as_ref().unwrap())
        .join(format!("{}.age", entry.device));
    assert!(path.is_file(), "no wrapped identity at {}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }
    assert!(
        !path.starts_with(&entry.path),
        "the identity file is inside the vault"
    );
}

#[test]
fn init_extra_recipient_follows_the_device_key() {
    let env = Env::new();
    env.ok(&[
        "init",
        "personal",
        "--recipient",
        RECIPIENT_1,
        "--no-recovery-key",
    ]);
    let entry = env.entry("personal");
    let file = env.vault_config("personal");
    assert_eq!(file.recipients.len(), 2);
    assert_eq!(file.recipients[0].device, entry.device);
    assert_eq!(file.recipients[1].pubkey, RECIPIENT_1);
    assert_eq!(
        recipients::read(&entry.path.join(".age-recipients")).unwrap(),
        [file.recipients[0].pubkey.clone(), RECIPIENT_1.to_string()],
        "same keys, same order, in both recipient-defining files"
    );
}

#[test]
fn the_id_init_files_the_identity_under_is_the_id_it_commits() {
    let env = Env::new();
    env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);
    let committed = env.vault_config("personal").vault.id;
    let path = paths::identity_file_path(&env.dirs, &committed, "laptop-1").unwrap();
    assert!(path.is_file(), "no identity under the id the vault carries");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap(),
        committed.as_str()
    );
    assert_eq!(env.entry("personal").id, Some(committed));
}

// The identity
// half (the [[recipients]] half is the library's).
#[test]
fn create_device_name_lands_in_recipient_and_identity_path() {
    let env = Env::new();
    env.ok(&[
        "init",
        "myvault",
        "--device",
        "custom-device",
        "--no-recovery-key",
    ]);
    let id = env.entry("myvault").id.unwrap();
    assert_eq!(
        env.vault_config("myvault").recipients[0].device,
        "custom-device"
    );
    let path = paths::identity_file_path(&env.dirs, &id, "custom-device").unwrap();
    assert!(
        path.ends_with(Path::new(id.as_str()).join("custom-device.age")),
        "{}",
        path.display()
    );
    assert!(path.is_file());
}

// The init half
// (clone's and identity add's have their own tests).
#[test]
fn init_records_the_pubkey() {
    let env = Env::new();
    env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);
    assert_eq!(
        env.entry("personal").pubkey,
        Some(env.vault_config("personal").recipients[0].pubkey.clone())
    );
}

// A failure knowable from the flags is reported before a
// human is asked to type a new passphrase twice. FakePrompter::new() fails
// the test if it is asked anything.
#[test]
fn init_refuses_bad_flags_before_asking_for_a_passphrase() {
    let env = Env::new();
    for args in [
        &["init", "a", "--type", "s3", "--no-recovery-key"][..],
        &["init", "a", "--method", "ssh", "--no-recovery-key"],
        &["init", "a", "--recipient", "nope", "--no-recovery-key"],
        &["init", "a", "--device", "../x", "--no-recovery-key"],
    ] {
        let r = env.run_with(args, &FakePrompter::new());
        assert_eq!(r.code, Code::Usage, "{args:?}");
    }
    assert!(!identities_root(&env).exists());
}

/// A terminal prompter reading `input` as piped stdin, its prompts
/// discarded: these tests assert on the command's own streams.
fn piped_prompter(input: &'static str) -> Box<dyn Prompter> {
    Box::new(TerminalPrompter::new(
        input.as_bytes(),
        std::io::sink(),
        SecretInput::Lines,
    ))
}

// When no passphrase can be had, init fails as an authentication failure
// (the exit status a script branches on) before writing anything: the
// passphrase is asked for before the key is generated, so there is no
// identity to roll back, no vault, and no registration. Through the fake
// prompter for each library-visible outcome, and through the real terminal
// prompter on piped stdin for the ways a human or script actually gets
// there.
#[test]
fn init_that_gets_no_passphrase_fails_locked_and_writes_nothing() {
    let cases: Vec<(&str, Box<dyn Prompter>)> = vec![
        (
            "prompter cancelled",
            Box::new(FakePrompter::new().with_unlock_error(PromptError::Cancelled)),
        ),
        (
            "empty passphrase",
            Box::new(FakePrompter::new().with_passphrase("")),
        ),
        ("stdin at end of input", piped_prompter("")),
        (
            "confirmation never matches",
            piped_prompter("a\nb\nc\nd\ne\nf\n"),
        ),
        ("only empty passphrases", piped_prompter("\n\n\n")),
        ("input ends mid-confirmation", piped_prompter("s3cret\n")),
    ];
    for (name, p) in cases {
        let env = Env::new();
        let r = env.run_with(
            &[
                "init",
                "personal",
                "--device",
                "laptop-1",
                "--no-recovery-key",
            ],
            &*p,
        );

        assert_eq!(r.code, Code::LockedOrAuth, "{name}: {}", r.stderr);
        assert!(r.stderr.starts_with("gage: "), "{name}: {}", r.stderr);
        assert_eq!(r.stdout, "", "{name}: reported success on stdout");
        assert!(
            !identities_root(&env).exists(),
            "{name}: wrote under identities/"
        );
        assert!(
            !env.default_vault_path("personal").exists(),
            "{name}: created the vault"
        );
        assert!(
            !paths::global_config_file(&env.dirs).exists(),
            "{name}: registered the vault"
        );

        // And nothing stands in the way of the retry.
        env.ok(&[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ]);
    }
}

// The in-process half of "prompts go to the error stream, not stdout":
// init driven through the real terminal prompter (on piped input) prints
// its prompts on the prompter's stream and nothing but its report on
// stdout. That main gives the prompter stderr is the smoke test's to prove.
#[test]
fn init_through_the_terminal_prompter_keeps_prompts_off_stdout() {
    let env = Env::new();
    let prompts = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let p = TerminalPrompter::new(
        "s3cret\ns3cret\n".as_bytes(),
        Sink(prompts.clone()),
        SecretInput::Lines,
    );

    let r = env.run_with(
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ],
        &p,
    );

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let prompts = String::from_utf8(prompts.borrow().clone()).unwrap();
    assert!(prompts.contains("Choose a passphrase"), "{prompts}");
    assert!(
        !r.stdout.contains("passphrase"),
        "a prompt reached stdout:\n{}",
        r.stdout
    );
    assert!(!r.stdout.contains("s3cret") && !prompts.contains("s3cret"));

    let vault = Vault::from_registration(&env.dirs, "personal", &env.entry("personal")).unwrap();
    vault
        .unlock(&FakePrompter::new().with_passphrase("s3cret"))
        .expect("the passphrase typed at the prompt is the one that unlocks");
}
