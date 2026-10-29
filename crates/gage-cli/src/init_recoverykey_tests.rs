//! `gage init`'s recovery key through the real CLI, in-process.
//! The destination guard's own helpers are here too.
//! `recovery rotate` is in
//! `recovery_cli_tests`, which shares the harness below.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::exitcode::ExitCode;
use gage::entry::CandidateList;
use gage::prompter::{FakePrompter, PromptError, Prompter, UnlockRequest, UnlockResponse, Warning};
use gage::recovery::RECOVERY_DEVICE_LABEL;
use gage::syncresolve::{EntryConflict, Resolution};
use gage::xdgpaths::Dirs;
use zeroize::Zeroizing;

use crate::commands::{
    MAX_RECOVERY_CONFIRM_ATTEMPTS, RECOVERY_CONFIRM_CHARS, check_recovery_key_out_path,
    path_is_inside,
};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE, SharedBuf, Tty};

/// Answers a re-type the way a human does: by reading the key off the
/// screen and typing its last characters back.
/// Answering from the screen rather than from a planted value is what
/// makes "the key shown is the key that satisfies the confirmation"
/// something a test proves. Everything but `value` goes to `inner`.
pub(crate) struct ScreenPrompter {
    pub(crate) inner: FakePrompter,
    pub(crate) screen: SharedBuf,
    /// Typed instead of what is on screen: the wrong-answer path.
    pub(crate) answer: Option<String>,
    /// Every re-type prompt, in order.
    pub(crate) asked: RefCell<Vec<String>>,
    /// Run each time a value is asked for, before answering: how a test
    /// checks the state of the world at the moment of the question.
    pub(crate) on_value: Option<Box<dyn Fn()>>,
    /// Applied to the characters read off the screen before typing them.
    pub(crate) transform: Option<fn(&str) -> String>,
    /// Run on each unlock (init's identity passphrase), before answering.
    pub(crate) on_unlock: Option<Box<dyn Fn()>>,
    /// Answers to the first value prompts, ahead of any re-type:
    /// `recovery enroll`'s paste.
    pub(crate) pastes: RefCell<std::collections::VecDeque<String>>,
}

impl ScreenPrompter {
    /// Unlocks (or creates an identity) with [`PASSPHRASE`] once.
    pub(crate) fn new() -> ScreenPrompter {
        ScreenPrompter {
            inner: FakePrompter::new().with_passphrase(PASSPHRASE),
            screen: SharedBuf::default(),
            answer: None,
            asked: RefCell::default(),
            on_value: None,
            on_unlock: None,
            transform: None,
            pastes: RefCell::default(),
        }
    }

    /// Answers the next value prompt with `key`, as a paste.
    pub(crate) fn pasting(self, key: &str) -> ScreenPrompter {
        self.pastes.borrow_mut().push_back(key.into());
        self
    }

    pub(crate) fn answering(mut self, answer: &str) -> ScreenPrompter {
        self.answer = Some(answer.into());
        self
    }

    /// Runs `args` at a terminal, with this prompter watching the run's
    /// stderr.
    pub(crate) fn run(&self, env: &Env, args: &[&str]) -> CliResult {
        env.run_io(args, self, &mut &b""[..], Tty::BOTH, self.screen.clone())
    }
}

impl Prompter for ScreenPrompter {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        if let Some(hook) = &self.on_unlock {
            hook();
        }
        self.inner.unlock(req)
    }

    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm(prompt)
    }

    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm_default_yes(prompt)
    }

    fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
        self.asked.borrow_mut().push(title.to_string());
        if let Some(hook) = &self.on_value {
            hook();
        }
        if let Some(pasted) = self.pastes.borrow_mut().pop_front() {
            return Ok(Zeroizing::new(pasted));
        }
        if let Some(answer) = &self.answer {
            return Ok(Zeroizing::new(answer.clone()));
        }
        let shown = secret_keys(&self.screen.text());
        let key = shown.last().map(String::as_str).unwrap_or_default();
        let tail = &key[key.len().saturating_sub(RECOVERY_CONFIRM_CHARS)..];
        Ok(Zeroizing::new(match self.transform {
            Some(f) => f(tail),
            None => tail.to_string(),
        }))
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

    fn warn(&self, warning: &Warning) {
        self.inner.warn(warning);
    }
}

