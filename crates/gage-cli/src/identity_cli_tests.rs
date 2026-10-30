//! `gage identity add/list` and `gage recipient list/verify` through the
//! real CLI, in-process.
//!
//! [`TwoDevices`] is the CLI's two-machine fixture: two [`Env`]s, each with
//! its own roots and hostname, over one bare origin. Device A ran
//! `gage init --remote`; device B ran `gage clone` and holds no identity.
//! The tests build on it: B's `identity add` prints the key A's
//! `recipient add` takes, and a recipient change pushed by A is what B
//! pulls.

use crate::exitcode::ExitCode;
use gage::gittest::{self, BareRemote};
use gage::prompter::{FakePrompter, Purpose};
use gage::{storage::paths, storage::vaultconfig, sync::gitrepo, trust::recipient};

use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, config_file};

pub(crate) const A_DEVICE: &str = "laptop-1";
pub(crate) const B_DEVICE: &str = "laptop-2";
pub(crate) const VAULT: &str = "personal";

pub(crate) struct TwoDevices {
    pub(crate) a: Env,
    pub(crate) b: Env,
    /// Kept so the bare remote outlives the test body.
    pub(crate) _origin: BareRemote,
}

impl TwoDevices {
    pub(crate) fn new() -> TwoDevices {
        let a = Env::new();
        let b = Env::new();
        let origin = gittest::new_bare_remote();
        let url = origin.path().to_str().unwrap();
        a.ok(&[
            "init",
            VAULT,
            "--device",
            A_DEVICE,
            "--remote",
            url,
            "--no-recovery-key",
        ]);
        let r = b.run_with(
            &["clone", url, "--name", VAULT, "--device", B_DEVICE],
            &FakePrompter::new(),
        );
        assert_eq!(r.code, ExitCode::Success, "clone: {}", r.stderr);
        TwoDevices {
            a,
            b,
            _origin: origin,
        }
    }
}

impl Env {
    /// `gage identity add` with a passphrase for the new identity.
    pub(crate) fn identity_add(&self, extra: &[&str]) -> CliResult {
        let mut args = vec!["identity", "add"];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn vault_path(&self) -> std::path::PathBuf {
        self.entry(VAULT).path
    }
}

/// The public key a command printed: the first `age1…` word.
fn printed_key(stdout: &str) -> String {
    stdout
        .split_whitespace()
        .find(|w| w.starts_with("age1"))
        .unwrap_or_else(|| panic!("no public key in {stdout:?}"))
        .to_string()
}

fn stray_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

pub(crate) fn identity_files(env: &Env) -> Vec<String> {
    let id = env.entry(VAULT).id.unwrap();
    match std::fs::read_dir(paths::identities_dir(&env.dirs, &id)) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".age"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

// --- identity add -----------------------------------------------------------

// The device
// name, method and key land in global config; the output carries the key
// and the next command to run elsewhere.
#[test]
fn identity_add_registers_a_device_and_prints_its_public_key() {
    let t = TwoDevices::new();
    let prompter = FakePrompter::new().with_passphrase(PASSPHRASE);

    let r =
        t.b.run_with(&["identity", "add", "--device", B_DEVICE], &prompter);

    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let key = printed_key(&r.stdout);
    let entry = t.b.entry(VAULT);
    assert_eq!(entry.pubkey.as_ref(), Some(&key));
    assert_eq!(entry.device, B_DEVICE);
    assert_eq!(entry.method, "passphrase");
    assert_eq!(identity_files(&t.b), [format!("{B_DEVICE}.age")]);
    assert!(
        r.stdout
            .contains(&format!("gage recipient add {key} --device {B_DEVICE}")),
        "{}",
        r.stdout
    );
    let requests = prompter.unlock_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].purpose, Purpose::Create);
    assert_eq!(requests[0].device, B_DEVICE);
}

#[test]
fn identity_add_device_flag_overrides_the_hostname_default() {
    let t = TwoDevices::new();

    t.b.ok(&["identity", "add"]);
    assert_eq!(
        t.b.entry(VAULT).device,
        "test-host",
        "the normalized hostname"
    );
    assert_eq!(identity_files(&t.b), ["test-host.age"]);

    t.b.ok(&["identity", "add", "--device", "phone"]);
    assert_eq!(t.b.entry(VAULT).device, "phone");
    assert_eq!(identity_files(&t.b).len(), 2);
}

