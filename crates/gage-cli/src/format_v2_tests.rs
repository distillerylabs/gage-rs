//! The golden format-v2 vault (`tests/fixtures/format-v2/`) read end to
//! end through the real CLI, in-process, and the generator that wrote it.
//! The format is `doc/format/v2.md`; the fixture's constants and install
//! helper are `gage::testsupport::format_v2`. These tests are a format
//! guard: a change that needs one of them edited to pass is a format
//! change, which takes a `format_version` bump, a new fixture and an ADR.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::exitcode::ExitCode;
use gage::Vault;
use gage::entry::NewEntry;
use gage::gittest;
use gage::prompter::FakePrompter;
use gage::storage::config::Shell;
use gage::testsupport::format_v2::{self as fx, Installed};

use crate::enroll_cli_tests::code_in;
use crate::lifecycle_tests::{CliResult, Env};

/// A prompter that answers one unlock with the fixture's passphrase.
fn unlocks() -> FakePrompter {
    FakePrompter::new().with_passphrase(fx::PASSPHRASE)
}

impl Env {
    /// The machine the fixture captures: its roots are the installed ones.
    fn installed(i: Installed) -> Env {
        let mut env = Env::new();
        env.root = i.root;
        env.dirs = i.dirs;
        env
    }
}

fn ok(r: CliResult, what: &str) -> CliResult {
    assert_eq!(r.code, ExitCode::Success, "{what}:\n{}", r.stderr);
    r
}

// --- the generator ----------------------------------------------------------

const README: &str = include_str!("format_v2_readme.md");

