//! `gage recipient add` and `recipient remove` through the real CLI,
//! in-process: the write-side rows of Go's `recipient_cli_test.go`. Each
//! test names its Go row. Built on M9a's [`TwoDevices`], so the key a
//! recipient change grants is a real second device's, and what it can read
//! is checked from that device.

use gage::exitcode::Code;
use gage::prompter::FakePrompter;
use gage::{gitrepo, recipients};

use crate::commands::short_hash;
use crate::identity_cli_tests::{A_DEVICE, B_DEVICE, TwoDevices, VAULT};
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::registry::{GROUP_IDENTITY, GROUP_RECIPIENT, registry};

const ADD_SHORT: &str = "Authorize a public key and re-encrypt the vault to include it";

fn new_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

impl Env {
    fn path(&self) -> std::path::PathBuf {
        self.entry(VAULT).path
    }

    /// HEAD, as far as a CLI test can see it: how many commits, and the
    /// newest one's message. Nothing these tests run rewrites history, so
    /// an unchanged pair is an unchanged HEAD.
    fn head(&self) -> (usize, String) {
        (
            self.commits(VAULT),
            gitrepo::head_commit(&self.path()).unwrap().message,
        )
    }

    fn recipients_file(&self) -> Vec<String> {
        recipients::read(&self.path().join(".age-recipients")).unwrap()
    }

    /// `gage insert <title> --value-stdin`, with `value` on stdin.
    fn insert(&self, title: &str, value: &str) {
        let r = self.run_stdin(
            &["insert", title, "--value-stdin"],
            &FakePrompter::new().with_passphrase(PASSPHRASE),
            format!("{value}\n").as_bytes(),
        );
        assert_eq!(r.code, Code::Success, "insert {title}: {}", r.stderr);
    }

    /// `gage recipient add <key> --device <device>`, required to succeed.
    pub(crate) fn add_recipient(&self, key: &str, device: &str) -> String {
        self.ok(&["recipient", "add", key, "--device", device])
            .stdout
    }
}

/// B registers its own identity; returns the key it printed.
fn b_identity(t: &TwoDevices) -> String {
    let r = t.b.ok(&["identity", "add", "--device", B_DEVICE]);
    let key = t.b.entry(VAULT).pubkey;
    assert!(r.stdout.contains(&key), "{}", r.stdout);
    key
}

// Go: TestRecipientAddCommitsBothFilesInOneCommit. One commit, a clean
// tree, the key in both files and labelled with its device.
#[test]
fn recipient_add_commits_both_files_in_one_commit() {
    let t = TwoDevices::new();
    let key = new_key();
    let before = t.a.commits(VAULT);

    let stdout = t.a.add_recipient(&key, "phone-1");

    assert_eq!(t.a.commits(VAULT), before + 1);
    assert!(gitrepo::is_clean(&t.a.path()).unwrap());
    assert!(t.a.recipients_file().contains(&key));
    let config = t.a.vault_config(VAULT);
    let added = config.recipients.iter().find(|r| r.pubkey == key).unwrap();
    assert_eq!(added.device, "phone-1");
    assert!(
        stdout.contains(&format!("added recipient \"phone-1\" ({key})")),
        "{stdout}"
    );
    let short = stdout
        .rsplit_once("committed locally as ")
        .map(|(_, h)| h.trim())
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(
        short.len() == 12 && short.chars().all(|c| c.is_ascii_hexdigit()),
        "{stdout}"
    );
    assert_eq!(t.a.head().1, "gage: recipient add phone-1 (reencrypt)");
}

