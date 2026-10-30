//! Vault lifecycle through the real CLI, in-process: `init`, `vault
//! list/info/remove/set-default` and `git set-remote`, each run against a
//! fresh temp directory standing in for gage's config, data and state
//! roots, and an injected hostname.

use std::path::{Path, PathBuf};

use crate::exitcode::ExitCode;
use gage::Warning;
use gage::entry::CandidateList;
use gage::prompter::{FakePrompter, PromptError, Prompter, UnlockRequest, UnlockResponse};
use gage::storage::config::{self, Global, VaultEntry};
use gage::storage::vaultconfig;
use gage::storage::xdgpaths::Dirs;
use gage::sync::syncresolve::{EntryConflict, Resolution};
use gage::{storage::paths, sync::gitrepo, trust::recipient};

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
    pub(crate) code: ExitCode,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// A byte buffer two writers share: the command's stderr, and the
/// prompter's warnings, interleaved in the order they happened. A test's
/// prompter can hold a clone to read the screen as it was when a question
/// was asked.
#[derive(Clone, Default)]
pub(crate) struct SharedBuf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl SharedBuf {
    /// Everything written so far.
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

/// Which of the standard streams a run sees as a terminal. In-process
/// runs default to [`Tty::NONE`]: a test is a pipe unless it says
/// otherwise.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tty {
    pub(crate) stdin: bool,
    pub(crate) stderr: bool,
}

impl Tty {
    pub(crate) const NONE: Tty = Tty {
        stdin: false,
        stderr: false,
    };
    /// A human at a terminal: what showing a recovery key needs.
    pub(crate) const BOTH: Tty = Tty {
        stdin: true,
        stderr: true,
    };
}

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
/// emitting one with nothing going red. Everything
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

    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm_default_yes(prompt)
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
        warning: &gage::trust::trustcache::RecipientChangeWarning,
    ) -> Result<bool, PromptError> {
        self.inner.confirm_recipient_change(warning)
    }

    fn warn(&self, warning: &Warning) {
        use std::io::Write;
        let _ = writeln!(
            self.stderr.clone(),
            "{}",
            crate::remedy::render_warning(warning)
        );
        self.inner.warn(warning);
    }
}