/// `init` at a terminal, confirming the key from the screen:
/// the result, and the key shown, if any.
fn run_init_with_recovery_key(env: &Env, args: &[&str]) -> (CliResult, Option<String>) {
    let r = ScreenPrompter::new().run(env, args);
    let shown = secret_keys(&r.stderr).into_iter().next();
    (r, shown)
}

/// The identity files under the data root.
fn identity_files(env: &Env) -> Vec<PathBuf> {
    env.tree()
        .into_iter()
        .filter(|p| p.is_file() && p.components().any(|c| c.as_os_str() == "identities"))
        .collect()
}

/// Every `AGE-SECRET-KEY-1...` in `text`, in order: how a test reads a key
/// back off the screen. Every hygiene
/// assertion then searches for the real key, not this pattern.
pub(crate) fn secret_keys(text: &str) -> Vec<String> {
    const PREFIX: &str = "AGE-SECRET-KEY-1";
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        let tail = &rest[at + PREFIX.len()..];
        let len = tail
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit()))
            .unwrap_or(tail.len());
        found.push(format!("{PREFIX}{}", &tail[..len]));
        rest = &tail[len..];
    }
    found
}

/// Everything under the data and state roots, and the config root, so a
/// refusal can be shown to have created nothing.
fn snapshot(env: &Env) -> (Vec<PathBuf>, bool) {
    (env.tree(), env.dirs.config.exists())
}

fn args_with_out<'a>(name: &'a str, dir: &'a Path, out: &'a Path) -> Vec<&'a str> {
    vec![
        "init",
        name,
        "--dir",
        dir.to_str().unwrap(),
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]
}

/// Runs `args` expecting a `Usage` refusal before any prompt (the
/// prompter is unscripted, so a passphrase request panics), and checks it
/// created nothing.
fn refused_before_anything(env: &Env, args: &[&str]) -> String {
    let before = snapshot(env);
    let r = env.run_with(args, &FakePrompter::new());
    assert_eq!(r.code, ExitCode::Usage, "{args:?}: {}", r.stderr);
    assert_eq!(snapshot(env), before, "{args:?} created something");
    assert!(secret_keys(&(r.stdout.clone() + &r.stderr)).is_empty());
    r.stderr
}

// --- the destination guard --------------------------------------------------

// Gage
// resets a vault's working tree after an interrupted write, which removes
// untracked files, and the data dir is gage's to rewrite: a key filed in
// either is a key gage deletes itself.
#[test]
fn init_refuses_to_write_the_recovery_key_where_gage_will_delete_it() {
    let env = Env::new();
    // Another vault, unregistered afterwards, so the guard can only find
    // it the way a vault is recognisable from outside: .gage/config.toml.
    let existing = env.root.path().join("other");
    env.init("other", &["--dir", existing.to_str().unwrap()]);
    let mut global = env.global();
    global.vaults.remove("other");
    global.current = None;
    env.save_global(&global);

    let target = env.root.path().join("personal");
    let cases = [
        (
            "inside the vault being created",
            target.join("recovery.key"),
            "vault",
        ),
        (
            "below the vault being created",
            target.join("keys").join("recovery.key"),
            "vault",
        ),
        (
            "inside another vault",
            existing.join("recovery.key"),
            "vault",
        ),
        (
            "below another vault's entries",
            existing.join("entries").join("r.key"),
            "vault",
        ),
        (
            "under the data dir",
            env.dirs.data.join("recovery.key"),
            "gage manages",
        ),
    ];
    for (case, out, why) in cases {
        let stderr = refused_before_anything(&env, &args_with_out("personal", &target, &out));
        assert!(stderr.contains(why), "{case}: {stderr}");
        assert!(!out.exists(), "{case}: written anyway");
        assert!(!target.join(".gage").exists(), "{case}: vault created");
        assert!(!env.global().vaults.contains_key("personal"), "{case}");
    }
}

