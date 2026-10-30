//! The trust cache through the real CLI, in-process, including the two
//! `--repair` rows and the row about two vaults under one
//! local name. The terminal rendering of the
//! question is tested beside the prompter (`prompter.rs`).

use std::path::{Path, PathBuf};

use crate::exitcode::ExitCode;
use gage::Prompter;
use gage::prompter::FakePrompter;
use gage::{gittest, storage::vaultconfig, sync::gitrepo, trust::recipient};

use crate::entry_tests::{DEVICE, unlocks};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::repl_tests::script;
use crate::{BuildInfo, Io};

const VAULT: &str = "personal";

pub(crate) fn new_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

/// [`unlocks`], ready to answer the recipient-change question with
/// `answers` and failing the test if asked more often than that.
fn answering(answers: &[bool]) -> FakePrompter {
    unlocks().with_recipient_answers(answers)
}

impl Env {
    pub(crate) fn vault_dir(&self) -> PathBuf {
        self.entry(VAULT).path
    }

    /// One command that unlocks, so the vault has a reviewed baseline to
    /// change from. Without it every later edit is a first use, which is
    /// trusted silently by design.
    pub(crate) fn bootstrap_trust(&self) {
        self.cmd_ok(&["ls"]);
    }

    fn trust_dir(&self) -> PathBuf {
        // Keyed by the vault's id, spelled out rather than asked of the
        // code under test.
        self.dirs.state.join(self.entry(VAULT).id.unwrap())
    }

    fn known_config(&self) -> Vec<u8> {
        std::fs::read(self.trust_dir().join("known-config.toml")).unwrap()
    }

    fn age_recipients(&self) -> Vec<String> {
        recipient::load(&self.vault_dir().join(".age-recipients")).unwrap()
    }

    /// `gage insert <title> --value-stdin <extra...>` through `p`.
    fn insert_with(&self, title: &str, extra: &[&str], p: &dyn Prompter) -> CliResult {
        let mut args = vec!["insert", title, "--value-stdin"];
        args.extend_from_slice(extra);
        self.run_stdin(&args, p, b"hunter2\n")
    }

    /// `args` with stdin a terminal, so bare `gage` would start a session.
    fn run_tty(&self, args: &[&str]) -> CliResult {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let dirs = self.dirs.clone();
        let hostname = self.hostname.clone();
        let mut io = Io {
            stdin: &mut std::io::empty(),
            stdout: &mut stdout,
            stderr: &mut stderr,
            stdin_is_terminal: true,
            stderr_is_terminal: true,
            line_editor: false,
            build: BuildInfo::stamped(),
            dirs: &move || Ok(dirs.clone()),
            hostname: &move || Ok(hostname.clone()),
            editor: None,
            passphrase_env: None,
            clock: &gage::SystemClock,
            clipboard: &crate::clipboard::fake::unused(),
            clipboard_wait: &|_| {},
            time_zone: jiff::tz::TimeZone::UTC,
        };
        let code = crate::run(&args, &mut io, &FakePrompter::new());
        CliResult {
            code,
            stdout: String::from_utf8(stdout).unwrap(),
            stderr: String::from_utf8(stderr).unwrap(),
        }
    }
}

/// Appends a recipient to both files and commits: the shape a legitimate
/// `recipient add` on another device arrives in after a pull.
pub(crate) fn commit_routine_change(vault: &Path, device: &str, pubkey: &str) {
    let path = vault.join(".age-recipients");
    let mut keys = recipient::load(&path).unwrap();
    keys.push(pubkey.into());
    recipient::save(&path, &keys).unwrap();
    let config = vault.join(".gage").join("config.toml");
    let mut file = vaultconfig::load(&config).unwrap();
    file.recipients.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    vaultconfig::save(&config, &file).unwrap();
    gitrepo::commit_all(vault, &format!("gage: recipient add {device}")).unwrap();
}

/// Edits only the file encryption reads, and commits it: the divergence
/// `verify` reports and the trust cache refuses to forget.
pub(crate) fn tamper_recipients_file(vault: &Path, pubkey: &str) {
    let path = vault.join(".age-recipients");
    let mut keys = recipient::load(&path).unwrap();
    keys.push(pubkey.into());
    recipient::save(&path, &keys).unwrap();
    gitrepo::commit_all(vault, "add a key").unwrap();
}

