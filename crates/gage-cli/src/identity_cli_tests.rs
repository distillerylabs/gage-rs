//! `gage identity add/list` and `gage recipient list/verify` through the
//! real CLI, in-process: Go's `identity_cli_test.go` and the read-side rows
//! of `recipient_cli_test.go`. Each test names its Go row.
//!
//! [`TwoDevices`] is the CLI's two-machine fixture: two [`Env`]s, each with
//! its own roots and hostname, over one bare origin. Device A ran
//! `gage init --remote`; device B ran `gage clone` and holds no identity.
//! M9b and M10 build on it: B's `identity add` prints the key A's
//! `recipient add` takes, and a recipient change pushed by A is what B
//! pulls.

use gage::exitcode::Code;
use gage::gittest::{self, BareRemote};
use gage::prompter::{FakePrompter, Purpose};
use gage::{gitrepo, paths, recipients, vaultconfig};

use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, config_file};
use crate::registry::{GROUP_IDENTITY, GROUP_RECIPIENT, registry};

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
        a.ok(&["init", VAULT, "--device", A_DEVICE, "--remote", url]);
        let r = b.run_with(
            &["clone", url, "--name", VAULT, "--device", B_DEVICE],
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Success, "clone: {}", r.stderr);
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

fn identity_files(env: &Env) -> Vec<String> {
    let id = env.entry(VAULT).id;
    match std::fs::read_dir(paths::identities_dir(&env.dirs, &id).unwrap()) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".age"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

// --- identity add -----------------------------------------------------------

// Go: TestIdentityAddRegistersADeviceAndPrintsItsPublicKey. The device
// name, method and key land in global config; the output carries the key
// and the next command to run elsewhere.
#[test]
fn identity_add_registers_a_device_and_prints_its_public_key() {
    let t = TwoDevices::new();
    let prompter = FakePrompter::new().with_passphrase(PASSPHRASE);

    let r =
        t.b.run_with(&["identity", "add", "--device", B_DEVICE], &prompter);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let key = printed_key(&r.stdout);
    let entry = t.b.entry(VAULT);
    assert_eq!(entry.pubkey, key);
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

// Go: TestIdentityAddDeviceFlagOverridesTheHostnameDefault.
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

// Go: TestIdentityAddMethodDefaultsToTheVaultsAndStaysLocal. The method is
// this device's, recorded in global config and never in the vault
// (Q-METHOD-SCOPE): the vault's working tree and history don't move.
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
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("carrier-pigeon"), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t2.b).is_empty());
    t2.b.ok(&["identity", "add", "--method", "passphrase"]);
}

// Go: TestIdentityAddRejectsAHostnameCollisionWithoutAnyoneTypingTheName.
// B's hostname normalizes to the name device A already holds as a
// recipient; the refusal names the flag that fixes it, and nothing is
// asked or written.
#[test]
fn identity_add_rejects_a_hostname_collision_without_anyone_typing_the_name() {
    let mut t = TwoDevices::new();
    t.b.hostname = format!("{}.local", A_DEVICE);
    let prompter = FakePrompter::new();

    let r = t.b.run_with(&["identity", "add"], &prompter);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("--device"), "{}", r.stderr);
    assert!(r.stderr.contains(A_DEVICE), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t.b).is_empty());
    assert!(t.b.entry(VAULT).pubkey.is_empty());
}

// Go: TestIdentityAddRejectsANameTheVaultAlreadyLists.
#[test]
fn identity_add_rejects_a_name_the_vault_already_lists() {
    let t = TwoDevices::new();
    let prompter = FakePrompter::new();

    let r =
        t.b.run_with(&["identity", "add", "--device", A_DEVICE], &prompter);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("already listed"), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(identity_files(&t.b).is_empty());
    assert!(t.b.entry(VAULT).pubkey.is_empty());
}

#[test]
fn identity_add_rejects_an_invalid_device_name_as_usage() {
    let t = TwoDevices::new();
    for bad in ["../escape", "Laptop", "has space"] {
        let r =
            t.b.run_with(&["identity", "add", "--device", bad], &FakePrompter::new());
        assert_eq!(r.code, Code::Usage, "{bad}: {}", r.stderr);
    }
    assert!(identity_files(&t.b).is_empty());
}

// --- identity list ----------------------------------------------------------

// Go: TestIdentityListReportsThisDevicesIdentities.
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

// Go: TestIdentityListOnADeviceHoldingNoneSaysSo. Cloned, not yet added.
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

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
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