// The ancestor walk
// has to start from an absolute path, or a bare `recovery.key` run from
// inside a vault walks only its own relative prefix and is allowed. The
// working directory is a parameter, so no test moves the process's.
#[test]
fn recovery_key_out_rejects_a_relative_path_inside_a_vault() {
    let env = Env::new();
    let vault = env.init(
        "other",
        &["--dir", env.root.path().join("other").to_str().unwrap()],
    );
    let cwd = vault.join("entries");
    let new_vault = env.root.path().join("personal");

    for rel in ["recovery.key", "./recovery.key", "../recovery.key"] {
        let err =
            check_recovery_key_out_path(Path::new(rel), &cwd, &new_vault, &env.dirs).unwrap_err();
        assert_eq!(err.code(), ExitCode::Usage, "{rel}: {err}");
        assert!(err.to_string().contains("vault"), "{rel}: {err}");
        assert!(!cwd.join(rel).exists(), "{rel}");
    }
}

#[test]
fn recovery_key_out_rejects_a_parent_that_does_not_exist() {
    let env = Env::new();
    let out = env
        .root
        .path()
        .join("no")
        .join("such")
        .join("dir")
        .join("recovery.key");
    let target = env.root.path().join("personal");
    let stderr = refused_before_anything(&env, &args_with_out("personal", &target, &out));
    assert!(stderr.contains("cannot write"), "{stderr}");
}

// A stat through a regular file fails with ENOTDIR, which is not an
// internal error: a path through a regular file does not exist, so the
// parent check answers, with Usage.
#[test]
fn recovery_key_out_rejects_a_parent_that_is_a_regular_file() {
    let env = Env::new();
    let not_a_dir = env.root.path().join("iam-a-file");
    std::fs::write(&not_a_dir, "not a directory").unwrap();
    let target = env.root.path().join("personal");
    let stderr = refused_before_anything(
        &env,
        &args_with_out("personal", &target, &not_a_dir.join("recovery.key")),
    );
    assert!(stderr.contains("not a directory"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(&not_a_dir).unwrap(),
        "not a directory"
    );
}

// A
// read-only directory stats perfectly well and fails at the write, by
// which time the vault exists with the lost key as a recipient. Unix only,
// Windows doesn't enforce directory write permission this way.
#[cfg(unix)]
#[test]
fn init_checks_the_recovery_key_destination_is_writable_before_creating() {
    use std::os::unix::fs::PermissionsExt;

    let env = Env::new();
    let dir = env.root.path().join("readonly");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    struct Restore<'a>(&'a Path);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _restore = Restore(&dir);
    if std::fs::write(dir.join("probe"), b"").is_ok() {
        // Root ignores directory permissions: there is nothing to prove.
        return;
    }

    let target = env.root.path().join("personal");
    let stderr = refused_before_anything(
        &env,
        &args_with_out("personal", &target, &dir.join("recovery.key")),
    );
    assert!(stderr.contains("cannot write"), "{stderr}");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "the probe stayed"
    );
}

// The realistic
// thing at a path named recovery.key is another vault's recovery key.
#[test]
fn init_recovery_key_out_refuses_to_clobber_an_existing_file() {
    let env = Env::new();
    let out = env.root.path().join("recovery.key");
    std::fs::write(&out, "someone else's key").unwrap();
    let target = env.root.path().join("personal");
    let stderr = refused_before_anything(&env, &args_with_out("personal", &target, &out));
    assert!(stderr.contains("already exists"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "someone else's key");
}

#[test]
fn path_is_inside_with_no_root_is_not_inside() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!path_is_inside(
        &dir.path().join("recovery.key"),
        Path::new("")
    ));
}