/// One isolated machine: its own config, data and state roots
/// and hostname.
pub(crate) struct Env {
    pub(crate) root: tempfile::TempDir,
    pub(crate) dirs: Dirs,
    pub(crate) hostname: String,
    /// What the commands see as `$EDITOR`; unset unless a test sets one.
    pub(crate) editor: std::sync::Mutex<Option<String>>,
    /// The clipboard every run copies to. Never the real one.
    pub(crate) clip: crate::clipboard::fake::Harness,
    /// The zone local times render in; UTC unless a test sets another.
    pub(crate) time_zone: std::sync::Mutex<jiff::tz::TimeZone>,
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
            clip: Default::default(),
            time_zone: std::sync::Mutex::new(jiff::tz::TimeZone::UTC),
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
        self.run_io(args, prompter, stdin, Tty::NONE, SharedBuf::default())
    }

    /// Like [`Env::run_with`], at a terminal on both stdin and stderr.
    pub(crate) fn run_terminal(&self, args: &[&str], prompter: &dyn Prompter) -> CliResult {
        self.run_io(
            args,
            prompter,
            &mut &b""[..],
            Tty::BOTH,
            SharedBuf::default(),
        )
    }

    /// The general form: which streams are terminals, and the buffer the
    /// run's stderr lands in, so a prompter holding a clone of `screen`
    /// can read what was on it when it was asked.
    pub(crate) fn run_io(
        &self,
        args: &[&str],
        prompter: &dyn Prompter,
        stdin: &mut dyn std::io::Read,
        tty: Tty,
        stderr: SharedBuf,
    ) -> CliResult {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (code, stdout, stderr) = self.with_io(prompter, stdin, tty, stderr, |io, p| {
            crate::run(&args, io, p)
        });
        CliResult {
            code,
            stdout,
            stderr,
        }
    }

    /// Runs `args` and returns the command's error itself rather than its
    /// exit code, so a test can match the typed error inside it.
    /// Nothing is printed for it: rendering is `run`'s.
    pub(crate) fn dispatch(
        &self,
        args: &[&str],
        prompter: &dyn Prompter,
        tty: Tty,
    ) -> Result<(), crate::error::CliError> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (result, _, _) = self.with_io(
            prompter,
            &mut &b""[..],
            tty,
            SharedBuf::default(),
            |io, p| crate::dispatch(&crate::registry::registry(), &args, io, p),
        );
        result
    }

    /// The `Io` every run gets, with `f` run inside it: what it returned,
    /// and what reached stdout and stderr.
    fn with_io<T>(
        &self,
        prompter: &dyn Prompter,
        stdin: &mut dyn std::io::Read,
        tty: Tty,
        stderr: SharedBuf,
        f: impl FnOnce(&mut Io, &dyn Prompter) -> T,
    ) -> (T, String, String) {
        let mut stdout = Vec::new();
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
            stdin_is_terminal: tty.stdin,
            stderr_is_terminal: tty.stderr,
            line_editor: false,
            build: BuildInfo::stamped(),
            dirs: &move || Ok(dirs.clone()),
            hostname: &move || Ok(hostname.clone()),
            editor: self.editor.lock().unwrap().clone(),
            passphrase_env: None,
            clock: &gage::SystemClock,
            clipboard: &self.clip.keeper(),
            clipboard_wait: &|d| self.clip.wait(d),
            time_zone: self.time_zone.lock().unwrap().clone(),
        };
        let result = f(&mut io, &prompter);
        drop(io);
        (result, String::from_utf8(stdout).unwrap(), stderr.text())
    }

    /// Runs `args` and requires success.
    pub(crate) fn ok(&self, args: &[&str]) -> CliResult {
        let r = self.run(args);
        assert_eq!(r.code, ExitCode::Success, "{args:?} failed:\n{}", r.stderr);
        r
    }

    /// `gage init <name> <extra...>`; returns the registered path. Without
    /// a recovery flag in `extra` it passes `--no-recovery-key`: every run
    /// here is a pipe, where a default init refuses.
    pub(crate) fn init(&self, name: &str, extra: &[&str]) -> PathBuf {
        let mut args = vec!["init", name];
        args.extend_from_slice(&with_recovery_flag(extra));
        self.ok(&args);
        self.entry(name).path
    }

    pub(crate) fn global(&self) -> Global {
        config::load_or_default(&paths::global_config_file(&self.dirs)).unwrap()
    }

    pub(crate) fn save_global(&self, global: &Global) {
        config::save_creating_dir(&paths::global_config_file(&self.dirs), global).unwrap();
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
        vaultconfig::load(&config_file(&self.entry(name).path)).unwrap()
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

/// `extra`, plus `--no-recovery-key` unless it already makes a recovery
/// choice of its own.
pub(crate) fn with_recovery_flag<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = extra.to_vec();
    if !extra
        .iter()
        .any(|a| *a == "--no-recovery-key" || a.starts_with("--recovery-key-out"))
    {
        args.push("--no-recovery-key");
    }
    args
}