pub(crate) fn env_with_vault() -> Env {
    let env = Env::new();
    env.init(VAULT, &[]);
    env
}

// A change nobody here
// reviewed stops the write; declining leaves the vault as it was.
#[test]
fn insert_blocks_on_an_unreviewed_recipient_change() {
    let env = env_with_vault();
    env.bootstrap_trust();
    commit_routine_change(&env.vault_dir(), "phone-1", &new_key());
    let before = env.commits(VAULT);

    let p = answering(&[false]);
    let r = env.insert_with("ProtonMail", &[], &p);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("was not confirmed"), "{}", r.stderr);
    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].diff.contains("phone-1"), "{}", asked[0].diff);
    assert_eq!(env.commits(VAULT), before, "a declined write committed");
    assert!(!env.cmd_ok(&["ls"]).stdout.contains("ProtonMail"));
}

// `--yes` answers before
// the prompter is asked (this one fails the test if it is), and a routine
// change stays acknowledged afterwards, as an interactive yes would.
#[test]
fn yes_bypasses_the_recipient_prompt_for_scripting() {
    let env = env_with_vault();
    env.bootstrap_trust();
    commit_routine_change(&env.vault_dir(), "phone-1", &new_key());

    let p = answering(&[]);
    let r = env.insert_with("ProtonMail", &["--yes"], &p);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(p.recipient_changes().is_empty(), "--yes still asked");

    // Also before the command, where a root flag usually goes.
    let r = env.run_stdin(
        &["--yes", "insert", "Router", "--value-stdin"],
        &answering(&[]),
        b"v\n",
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

    let p = answering(&[]);
    let r = env.insert_with("Bank", &[], &p);
    assert_eq!(
        r.code,
        ExitCode::Success,
        "the warning came back: {}",
        r.stderr
    );
}

// `--yes` proceeds but
// buys no silence: regeneration is decided from `verify`, so who answered
// can't change it.
#[test]
fn yes_does_not_regenerate_the_cache_on_a_mismatch() {
    let env = env_with_vault();
    env.bootstrap_trust();
    let cache = env.known_config();
    let stray = new_key();
    tamper_recipients_file(&env.vault_dir(), &stray);

    for title in ["ProtonMail", "Bank"] {
        let r = env.insert_with(title, &["--yes"], &unlocks());
        assert_eq!(r.code, ExitCode::Success, "{title}: {}", r.stderr);
    }
    assert_eq!(env.known_config(), cache, "--yes regenerated the cache");

    // Without --yes, the next write still asks.
    let p = answering(&[false]);
    let r = env.insert_with("Router", &[], &p);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(p.recipient_changes()[0].mismatched());

    let verify = env.run_with(&["recipient", "verify"], &FakePrompter::new());
    assert_eq!(verify.code, ExitCode::Conflict, "{}", verify.stdout);
    assert!(verify.stderr.contains(&stray), "{}", verify.stderr);
}

// `--yes` answers the
// trust question and nothing else: "this device will no longer be able to
// read this vault" still fails closed in a script.
#[test]
fn yes_does_not_answer_the_self_removal_confirmation() {
    let env = env_with_vault();
    // A second recipient, so the removal isn't refused as the last one.
    env.cmd_ok(&["recipient", "add", &new_key(), "--device", "phone-1"]);
    let own = env.entry(VAULT).pubkey.unwrap();

    let p = unlocks().with_confirm(false);
    let r = env.run_with(&["recipient", "remove", DEVICE, "--reencrypt", "--yes"], &p);
    assert_ne!(r.code, ExitCode::Success, "{}", r.stdout);
    assert_eq!(p.confirm_prompts().len(), 1, "--yes answered the question");
    assert!(env.age_recipients().contains(&own), "removed anyway");
}

// Refused,
// naming the difference, and the stray key left for `verify`.
#[test]
fn recipient_add_is_refused_over_a_diverged_vault_from_the_cli() {
    let env = env_with_vault();
    env.bootstrap_trust();
    let stray = new_key();
    tamper_recipients_file(&env.vault_dir(), &stray);
    let before = env.age_recipients();

    let r = env.cmd(&["recipient", "add", &new_key(), "--device", "phone-1"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(&stray), "{}", r.stderr);
    assert!(r.stderr.contains("--repair"), "{}", r.stderr);
    assert_eq!(
        env.age_recipients(),
        before,
        "the evidence was rebuilt away"
    );
}

// The refusal's exit:
// the same overwrite, deliberate, saying what it dropped.
#[test]
fn recipient_verify_repair_fixes_the_divergence() {
    let env = env_with_vault();
    env.bootstrap_trust();
    let stray = new_key();
    tamper_recipients_file(&env.vault_dir(), &stray);
    let none = FakePrompter::new();
    assert_eq!(
        env.run_with(&["recipient", "verify"], &none).code,
        ExitCode::Conflict
    );

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["recipient", "verify", "--repair"], &p);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains(&format!("dropped {stray}")),
        "{}",
        r.stdout
    );
    assert!(p.confirm_prompts()[0].contains(&stray));
    assert!(p.unlock_requests().is_empty(), "repair unlocked");

    assert_eq!(
        env.run_with(&["recipient", "verify"], &none).code,
        ExitCode::Success
    );
    assert!(!env.age_recipients().contains(&stray));
    assert!(gitrepo::is_clean(&env.vault_dir()).unwrap());
}

