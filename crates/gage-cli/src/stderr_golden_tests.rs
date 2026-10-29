//! Exact stderr and exit codes for the error and warning shapes the
//! error boundary renders (ADR-0037, D3b). Each test pins the whole of
//! stderr, normalized only for what varies by machine: the temp root, a
//! loopback port and the path separator. A change to one of these strings
//! is a deliberate change to what a user reads, and shows in review.

use crate::exitcode::ExitCode;
use gage::gittest::{self, Answer, Device, HttpRecorder};

use crate::entry_tests::unlocks;
use crate::lifecycle_tests::{CliResult, Env, config_file};
use crate::trustcache_cli_tests::{env_with_vault, new_key, tamper_recipients_file};

/// `r`'s stderr with the machine-specific parts replaced: `$ROOT` for the
/// test's temp root, `$HOST` for `127.0.0.1:<port>`, and `/` for every
/// path separator.
fn stderr(env: &Env, r: &CliResult) -> String {
    let root = env.root.path().to_str().unwrap();
    let escaped = format!("{root:?}");
    let escaped = &escaped[1..escaped.len() - 1];
    let text = r.stderr.replace(escaped, "$ROOT").replace(root, "$ROOT");
    let text = text.replace("\\\\", "/").replace('\\', "/");
    loopback(&text)
}