/// `gage init <name> --device laptop-1 --recovery-key-out <file>`, and the
/// key it wrote. The file sits in
/// the env's root, outside every directory gage manages.
pub(crate) fn init_vault_with_recovery_key_file(env: &Env, name: &str) -> String {
    let out = env.root.path().join(format!("{name}-recovery.key"));
    env.ok(&[
        "init",
        name,
        "--device",
        "laptop-1",
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]);
    std::fs::read_to_string(&out).unwrap().trim().to_string()
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
/// `prefix`, standing in for any git-writer (ADR-0010).
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

#[test]
fn resolve_device_name_explicit_overrides_hostname() {
    assert_eq!(
        resolve_device_name(Some("custom-device"), "andrews-macbook-pro.local").unwrap(),
        "custom-device"
    );
}

#[test]
fn resolve_device_name_falls_back_to_normalized_hostname() {
    assert_eq!(
        resolve_device_name(None, "Andrews-MacBook-Pro.local").unwrap(),
        "andrews-macbook-pro"
    );
}

// Refused,
// never normalized into some other device's name.
#[test]
fn resolve_device_name_explicit_failing_allowlist_is_usage_error() {
    for bad in ["../../etc/x", "Laptop-1", ""] {
        let err = resolve_device_name(Some(bad), "irrelevant").unwrap_err();
        assert_eq!(err.code(), ExitCode::Usage, "{bad:?}");
    }
}

// One-shot mode has no free-text prompt, so "asking" is a usage error that
// names the flag, never an invented name.
#[test]
fn resolve_device_name_hostname_normalizes_to_nothing_prompts_rather_than_inventing() {
    for hostname in ["", "...", "!!!", "---"] {
        let err = resolve_device_name(None, hostname).unwrap_err();
        assert_eq!(err.code(), ExitCode::Usage, "{hostname:?}");
        assert!(err.to_string().contains("--device"), "{err}");
    }
}

// The same through `init`: nothing is created or registered.
#[test]
fn init_with_an_unusable_hostname_fails_before_creating_files() {
    let mut env = Env::new();
    env.hostname = "!!!".into();

    let r = env.run_with(
        &["init", "personal", "--no-recovery-key"],
        &FakePrompter::new(),
    );

    assert_eq!(r.code, ExitCode::Usage);
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

#[test]
fn init_creates_full_skeleton_and_registers() {
    let env = Env::new();

    let r = env.ok(&["init", "personal", "--no-recovery-key"]);

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

// Both halves: created there,
// and registered under exactly that path.
#[test]
fn init_without_dir_uses_gage_data_vaults_dir() {
    let env = Env::new();
    env.init("personal", &[]);
    let want = env.dirs.data.join("vaults").join("personal");
    assert!(config_file(&want).exists());
    assert_eq!(env.entry("personal").path, want);
}

#[test]
fn init_with_dir_uses_given_path() {
    let env = Env::new();
    let dir = env.root.path().join("somewhere-else");

    env.init("personal", &["--dir", dir.to_str().unwrap()]);

    assert_eq!(env.entry("personal").path, dir);
    assert!(dir.join(".git").exists());
    assert!(!env.default_vault_path("personal").exists());
}

#[test]
fn init_into_non_empty_non_gage_directory_fails_without_touching_files() {
    let env = Env::new();
    let dir = env.root.path().join("mine");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();

    let r = env.run(&[
        "init",
        "personal",
        "--dir",
        dir.to_str().unwrap(),
        "--no-recovery-key",
    ]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
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

#[test]
fn init_into_already_registered_name_fails_and_leaves_existing_vault_untouched() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let before = env.global();
    let keys_before = recipient::load(&path.join(".age-recipients")).unwrap();

    let r = env.run_with(
        &[
            "init",
            "personal",
            "--recipient",
            RECIPIENT_2,
            "--no-recovery-key",
        ],
        &FakePrompter::new(),
    );

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("already a registered vault"),
        "{}",
        r.stderr
    );
    assert_eq!(env.global(), before);
    let keys = recipient::load(&path.join(".age-recipients")).unwrap();
    assert_eq!(keys, keys_before);
}

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

// Equivalent, not
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
    let same_id = vaultconfig::VaultId::new();
    for file in [&mut implicit, &mut explicit] {
        file.vault.name.clear();
        file.vault.id = same_id.clone();
        file.vault.created.clear();
        for r in &mut file.recipients {
            r.pubkey.clear();
        }
    }
    assert_eq!(implicit, explicit);

    let (a, b) = (env.entry("implicit"), env.entry("explicit"));
    assert_eq!((a.vault_type, a.method), (b.vault_type, b.method));
}

#[test]
fn init_unknown_type_fails_with_usage_and_lists_git() {
    let env = Env::new();
    let r = env.run_with(
        &["init", "personal", "--type", "s3", "--no-recovery-key"],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage);
    assert!(r.stderr.contains("git"), "{}", r.stderr);
    assert!(!env.default_vault_path("personal").exists());
}

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

        let r = env.run_with(
            &["init", &name, "--method", method, "--no-recovery-key"],
            &FakePrompter::new(),
        );

        assert_eq!(r.code, ExitCode::Usage, "{method}");
        assert!(r.stderr.contains("passphrase"), "{}", r.stderr);
        assert!(!env.default_vault_path(&name).exists(), "{method}");
        assert!(!env.global().vaults.contains_key(&name), "{method}");
    }
}