#[test]
fn recipient_verify_repair_on_a_vault_in_sync_says_there_is_nothing_to_do() {
    let env = env_with_vault();
    let before = gitrepo::head_commit(&env.vault_dir()).unwrap();
    let r = env.run_with(&["recipient", "verify", "--repair"], &FakePrompter::new());
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(r.stdout.contains("already in sync"), "{}", r.stdout);
    assert!(r.stdout.contains("nothing to repair"), "{}", r.stdout);
    assert_eq!(gitrepo::head_commit(&env.vault_dir()).unwrap(), before);
}

// A key only
// config.toml knew about is added by the rewrite, and can read everything
// written from here on: saying so matters as much as what was dropped.
#[test]
fn recipient_verify_repair_reports_what_it_added() {
    let env = env_with_vault();
    let declared = new_key();
    let config = env.vault_dir().join(".gage").join("config.toml");
    let mut file = vaultconfig::load(&config).unwrap();
    file.recipients.push(vaultconfig::Recipient {
        device: "phone-1".into(),
        pubkey: declared.clone(),
    });
    vaultconfig::save(&config, &file).unwrap();
    gitrepo::commit_all(&env.vault_dir(), "declare a key in config only").unwrap();

    let r = env.run_with(
        &["recipient", "verify", "--repair"],
        &FakePrompter::new().with_confirm(true),
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains(&format!("added {declared}")),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("committed locally as"), "{}", r.stdout);
    assert!(env.age_recipients().contains(&declared));
}

// A declined repair is a refusal, exit Conflict, with nothing written.
#[test]
fn recipient_verify_repair_declined_writes_nothing() {
    let env = env_with_vault();
    tamper_recipients_file(&env.vault_dir(), &new_key());
    let before = env.age_recipients();
    let r = env.run_with(
        &["recipient", "verify", "--repair"],
        &FakePrompter::new().with_confirm(false),
    );
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("not repairing"), "{}", r.stderr);
    assert_eq!(env.age_recipients(), before);
}

// Keeping it would let a re-add
// inherit an approval of a list nobody looked at in between.
#[test]
fn vault_remove_deletes_the_trust_cache() {
    let env = env_with_vault();
    env.bootstrap_trust();
    let dir = env.trust_dir();
    assert!(dir.join("known-config.toml").is_file(), "nothing to remove");
    env.ok(&["vault", "remove", VAULT]);
    assert!(!dir.exists(), "{} is still there", dir.display());
}

// On a line it would do nothing (the
// question goes through the prompter the session holds); on the way in it
// would approve every change for the session's life. Both say so.
#[test]
fn yes_is_refused_in_a_session() {
    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), &new_key());

    let r = env.session(&script(&[
        "use personal",
        PASSPHRASE,
        "insert --yes First",
        "exit",
    ]));
    assert!(
        r.stderr.contains("isn't available in a session"),
        "{}",
        r.stderr
    );
    assert!(
        !env.cmd_ok(&["ls"]).stdout.contains("First"),
        "a command whose --yes was refused wrote anyway"
    );

    let r = env.run_tty(&["--yes"]);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(
        r.stderr.contains("isn't available in a session"),
        "{}",
        r.stderr
    );
    assert!(!r.stdout.contains("gage> "), "a session started anyway");
}