// Go: TestRecipientAddMakesHistoryReadable, end to end across two devices:
// no flag asks for the re-encryption, the count line reports it, and B
// reads what A wrote before the add.
#[test]
fn recipient_add_makes_history_readable() {
    let t = TwoDevices::new();
    for title in ["ProtonMail", "Bank"] {
        t.a.insert(title, &format!("secret-{title}"));
    }
    let key = b_identity(&t);
    let before = t.a.commits(VAULT);

    let stdout = t.a.add_recipient(&key, B_DEVICE);

    assert!(stdout.contains("re-encrypted 2 entries"), "{stdout}");
    assert_eq!(t.a.commits(VAULT), before + 1);
    assert!(gitrepo::is_clean(&t.a.path()).unwrap());
    // A still reads its own; B, after its unlock's pull, reads both.
    assert!(
        t.a.ok(&["show", "ProtonMail"])
            .stdout
            .contains("secret-ProtonMail")
    );
    for title in ["ProtonMail", "Bank"] {
        let r = t.b.ok(&["show", title]);
        assert!(
            r.stdout.contains(&format!("secret-{title}")),
            "{}",
            r.stdout
        );
    }
}

// Go: TestRecipientAddRejectsTheRemovedReencryptFlag. A usage error naming
// the flag, never a silently ignored no-op.
#[test]
fn recipient_add_rejects_the_removed_reencrypt_flag() {
    let t = TwoDevices::new();
    let key = new_key();
    let head = t.a.head();
    let p = FakePrompter::new();

    let r = t.a.run_with(
        &[
            "recipient",
            "add",
            &key,
            "--device",
            "phone-1",
            "--reencrypt",
        ],
        &p,
    );

    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--reencrypt"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
    assert_eq!(t.a.head(), head);
    assert!(!t.a.recipients_file().contains(&key));
}

// Go: TestRecipientAddShortDescribesTheReEncryption. The add is a
// whole-vault rewrite, which is what makes it refusable, so its one-line
// description says so, on the surface people read it from.
#[test]
fn recipient_add_short_describes_the_re_encryption() {
    assert_eq!(registry().find("recipient add").unwrap().short, ADD_SHORT);
    let help = Env::new().ok(&["help"]).stdout;
    assert!(help.contains(ADD_SHORT), "{help}");
}

// Go: TestIdentityAddPrintsANextCommandThatWorks. The printed line is
// copy-pasted, so it is taken from B's stdout and run verbatim on A, the
// device that can already read the vault, as the text says.
#[test]
fn identity_add_prints_a_next_command_that_works() {
    let t = TwoDevices::new();
    t.a.insert("ProtonMail", "hunter2");
    let r = t.b.ok(&["identity", "add", "--device", B_DEVICE]);

    let next = r
        .stdout
        .lines()
        .find(|l| l.contains("gage recipient add "))
        .unwrap_or_else(|| panic!("no next command in:\n{}", r.stdout))
        .trim()
        .trim_start_matches("gage:")
        .trim()
        .to_string();
    assert!(!next.contains("--reencrypt"), "{next}");
    let words: Vec<&str> = next.split_whitespace().collect();
    assert_eq!(words[0], "gage", "{next}");

    t.a.ok(&words[1..]);
    assert!(t.b.ok(&["show", "ProtonMail"]).stdout.contains("hunter2"));
}

// Go: TestRecipientRemoveWithoutReencryptIsRejectedByTheCLI. Refused
// before the unlock: nobody types a passphrase only to be told the command
// line was wrong.
#[test]
fn recipient_remove_without_reencrypt_is_rejected_by_the_cli() {
    let t = TwoDevices::new();
    let key = new_key();
    t.a.add_recipient(&key, "phone-1");
    let head = t.a.head();
    let p = FakePrompter::new();

    let r = t.a.run_with(&["recipient", "remove", "phone-1"], &p);

    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--reencrypt"), "{}", r.stderr);
    assert!(
        p.unlock_requests().is_empty(),
        "it asked for a passphrase first"
    );
    assert_eq!(t.a.head(), head);
    assert!(t.a.recipients_file().contains(&key));
}