// The exact refusals, pinned before D3a turned type and method into enums:
// a string the enum doesn't know must still fail the same way, with the
// same words. `identity add --method` is pinned in identity_cli_tests.
#[test]
fn init_unknown_type_and_method_messages_are_pinned() {
    let env = Env::new();
    for (args, want) in [
        (
            &["init", "a", "--type", "s3", "--no-recovery-key"][..],
            "gage: unknown vault type \"s3\"; accepted: git\n",
        ),
        (
            &["init", "a", "--method", "yubikey", "--no-recovery-key"],
            "gage: unknown method \"yubikey\"; accepted: passphrase\n",
        ),
    ] {
        let r = env.run_with(args, &FakePrompter::new());
        assert_eq!(r.code, ExitCode::Usage, "{args:?}");
        assert_eq!(r.stderr, want, "{args:?}");
    }
}

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
        let mut args = vec!["init", "personal", "--no-recovery-key"];
        args.extend_from_slice(keys);
        let r = env.run_with(&args, &FakePrompter::new());
        assert_eq!(r.code, ExitCode::Usage, "{keys:?}");
        assert!(!env.default_vault_path("personal").exists());
        assert!(env.global().vaults.is_empty());
    }
}

// This device's key first, then
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

    let keys = recipient::load(&path.join(".age-recipients")).unwrap();
    assert_eq!(Some(&keys[0]), env.entry("personal").pubkey.as_ref());
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

#[test]
fn init_device_flag_overrides_hostname_default() {
    let env = Env::new();
    env.init("personal", &["--device", "custom-device"]);
    assert_eq!(env.entry("personal").device, "custom-device");
}

#[test]
fn init_device_flag_failing_allowlist_rejected_before_creating_files() {
    let env = Env::new();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--device",
            "../../../etc/x",
            "--no-recovery-key",
        ],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage);
    assert!(env.tree().is_empty(), "{:?}", env.tree());
    assert!(env.global().vaults.is_empty());
}

// Unlock resolves the
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

// Global config, [[recipients]]
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
        .join(entry.id.as_ref().unwrap())
        .join("workstation-7.age");
    assert!(
        identity.is_file(),
        "no identity file at {}",
        identity.display()
    );
}

// Re-init under a
// removed name succeeds, mints a new id, and files a new identity under it,
// leaving the removed vault's leftover identity file untouched.
#[test]
fn reinit_after_vault_remove_is_an_ordinary_retry() {
    let env = Env::new();
    let path = env.init("vault-test-newinit", &["--device", "laptop-1"]);
    let old_id = env.entry("vault-test-newinit").id.unwrap();
    let old_identity = paths::identity_file_path(&env.dirs, &old_id, "laptop-1").unwrap();
    let old_bytes = std::fs::read(&old_identity).unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    env.ok(&["vault", "remove", "vault-test-newinit"]);
    assert!(
        old_identity.is_file(),
        "vault remove took the identity file"
    );

    env.init("vault-test-newinit", &["--device", "laptop-1"]);

    let new_id = env.entry("vault-test-newinit").id.unwrap();
    assert_ne!(new_id, old_id, "a new vault gets a new id");
    assert_eq!(env.vault_config("vault-test-newinit").vault.id, new_id);
    let new_identity = paths::identity_file_path(&env.dirs, &new_id, "laptop-1").unwrap();
    assert_ne!(new_identity, old_identity);
    assert!(new_identity.is_file(), "the re-init wrote no identity file");
    assert_eq!(
        std::fs::read(&old_identity).unwrap(),
        old_bytes,
        "the leftover was disturbed"
    );
}

#[test]
fn init_with_no_recipient_generates_this_devices_identity() {
    let env = Env::new();
    env.ok(&["init", "personal", "--no-recovery-key"]);

    let entry = env.entry("personal");
    let file = env.vault_config("personal");
    assert_eq!(
        file.recipients.len(),
        1,
        "want exactly this device's own key"
    );
    let pubkey = &file.recipients[0].pubkey;
    gage::crypto::agekey::validate_recipient(pubkey).unwrap();
    assert_eq!(file.recipients[0].device, entry.device);
    // The same key, in both recipient-defining files.
    let keys = recipient::load(&entry.path.join(".age-recipients")).unwrap();
    assert_eq!(keys, std::slice::from_ref(pubkey));
}

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
        assert_eq!(&id.as_str()[14..15], "4", "not a v4 UUID: {id}");
    }
    // Global config records the same id the vault commits.
    assert_eq!(env.entry("personal").id, Some(first));
    assert_eq!(env.entry("work").id, Some(second));
}

#[test]
fn init_writes_format_version_two() {
    let env = Env::new();
    env.init("personal", &[]);
    assert_eq!(env.vault_config("personal").vault.format_version, 2);
}