// The method is
// this device's, recorded in global config and never in the vault
// (ADR-0011): the vault's working tree and history don't move.
#[test]
fn identity_add_method_defaults_to_the_vaults_and_stays_local() {
    let t = TwoDevices::new();
    let path = t.b.vault_path();
    let before = (
        std::fs::read_to_string(config_file(&path)).unwrap(),
        std::fs::read_to_string(path.join(".age-recipients")).unwrap(),
        gitrepo::commit_count(&path).unwrap(),
    );

    t.b.ok(&["identity", "add", "--device", B_DEVICE]);

    assert_eq!(
        t.b.entry(VAULT).method,
        t.b.vault_config(VAULT).method.default
    );
    let after = (
        std::fs::read_to_string(config_file(&path)).unwrap(),
        std::fs::read_to_string(path.join(".age-recipients")).unwrap(),
        gitrepo::commit_count(&path).unwrap(),
    );
    assert_eq!(after, before, "identity add wrote to the vault");
    assert!(gitrepo::is_clean(&path).unwrap());

    // An explicit, accepted method is the same; an unknown one is refused
    // before anything is asked or written.
    let t2 = TwoDevices::new();
    let prompter = FakePrompter::new();
    let r = t2.b.run_with(
        &["identity", "add", "--method", "carrier-pigeon"],
        &prompter,
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert_eq!(
        r.stderr,
        "gage: unknown method \"carrier-pigeon\"; accepted: passphrase\n"
    );
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t2.b).is_empty());
    t2.b.ok(&["identity", "add", "--method", "passphrase"]);
}

// B's hostname normalizes to the name device A already holds as a
// recipient; the refusal names the flag that fixes it, and nothing is
// asked or written.
#[test]
fn identity_add_rejects_a_hostname_collision_without_anyone_typing_the_name() {
    let mut t = TwoDevices::new();
    t.b.hostname = format!("{}.local", A_DEVICE);
    let prompter = FakePrompter::new();

    let r = t.b.run_with(&["identity", "add"], &prompter);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("--device"), "{}", r.stderr);
    assert!(r.stderr.contains(A_DEVICE), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t.b).is_empty());
    assert!(t.b.entry(VAULT).pubkey.is_none());
}

#[test]
fn identity_add_rejects_a_name_the_vault_already_lists() {
    let t = TwoDevices::new();
    let prompter = FakePrompter::new();

    let r =
        t.b.run_with(&["identity", "add", "--device", A_DEVICE], &prompter);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("already listed"), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t.b).is_empty());
    assert!(t.b.entry(VAULT).pubkey.is_none());
}

#[test]
fn identity_add_rejects_an_invalid_device_name_as_usage() {
    let t = TwoDevices::new();
    for bad in ["../escape", "Laptop", "has space"] {
        let r =
            t.b.run_with(&["identity", "add", "--device", bad], &FakePrompter::new());
        assert_eq!(r.code, ExitCode::Usage, "{bad}: {}", r.stderr);
    }
    assert!(identity_files(&t.b).is_empty());
}

// --- identity list ----------------------------------------------------------

#[test]
fn identity_list_reports_this_devices_identities() {
    let t = TwoDevices::new();

    let r = t.a.ok(&["identity", "list"]);
    assert_eq!(listed(&r.stdout), [(true, A_DEVICE.to_string())]);

    // Adding another registers it as this device's current one.
    t.a.ok(&["identity", "add", "--device", "desktop"]);
    let r = t.a.ok(&["identity", "list"]);
    assert_eq!(
        listed(&r.stdout),
        [(true, "desktop".to_string()), (false, A_DEVICE.to_string())]
    );
}

/// `(is current, device)` for each line of an `identity list`, in order.
fn listed(stdout: &str) -> Vec<(bool, String)> {
    stdout
        .lines()
        .map(|l| {
            (
                l.starts_with('*'),
                l[1..].split_whitespace().next().unwrap().to_string(),
            )
        })
        .collect()
}

// Cloned, not yet added.
#[test]
fn identity_list_on_a_device_holding_none_says_so() {
    let t = TwoDevices::new();

    let r = t.b.ok(&["identity", "list"]);

    assert!(r.stdout.contains("no identit"), "{}", r.stdout);
    assert!(r.stdout.contains(VAULT), "{}", r.stdout);
}