// On macOS the temp dir is under /var, a symlink to /private/var: a
// comparison of the path as given against a resolved one must still
// answer yes, in both directions.
#[test]
fn path_is_inside_sees_through_a_symlinked_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let given = dir.path().to_path_buf();
    let resolved = std::fs::canonicalize(&given).unwrap();
    assert!(path_is_inside(&given.join("x").join("rk"), &resolved));
    assert!(path_is_inside(&resolved.join("rk"), &given));

    #[cfg(unix)]
    {
        let real = given.join("real");
        std::fs::create_dir(&real).unwrap();
        let link = given.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(path_is_inside(&link.join("not-yet").join("rk"), &real));
        assert!(path_is_inside(&real.join("rk"), &link));
    }
}

// Components, not characters: a sibling whose name extends the root's is
// not inside it.
#[test]
fn path_is_inside_is_not_fooled_by_a_shared_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    assert!(!path_is_inside(
        &dir.path().join("vault2").join("rk"),
        &vault
    ));
    assert!(!path_is_inside(&dir.path().join("vault.key"), &vault));
    assert!(path_is_inside(&vault, &vault), "the root itself is inside");
}

// The guard's answer for a path it accepts: absolute, so the write lands
// where the check looked even when the command's working directory isn't
// the process's.
#[test]
fn an_accepted_destination_is_made_absolute_against_the_given_cwd() {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: root.path().join("config"),
        data: root.path().join("data"),
        state: root.path().join("state"),
    };
    let cwd = root.path().join("elsewhere");
    std::fs::create_dir(&cwd).unwrap();
    let dest =
        check_recovery_key_out_path(Path::new("rk"), &cwd, &root.path().join("vault"), &dirs)
            .unwrap();
    assert_eq!(dest, cwd.join("rk"));
    assert_eq!(
        std::fs::read_dir(&cwd).unwrap().count(),
        0,
        "the probe stayed"
    );
}

// --- planning and refusals --------------------------------------------------

// With
// nowhere a human can read, init won't make a key that would land in a
// log, and says which flags resolve it. Nothing is asked or created.
#[test]
fn init_without_a_terminal_refuses_rather_than_logging_the_key() {
    let env = Env::new();
    let stderr = refused_before_anything(&env, &["init", "personal", "--device", "laptop-1"]);
    for want in [
        "--recovery-key-out",
        "--no-recovery-key",
        "needs a terminal",
    ] {
        assert!(stderr.contains(want), "{want:?} missing: {stderr}");
    }
    assert!(identity_files(&env).is_empty());
    assert!(!env.default_vault_path("personal").exists());
}

// `gage init foo 2>build.log`
// has a human at the keyboard and nowhere on screen for the key: stdin
// being a terminal doesn't stand in for stderr.
#[test]
fn init_with_redirected_stderr_refuses_too() {
    let env = Env::new();
    let before = snapshot(&env);
    let r = env.run_io(
        &["init", "personal", "--device", "laptop-1"],
        &FakePrompter::new(),
        &mut &b""[..],
        Tty {
            stdin: true,
            stderr: false,
        },
        SharedBuf::default(),
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(secret_keys(&(r.stdout + &r.stderr)).is_empty());
    assert_eq!(snapshot(&env), before);

    // And the other way round: a terminal on stderr alone has nobody
    // typing the re-type.
    let r = env.run_io(
        &["init", "personal", "--device", "laptop-1"],
        &FakePrompter::new(),
        &mut &b""[..],
        Tty {
            stdin: false,
            stderr: true,
        },
        SharedBuf::default(),
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert_eq!(snapshot(&env), before);
}

#[test]
fn init_rejects_recovery_key_out_with_no_recovery_key() {
    let env = Env::new();
    let out = env.root.path().join("recovery.key");
    let stderr = refused_before_anything(
        &env,
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--recovery-key-out",
            out.to_str().unwrap(),
        ],
    );
    assert!(stderr.contains("mutually exclusive"), "{stderr}");
    assert!(!out.exists());
}

// As R5 flipped it:
// refused before the passphrase prompt, and --no-recovery-key doesn't
// free the label, since `recovery rotate` can add a key later and would
// evict whatever device held it.
#[test]
fn init_rejects_a_device_named_like_the_recovery_label() {
    let env = Env::new();
    let before = snapshot(&env);
    let r = env.run_terminal(
        &["init", "personal", "--device", RECOVERY_DEVICE_LABEL],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains(RECOVERY_DEVICE_LABEL), "{}", r.stderr);
    assert_eq!(snapshot(&env), before);

    let stderr = refused_before_anything(
        &env,
        &[
            "init",
            "personal",
            "--device",
            RECOVERY_DEVICE_LABEL,
            "--no-recovery-key",
        ],
    );
    assert!(stderr.contains("reserved"), "{stderr}");
}

#[test]
fn init_no_recovery_key_opts_out() {
    let env = Env::new();
    let r = env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--no-recovery-key",
    ]);
    assert_eq!(env.vault_config("personal").recipients.len(), 1);
    let all = r.stdout + &r.stderr;
    assert!(secret_keys(&all).is_empty(), "{all}");
    assert!(!all.contains(RECOVERY_DEVICE_LABEL), "{all}");
}