// init's help describes what has shipped: the identity it generates, an
// additive --recipient, --remote, and the recovery
// key on by default with its two flags.
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
        "offline recovery key and shows it once",
        "gage keeps no copy",
        "--no-recovery-key",
        "--recovery-key-out",
        "(mode 0600)",
    ] {
        assert!(help.contains(want), "{want:?} missing:\n{help}");
    }
    assert!(!help.contains("required for now"), "{help}");
    assert_eq!(env.ok(&["help", "init"]).stdout, help);
    assert_eq!(env.ok(&["init", "-h"]).stdout, help);
}

// --- vault list / info ------------------------------------------------------

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

// Three: this device's own
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

    let id = env.entry("personal").id.unwrap();
    assert_eq!(
        r.stdout,
        format!(
            "name: personal\nid: {id}\ntype: git\nmethod: passphrase\nrecipients: 3\nremote: (none)\nstatus: clean\n"
        )
    );
}

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
    assert_eq!(r.code, ExitCode::Usage);
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
    assert_eq!(r.code, ExitCode::NotFound);
    assert_eq!(r.stderr, "gage: no such vault \"nope\"\n");
}

#[test]
fn vault_info_reports_dirty_working_tree() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    std::fs::write(path.join("entries").join("stray.age"), "ciphertext").unwrap();

    let r = env.ok(&["vault", "info", "personal"]);

    assert!(r.stdout.contains("status: dirty"), "{}", r.stdout);
}

// --- untrusted input through a real command ---------------------------------

#[test]
fn tampered_format_version_refused_by_real_command() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    rewrite_config_line(&path, "format_version", "format_version = 99");

    let r = env.run(&["vault", "info", "personal"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("upgrade gage"), "{}", r.stderr);
    assert_eq!(r.stdout, "", "rendered fields from a config it refused");
}

// The committed config is
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

        assert_eq!(r.code, ExitCode::Conflict, "{evil}: {}", r.stderr);
        assert!(r.stderr.contains("device name"), "{}", r.stderr);
        assert_eq!(r.stdout, "");
        assert_eq!(env.tree(), before, "{evil}");
    }
}

// Exactly
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

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("re-create"), "{}", r.stderr);
    assert!(r.stderr.contains("make reset-local-state"), "{}", r.stderr);
    assert!(!r.stderr.contains("upgrade gage"), "{}", r.stderr);
}

// The committed
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

        assert_eq!(r.code, ExitCode::Conflict, "{case}: {}", r.stderr);
        assert!(r.stderr.contains("vault id"), "{case}: {}", r.stderr);
        assert_eq!(env.tree(), before, "{case}");
    }
}

// A mismatched registration is refused rather than silently preferred,
// through `vault info`.
#[test]
fn a_mismatched_registration_is_refused_rather_than_silently_preferred() {
    let env = Env::new();
    env.init("personal", &[]);
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id = Some(vaultconfig::VaultId::new());
    env.save_global(&global);

    let r = env.run(&["vault", "info", "personal"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    for want in ["different vault", "re-register"] {
        assert!(r.stderr.contains(want), "{want:?}: {}", r.stderr);
    }
    assert_eq!(r.stdout, "");
}

// The half that
// can run: `vault remove` works on a vault whose files are gone. The
// identity-file and stderr-names-the-identities-dir halves are in
// vault_remove_tests.
#[test]
fn an_unreadable_vault_falls_back_to_global_configs_id_alone() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    std::fs::remove_dir_all(&path).unwrap();

    env.ok(&["vault", "remove", "personal"]);

    assert!(!env.global().vaults.contains_key("personal"));
}

// A
// registration with no id fails `vault info` (the vault still carries an
// id, so it is a mismatch), and with the vault gone too there is no id
// anywhere, which is refused before any path could be built (exit 5, as
// the empty id it replaces had). The show/insert/identity-list halves are
// in their own test files.
#[test]
fn a_vault_without_an_id_fails_loudly_on_every_derived_path() {
    let env = Env::new();
    let path = env.init("personal", &["--device", "laptop-1"]);
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id = None;
    env.save_global(&global);

    let r = env.run(&["vault", "info", "personal"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("none recorded"), "{}", r.stderr);

    std::fs::remove_dir_all(&path).unwrap();
    let r = env.run(&["identity", "list", "--use", "personal"]);
    assert_eq!(r.code, ExitCode::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("records no vault id"), "{}", r.stderr);
    assert!(!env.dirs.state.join("locks").exists());
}

// --- vault remove / set-default ---------------------------------------------

#[test]
fn vault_remove_drops_registration_but_leaves_files() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    let before = env.tree();

    env.ok(&["vault", "remove", "personal"]);

    let global = env.global();
    assert!(!global.vaults.contains_key("personal"));
    assert_eq!(global.current, None, "current named the removed vault");
    assert!(path.join(".git").exists());
    assert_eq!(env.tree(), before, "remove touched the vault's files");
}

#[test]
fn vault_remove_of_an_unregistered_name_is_not_found_and_changes_nothing() {
    let env = Env::new();
    env.init("personal", &[]);
    let before = env.global();
    let r = env.run(&["vault", "remove", "nope"]);
    assert_eq!(r.code, ExitCode::NotFound);
    assert_eq!(env.global(), before);
}

#[test]
fn vault_set_default_changes_current_and_refuses_an_unknown_name() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);

    env.ok(&["vault", "set-default", "work"]);
    assert_eq!(env.global().current, Some("work".into()));

    let before = env.global();
    let r = env.run(&["vault", "set-default", "nope"]);
    assert_eq!(r.code, ExitCode::NotFound);
    assert_eq!(env.global(), before);
}

