//! `gage vault remove`'s orphan cleanup through the real CLI, in-process;
//! ⚑ marks the tests that
//! guard an irreversible delete (the only copy of a private key).
//!
//! The library's [`gage::orphan`] tests prove each keep reason without a
//! CLI in the way; these prove the CLI renders them, asks, and deletes
//! only on a yes.

use std::path::{Path, PathBuf};

use gage::exitcode::Code;
use gage::prompter::FakePrompter;
use gage::{paths, recipients, vaultconfig};

use crate::identity_cli_tests::{B_DEVICE, TwoDevices, VAULT};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, config_file};
use crate::prompter::{SecretInput, TerminalPrompter};

fn new_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

impl Env {
    fn identity_file(&self, name: &str, device: &str) -> PathBuf {
        paths::identity_file_path(&self.dirs, self.entry(name).id.as_ref().unwrap(), device)
            .unwrap()
    }

    /// `vault remove <name>` answering the one confirmation with `answer`.
    fn remove_answering(&self, name: &str, answer: bool) -> (CliResult, FakePrompter) {
        let p = FakePrompter::new().with_confirm(answer);
        let r = self.run_with(&["vault", "remove", name], &p);
        assert_eq!(r.code, Code::Success, "vault remove: {}", r.stderr);
        (r, p)
    }

    /// Rewrites the vault's committed recipient list so this device's key
    /// is no longer in it: what a `recipient remove` from another device
    /// leaves behind, without needing that device.
    fn orphan_this_devices_key(&self, name: &str) {
        let entry = self.entry(name);
        let replacement = new_key();
        let path = config_file(&entry.path);
        let mut file = vaultconfig::read(&path).unwrap();
        for r in &mut file.recipients {
            if Some(&r.pubkey) == entry.pubkey.as_ref() {
                r.pubkey = replacement.clone();
            }
        }
        vaultconfig::write(&path, &file).unwrap();
        recipients::write(
            &entry.path.join(".age-recipients"),
            std::slice::from_ref(&replacement),
        )
        .unwrap();
    }

    fn set_registration(&self, name: &str, f: impl FnOnce(&mut gage::config::VaultEntry)) {
        let mut global = self.global();
        f(global.vaults.get_mut(name).unwrap());
        self.save_global(&global);
    }
}

fn listing(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.push((p.clone(), None));
                walk(&p, out);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                out.push((p, Some(bytes)));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

// Confirmed, the file is
// gone: laptop-1 dropped its own recipiency, leaving phone-1 the only
// recipient.
#[test]
fn vault_remove_deletes_an_orphaned_identity_file() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    env.ok(&["recipient", "add", &new_key(), "--device", "phone-1"]);
    // Removing this device's own key asks first; the answer is yes.
    let own = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_confirm(true);
    let r = env.run_with(&["recipient", "remove", "laptop-1", "--reencrypt"], &own);
    assert_eq!(r.code, Code::Success, "recipient remove: {}", r.stderr);
    let id_path = env.identity_file("personal", "laptop-1");
    assert!(id_path.is_file(), "the file exists before vault remove");

    let (r, p) = env.remove_answering("personal", true);

    assert!(!id_path.exists(), "{}", r.stderr);
    let prompts = p.confirm_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains("cannot be undone"), "{}", prompts[0]);
    assert!(!env.global().vaults.contains_key("personal"));
}

// ⚑ Deleting
// it would destroy the only copy of the key that reads the vault.
#[test]
fn vault_remove_keeps_identity_file_still_listed_as_recipient() {
    let env = Env::new();
    env.init(
        "personal",
        &["--device", "laptop-1", "--recipient", &new_key()],
    );
    let id_path = env.identity_file("personal", "laptop-1");

    // Scripted to say yes: it must never be asked.
    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(id_path.is_file());
    assert!(
        r.stderr.contains("still listed as a recipient"),
        "{}",
        r.stderr
    );
    assert!(p.confirm_prompts().is_empty());
}

// ⚑ With the
// store deleted, gage cannot confirm the key was dropped, so it keeps it.
#[test]
fn vault_remove_keeps_identity_file_when_recipients_unreadable() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    std::fs::remove_dir_all(&path).unwrap();

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(id_path.is_file());
    assert!(r.stderr.contains("could not confirm"), "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
}

