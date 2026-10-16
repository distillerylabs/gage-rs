//! Vault lifecycle through the real CLI, in-process: `init`, `vault
//! list/info/remove/set-default` and `git set-remote`, each run against a
//! fresh temp directory standing in for gage's config, data and state
//! roots, and an injected hostname. The Go rows each test ports are named
//! above it; where only part of a Go test can run before identities
//! exist, the comment says which part and where the rest went.

use std::path::{Path, PathBuf};

use gage::config::{self, Global, VaultEntry};
use gage::entry::CandidateList;
use gage::exitcode::Code;
use gage::prompter::{FakePrompter, PromptError, Prompter, UnlockRequest, UnlockResponse};
use gage::syncresolve::{EntryConflict, Resolution};
use gage::vaultconfig;
use gage::xdgpaths::Dirs;
use gage::{gitrepo, paths, recipients};

use crate::commands::resolve_device_name;
use crate::error::ERROR_PREFIX;
use crate::{BuildInfo, Io};

const RECIPIENT_1: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";
/// What every [`Env::run`] answers a passphrase request with.
pub(crate) const PASSPHRASE: &str = "correct horse battery staple";
const RECIPIENT_2: &str = "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0";

/// The hostname every [`Env`] reports unless a test sets another.
const HOSTNAME: &str = "Test-Host.local";

pub(crate) struct CliResult {
    pub(crate) code: Code,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// A byte buffer two writers share: the command's stderr, and the
/// prompter's warnings, interleaved in the order they happened.
#[derive(Clone, Default)]
struct SharedBuf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The test's prompter, with every warning also written to stderr, as the
/// real `TerminalPrompter` writes it. Without this, a library warning is
/// invisible to every CLI test that reads stderr, and a command could stop
/// emitting one with nothing going red (Go's M9 review fix). Everything
/// else is forwarded untouched, so the inner fake still scripts and
/// records.
struct EchoingPrompter<'p> {
    inner: &'p dyn Prompter,
    stderr: SharedBuf,
}

impl Prompter for EchoingPrompter<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.inner.unlock(req)
    }

    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm(prompt)
    }

    fn value(&self, title: &str) -> Result<zeroize::Zeroizing<String>, PromptError> {
        self.inner.value(title)
    }

    fn choose(&self, list: &CandidateList) -> Result<uuid::Uuid, PromptError> {
        self.inner.choose(list)
    }

    fn resolve_conflict(&self, conflict: &EntryConflict) -> Result<Resolution, PromptError> {
        self.inner.resolve_conflict(conflict)
    }

    fn confirm_recipient_change(
        &self,
        warning: &gage::trustcache::RecipientChangeWarning,
    ) -> Result<bool, PromptError> {
        self.inner.confirm_recipient_change(warning)
    }

    fn warn(&self, msg: &str) {
        use std::io::Write;
        let _ = writeln!(self.stderr.clone(), "{msg}");
        self.inner.warn(msg);
    }
}

/// One isolated machine: its own config, data and state roots (Go's
/// `isolateXDG`) and hostname.
pub(crate) struct Env {
    pub(crate) root: tempfile::TempDir,
    pub(crate) dirs: Dirs,
    pub(crate) hostname: String,
    /// What the commands see as `$EDITOR`; unset unless a test sets one.
    pub(crate) editor: std::sync::Mutex<Option<String>>,
}