/// Writes `tests/fixtures/format-v2/`. Run once, by hand, when the
/// fixture is first made: `cargo test -p gage-cli write_format_v2_fixture
/// -- --ignored --nocapture`. It refuses to overwrite: a v2 fixture is
/// never regenerated in place, and a format change adds `format-v3/`
/// beside it. It prints the ids and keys the run minted, which are then
/// written into `gage::testsupport::format_v2`.
#[test]
#[ignore = "writes the committed golden fixture; run by hand, once"]
fn write_format_v2_fixture() {
    let out = fx::fixture_dir();
    assert!(
        !out.exists(),
        "{} exists; a fixture is never regenerated in place",
        out.display()
    );

    let laptop = Env::new();
    let phone = Env::new();
    let origin = gittest::new_bare_remote();
    let url = origin.path().to_str().unwrap().to_string();

    // laptop-1 creates the vault, with a recovery key, and publishes it.
    let recovery_file = laptop.root.path().join("recovery.key");
    ok(
        laptop.run_with(
            &[
                "init",
                fx::VAULT_NAME,
                "--device",
                fx::DEVICE,
                "--remote",
                &url,
                "--recovery-key-out",
                recovery_file.to_str().unwrap(),
            ],
            &unlocks(),
        ),
        "init",
    );

    // The entry with unknown keys, through the library: no command of
    // this build writes keys it doesn't know.
    let registration = laptop.entry(fx::VAULT_NAME);
    let vault = Vault::from_registration(&laptop.dirs, fx::VAULT_NAME, &registration).unwrap();
    let p = unlocks();
    let identity = vault.unlock(&p).unwrap();
    vault
        .insert(
            &identity,
            NewEntry {
                title: fx::RICH.title.into(),
                description: fx::RICH.description.into(),
                value: fx::RICH.value.into(),
                fields: fx::RICH
                    .fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                extra: fx::rich_extra(),
            },
            false,
            &p,
        )
        .unwrap();
    drop(identity);

    ok(
        laptop.run_stdin(
            &["insert", fx::MULTILINE.title, "--value-stdin"],
            &unlocks(),
            format!("{}\n", fx::MULTILINE.value).as_bytes(),
        ),
        "insert multiline",
    );

    // phone-1 clones and asks to join; the request is published.
    ok(
        phone.run_with(
            &[
                "clone",
                &url,
                "--name",
                fx::VAULT_NAME,
                "--device",
                fx::ENROLLING_DEVICE,
            ],
            &FakePrompter::new(),
        ),
        "clone",
    );
    let enrolled = ok(
        phone.run_with(
            &["identity", "enroll", "--device", fx::ENROLLING_DEVICE],
            &FakePrompter::new().with_passphrase("phone-1 passphrase"),
        ),
        "enroll",
    );
    let code = code_in(&enrolled.stdout);

    // Meanwhile laptop-1 writes with its remote unreachable, so the two
    // diverge, and `sync` merges them.
    let offline = origin.path().with_extension("offline");
    std::fs::rename(origin.path(), &offline).unwrap();
    ok(
        laptop.run_stdin(
            &["insert", fx::MINIMAL.title, "--value-stdin"],
            &unlocks(),
            format!("{}\n", fx::MINIMAL.value).as_bytes(),
        ),
        "offline insert",
    );
    std::fs::rename(&offline, origin.path()).unwrap();
    let synced = ok(laptop.run_with(&["sync"], &FakePrompter::new()), "sync");
    assert!(synced.stdout.contains("merged"), "{}", synced.stdout);

    // [shell], with durations.
    let mut global = laptop.global();
    global.shell = Shell {
        prompt: Some(fx::SHELL_PROMPT.into()),
        idle_timeout: Some(fx::SHELL_IDLE_TIMEOUT.into()),
        history_file: Some(fx::SHELL_HISTORY_FILE.into()),
        clipboard_timeout: Some(fx::SHELL_CLIPBOARD_TIMEOUT.into()),
    };
    laptop.save_global(&global);

    // Export.
    let id = registration.id.clone().unwrap();
    let trust = laptop.dirs.state.join(&id);
    assert!(trust.join("known-config.toml").exists(), "no trust cache");
    assert!(trust.join("trust.toml").exists(), "no trust cache");

    // The repository's objects and refs, and a config with nothing the
    // writing machine's filesystem chose (libgit2 records case and
    // Unicode folding on macOS).
    fx::copy_tree(origin.path(), &out.join("vault.git"));
    for scratch in ["hooks", "description", "info", "logs"] {
        let path = out.join("vault.git").join(scratch);
        if path.is_dir() {
            std::fs::remove_dir_all(&path).unwrap();
        } else if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
    }
    write(
        &out.join("vault.git").join("config"),
        "[core]\n\tbare = true\n\trepositoryformatversion = 0\n",
    );
    fx::copy_tree(
        &laptop.dirs.data.join("identities").join(&id),
        &out.join("data").join("identities").join(&id),
    );
    fx::copy_tree(&trust, &out.join("state").join(&id));

    let text =
        std::fs::read_to_string(gage::storage::paths::global_config_file(&laptop.dirs)).unwrap();
    let template = text
        .replace(
            registration.path.to_str().unwrap(),
            fx::VAULT_PATH_PLACEHOLDER,
        )
        .replace(&url, fx::ORIGIN_PLACEHOLDER);
    let root = laptop.root.path().to_str().unwrap();
    assert!(
        !template.contains(root),
        "an absolute path is left:\n{template}"
    );
    write(&out.join("config").join("config.toml.in"), &template);

    let recovery_key = std::fs::read_to_string(&recovery_file).unwrap();
    write(&out.join("recovery-key.txt"), &recovery_key);

    let request = std::fs::read_dir(registration.path.join(".gage/pending"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .next()
        .expect("a pending request");
    let request_id = &request[..36];

    let laptop_entry = laptop.entry(fx::VAULT_NAME);
    let recipients = laptop.vault_config(fx::VAULT_NAME).recipients;
    let recovery_pubkey = &recipients
        .iter()
        .find(|r| r.device == "recovery-paper-key")
        .unwrap()
        .pubkey;
    write(
        &out.join("README.md"),
        &README
            .replace("@VAULT_ID@", id.as_str())
            .replace("@DEVICE_PUBKEY@", laptop_entry.pubkey.as_deref().unwrap())
            .replace("@RECOVERY_PUBKEY@", recovery_pubkey)
            .replace("@REQUEST_ID@", request_id)
            .replace("@ENROLLMENT_CODE@", &code),
    );
    write(&out.join(fx::MANIFEST_FILE), &fx::manifest(&out));

    println!("VAULT_ID = {id:?}");
    println!("DEVICE_PUBKEY = {:?}", laptop_entry.pubkey);
    println!("RECOVERY_PUBKEY = {recovery_pubkey:?}");
    println!("REQUEST_ID = {request_id:?}");
    println!("ENROLLMENT_CODE = {code:?}");
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

// --- reading it back --------------------------------------------------------

/// Where the installed fixture's clone and remote are.
struct Paths {
    vault: PathBuf,
    remote: PathBuf,
}

/// The fixture installed as an [`Env`]'s machine.
fn installed() -> (Env, Paths) {
    let i = fx::install();
    let paths = Paths {
        vault: i.vault.clone(),
        remote: i.remote.clone(),
    };
    (Env::installed(i), paths)
}

#[test]
fn format_v2_global_config_parses() {
    let (env, i) = installed();
    let global = env.global();

    assert_eq!(global.current, Some(fx::VAULT_NAME.into()));
    assert_eq!(global.vaults.len(), 1);
    let entry = &global.vaults[fx::VAULT_NAME];
    assert_eq!(entry.path, i.vault);
    assert_eq!(entry.id, Some(fx::vault_id()));
    assert_eq!(entry.vault_type, "git");
    assert_eq!(entry.device, fx::DEVICE);
    assert_eq!(entry.pubkey, Some(fx::DEVICE_PUBKEY.into()));
    assert_eq!(entry.method, "passphrase");
    assert_eq!(Path::new(entry.git.origin.as_deref().unwrap()), i.remote);

    assert_eq!(global.shell.prompt, Some(fx::SHELL_PROMPT.into()));
    assert_eq!(
        global.shell.idle_timeout,
        Some(fx::SHELL_IDLE_TIMEOUT.into())
    );
    assert_eq!(
        global.shell.history_file,
        Some(fx::SHELL_HISTORY_FILE.into())
    );
    assert_eq!(
        global.shell.clipboard_timeout,
        Some(fx::SHELL_CLIPBOARD_TIMEOUT.into())
    );
    let shell = crate::shell::resolve(&global.shell, &env.dirs, &|| None).unwrap();
    assert_eq!(shell.prompt, fx::SHELL_PROMPT);
    assert_eq!(shell.idle_timeout, Some(Duration::from_secs(15 * 60)));
    assert_eq!(shell.clipboard_timeout, Duration::from_secs(90));
    assert_eq!(shell.history_file, env.dirs.state.join("history"));
}

#[test]
fn format_v2_unlocks_and_lists() {
    let (env, _i) = installed();
    let r = ok(env.run_with(&["ls"], &unlocks()), "ls");
    for e in fx::ENTRIES {
        assert!(
            r.stdout.contains(e.title),
            "{:?} not listed:\n{}",
            e.title,
            r.stdout
        );
    }
    assert_eq!(r.stdout.lines().count(), fx::ENTRIES.len(), "{}", r.stdout);
    // The trust cache matches the committed recipients: no warning.
    assert!(!r.stderr.contains("recipient list changed"), "{}", r.stderr);
}

/// `show` prints the value (or one field) exactly; `cat` prints the whole
/// decrypted entry, the keys this build doesn't know included.
#[test]
fn format_v2_shows_every_entry() {
    let (env, _i) = installed();
    for e in fx::ENTRIES {
        let r = ok(env.run_with(&["show", e.title], &unlocks()), "show");
        assert_eq!(r.stdout, format!("{}\n", e.value), "show {:?}", e.title);

        let r = ok(env.run_with(&["cat", e.title], &unlocks()), "cat");
        let all = &r.stdout;
        assert!(all.contains(e.title), "cat {:?}:\n{all}", e.title);
        assert!(all.contains("updated_by"), "cat {:?}:\n{all}", e.title);
        assert!(all.contains(fx::DEVICE), "cat {:?}:\n{all}", e.title);
        for line in e.value.lines().filter(|l| !l.is_empty()) {
            assert!(all.contains(line), "cat {:?}:\n{all}", e.title);
        }
        if !e.description.is_empty() {
            assert!(all.contains(e.description), "cat {:?}:\n{all}", e.title);
        }
        for (name, value) in e.fields {
            assert!(
                all.contains(name) && all.contains(value),
                "cat {:?}:\n{all}",
                e.title
            );
            let r = ok(
                env.run_with(&["show", e.title, "--field", name], &unlocks()),
                "show --field",
            );
            assert_eq!(
                r.stdout,
                format!("{value}\n"),
                "show {:?} --field {name}",
                e.title
            );
        }
    }
    // The unknown keys came through intact.
    let r = ok(env.run_with(&["cat", fx::RICH.title], &unlocks()), "cat");
    for needle in [
        "x-future-plain",
        "0x1F",
        "x-future-map",
        "rotate",
        "true",
        "tags",
    ] {
        assert!(
            r.stdout.contains(needle),
            "{needle:?} missing:\n{}",
            r.stdout
        );
    }
}

#[test]
fn format_v2_trust_cache_verifies() {
    let (env, _i) = installed();
    let r = ok(
        env.run_with(&["recipient", "verify"], &FakePrompter::new()),
        "verify",
    );
    assert!(!r.stderr.contains("disagree"), "{}", r.stderr);
}

/// `recipient pending` reads the filename's epoch against the real clock,
/// and the CLI has no clock seam, so the committed request is re-filed in
/// this clone under a live epoch: same id, same sealed bytes. The library
/// test `format_v2_pending_request_opens_at_a_fixed_clock` reads it as
/// committed.
#[test]
fn format_v2_pending_request_lists() {
    let (env, i) = installed();
    let pending = i.vault.join(".gage/pending");
    let committed = std::fs::read_dir(&pending)
        .unwrap()
        .map(|e| e.unwrap().path())
        .next()
        .unwrap();
    let live = jiff::Timestamp::now().as_second() + 3600;
    std::fs::rename(
        &committed,
        pending.join(format!("{}-{live}.age", fx::REQUEST_ID)),
    )
    .unwrap();

    let r = ok(
        env.run_with(&["recipient", "pending"], &FakePrompter::new()),
        "recipient pending",
    );
    let short = &fx::REQUEST_ID[..8];
    assert!(
        r.stdout.contains(short),
        "{short} not listed:\n{}",
        r.stdout
    );
}