// ⚑ (the vault-id keying
// regression, end to end). Removing B, cloned under A's freed-up name, must
// not touch A's key.
#[test]
fn vault_remove_never_deletes_a_key_belonging_to_a_different_vault() {
    let origin = gage::gittest::new_bare_remote();
    let url = origin.path().to_str().unwrap();
    // Vault B, published from another machine.
    let other = Env::new();
    other.ok(&[
        "init",
        "vault-b",
        "--device",
        "phone-9",
        "--remote",
        url,
        "--no-recovery-key",
    ]);

    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let a_identity = env.identity_file("personal", "laptop-1");

    // Removing A keeps its key: laptop-1 is still a recipient there.
    env.ok(&["vault", "remove", "personal"]);
    assert!(a_identity.is_file(), "deleted while still a recipient");

    // B, cloned under A's freed-up name. This device is not a recipient of
    // B and holds no key for it.
    let dir = env.root.path().join("vault-b");
    let r = env.run_with(
        &[
            "clone",
            url,
            "--name",
            "personal",
            "--device",
            "laptop-1",
            "--dir",
            dir.to_str().unwrap(),
        ],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Success, "clone: {}", r.stderr);

    // Whatever it is asked, B's removal must not touch A's key.
    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(a_identity.is_file(), "removing B destroyed A's only key");
}

// ⚑ The registration-versus-vault check, reached through the
// CLI. "personal" is registered with the id of a different vault ("other")
// whose key this device holds, and the recorded key is one "personal" does
// not list. Without the check the key would be judged against the wrong
// vault's recipients and offered for deletion.
#[test]
fn vault_remove_keeps_the_file_when_the_registration_points_at_another_vault() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    env.init("other", &["--device", "laptop-1"]);
    let other = env.entry("other");
    let id_path = env.identity_file("other", "laptop-1");
    env.set_registration("personal", |e| {
        e.id = other.id.clone();
        e.pubkey = Some(new_key());
    });

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        p.confirm_prompts().is_empty(),
        "offered a key of another vault"
    );
    assert!(id_path.is_file());
    assert!(r.stderr.contains("different vault"), "{}", r.stderr);
}

// ⚑ (ADR-0013's
// false-delete case). The vault lists this key under another label; the key
// matches, so it stays, and the message names the label the vault uses.
#[test]
fn vault_remove_keeps_a_relabelled_recipients_key() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    let mut file = vaultconfig::read(&config_file(&path)).unwrap();
    file.recipients[0].device = "laptop-1-work".into();
    vaultconfig::write(&config_file(&path), &file).unwrap();

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(id_path.is_file(), "deleted an active recipient's key");
    assert!(
        r.stderr.contains("still listed as a recipient"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("laptop-1-work"), "{}", r.stderr);
    assert!(p.confirm_prompts().is_empty());
}

// A stale key whose name now labels another (the
// false-keep direction). The name matches and the key does not, so the
// file is a genuine orphan and the offer is made.
#[test]
fn vault_remove_identifies_a_stale_key_whose_name_now_labels_another() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    // Someone else's key takes over the label "laptop-1".
    let mut file = vaultconfig::read(&config_file(&path)).unwrap();
    assert_eq!(file.recipients[0].device, "laptop-1");
    file.recipients[0].pubkey = new_key();
    vaultconfig::write(&config_file(&path), &file).unwrap();

    let (_, p) = env.remove_answering("personal", true);

    assert_eq!(p.confirm_prompts().len(), 1);
    assert!(
        !id_path.exists(),
        "a real orphan was kept for its old label"
    );
}

// ⚑ The prompt names the
// path, which is the whole content of the decision.
#[test]
fn vault_remove_never_deletes_without_confirming() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    env.orphan_this_devices_key("personal");

    let (r, p) = env.remove_answering("personal", false);

    let prompts = p.confirm_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].contains(id_path.to_str().unwrap()),
        "{}",
        prompts[0]
    );
    assert!(id_path.is_file(), "answering no still deleted the file");
    assert!(r.stderr.contains("not confirmed"), "{}", r.stderr);
}

// ⚑ Through the
// real terminal prompter with nothing to read, the answer is no, by
// construction rather than by a check somebody remembered.
#[test]
fn non_interactive_vault_remove_never_deletes_an_identity() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    env.orphan_this_devices_key("personal");

    let p = TerminalPrompter::new("".as_bytes(), std::io::sink(), SecretInput::Lines);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(id_path.is_file(), "a scripted vault remove deleted a key");
    assert!(!env.global().vaults.contains_key("personal"));
}