impl Env {
    pub(crate) fn new() -> Env {
        gage::testsupport::fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config").join("gage"),
            data: root.path().join("data").join("gage"),
            state: root.path().join("state").join("gage"),
        };
        Env {
            root,
            dirs,
            hostname: HOSTNAME.into(),
            editor: Default::default(),
        }
    }

    /// Runs `args` with a prompter that answers one passphrase request
    /// (init's) with [`PASSPHRASE`]; anything else it is asked fails the
    /// test.
    pub(crate) fn run(&self, args: &[&str]) -> CliResult {
        self.run_with(args, &FakePrompter::new().with_passphrase(PASSPHRASE))
    }

    pub(crate) fn run_with(&self, args: &[&str], prompter: &dyn Prompter) -> CliResult {
        self.run_stdin(args, prompter, b"")
    }

    /// Like [`Env::run_with`], with `stdin` as the command's standard input.
    pub(crate) fn run_stdin(
        &self,
        args: &[&str],
        prompter: &dyn Prompter,
        mut stdin: &[u8],
    ) -> CliResult {
        self.run_reader(args, prompter, &mut stdin)
    }

    /// Like [`Env::run_stdin`], reading stdin from any reader (one that
    /// fails the test if it is read, say).
    pub(crate) fn run_reader(
        &self,
        args: &[&str],
        prompter: &dyn Prompter,
        stdin: &mut dyn std::io::Read,
    ) -> CliResult {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut stdout = Vec::new();
        let stderr = SharedBuf::default();
        let prompter = EchoingPrompter {
            inner: prompter,
            stderr: stderr.clone(),
        };
        let dirs = self.dirs.clone();
        let hostname = self.hostname.clone();
        let mut io = Io {
            stdin,
            stdout: &mut stdout,
            stderr: &mut stderr.clone(),
            stdin_is_terminal: false,
            line_editor: false,
            build: BuildInfo::stamped(),
            dirs: &move || Ok(dirs.clone()),
            hostname: &move || Ok(hostname.clone()),
            editor: self.editor.lock().unwrap().clone(),
            now: &std::time::Instant::now,
        };
        let code = crate::run(&args, &mut io, &prompter);
        CliResult {
            code,
            stdout: String::from_utf8(stdout).unwrap(),
            stderr: String::from_utf8(stderr.0.borrow().clone()).unwrap(),
        }
    }

    /// Runs `args` and requires success.
    pub(crate) fn ok(&self, args: &[&str]) -> CliResult {
        let r = self.run(args);
        assert_eq!(r.code, Code::Success, "{args:?} failed:\n{}", r.stderr);
        r
    }

    /// `gage init <name> <extra...>`; returns the registered path.
    pub(crate) fn init(&self, name: &str, extra: &[&str]) -> PathBuf {
        let mut args = vec!["init", name];
        args.extend_from_slice(extra);
        self.ok(&args);
        self.entry(name).path
    }

    pub(crate) fn global(&self) -> Global {
        config::load(&paths::global_config_file(&self.dirs)).unwrap()
    }

    pub(crate) fn save_global(&self, global: &Global) {
        config::save(&paths::global_config_file(&self.dirs), global).unwrap();
    }

    pub(crate) fn entry(&self, name: &str) -> VaultEntry {
        self.global()
            .vaults
            .get(name)
            .unwrap_or_else(|| panic!("{name:?} is not registered"))
            .clone()
    }

    /// The registered vault's committed config: the file init wrote, not
    /// the global registry.
    pub(crate) fn vault_config(&self, name: &str) -> vaultconfig::File {
        vaultconfig::read(&config_file(&self.entry(name).path)).unwrap()
    }

    pub(crate) fn default_vault_path(&self, name: &str) -> PathBuf {
        paths::vaults_dir(&self.dirs).join(name)
    }

    /// Every path under the data and state roots, so a test can assert a
    /// command created, moved or removed nothing there.
    pub(crate) fn tree(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for root in [&self.dirs.data, &self.dirs.state] {
            walk(root, &mut found);
        }
        found.sort();
        found
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        out.push(path.clone());
        if path.is_dir() {
            walk(&path, out);
        }
    }
}

pub(crate) fn config_file(vault: &Path) -> PathBuf {
    vault.join(".gage").join("config.toml")
}