// --- recipient list / verify -------------------------------------------------

#[test]
fn recipient_list_prints_every_device_and_key_as_the_config_lists_them() {
    let t = TwoDevices::new();
    let prompter = FakePrompter::new();

    let r = t.b.run_with(&["recipient", "list"], &prompter);

    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let listed = t.b.vault_config(VAULT).recipients;
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), listed.len(), "{}", r.stdout);
    for (line, rec) in lines.iter().zip(&listed) {
        assert_eq!(
            line.split_whitespace().collect::<Vec<_>>(),
            [rec.device.as_str(), rec.pubkey.as_str()]
        );
    }
    assert!(
        prompter.unlock_requests().is_empty(),
        "a read needs no unlock"
    );
}

// In sync: 0. Diverged: 1
// (the taxonomy's Conflict), with both directions' differences named.
#[test]
fn recipient_verify_exit_codes_and_differences() {
    let t = TwoDevices::new();
    let r = t.a.ok(&["recipient", "verify"]);
    assert!(r.stdout.contains("in sync"), "{}", r.stdout);

    let vault = t.a.vault_path();
    let own = t.a.vault_config(VAULT).recipients[0].pubkey.clone();
    let stray = stray_key();
    recipient::save(&vault.join(".age-recipients"), &[own, stray.clone()]).unwrap();

    let r = t.a.run(&["recipient", "verify"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(&stray), "{}", r.stderr);
    assert!(r.stderr.contains(".age-recipients"), "{}", r.stderr);
}

// The other
// direction: config lists a key the file lacks. The device label is named
// too, since that is what a human recognizes.
#[test]
fn recipient_verify_names_a_key_missing_from_the_recipients_file() {
    let t = TwoDevices::new();
    let vault = t.a.vault_path();
    let ghost = stray_key();
    let mut config = vaultconfig::load(&config_file(&vault)).unwrap();
    config.recipients.push(vaultconfig::Recipient {
        device: "ghost".into(),
        pubkey: ghost.clone(),
    });
    vaultconfig::save(&config_file(&vault), &config).unwrap();

    let r = t.a.run(&["recipient", "verify"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(&ghost), "{}", r.stderr);
    assert!(r.stderr.contains("ghost"), "{}", r.stderr);
    assert!(r.stderr.contains("config.toml"), "{}", r.stderr);
}

// Device B holds
// no identity; the prompter records any question, and none is asked.
#[test]
fn recipient_verify_runs_with_no_identity_and_no_prompter() {
    let t = TwoDevices::new();
    assert!(identity_files(&t.b).is_empty());
    let prompter = FakePrompter::new();

    let r = t.b.run_with(&["recipient", "verify"], &prompter);

    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(prompter.value_prompts().is_empty());
    assert!(prompter.confirm_prompts().is_empty());
}

// --- inbound rows -------------------------------------------------------------

// The init half is in init_identity_tests; this adds the identity add and clone halves.
#[test]
fn init_and_identity_add_record_the_pubkey_and_clone_does_not() {
    let t = TwoDevices::new();
    let init_key = t.a.vault_config(VAULT).recipients[0].pubkey.clone();
    assert_eq!(
        t.a.entry(VAULT).pubkey.as_ref(),
        Some(&init_key),
        "init records it"
    );
    assert!(t.b.entry(VAULT).pubkey.is_none(), "clone does not");

    let r = t.b.identity_add(&["--device", B_DEVICE]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

    assert_eq!(t.b.entry(VAULT).pubkey, Some(printed_key(&r.stdout)));
    assert_ne!(t.b.entry(VAULT).pubkey, Some(init_key));
}

// The identity list half of "a vault without an id fails loudly on every derived path":
// a registration with
// an empty id fails before any path is built from it.
#[test]
fn a_registration_without_an_id_fails_identity_list_before_any_path_is_built() {
    let t = TwoDevices::new();
    let mut global = t.a.global();
    global.vaults.get_mut(VAULT).unwrap().id = None;
    t.a.save_global(&global);
    let before = t.a.tree();

    let r = t.a.run(&["identity", "list"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("points at a different vault"),
        "{}",
        r.stderr
    );
    assert_eq!(t.a.tree(), before);
}