// `ls` and `show` warn on
// stderr (stdout carries only the entry), succeed, ask nothing, and leave
// the cache alone.
#[test]
fn reads_warn_opportunistically_in_one_shot_mode() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["ProtonMail"]);
    commit_routine_change(&env.vault_dir(), "phone-1", &new_key());
    let cache = env.known_config();

    for (args, entry) in [
        (&["ls"][..], "ProtonMail"),
        (&["show", "ProtonMail"], "hunter2"),
    ] {
        let p = answering(&[]);
        let r = env.run_with(args, &p);
        assert_eq!(r.code, ExitCode::Success, "{args:?}: {}", r.stderr);
        assert!(
            r.stderr.contains("recipient list changed"),
            "{args:?}: {}",
            r.stderr
        );
        assert!(!r.stdout.contains("recipient list changed"), "{}", r.stdout);
        assert!(r.stdout.contains(entry), "{args:?}: {}", r.stdout);
        assert!(p.recipient_changes().is_empty(), "{args:?} asked");
    }
    assert_eq!(env.known_config(), cache, "a read regenerated the cache");
}

// The real terminal
// prompter, in a session, asks the same question from the same hook a
// one-shot write uses; `rename` has no value prompt, so the `n` can only
// be answering it. Declining blocks, as outside a session.
#[test]
fn session_mode_asks_the_same_recipient_question() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["ProtonMail"]);
    commit_routine_change(&env.vault_dir(), "phone-1", &new_key());

    let r = env.session(&script(&[
        "use personal",
        PASSPHRASE,
        r#"rename ProtonMail "Proton Mail""#,
        "n",
        "exit",
    ]));
    let transcript = format!("{}{}", r.stdout, r.stderr);
    for want in ["Recipients for", "phone-1", "[y/N]"] {
        assert!(transcript.contains(want), "missing {want:?}:\n{transcript}");
    }
    let ls = env.cmd_ok(&["ls"]).stdout;
    assert!(
        ls.contains("ProtonMail") && !ls.contains("Proton Mail"),
        "{ls}"
    );
}

// An unrelated vault cloned under a removed vault's local name
// inherits neither its identity nor its trust cache.
#[test]
fn two_vaults_under_one_local_name_share_neither_directory() {
    let publisher = Env::new();
    let remote = gittest::new_bare_remote();
    publisher.ok(&[
        "init",
        "vault-b",
        "--no-recovery-key",
        "--remote",
        remote.path().to_str().unwrap(),
    ]);

    let env = Env::new();
    env.ok(&["init", VAULT, "--device", "laptop-1", "--no-recovery-key"]);
    let a_id = env.entry(VAULT).id.unwrap();
    let a_identity =
        gage::storage::paths::identity_file_path(&env.dirs, &a_id, "laptop-1").unwrap();
    let a_trust = env.trust_dir();
    env.insert_ok("hunter2", &["SomethingOfAs"]);
    assert!(
        a_trust.join("known-config.toml").is_file(),
        "A has no trust cache to inherit; the test would prove nothing"
    );

    env.ok(&["vault", "remove", VAULT]);
    let r = env.run_with(
        &[
            "clone",
            remote.path().to_str().unwrap(),
            "--name",
            VAULT,
            "--device",
            "laptop-1",
            "--dir",
            env.root.path().join("vault-b").to_str().unwrap(),
        ],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let b_id = env.entry(VAULT).id.unwrap();
    assert_ne!(b_id, a_id, "two unrelated vaults share an id");

    let b_identity =
        gage::storage::paths::identity_file_path(&env.dirs, &b_id, "laptop-1").unwrap();
    assert!(!b_identity.exists(), "B sees an identity it never created");
    assert!(a_identity.is_file(), "A's identity was disturbed");
    assert_ne!(env.trust_dir(), a_trust, "B inherited A's trust cache");
}