/// Replaces the first line of a vault's committed config starting with
/// `prefix`, standing in for any git-writer (Q-DEVICE-NAME).
fn rewrite_config_line(vault: &Path, prefix: &str, replacement: &str) {
    let path = config_file(vault);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut done = false;
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if !done && l.trim_start().starts_with(prefix) {
                done = true;
                replacement.to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    assert!(done, "no line starting with {prefix:?} in:\n{text}");
    std::fs::write(&path, lines.join("\n")).unwrap();
}

// --- device name ------------------------------------------------------------

// Go: TestResolveDeviceNameExplicitOverridesHostname
#[test]
fn resolve_device_name_explicit_overrides_hostname() {
    assert_eq!(
        resolve_device_name(Some("custom-device"), "andrews-macbook-pro.local").unwrap(),
        "custom-device"
    );
}

// Go: TestResolveDeviceNameFallsBackToNormalizedHostname
#[test]
fn resolve_device_name_falls_back_to_normalized_hostname() {
    assert_eq!(
        resolve_device_name(None, "Andrews-MacBook-Pro.local").unwrap(),
        "andrews-macbook-pro"
    );
}

// Go: TestResolveDeviceNameExplicitFailingAllowlistIsUsageError. Refused,
// never normalized into some other device's name.
#[test]
fn resolve_device_name_explicit_failing_allowlist_is_usage_error() {
    for bad in ["../../etc/x", "Laptop-1", ""] {
        let err = resolve_device_name(Some(bad), "irrelevant").unwrap_err();
        assert_eq!(err.code(), Code::Usage, "{bad:?}");
    }
}

// Go: TestResolveDeviceNameHostnameNormalizesToNothingPromptsRatherThanInventing.
// One-shot mode has no free-text prompt, so "asking" is a usage error that
// names the flag, never an invented name.
#[test]
fn resolve_device_name_hostname_normalizes_to_nothing_prompts_rather_than_inventing() {
    for hostname in ["", "...", "!!!", "---"] {
        let err = resolve_device_name(None, hostname).unwrap_err();
        assert_eq!(err.code(), Code::Usage, "{hostname:?}");
        assert!(err.to_string().contains("--device"), "{err}");
    }
}

// The same through `init`: nothing is created or registered.
#[test]
fn init_with_an_unusable_hostname_fails_before_creating_files() {
    let mut env = Env::new();
    env.hostname = "!!!".into();

    let r = env.run_with(&["init", "personal"], &FakePrompter::new());

    assert_eq!(r.code, Code::Usage);
    assert!(r.stderr.contains("--device"), "{}", r.stderr);
    assert!(!env.default_vault_path("personal").exists());
    assert!(env.global().vaults.is_empty());
}

#[test]
fn init_defaults_the_device_to_the_normalized_hostname() {
    let env = Env::new();
    env.init("personal", &[]);
    assert_eq!(env.entry("personal").device, "test-host");
    assert_eq!(
        env.vault_config("personal").recipients[0].device,
        "test-host"
    );
}

// --- init -------------------------------------------------------------------

// Go: TestInitCreatesFullSkeletonAndRegisters
#[test]
fn init_creates_full_skeleton_and_registers() {
    let env = Env::new();

    let r = env.ok(&["init", "personal"]);

    let entry = env.entry("personal");
    for rel in [
        ".gage/config.toml",
        ".age-recipients",
        "entries",
        ".gitignore",
        ".gitattributes",
        ".git",
    ] {
        assert!(entry.path.join(rel).exists(), "{rel} missing");
    }
    assert_eq!(gitrepo::commit_count(&entry.path).unwrap(), 1);
    assert!(
        r.stdout.contains("initialized vault \"personal\""),
        "{}",
        r.stdout
    );
    assert_eq!(r.stderr, "");
}

// Go: TestInitWithoutDirUsesGageDataVaultsDir. Both halves: created there,
// and registered under exactly that path.
#[test]
fn init_without_dir_uses_gage_data_vaults_dir() {
    let env = Env::new();
    env.init("personal", &[]);
    let want = env.dirs.data.join("vaults").join("personal");
    assert!(config_file(&want).exists());
    assert_eq!(env.entry("personal").path, want);
}

// Go: TestInitWithDirUsesGivenPath
#[test]
fn init_with_dir_uses_given_path() {
    let env = Env::new();
    let dir = env.root.path().join("somewhere-else");

    env.init("personal", &["--dir", dir.to_str().unwrap()]);

    assert_eq!(env.entry("personal").path, dir);
    assert!(dir.join(".git").exists());
    assert!(!env.default_vault_path("personal").exists());
}

// Go: TestInitIntoNonEmptyNonGageDirectoryFailsWithoutTouchingFiles
#[test]
fn init_into_non_empty_non_gage_directory_fails_without_touching_files() {
    let env = Env::new();
    let dir = env.root.path().join("mine");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();

    let r = env.run(&["init", "personal", "--dir", dir.to_str().unwrap()]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["keep.txt"]);
    assert_eq!(
        std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(env.global().vaults.is_empty());
}

// Go: TestInitIntoAlreadyRegisteredNameFailsAndLeavesExistingVaultUntouched
#[test]
fn init_into_already_registered_name_fails_and_leaves_existing_vault_untouched() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let before = env.global();
    let keys_before = recipients::read(&path.join(".age-recipients")).unwrap();

    let r = env.run_with(
        &["init", "personal", "--recipient", RECIPIENT_2],
        &FakePrompter::new(),
    );

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("already a registered vault"),
        "{}",
        r.stderr
    );
    assert_eq!(env.global(), before);
    let keys = recipients::read(&path.join(".age-recipients")).unwrap();
    assert_eq!(keys, keys_before);
}

// Go: TestInitDefaultsLandInVaultConfigAndGlobalConfig
#[test]
fn init_defaults_land_in_vault_config_and_global_config() {
    let env = Env::new();
    env.init("personal", &[]);

    let file = env.vault_config("personal");
    assert_eq!(file.vault.vault_type, "git");
    assert_eq!(file.method.default, "passphrase");
    assert_eq!(
        file.vault.format_version,
        vaultconfig::CURRENT_FORMAT_VERSION
    );
    assert_eq!(file.vault.name, "personal");

    let entry = env.entry("personal");
    assert_eq!(entry.vault_type, "git");
    assert_eq!(entry.method, "passphrase");
}