// The key is shown only once
// the vault it opens exists, so a failed init never leaves anyone holding
// a key to nothing; and the rollback leaves no identity behind.
#[test]
fn init_shows_no_recovery_key_when_create_fails() {
    let env = Env::new();
    let dir = env.root.path().join("occupied");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();

    let (r, shown) = run_init_with_recovery_key(
        &env,
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--dir",
            dir.to_str().unwrap(),
        ],
    );
    assert_ne!(r.code, ExitCode::Success, "{}", r.stdout);
    assert_eq!(shown, None, "{}", r.stderr);
    assert!(secret_keys(&r.stdout).is_empty());
    assert!(!r.stdout.contains(RECOVERY_DEVICE_LABEL), "{}", r.stdout);
    assert!(!env.global().vaults.contains_key("personal"));
    assert!(
        identity_files(&env).is_empty(),
        "{:?}",
        identity_files(&env)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
        "mine"
    );
}

// --- generation, registration and delivery ----------------------------------

/// The `age1...` recipient a shown secret belongs to.
pub(crate) fn pubkey_of(secret: &str) -> String {
    secret
        .parse::<age::x25519::Identity>()
        .unwrap()
        .to_public()
        .to_string()
}

fn files_under(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The hygiene walk: `secret` is in no file under the three XDG roots or
/// `vault`, and in no object of `vault`'s repository read through the
/// object database, decompressed. Case-insensitively, since a bech32 key
/// is the same key in either case.
pub(crate) fn assert_never_stored(env: &Env, vault: &Path, secret: &str, what: &str) {
    let mut files = Vec::new();
    for root in [&env.dirs.config, &env.dirs.data, &env.dirs.state] {
        files_under(root, &mut files);
    }
    files_under(vault, &mut files);
    assert!(
        files.iter().any(|f| f.ends_with(".age-recipients")),
        "the walk never reached the vault: {files:?}"
    );
    for file in &files {
        assert!(
            !contains(&std::fs::read(file).unwrap(), secret),
            "the {what} was written to {}",
            file.display()
        );
    }
    let objects = gage::gittest::all_object_data(vault);
    assert!(!objects.is_empty(), "no git objects to search");
    for object in &objects {
        assert!(!contains(object, secret), "the {what} is in a git object");
    }
}

fn default_init(env: &Env) -> (CliResult, String) {
    let (r, shown) = run_init_with_recovery_key(env, &["init", "personal", "--device", "laptop-1"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let shown = shown.unwrap_or_else(|| panic!("no key shown:\n{}", r.stderr));
    (r, shown)
}

#[test]
fn init_registers_a_recovery_recipient_by_default() {
    let env = Env::new();
    let (r, secret) = default_init(&env);

    let recipients = env.vault_config("personal").recipients;
    let labels: Vec<&str> = recipients.iter().map(|r| r.device.as_str()).collect();
    assert_eq!(labels, ["laptop-1", RECOVERY_DEVICE_LABEL]);
    // The two halves of one key, not merely two keys.
    assert_eq!(pubkey_of(&secret), recipients[1].pubkey);
    let file = std::fs::read_to_string(env.entry("personal").path.join(".age-recipients")).unwrap();
    assert_eq!(
        file,
        format!("{}\n{}\n", recipients[0].pubkey, recipients[1].pubkey)
    );
    // Named, so the printed secret can later be matched to its recipient.
    assert!(r.stdout.contains(&recipients[1].pubkey), "{}", r.stdout);
    assert!(r.stdout.contains(RECOVERY_DEVICE_LABEL), "{}", r.stdout);
}

// Through a real entry: one
// inserted after init decrypts with the shown key and nothing else.
#[test]
fn init_recovery_key_alone_decrypts_the_vault() {
    let env = Env::new();
    let (_, secret) = default_init(&env);
    let r = env.run_stdin(
        &["insert", "api-token", "--value-stdin"],
        &FakePrompter::new().with_passphrase(PASSPHRASE),
        b"s3cr3t-value\n",
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);

    let identity: age::x25519::Identity = secret.parse().unwrap();
    let mut entries = Vec::new();
    files_under(&env.entry("personal").path.join("entries"), &mut entries);
    assert_eq!(entries.len(), 1, "{entries:?}");
    let plaintext = age::decrypt(&identity, &std::fs::read(&entries[0]).unwrap()).unwrap();
    assert!(
        String::from_utf8(plaintext)
            .unwrap()
            .contains("s3cr3t-value"),
        "the recovery key decrypted something else"
    );
}

// ⚑ No copy anywhere gage writes,
// git objects included; shown exactly once, on stderr; and the re-type
// doesn't echo what it asked for.
#[test]
fn init_recovery_key_is_never_stored() {
    let env = Env::new();
    let (r, secret) = default_init(&env);

    assert_never_stored(&env, &env.entry("personal").path, &secret, "recovery key");
    let all = format!("{}{}", r.stdout, r.stderr);
    assert_eq!(all.matches(&secret).count(), 1, "{all}");
    assert!(!r.stdout.contains(&secret), "the key went to stdout");
    let tail = &secret[secret.len() - RECOVERY_CONFIRM_CHARS..];
    assert!(
        !r.stderr.contains(&format!("\n{tail}\n")),
        "the re-type echoed its answer"
    );
}

// The key is
// unencrypted, so what's printed beside it is all that stands between it
// and a screenshot in a synced notes app. All of it on stderr.
#[test]
fn init_recovery_key_is_shown_with_its_warnings_and_a_qr() {
    let env = Env::new();
    let (r, secret) = default_init(&env);

    for want in [
        "This is the only time it will be shown",
        "not encrypted",
        "anyone who has it can read",
        "cannot be shown again",
        "offline",
        "printer you trust",
        "offline encrypted",
        "cloud-synced notes, a password manager",
        "email, or a photo",
        "scrollback",
    ] {
        assert!(r.stderr.contains(want), "{want:?} missing:\n{}", r.stderr);
        assert!(!r.stdout.contains(want), "{want:?} on stdout");
    }
    assert!(
        r.stderr.contains(crate::qr::QR_BOTH),
        "no QR:\n{}",
        r.stderr
    );
    assert!(!r.stdout.contains(crate::qr::QR_BOTH), "QR on stdout");
    assert!(
        r.stderr.contains(&format!("    {secret}\n")),
        "{}",
        r.stderr
    );
    // Key, then QR, then the warnings, then the question.
    let at = |s: &str| r.stderr.find(s).unwrap();
    assert!(at(&secret) < at(&crate::qr::QR_BOTH.to_string()));
    assert!(at(&crate::qr::QR_BOTH.to_string()) < at("not encrypted"));
}

// No
// terminal needed, and no re-type: the prompter answers only the
// passphrase, so a value request would fail the test.
#[test]
fn init_recovery_key_out_writes_a_private_file_and_nothing_to_stdout() {
    let env = Env::new();
    let out = env.root.path().join("recovery.key");
    let r = env.ok(&[
        "init",
        "personal",
        "--device",
        "laptop-1",
        "--recovery-key-out",
        out.to_str().unwrap(),
    ]);

    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.ends_with('\n'), "{written:?}");
    let secret = written.trim();
    assert_eq!(secret_keys(secret), [secret], "{written:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(!format!("{}{}", r.stdout, r.stderr).contains(secret));
    assert!(r.stderr.contains(out.to_str().unwrap()), "{}", r.stderr);
    assert!(r.stderr.contains("not encrypted"), "{}", r.stderr);
    assert!(r.stderr.contains("delete this file"), "{}", r.stderr);

    let recipients = env.vault_config("personal").recipients;
    assert_eq!(recipients.len(), 2);
    assert_eq!(recipients[1].device, RECOVERY_DEVICE_LABEL);
    assert_eq!(pubkey_of(secret), recipients[1].pubkey);
    assert_never_stored(&env, &env.entry("personal").path, secret, "recovery key");
}

// One
// masked re-type of a fixed number of the key's last characters.
#[test]
fn init_recovery_key_confirmation_asks_for_the_keys_own_characters() {
    assert_eq!(
        RECOVERY_CONFIRM_CHARS, 6,
        "the prompt below is written for 6"
    );
    let env = Env::new();
    let p = ScreenPrompter::new();
    let r = p.run(&env, &["init", "personal", "--device", "laptop-1"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    let asked = p.asked.borrow();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains("last 6 characters"), "{asked:?}");
    assert!(p.inner.warnings().is_empty(), "{:?}", p.inner.warnings());
}

// The re-type is an attention check, not a parse: case and surrounding
// whitespace don't matter.
#[test]
fn init_recovery_key_confirmation_ignores_case_and_whitespace() {
    let env = Env::new();
    let mut p = ScreenPrompter::new();
    p.transform = Some(|tail| format!("  {}\t", tail.to_lowercase()));
    let r = p.run(&env, &["init", "personal", "--device", "laptop-1"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(p.asked.borrow().len(), 1);
}

// The vault exists by the time the question is asked, so a wrong answer
// undoes nothing, but it doesn't pass silently, and the key is never
// printed a second time.
#[test]
fn init_recovery_key_wrong_confirmation_retries_then_fails_without_reshowing() {
    let env = Env::new();
    let p = ScreenPrompter::new().answering("NOTTHEKEY");
    let r = p.run(&env, &["init", "personal", "--device", "laptop-1"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert_eq!(p.asked.borrow().len(), MAX_RECOVERY_CONFIRM_ATTEMPTS);
    // Warned between attempts, not after the last.
    let warnings = p.inner.warnings();
    assert_eq!(
        warnings.len(),
        MAX_RECOVERY_CONFIRM_ATTEMPTS - 1,
        "{warnings:?}"
    );
    assert!(warnings[0].contains("doesn't match"), "{warnings:?}");

    assert!(env.global().vaults.contains_key("personal"));
    assert_eq!(env.vault_config("personal").recipients.len(), 2);
    let shown = secret_keys(&r.stderr);
    assert_eq!(shown.len(), 1, "{}", r.stderr);
    assert_eq!(r.stderr.matches(&shown[0]).count(), 1);
    for want in ["not confirmed", "cannot be shown", "gage recovery rotate"] {
        assert!(r.stderr.contains(want), "{want:?} missing:\n{}", r.stderr);
    }
    // The summary still says what exists.
    assert!(r.stdout.contains("initialized vault"), "{}", r.stdout);
}

// The re-type
// is the last gate, not an early exit: skipping the push would leave a
// vault created, registered and quietly unpublished.
#[test]
fn init_still_publishes_when_the_recovery_confirmation_fails() {
    let env = Env::new();
    let remote = gage::gittest::new_bare_remote();
    let p = ScreenPrompter::new().answering("NOTTHEKEY");
    let r = p.run(
        &env,
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--remote",
            remote.path().to_str().unwrap(),
        ],
    );

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    let path = env.entry("personal").path;
    assert!(r.stdout.contains(path.to_str().unwrap()), "{}", r.stdout);
    assert!(r.stdout.contains("published"), "{}", r.stdout);
    assert!(gage::gittest::Device::clone(remote.path()).exists(".age-recipients"));
}

// The key is shown only once the vault it opens is committed
// and registered. Checked at the moment the re-type is asked, which is
// after the key is on screen.
#[test]
fn init_key_is_shown_only_after_the_commit_exists() {
    let env = Env::new();
    let vault = env.default_vault_path("personal");
    let global = gage::paths::global_config_file(&env.dirs);
    let checked = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut p = ScreenPrompter::new();
    let (seen, screen) = (checked.clone(), p.screen.clone());
    p.on_value = Some(Box::new(move || {
        assert_eq!(secret_keys(&screen.text()).len(), 1, "asked before shown");
        assert_eq!(gage::gitrepo::commit_count(&vault).unwrap(), 1);
        assert!(gage::gitrepo::is_clean(&vault).unwrap());
        let registered = gage::config::load(&global).unwrap();
        assert!(registered.vaults.contains_key("personal"));
        seen.set(true);
    }));
    let r = p.run(&env, &["init", "personal", "--device", "laptop-1"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert!(checked.get(), "the re-type was never asked");
}

// When both delivery and the push fail, the
// delivery error sets the exit code — an unpublished vault can be pushed
// again, a key nobody has can't be reissued — and the push failure is
// reported beside it. Both failures are real: a file appearing at the
// destination after the up-front check (while the passphrase is being
// typed) is refused by write_new's backstop, and a remote that is a
// missing local path fails to publish.
#[test]
fn init_delivery_error_outranks_a_failed_push() {
    let env = Env::new();
    let out = env.root.path().join("recovery.key");
    let missing = env.root.path().join("nothing-here.git");
    let mut p = ScreenPrompter::new();
    let squatter = out.clone();
    p.on_unlock = Some(Box::new(move || {
        std::fs::write(&squatter, "appeared meanwhile").unwrap();
    }));
    let r = p.run(
        &env,
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--recovery-key-out",
            out.to_str().unwrap(),
            "--remote",
            missing.to_str().unwrap(),
        ],
    );

    assert_eq!(r.code, ExitCode::Internal, "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "appeared meanwhile");
    assert!(r.stderr.contains("gage recovery rotate"), "{}", r.stderr);
    assert!(r.stderr.contains("create an empty one"), "{}", r.stderr);
    assert!(r.stdout.contains("initialized vault"), "{}", r.stdout);
    assert!(env.global().vaults.contains_key("personal"));
    assert!(secret_keys(&format!("{}{}", r.stdout, r.stderr)).is_empty());

    // And a refused re-type outranks a failed push the same way.
    let env = Env::new();
    let p = ScreenPrompter::new().answering("NOTTHEKEY");
    let r = p.run(
        &env,
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--remote",
            env.root.path().join("nothing-here.git").to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("create an empty one"), "{}", r.stderr);
}

// An empty `--recovery-key-out` named the working
// directory, which then "already exists". It is refused as what it is,
// before anything is created, rather than treated as unset, which would
// show the key on a terminal the user asked
// to keep it off (ADR-0051).
#[test]
fn an_empty_recovery_key_out_is_refused_as_empty() {
    let env = Env::new();
    let stderr = refused_before_anything(&env, &["init", "personal", "--recovery-key-out", ""]);
    assert!(stderr.contains("needs a file name"), "{stderr}");
    let before = snapshot(&env);
    let r = env.run_terminal(
        &["init", "personal", "--recovery-key-out", ""],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert_eq!(snapshot(&env), before);
}