// --- git set-remote ---------------------------------------------------------

// set-remote: origin and [vaults.<name>.git].origin set in
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
        assert_eq!(env.entry("personal").git.origin, Some(url.into()));
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
        assert_eq!(r.code, ExitCode::Usage, "{args:?}");
        assert!(r.stderr.contains("Usage:"), "{}", r.stderr);
        assert_eq!(env.global(), before);
        assert_eq!(gitrepo::remote_url(&path).unwrap(), None);
    }
}

#[test]
fn git_set_remote_on_an_unregistered_name_is_not_found() {
    let env = Env::new();
    let r = env.run(&["git", "set-remote", "nope", "/x.git"]);
    assert_eq!(r.code, ExitCode::NotFound);
    assert_eq!(r.stderr, "gage: no such vault \"nope\"\n");
}

// --- error prefix -----------------------------------------------------------

// Every failure the vault commands can produce, prefixed exactly once
// (the base error types are in `tests`).
#[test]
fn no_vault_command_doubles_gages_own_prefix() {
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    let cases: &[(&str, &[&str])] = &[
        (
            "init bad device",
            &["init", "a", "--device", "../x", "--no-recovery-key"],
        ),
        (
            "init bad type",
            &["init", "a", "--type", "s3", "--no-recovery-key"],
        ),
        (
            "init bad method",
            &["init", "a", "--method", "ssh", "--no-recovery-key"],
        ),
        (
            "init bad recipient",
            &["init", "a", "--recipient", "nope", "--no-recovery-key"],
        ),
        (
            "init registered name",
            &["init", "personal", "--no-recovery-key"],
        ),
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
    global.vaults.get_mut("mismatched").unwrap().id = Some(vaultconfig::VaultId::new());
    env.save_global(&global);
    let tampered = env.init("tampered", &[]);
    rewrite_config_line(&tampered, "format_version", "format_version = 99");
    let no_current = Env::new();

    let mut results: Vec<(&str, CliResult)> = cases
        .iter()
        .map(|(name, args)| (*name, env.run(args)))
        .collect();
    results.push(("vault info no current", no_current.run(&["vault", "info"])));
    // init failing at the passphrase, and failing after the
    // identity exists (the rollback path, whose error can carry the
    // rollback's own failure appended).
    results.push((
        "init passphrase cancelled",
        no_current.run_with(
            &["init", "fresh", "--no-recovery-key"],
            &FakePrompter::new().with_unlock_error(gage::prompter::PromptError::Cancelled),
        ),
    ));
    results.push((
        "init empty passphrase",
        no_current.run_with(
            &["init", "fresh", "--no-recovery-key"],
            &FakePrompter::new().with_passphrase(""),
        ),
    ));
    let occupied = no_current.root.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), "mine").unwrap();
    results.push((
        "init into a non-empty directory after generating the identity",
        no_current.run(&[
            "init",
            "fresh",
            "--dir",
            occupied.to_str().unwrap(),
            "--no-recovery-key",
        ]),
    ));
    for (name, r) in &results {
        assert_ne!(
            r.code,
            ExitCode::Success,
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