// Go: TestInitExplicitTypeAndMethodEquivalentToDefault. Equivalent, not
// merely accepted: the files must match bar the name, id, date and this
// device's key, which every vault generates afresh.
#[test]
fn init_explicit_type_and_method_equivalent_to_default() {
    let env = Env::new();
    env.init("implicit", &[]);
    env.init("explicit", &["--type", "git", "--method", "passphrase"]);

    let (mut implicit, mut explicit) = (env.vault_config("implicit"), env.vault_config("explicit"));
    assert_ne!(
        implicit.vault.id, explicit.vault.id,
        "every vault gets its own id"
    );
    for file in [&mut implicit, &mut explicit] {
        assert!(vaultconfig::valid_id(&file.vault.id));
        file.vault.name.clear();
        file.vault.id.clear();
        file.vault.created.clear();
        for r in &mut file.recipients {
            r.pubkey.clear();
        }
    }
    assert_eq!(implicit, explicit);

    let (a, b) = (env.entry("implicit"), env.entry("explicit"));
    assert_eq!((a.vault_type, a.method), (b.vault_type, b.method));
}

// Go: TestInitUnknownTypeFailsWithUsageAndListsGit
#[test]
fn init_unknown_type_fails_with_usage_and_lists_git() {
    let env = Env::new();
    let r = env.run_with(&["init", "personal", "--type", "s3"], &FakePrompter::new());
    assert_eq!(r.code, Code::Usage);
    assert!(r.stderr.contains("git"), "{}", r.stderr);
    assert!(!env.default_vault_path("personal").exists());
}

// Go: TestInitUnknownMethodFailsWithUsage
#[test]
fn init_unknown_method_fails_with_usage() {
    let env = Env::new();
    for method in [
        "ssh",
        "yubikey",
        "age-key",
        "secure-enclave",
        "plugin:foo",
        "bogus",
    ] {
        let name = format!("personal-{}", method.replace(':', "-"));

        let r = env.run_with(&["init", &name, "--method", method], &FakePrompter::new());

        assert_eq!(r.code, Code::Usage, "{method}");
        assert!(r.stderr.contains("passphrase"), "{}", r.stderr);
        assert!(!env.default_vault_path(&name).exists(), "{method}");
        assert!(!env.global().vaults.contains_key(&name), "{method}");
    }
}

// Go: TestInitMalformedRecipientFailsBeforeCreatingFiles
#[test]
fn init_malformed_recipient_fails_before_creating_files() {
    let env = Env::new();
    for keys in [
        &["--recipient", "not-a-valid-key"][..],
        &[
            "--recipient",
            RECIPIENT_1,
            "--recipient",
            "AGE-SECRET-KEY-1QQQ",
        ],
    ] {
        let mut args = vec!["init", "personal"];
        args.extend_from_slice(keys);
        let r = env.run_with(&args, &FakePrompter::new());
        assert_eq!(r.code, Code::Usage, "{keys:?}");
        assert!(!env.default_vault_path("personal").exists());
        assert!(env.global().vaults.is_empty());
    }
}

// Go: TestInitRepeatedRecipientWritesAll. This device's key first, then
// every --recipient in the order given (repeats included), labelled by
// position.
#[test]
fn init_repeated_recipient_writes_all() {
    let env = Env::new();
    let path = env.init(
        "personal",
        &[
            "--recipient",
            RECIPIENT_1,
            "--recipient",
            RECIPIENT_2,
            "--recipient",
            RECIPIENT_1,
        ],
    );

    let keys = recipients::read(&path.join(".age-recipients")).unwrap();
    assert_eq!(keys[0], env.entry("personal").pubkey);
    assert_eq!(keys[1..], [RECIPIENT_1, RECIPIENT_2, RECIPIENT_1]);
    let labels: Vec<String> = env
        .vault_config("personal")
        .recipients
        .into_iter()
        .map(|r| r.device)
        .collect();
    assert_eq!(
        labels,
        ["test-host", "recipient-2", "recipient-3", "recipient-4"]
    );
}

// Go: TestInitDeviceFlagOverridesHostnameDefault
#[test]
fn init_device_flag_overrides_hostname_default() {
    let env = Env::new();
    env.init("personal", &["--device", "custom-device"]);
    assert_eq!(env.entry("personal").device, "custom-device");
}

// Go: TestInitDeviceFlagFailingAllowlistRejectedBeforeCreatingFiles
#[test]
fn init_device_flag_failing_allowlist_rejected_before_creating_files() {
    let env = Env::new();
    let r = env.run_with(
        &["init", "personal", "--device", "../../../etc/x"],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage);
    assert!(env.tree().is_empty(), "{:?}", env.tree());
    assert!(env.global().vaults.is_empty());
}

// Go: TestInitRecordsThisDevicesDeviceAndMethod. Unlock (M2) resolves the
// identity file from the recorded device and dispatches on the recorded
// method, so both must be written.
#[test]
fn init_records_this_devices_device_and_method() {
    let env = Env::new();
    env.init("personal", &["--device", "laptop-1"]);
    let entry = env.entry("personal");
    assert_eq!(entry.device, "laptop-1");
    assert_eq!(entry.method, "passphrase");
}