// A confirmation that fails outright is not a yes ⚑.
#[test]
fn vault_remove_treats_a_failed_confirmation_as_no() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    env.orphan_this_devices_key("personal");

    let p = FakePrompter::new().with_confirm_error(gage::prompter::PromptError::NotInteractive);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(id_path.is_file());
    assert!(r.stderr.contains("not confirmed"), "{}", r.stderr);
}

// ⚑ A clone that
// never ran `identity add` leaves nothing to compare: say so, don't ask.
#[test]
fn vault_remove_keeps_the_file_when_no_pubkey_is_recorded() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let id_path = env.identity_file("personal", "laptop-1");
    env.orphan_this_devices_key("personal");
    env.set_registration("personal", |e| e.pubkey = None);

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal"], &p);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        p.confirm_prompts().is_empty(),
        "asked about a key it cannot identify"
    );
    assert!(id_path.is_file());
    assert!(
        r.stderr.contains("doesn't know which public key"),
        "{}",
        r.stderr
    );
}

// ⚑ A local key
// the vault doesn't list yet looks exactly like an orphan, so gage offers,
// and the confirmation is all that stands between the human and a key whose
// approval is in flight.
#[test]
fn vault_remove_on_a_pending_enrollment_asks_before_deleting() {
    let t = TwoDevices::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let r = t.b.run_with(&["identity", "add", "--device", B_DEVICE], &p);
    assert_eq!(r.code, Code::Success, "identity add: {}", r.stderr);
    let id_path = t.b.identity_file(VAULT, B_DEVICE);

    let (_, p) = t.b.remove_answering(VAULT, false);

    let prompts = p.confirm_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].contains(id_path.to_str().unwrap()),
        "{}",
        prompts[0]
    );
    assert!(id_path.is_file(), "answering no deleted a pending key");
}

// The same guard against a *real* pending request, in
// place of a simulated `identity add`. The request is in flight, the
// key is not yet a recipient, and the confirmation is all that stands
// between the human and a key whose approval is on its way.
#[test]
fn vault_remove_with_a_real_pending_request_asks_before_deleting() {
    let t = TwoDevices::new();
    let r = t.b.run(&["identity", "enroll", "--device", B_DEVICE]);
    assert_eq!(r.code, Code::Success, "identity enroll: {}", r.stderr);
    let id_path = t.b.identity_file(VAULT, B_DEVICE);

    let (_, p) = t.b.remove_answering(VAULT, false);

    let prompts = p.confirm_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].contains(id_path.to_str().unwrap()),
        "{}",
        prompts[0]
    );
    assert!(id_path.is_file(), "answering no deleted a pending key");
}

// `vault remove`
// forgets a local registration and never touches the store, even when the
// identity file is deleted.
#[test]
fn vault_remove_leaves_the_vaults_own_files_untouched() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    env.insert_ok("hunter2", &["ProtonMail"]);
    env.orphan_this_devices_key("personal");
    let before = listing(&path);

    let (_, p) = env.remove_answering("personal", true);

    assert_eq!(p.confirm_prompts().len(), 1);
    assert_eq!(listing(&path), before, "vault remove changed the store");
}

// ⚑ One repository registered twice shares one identities directory.
// Removing either registration keeps a key the other still needs: the one
// still a recipient (by the key comparison), and the one that holds a key
// but is not yet a recipient (by the confirmation).
#[test]
fn one_repository_registered_twice_shares_one_identities_directory() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let mut global = env.global();
    let entry = global.vaults["personal"].clone();
    global.vaults.insert("personal-alias".into(), entry);
    env.save_global(&global);
    assert_eq!(env.entry("personal").id, env.entry("personal-alias").id);
    let id_path = env.identity_file("personal", "laptop-1");

    // Still a recipient: removing the alias asks nothing and keeps the key.
    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["vault", "remove", "personal-alias"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        p.confirm_prompts().is_empty(),
        "offered a live recipient's key"
    );
    assert!(id_path.is_file());

    // Holds a key but is not yet a recipient: answering no keeps it.
    env.orphan_this_devices_key("personal");
    let (_, p) = env.remove_answering("personal", false);
    assert_eq!(p.confirm_prompts().len(), 1);
    assert!(id_path.is_file());
}

// No identity file means nothing to say.
#[test]
fn vault_remove_with_no_identity_file_says_nothing() {
    let t = TwoDevices::new();
    // B cloned and holds no identity.
    let p = FakePrompter::new().with_confirm(true);
    let r = t.b.run_with(&["vault", "remove", VAULT], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(r.stderr, "");
    assert!(p.confirm_prompts().is_empty());
}