// Go: TestRecipientRemoveReencryptPrintsTheRevocationWarning. On stderr, so
// stdout stays usable in a pipe; and the removed device can no longer read.
#[test]
fn recipient_remove_reencrypt_prints_the_revocation_warning() {
    let t = TwoDevices::new();
    let key = b_identity(&t);
    t.a.add_recipient(&key, B_DEVICE);
    t.a.insert("ProtonMail", "hunter2");
    assert!(t.b.ok(&["show", "ProtonMail"]).stdout.contains("hunter2"));

    let r = t.a.ok(&["recipient", "remove", B_DEVICE, "--reencrypt"]);

    for want in ["future access", "already read"] {
        assert!(r.stderr.contains(want), "{}", r.stderr);
        assert!(!r.stdout.contains(want), "{}", r.stdout);
    }
    assert!(r.stdout.contains("removed recipient"), "{}", r.stdout);
    assert!(!t.a.recipients_file().contains(&key));
    let r = t.b.run(&["show", "ProtonMail"]);
    assert_ne!(r.code, Code::Success, "B still reads it:\n{}", r.stdout);
}

// Go: TestRecipientCommandsAreAvailableInBothModes. Vault-domain commands,
// in both help surfaces, in their own groups.
#[test]
fn recipient_commands_are_available_in_both_modes() {
    let reg = registry();
    for name in [
        "identity add",
        "identity list",
        "recipient add",
        "recipient remove",
        "recipient list",
        "recipient verify",
    ] {
        let c = reg
            .find(name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        assert!(
            c.availability.session_visible() && c.availability.one_shot_visible(),
            "{name}"
        );
        let want = if name.starts_with("identity") {
            GROUP_IDENTITY
        } else {
            GROUP_RECIPIENT
        };
        assert_eq!(c.group, want, "{name}");
        assert!(!c.short.is_empty(), "{name}");
    }
}

// Go: TestRecipientListMatchesConfigAndReflectsChanges. Line for line, in
// config order, through an add and a remove in the same run.
#[test]
fn recipient_list_matches_config_and_reflects_changes() {
    let t = TwoDevices::new();
    let assert_matches = |when: &str| {
        let out = t.a.ok(&["recipient", "list"]).stdout;
        let lines: Vec<&str> = out.lines().collect();
        let want = t.a.vault_config(VAULT).recipients;
        assert_eq!(lines.len(), want.len(), "{when}:\n{out}");
        for (line, r) in lines.iter().zip(&want) {
            assert!(
                line.contains(&r.device) && line.contains(&r.pubkey),
                "{when}: {line:?}"
            );
        }
        out
    };

    assert!(assert_matches("at creation").contains(A_DEVICE));
    let key = new_key();
    t.a.add_recipient(&key, "phone-1");
    assert!(assert_matches("after add").contains(&key));
    t.a.ok(&["recipient", "remove", "phone-1", "--reencrypt"]);
    assert!(!assert_matches("after remove").contains(&key));
}

// Go: TestShortHashLeavesShortStringsAlone
#[test]
fn short_hash_leaves_short_strings_alone() {
    for s in ["", "abc", "0123456789ab"] {
        assert_eq!(short_hash(s), s);
    }
    let full = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(short_hash(full), &full[..12]);
}

// Not a Go row: removing this device's own key through the CLI asks, and
// the default no aborts with nothing written; the library half is
// `recipient_remove_of_this_device_needs_confirmation`.
#[test]
fn recipient_remove_of_this_device_asks_and_a_no_writes_nothing() {
    let t = TwoDevices::new();
    t.a.add_recipient(&new_key(), "phone-1");
    let head = t.a.head();
    let p = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_confirm(false);

    let r =
        t.a.run_with(&["recipient", "remove", A_DEVICE, "--reencrypt"], &p);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("nothing was written"), "{}", r.stderr);
    assert_eq!(p.confirm_prompts().len(), 1);
    assert_eq!(t.a.head(), head);
}