// Go: TestInitDeviceFlagAgreesEverywhere: global config, [[recipients]]
// and the identity file's name.
#[test]
fn init_device_flag_agrees_everywhere() {
    let env = Env::new();
    env.init("personal", &["--device", "workstation-7"]);
    let entry = env.entry("personal");
    assert_eq!(entry.device, "workstation-7");
    assert_eq!(
        env.vault_config("personal").recipients[0].device,
        "workstation-7"
    );
    let identity = env
        .dirs
        .data
        .join("identities")
        .join(&entry.id)
        .join("workstation-7.age");
    assert!(
        identity.is_file(),
        "no identity file at {}",
        identity.display()
    );
}

// Go: TestReinitAfterVaultRemoveIsAnOrdinaryRetry: re-init under a
// removed name succeeds, mints a new id, and files a new identity under it,
// leaving the removed vault's leftover identity file untouched.
#[test]
fn reinit_after_vault_remove_is_an_ordinary_retry() {
    let env = Env::new();
    let path = env.init("vault-test-newinit", &["--device", "laptop-1"]);
    let old_id = env.entry("vault-test-newinit").id;
    let old_identity = paths::identity_file_path(&env.dirs, &old_id, "laptop-1").unwrap();
    let old_bytes = std::fs::read(&old_identity).unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    env.ok(&["vault", "remove", "vault-test-newinit"]);
    assert!(
        old_identity.is_file(),
        "vault remove took the identity file"
    );

    env.init("vault-test-newinit", &["--device", "laptop-1"]);

    let entry = env.entry("vault-test-newinit");
    assert_ne!(entry.id, old_id, "a new vault gets a new id");
    assert_eq!(env.vault_config("vault-test-newinit").vault.id, entry.id);
    let new_identity = paths::identity_file_path(&env.dirs, &entry.id, "laptop-1").unwrap();
    assert_ne!(new_identity, old_identity);
    assert!(new_identity.is_file(), "the re-init wrote no identity file");
    assert_eq!(
        std::fs::read(&old_identity).unwrap(),
        old_bytes,
        "the leftover was disturbed"
    );
}

// Go: TestInitWithNoRecipientGeneratesThisDevicesIdentity. Replaces M1's
// interim "init requires --recipient" (Q-INIT-INTERIM, closed by M2).
#[test]
fn init_with_no_recipient_generates_this_devices_identity() {
    let env = Env::new();
    env.ok(&["init", "personal"]);

    let entry = env.entry("personal");
    let file = env.vault_config("personal");
    assert_eq!(
        file.recipients.len(),
        1,
        "want exactly this device's own key"
    );
    let pubkey = &file.recipients[0].pubkey;
    gage::agekey::validate_recipient(pubkey).unwrap();
    assert_eq!(file.recipients[0].device, entry.device);
    // The same key, in both recipient-defining files.
    let keys = recipients::read(&entry.path.join(".age-recipients")).unwrap();
    assert_eq!(keys, std::slice::from_ref(pubkey));
}

// Go: TestInitMintsADistinctUUIDv4PerVault
#[test]
fn init_mints_a_distinct_uuid_v4_per_vault() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);

    let (first, second) = (
        env.vault_config("personal").vault.id,
        env.vault_config("work").vault.id,
    );

    assert_ne!(first, second);
    for id in [&first, &second] {
        assert!(vaultconfig::valid_id(id), "{id}");
        assert_eq!(&id[14..15], "4", "not a v4 UUID: {id}");
    }
    // Global config records the same id the vault commits.
    assert_eq!(env.entry("personal").id, first);
    assert_eq!(env.entry("work").id, second);
}

// Go: TestInitWritesFormatVersionTwo
#[test]
fn init_writes_format_version_two() {
    let env = Env::new();
    env.init("personal", &[]);
    assert_eq!(env.vault_config("personal").vault.format_version, 2);
}

// init's help describes what has shipped: the identity it generates, an
// additive --recipient and (P4 · M8a2) --remote, and no recovery-key flags
// yet.
#[test]
fn init_help_describes_this_milestone() {
    let env = Env::new();
    let help = env.ok(&["init", "--help"]).stdout;
    for want in [
        "Create a new vault.",
        "Usage:\n  gage init",
        "--recipient",
        "--device",
        "--dir",
        "--type",
        "--method",
        "Generates this device's identity",
        "passphrase\nyou choose",
        "never committed or synced",
        "--recipient is additive",
        "this device's own key is always included",
        "--remote",
        "publishes the new vault immediately",
    ] {
        assert!(help.contains(want), "{want:?} missing:\n{help}");
    }
    for absent in ["recovery", "required for now"] {
        assert!(!help.contains(absent), "{absent:?} present:\n{help}");
    }
    assert_eq!(env.ok(&["help", "init"]).stdout, help);
    assert_eq!(env.ok(&["init", "-h"]).stdout, help);
}