// Go: TestRecipientVerifyExitCodesAndDifferences. In sync: 0. Diverged: 1
// (the taxonomy's Conflict), with both directions' differences named.
#[test]
fn recipient_verify_exit_codes_and_differences() {
    let t = TwoDevices::new();
    let r = t.a.ok(&["recipient", "verify"]);
    assert!(r.stdout.contains("in sync"), "{}", r.stdout);

    let vault = t.a.vault_path();
    let own = t.a.vault_config(VAULT).recipients[0].pubkey.clone();
    let stray = stray_key();
    recipients::write(&vault.join(".age-recipients"), &[own, stray.clone()]).unwrap();

    let r = t.a.run(&["recipient", "verify"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(&stray), "{}", r.stderr);
    assert!(r.stderr.contains(".age-recipients"), "{}", r.stderr);
}

// Go: TestRecipientVerifyNamesAKeyMissingFromTheRecipientsFile. The other
// direction: config lists a key the file lacks. The device label is named
// too, since that is what a human recognizes.
#[test]
fn recipient_verify_names_a_key_missing_from_the_recipients_file() {
    let t = TwoDevices::new();
    let vault = t.a.vault_path();
    let ghost = stray_key();
    let mut config = vaultconfig::read(&config_file(&vault)).unwrap();
    config.recipients.push(vaultconfig::Recipient {
        device: "ghost".into(),
        pubkey: ghost.clone(),
    });
    vaultconfig::write(&config_file(&vault), &config).unwrap();

    let r = t.a.run(&["recipient", "verify"]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains(&ghost), "{}", r.stderr);
    assert!(r.stderr.contains("ghost"), "{}", r.stderr);
    assert!(r.stderr.contains("config.toml"), "{}", r.stderr);
}

// Go: TestRecipientVerifyRunsWithNoIdentityAndNoPrompter. Device B holds
// no identity; the prompter records any question, and none is asked.
#[test]
fn recipient_verify_runs_with_no_identity_and_no_prompter() {
    let t = TwoDevices::new();
    assert!(identity_files(&t.b).is_empty());
    let prompter = FakePrompter::new();

    let r = t.b.run_with(&["recipient", "verify"], &prompter);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(prompter.value_prompts().is_empty());
    assert!(prompter.confirm_prompts().is_empty());
}

// --- inbound rows -------------------------------------------------------------

// Go: vaultid_keying TestInitAndIdentityAddRecordThePubkeyAndCloneDoesNot.
// The init half was P2 · M2's; this adds the identity add and clone halves.
#[test]
fn init_and_identity_add_record_the_pubkey_and_clone_does_not() {
    let t = TwoDevices::new();
    let init_key = t.a.vault_config(VAULT).recipients[0].pubkey.clone();
    assert_eq!(t.a.entry(VAULT).pubkey, init_key, "init records it");
    assert!(t.b.entry(VAULT).pubkey.is_empty(), "clone does not");

    let r = t.b.identity_add(&["--device", B_DEVICE]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    assert_eq!(t.b.entry(VAULT).pubkey, printed_key(&r.stdout));
    assert_ne!(t.b.entry(VAULT).pubkey, init_key);
}

// The identity list half of Go's
// TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath: a registration with
// an empty id fails before any path is built from it.
#[test]
fn a_registration_without_an_id_fails_identity_list_before_any_path_is_built() {
    let t = TwoDevices::new();
    let mut global = t.a.global();
    global.vaults.get_mut(VAULT).unwrap().id.clear();
    t.a.save_global(&global);
    let before = t.a.tree();

    let r = t.a.run(&["identity", "list"]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("points at a different vault"),
        "{}",
        r.stderr
    );
    assert_eq!(t.a.tree(), before);
}

// --- parity -------------------------------------------------------------------

// The help-fixture check for P5 · M9a and M10: each command's description,
// long help and flags as Go's help shows them, `recipient verify --repair`
// included. The root `--yes` is checked at the end: accepted everywhere,
// and listed nowhere, as Go's help lists no inherited flag.
#[test]
fn p5_identity_commands_have_their_go_fixture_flags_and_descriptions() {
    let env = Env::new();
    for (name, fixture, prose, flags) in [
        (
            "identity add",
            include_str!("../../../tests/fixtures/go/help/identity_add.txt"),
            true,
            &["--device", "--method", "-u", "--use"][..],
        ),
        (
            "identity list",
            include_str!("../../../tests/fixtures/go/help/identity_list.txt"),
            true,
            &["-u", "--use"][..],
        ),
        (
            "recipient add",
            include_str!("../../../tests/fixtures/go/help/recipient_add.txt"),
            true,
            &["--device", "-u", "--use"][..],
        ),
        (
            "recipient remove",
            include_str!("../../../tests/fixtures/go/help/recipient_remove.txt"),
            true,
            &["--reencrypt", "-u", "--use"][..],
        ),
        (
            "recipient list",
            include_str!("../../../tests/fixtures/go/help/recipient_list.txt"),
            true,
            &["-u", "--use"][..],
        ),
        (
            "recipient verify",
            include_str!("../../../tests/fixtures/go/help/recipient_verify.txt"),
            true,
            &["--repair", "-u", "--use"][..],
        ),
    ] {
        let args: Vec<&str> = name.split(' ').chain(["--help"]).collect();
        let help = env.ok(&args).stdout;
        let first = fixture.lines().next().unwrap().trim_end_matches('.');
        assert!(help.starts_with(first), "{name}:\n{help}");
        if prose {
            for line in fixture
                .lines()
                .skip(1)
                .take_while(|l| !l.starts_with("Usage:"))
            {
                assert!(help.contains(line.trim()), "{name}: {line:?}\n{help}");
            }
        }
        for line in fixture.lines().filter(|l| l.trim_start().starts_with('-')) {
            let text = line.split("   ").last().unwrap().trim();
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
        for flag in flags {
            assert!(help.contains(flag), "{name}: {flag}\n{help}");
        }
    }
    for help in [
        env.ok(&["--help"]).stdout,
        env.ok(&["insert", "--help"]).stdout,
        env.ok(&["recipient", "remove", "--help"]).stdout,
    ] {
        assert!(!help.contains("--yes"), "{help}");
    }
    let r = env.run(&["--yes", "vault", "list"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let r = env.run(&["vault", "list", "--yes"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        !env.ok(&["recipient", "add", "--help"])
            .stdout
            .contains("--reencrypt")
    );

    // The parents list only the children that exist: enroll and the
    // enrollment children of `recipient` are later milestones'. `recipient
    // add` takes no --reencrypt (A19); `remove` requires it.
    let identity = env.ok(&["identity"]).stdout;
    assert!(identity.starts_with("Manage this device's identities for a vault"));
    for (child, short) in [
        (
            "add",
            "Register this device's identity and print its public key",
        ),
        ("list", "List the keys this machine holds for a vault"),
    ] {
        assert_child(&identity, child, short);
    }
    let recipient = env.ok(&["recipient"]).stdout;
    assert!(recipient.starts_with("Manage who can read this vault"));
    for (child, short) in [
        (
            "add",
            "Authorize a public key and re-encrypt the vault to include it",
        ),
        (
            "remove",
            "Revoke a recipient and re-encrypt the vault without its key",
        ),
        ("list", "List the keys this vault is encrypted to"),
        (
            "verify",
            "Check .age-recipients and config.toml still agree",
        ),
    ] {
        assert_child(&recipient, child, short);
    }

    let root = env.ok(&["help"]).stdout;
    for short in [
        "identity add",
        "identity list",
        "recipient add",
        "recipient remove",
        "recipient list",
        "recipient verify",
    ] {
        assert!(root.contains(short), "{short}\n{root}");
    }
    let reg = registry();
    for (name, group) in [
        ("identity add", GROUP_IDENTITY),
        ("identity list", GROUP_IDENTITY),
        ("recipient add", GROUP_RECIPIENT),
        ("recipient remove", GROUP_RECIPIENT),
        ("recipient list", GROUP_RECIPIENT),
        ("recipient verify", GROUP_RECIPIENT),
    ] {
        let c = reg.find(name).unwrap();
        assert_eq!(c.group, group, "{name}");
        assert!(
            c.availability.session_visible() && c.availability.one_shot_visible(),
            "{name}"
        );
    }
}

/// A parent's help lists `child` with its description on the same line.
/// Column widths are clap's own, not Cobra's (Q-HELP-RENDER).
fn assert_child(help: &str, child: &str, short: &str) {
    assert!(
        help.lines().any(|l| {
            let mut words = l.split_whitespace();
            words.next() == Some(child) && l.trim_start()[child.len()..].trim() == short
        }),
        "{child}: {short}\n{help}"
    );
}