fn loopback(s: &str) -> String {
    const LOOPBACK: &str = "127.0.0.1:";
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find(LOOPBACK) {
        out.push_str(&rest[..at]);
        out.push_str("$HOST");
        rest = rest[at + LOOPBACK.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

fn assert_golden(env: &Env, r: &CliResult, code: ExitCode, want: &str) {
    assert_eq!(r.code, code, "{:?}", r.stderr);
    assert_eq!(stderr(env, r), want);
}

// --- one per exit code ------------------------------------------------------

#[test]
fn conflict_is_the_multi_line_trust_refusal() {
    let env = env_with_vault();
    env.bootstrap_trust();
    let stray = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";
    tamper_recipients_file(&env.vault_dir(), stray);

    let r = env.cmd(&["recipient", "add", &new_key(), "--device", "phone-1"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Conflict,
        concat!(
            "gage: \"personal\"'s recipient list changed since this device last confirmed it, and its two recipient files now disagree; run `gage recipient verify`\n",
            "gage: the recipient files of \"personal\" disagree:\n",
            "  age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m is in .age-recipients but not in .gage/config.toml\n",
            "gage: fix this with `gage recipient verify --repair`, which rewrites .age-recipients from .gage/config.toml and says what it drops\n",
        ),
    );
}

#[test]
fn usage_is_an_unknown_vault_type() {
    let env = Env::new();
    let r = env.run(&["init", "personal", "--type", "svn", "--no-recovery-key"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Usage,
        "gage: unknown vault type \"svn\"; accepted: git\n",
    );
}

#[test]
fn not_found_is_a_missing_entry() {
    let env = env_with_vault();
    let r = env.cmd(&["show", "NoSuchEntry"]);
    assert_golden(
        &env,
        &r,
        ExitCode::NotFound,
        "gage: no entry matches \"NoSuchEntry\"\n",
    );
}

#[test]
fn ambiguous_is_a_query_with_two_matches() {
    let env = env_with_vault();
    let hub = env.insert_ok("one", &["GitHub"]);
    let lab = env.insert_ok("two", &["GitLab"]);
    let mut r = env.cmd(&["show", "Git"]);
    r.stderr = r
        .stderr
        .replace(&hub.to_string(), "$HUB")
        .replace(&lab.to_string(), "$LAB");
    assert_golden(
        &env,
        &r,
        ExitCode::Ambiguous,
        concat!(
            "gage: \"Git\" matches 2 entries; name one by its id:\n",
            "  $HUB  GitHub\n",
            "  $LAB  GitLab\n",
        ),
    );
}

#[test]
fn locked_or_auth_is_a_refused_push_with_the_login_hint() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Challenge);
    env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &unlocks().with_value("a-token"),
    );
    let r = env.run(&["push"]);
    assert_golden(
        &env,
        &r,
        ExitCode::LockedOrAuth,
        "gage: the remote refused access: 127.0.0.1 rejected the credentials gage offered; run `gage auth login --host 127.0.0.1` to store a token for 127.0.0.1 (an existing one may have expired or been revoked)\n",
    );
}

#[test]
fn internal_is_an_unreadable_global_config() {
    let env = env_with_vault();
    std::fs::write(env.dirs.config.join("config.toml"), "not = [toml").unwrap();
    let r = env.cmd(&["ls"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Internal,
        concat!(
            "gage: parsing $ROOT/config/gage/config.toml: TOML parse error at line 1, column 12\n",
            "  |\n",
            "1 | not = [toml\n",
            "  |            ^\n",
            "unclosed array, expected `]`\n",
            "\n",
        ),
    );
}

#[test]
fn a_module_prefixed_format_version_refusal() {
    let env = env_with_vault();
    let config = config_file(&env.vault_dir());
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        text.replace("format_version = 2", "format_version = 3"),
    )
    .unwrap();
    let r = env.cmd(&["ls"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Conflict,
        "gage: $ROOT/data/gage/vaults/personal/.gage/config.toml has format_version 3, this gage understands 2 — upgrade gage\n",
    );
}

#[test]
fn unreachable_is_a_push_to_a_missing_path() {
    let env = Env::new();
    let gone = env.root.path().join("gone.git");
    let remote = gittest::new_bare_remote();
    std::fs::rename(remote.path(), &gone).unwrap();
    let r = env.run(&[
        "init",
        "personal",
        "--remote",
        gone.to_str().unwrap(),
        "--no-recovery-key",
    ]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    std::fs::remove_dir_all(&gone).unwrap();
    let r = env.run(&["push"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Unreachable,
        "gage: could not reach the remote: $ROOT/gone.git does not exist (a missing or unmounted path?)\n",
    );
}

// --- hints, warnings, scripts ---------------------------------------------

#[test]
fn a_diverged_push_names_sync() {
    let env = Env::new();
    let remote = gittest::new_bare_remote();
    let r = env.run(&[
        "init",
        "personal",
        "--remote",
        remote.path().to_str().unwrap(),
        "--no-recovery-key",
    ]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    Device::clone(remote.path()).write_commit_push("shared.txt", "theirs", "theirs");
    let vault = env.vault_dir();
    std::fs::write(vault.join("shared.txt"), "mine").unwrap();
    gage::gitrepo::commit_all(&vault, "a local commit").unwrap();

    let r = env.run(&["push"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Conflict,
        concat!(
            "gage: 1 path(s) changed on both sides:\n",
            "  shared.txt\n",
            "gage: the vault is an ordinary git repository at $ROOT/data/gage/vaults/personal\n",
            "gage: origin has diverged — 1 local commit(s) pending, and 1 entr(y/ies) changed on both sides; run `gage sync`\n",
        ),
    );
}

#[test]
fn a_write_after_an_offline_push_warns_and_succeeds() {
    let env = Env::new();
    env.run_with(
        &[
            "init",
            "personal",
            "--remote",
            &gittest::refused_url(),
            "--no-recovery-key",
        ],
        &unlocks().with_value(""),
    );
    let r = env.run_stdin(
        &["insert", "GitHub", "--value-stdin"],
        &unlocks(),
        b"hunter2\n",
    );
    assert_golden(
        &env,
        &r,
        ExitCode::Success,
        concat!(
            "gage: could not reach origin; continuing with the local copy of \"personal\"\n",
            "gage: the write is committed locally but not pushed: could not reach origin\n",
        ),
    );
}

#[test]
fn a_read_warns_with_the_verify_hint() {
    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), &new_key());
    let r = env.cmd(&["ls"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Success,
        "gage: \"personal\"'s recipient list changed since this device last confirmed it, and its two recipient files now disagree; run `gage recipient verify`\n",
    );
}

#[test]
fn a_script_error_names_its_line() {
    let env = env_with_vault();
    let r = env.run_script(&["use personal", "ls", "show NoSuchEntry"]);
    assert_golden(
        &env,
        &r,
        ExitCode::NotFound,
        "gage: $ROOT/commands.gage:3: show NoSuchEntry: no entry matches \"NoSuchEntry\"\n",
    );
}

#[cfg(unix)]
#[test]
fn an_atomic_write_failure() {
    use gage::prompter::FakePrompter;

    let env = env_with_vault();
    // A directory where the token file goes: the rename onto it fails.
    let tokens = env.dirs.state.join("tokens");
    std::fs::create_dir_all(tokens.join("git.example.test")).unwrap();
    let r = env.run_with(
        &["auth", "login", "--host", "git.example.test"],
        &FakePrompter::new().with_value("a-token"),
    );
    assert_golden(
        &env,
        &r,
        ExitCode::Internal,
        "gage: renaming into place at $ROOT/state/gage/tokens/git.example.test: Is a directory (os error 21)\n",
    );
}

// --- the session opener ----------------------------------------------------

// Each way `open_registered` fails keeps the exit code it had before the
// opener's error became an enum.
#[test]
fn every_open_error_keeps_its_exit_code() {
    use gage::session::OpenError;

    use crate::commands::open_registered;
    use crate::error::CliError;
    use crate::exitcode::ExitCode;

    let code = |r: Result<gage::vault::Vault, OpenError>| CliError::from(r.unwrap_err()).code();

    let env = env_with_vault();
    assert!(matches!(
        open_registered(&env.dirs, "nope"),
        Err(OpenError::Registry(_))
    ));
    assert_eq!(code(open_registered(&env.dirs, "nope")), ExitCode::NotFound);

    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id = Some(gage::vaultconfig::VaultId::new());
    env.save_global(&global);
    assert!(matches!(
        open_registered(&env.dirs, "personal"),
        Err(OpenError::Resolve(_))
    ));
    assert_eq!(
        code(open_registered(&env.dirs, "personal")),
        ExitCode::Conflict
    );

    std::fs::write(env.dirs.config.join("config.toml"), "not = [toml").unwrap();
    assert!(matches!(
        open_registered(&env.dirs, "personal"),
        Err(OpenError::Config(_))
    ));
    assert_eq!(
        code(open_registered(&env.dirs, "personal")),
        ExitCode::Internal
    );
}

// A secret never reaches a message: URL userinfo is stripped when the
// error is built, and moving the hint into a remedy rendered by the CLI
// doesn't route around that.
#[test]
fn a_remedy_rendered_auth_failure_keeps_the_token_out() {
    use gage::syncerr::{Error, Kind};

    use crate::error::CliError;

    let refused = Error::new(
        Kind::Auth,
        "unexpected http status 403 for https://me:ghp_secret123@github.com/me/vault.git",
    )
    .annotate_auth("github.com");
    let rendered = CliError::from(gage::gitrepo::Error::Sync(refused)).to_string();
    assert!(!rendered.contains("ghp_secret123"), "{rendered}");
    assert!(!rendered.contains("me:"), "{rendered}");
    assert!(
        rendered.ends_with(
            "; run `gage auth login --host github.com` to store a token for github.com \
             (an existing one may have expired or been revoked)"
        ),
        "{rendered}"
    );
}

// `pull` builds its own refusal from the divergence report, so the report's
// remedy has to be handed over explicitly.
#[test]
fn a_diverged_pull_names_sync() {
    let env = Env::new();
    let remote = gittest::new_bare_remote();
    let r = env.run(&[
        "init",
        "personal",
        "--remote",
        remote.path().to_str().unwrap(),
        "--no-recovery-key",
    ]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    Device::clone(remote.path()).write_commit_push("notes.txt", "theirs", "theirs");
    let vault = env.vault_dir();
    std::fs::write(vault.join("local.txt"), "mine").unwrap();
    gage::gitrepo::commit_all(&vault, "a local commit").unwrap();

    let r = env.run(&["pull"]);
    assert_golden(
        &env,
        &r,
        ExitCode::Conflict,
        "gage: origin has diverged — 1 local commit(s) pending; run `gage sync`\n",
    );
}