// --- vault list / info ------------------------------------------------------

// Go: TestVaultListIncludesFreshlyInitedVault
#[test]
fn vault_list_includes_freshly_inited_vault() {
    let env = Env::new();
    assert_eq!(env.ok(&["vault", "list"]).stdout, "");
    env.init("personal", &[]);
    env.init("work", &[]);

    let r = env.ok(&["vault", "list"]);

    // Sorted; the current vault (the first registered) marked.
    assert_eq!(r.stdout, "* personal\n  work\n");
}

// Go: TestVaultInfoByNameReportsExpectedFields. Three: this device's own
// generated key plus the two --recipient keys, which are additive to it.
#[test]
fn vault_info_by_name_reports_expected_fields() {
    let env = Env::new();
    env.init(
        "personal",
        &["--recipient", RECIPIENT_1, "--recipient", RECIPIENT_2],
    );
    env.init("work", &[]);

    let r = env.ok(&["vault", "info", "personal"]);

    let id = env.entry("personal").id;
    assert_eq!(
        r.stdout,
        format!(
            "name: personal\nid: {id}\ntype: git\nmethod: passphrase\nrecipients: 3\nremote: (none)\nstatus: clean\n"
        )
    );
}

// Go: TestVaultInfoNoArgumentReportsOnCurrent
#[test]
fn vault_info_no_argument_reports_on_current() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    assert!(
        env.ok(&["vault", "info"])
            .stdout
            .starts_with("name: personal\n")
    );
    env.ok(&["vault", "set-default", "work"]);
    assert!(
        env.ok(&["vault", "info"])
            .stdout
            .starts_with("name: work\n")
    );
}

// The plan's decision: no argument and no current vault is a usage error.
#[test]
fn vault_info_with_no_argument_and_no_current_vault_is_usage() {
    let env = Env::new();
    let r = env.run(&["vault", "info"]);
    assert_eq!(r.code, Code::Usage);
    assert_eq!(
        r.stderr,
        "gage: no vault name given and no current vault is set\n"
    );
}

#[test]
fn vault_info_of_an_unregistered_name_is_not_found() {
    let env = Env::new();
    env.init("personal", &[]);
    let r = env.run(&["vault", "info", "nope"]);
    assert_eq!(r.code, Code::NotFound);
    assert_eq!(r.stderr, "gage: no such vault \"nope\"\n");
}

// Go: TestVaultInfoReportsDirtyWorkingTree
#[test]
fn vault_info_reports_dirty_working_tree() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    std::fs::write(path.join("entries").join("stray.age"), "ciphertext").unwrap();

    let r = env.ok(&["vault", "info", "personal"]);

    assert!(r.stdout.contains("status: dirty"), "{}", r.stdout);
}

// --- untrusted input through a real command ---------------------------------

// Go: TestTamperedFormatVersionRefusedByRealCommand
#[test]
fn tampered_format_version_refused_by_real_command() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    rewrite_config_line(&path, "format_version", "format_version = 99");

    let r = env.run(&["vault", "info", "personal"]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("upgrade gage"), "{}", r.stderr);
    assert_eq!(r.stdout, "", "rendered fields from a config it refused");
}

// Go: TestTamperedDeviceNameRefusedByRealCommand. The committed config is
// git-writable, so a device name that isn't a safe path component is
// refused by the command that reads it, and nothing appears anywhere a
// traversal would have escaped to.
#[test]
fn tampered_device_name_refused_by_real_command() {
    for evil in [
        "../../../etc/cron.d/x",
        "/etc/passwd",
        r"..\..\windows\x",
        "C:x",
        "..",
    ] {
        let env = Env::new();
        let path = env.init("personal", &[]);
        let before = env.tree();
        rewrite_config_line(&path, "device", &format!("device = '{evil}'"));

        let r = env.run(&["vault", "info", "personal"]);

        assert_eq!(r.code, Code::Conflict, "{evil}: {}", r.stderr);
        assert!(r.stderr.contains("device name"), "{}", r.stderr);
        assert_eq!(r.stdout, "");
        assert_eq!(env.tree(), before, "{evil}");
    }
}

// Go: TestAVaultFromBeforeThisChangeIsRefusedWithARecreateMessage. Exactly
// what a v1 vault looks like: format_version 1 and no [vault].id at all.
#[test]
fn a_vault_from_before_this_change_is_refused_with_a_recreate_message() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let raw = std::fs::read_to_string(config_file(&path)).unwrap();
    let v1 = raw.replacen("format_version = 2", "format_version = 1", 1);
    assert_ne!(v1, raw);
    let v1: String = v1
        .lines()
        .filter(|l| !l.trim_start().starts_with("id ="))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(config_file(&path), v1).unwrap();

    let r = env.run(&["vault", "info", "personal"]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("re-create"), "{}", r.stderr);
    assert!(r.stderr.contains("make reset-local-state"), "{}", r.stderr);
    assert!(!r.stderr.contains("upgrade gage"), "{}", r.stderr);
}

// Go: TestAnUnsafeCommittedIDIsRejectedBeforeAnyPathIsBuilt. The committed
// id is edited by hand, exactly as a git-writer would; nothing may be
// built from it under the data or state roots.
#[test]
fn an_unsafe_committed_id_is_rejected_before_any_path_is_built() {
    for (case, bad) in [
        ("traversal", "../../../../etc/cron.d/x"),
        ("absolute path", "/etc/passwd"),
        ("windows traversal", r"..\..\Windows\x"),
        ("separator", "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497/x"),
        ("not a uuid", "personal"),
        ("uppercase", "9F3A1C2E-7B41-4D58-A0C6-2E5F81B3D497"),
        ("empty", ""),
    ] {
        let env = Env::new();
        let path = env.init("personal", &[]);
        let before = env.tree();
        rewrite_config_line(&path, "id =", &format!("id = '{bad}'"));

        let r = env.run(&["vault", "info", "personal"]);

        assert_eq!(r.code, Code::Conflict, "{case}: {}", r.stderr);
        assert!(r.stderr.contains("vault id"), "{case}: {}", r.stderr);
        assert_eq!(env.tree(), before, "{case}");
    }
}

// Go: TestAMismatchedRegistrationIsRefusedRatherThanSilentlyPreferred,
// through `vault info`, M1's read command (Go's uses `show`, P2 · M4).
#[test]
fn a_mismatched_registration_is_refused_rather_than_silently_preferred() {
    let env = Env::new();
    env.init("personal", &[]);
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id = vaultconfig::new_id();
    env.save_global(&global);

    let r = env.run(&["vault", "info", "personal"]);

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    for want in ["different vault", "re-register"] {
        assert!(r.stderr.contains(want), "{want:?}: {}", r.stderr);
    }
    assert_eq!(r.stdout, "");
}

// Go: TestAnUnreadableVaultFallsBackToGlobalConfigsIDAlone, the half M1
// can run: `vault remove` works on a vault whose files are gone. The
// identity-file and stderr-names-the-identities-dir halves move to
// P6 · M12 (orphan cleanup).
#[test]
fn an_unreadable_vault_falls_back_to_global_configs_id_alone() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    std::fs::remove_dir_all(&path).unwrap();

    env.ok(&["vault", "remove", "personal"]);

    assert!(!env.global().vaults.contains_key("personal"));
}

// Go: TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath, M1's half: a
// registration with no id fails `vault info` (the vault still carries an
// id, so it is a mismatch), and with the vault gone every path builder
// refuses "". The show/insert/identity-list halves move to P2 · M4 and P5.
#[test]
fn a_vault_without_an_id_fails_loudly_on_every_derived_path() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id.clear();
    env.save_global(&global);

    let r = env.run(&["vault", "info", "personal"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("none recorded"), "{}", r.stderr);

    std::fs::remove_dir_all(&path).unwrap();
    assert!(paths::identities_dir(&env.dirs, "").is_err());
    assert!(paths::identity_file_path(&env.dirs, "", "laptop-1").is_err());
    assert!(paths::trust_cache_dir(&env.dirs, "").is_err());
    assert!(paths::lock_file_path(&env.dirs, "").is_err());
}

// --- vault remove / set-default ---------------------------------------------

// Go: TestVaultRemoveDropsRegistrationButLeavesFiles
#[test]
fn vault_remove_drops_registration_but_leaves_files() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let before = env.tree();

    env.ok(&["vault", "remove", "personal"]);

    let global = env.global();
    assert!(!global.vaults.contains_key("personal"));
    assert_eq!(global.current, "", "current named the removed vault");
    assert!(path.join(".git").exists());
    assert_eq!(env.tree(), before, "remove touched the vault's files");
}

#[test]
fn vault_remove_of_an_unregistered_name_is_not_found_and_changes_nothing() {
    let env = Env::new();
    env.init("personal", &[]);
    let before = env.global();
    let r = env.run(&["vault", "remove", "nope"]);
    assert_eq!(r.code, Code::NotFound);
    assert_eq!(env.global(), before);
}

// Go M1's set-default bullet.
#[test]
fn vault_set_default_changes_current_and_refuses_an_unknown_name() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);

    env.ok(&["vault", "set-default", "work"]);
    assert_eq!(env.global().current, "work");

    let before = env.global();
    let r = env.run(&["vault", "set-default", "nope"]);
    assert_eq!(r.code, Code::NotFound);
    assert_eq!(env.global(), before);
}

// --- git set-remote ---------------------------------------------------------

// Go M1's set-remote bullets: origin and [vaults.<name>.git].origin set in
// one call; changing it updates both, and `vault info` shows the new URL.
#[test]
fn git_set_remote_sets_and_changes_origin_in_both_places() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let (first, second) = (
        gage::gittest::new_bare_remote(),
        gage::gittest::new_bare_remote(),
    );

    for remote in [&first, &second] {
        let url = remote.path().to_str().unwrap();

        env.ok(&["git", "set-remote", "personal", url]);

        assert_eq!(gitrepo::remote_url(&path).unwrap().as_deref(), Some(url));
        assert_eq!(env.entry("personal").git.origin, url);
        let info = env.ok(&["vault", "info", "personal"]).stdout;
        assert!(info.contains(&format!("remote: {url}\n")), "{info}");
    }
}

#[test]
fn git_set_remote_with_a_missing_argument_is_usage_and_changes_nothing() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let before = env.global();
    for args in [
        &["git", "set-remote", "personal"][..],
        &["git", "set-remote"],
    ] {
        let r = env.run(args);
        assert_eq!(r.code, Code::Usage, "{args:?}");
        assert!(r.stderr.contains("Usage:"), "{}", r.stderr);
        assert_eq!(env.global(), before);
        assert_eq!(gitrepo::remote_url(&path).unwrap(), None);
    }
}

#[test]
fn git_set_remote_on_an_unregistered_name_is_not_found() {
    let env = Env::new();
    let r = env.run(&["git", "set-remote", "nope", "/x.git"]);
    assert_eq!(r.code, Code::NotFound);
    assert_eq!(r.stderr, "gage: no such vault \"nope\"\n");
}

// --- error prefix -----------------------------------------------------------

// Go: TestNoCommandDoublesGagesOwnPrefix, M1's and M2's rows (M0b's are in
// `tests`): every failure this milestone added, prefixed exactly once.
#[test]
fn no_vault_command_doubles_gages_own_prefix() {
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    let cases: &[(&str, &[&str])] = &[
        ("init bad device", &["init", "a", "--device", "../x"]),
        ("init bad type", &["init", "a", "--type", "s3"]),
        ("init bad method", &["init", "a", "--method", "ssh"]),
        ("init bad recipient", &["init", "a", "--recipient", "nope"]),
        ("init registered name", &["init", "personal"]),
        ("init missing name", &["init"]),
        ("vault info unknown", &["vault", "info", "nope"]),
        ("vault info mismatched", &["vault", "info", "mismatched"]),
        ("vault info tampered", &["vault", "info", "tampered"]),
        ("vault remove unknown", &["vault", "remove", "nope"]),
        (
            "vault set-default unknown",
            &["vault", "set-default", "nope"],
        ),
        (
            "git set-remote unknown",
            &["git", "set-remote", "nope", "/x"],
        ),
        (
            "git set-remote missing url",
            &["git", "set-remote", "personal"],
        ),
    ];
    let env = Env::new();
    env.init("personal", &[]);
    env.init("mismatched", &[]);
    let mut global = env.global();
    global.vaults.get_mut("mismatched").unwrap().id = vaultconfig::new_id();
    env.save_global(&global);
    let tampered = env.init("tampered", &[]);
    rewrite_config_line(&tampered, "format_version", "format_version = 99");
    let no_current = Env::new();

    let mut results: Vec<(&str, CliResult)> = cases
        .iter()
        .map(|(name, args)| (*name, env.run(args)))
        .collect();
    results.push(("vault info no current", no_current.run(&["vault", "info"])));
    // M2's rows: init failing at the passphrase, and failing after the
    // identity exists (the rollback path, whose error can carry the
    // rollback's own failure appended).
    results.push((
        "init passphrase cancelled",
        no_current.run_with(
            &["init", "fresh"],
            &FakePrompter::new().with_unlock_error(gage::prompter::PromptError::Cancelled),
        ),
    ));
    results.push((
        "init empty passphrase",
        no_current.run_with(&["init", "fresh"], &FakePrompter::new().with_passphrase("")),
    ));
    let occupied = no_current.root.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), "mine").unwrap();
    results.push((
        "init into a non-empty directory after generating the identity",
        no_current.run(&["init", "fresh", "--dir", occupied.to_str().unwrap()]),
    ));
    for (name, r) in &results {
        assert_ne!(
            r.code,
            Code::Success,
            "{name}: this table only covers failures"
        );
        for line in r.stdout.lines().chain(r.stderr.lines()) {
            assert!(!line.contains(&doubled), "{name}: {line}");
            assert!(line.matches(ERROR_PREFIX).count() <= 1, "{name}: {line}");
        }
        assert!(
            r.stderr.starts_with(ERROR_PREFIX),
            "{name}: unprefixed:\n{}",
            r.stderr
        );
    }
}
