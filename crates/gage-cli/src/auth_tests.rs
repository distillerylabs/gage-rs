//! `gage auth login|status|logout` through the real CLI, in-process.
//! Vaults get an origin through `git set-remote`.

use gage::exitcode::Code;
use gage::prompter::FakePrompter;
use gage::{gitrepo, gittest, remoteauth};

use crate::lifecycle_tests::Env;
use crate::registry::{Availability, registry};

/// What the fake prompter answers the token prompt with: distinctive, so a
/// leak test can look for it anywhere output goes.
const TOKEN: &str = "ghp_TESTTOKENVALUE0123456789";

impl Env {
    /// Runs `args` with a prompter that answers one token request.
    fn auth(&self, args: &[&str]) -> crate::lifecycle_tests::CliResult {
        self.run_with(args, &FakePrompter::new().with_value(TOKEN))
    }

    fn token_path(&self, host: &str) -> std::path::PathBuf {
        self.dirs.state.join("tokens").join(host)
    }

    /// A vault `personal` whose origin is `url`.
    fn vault_with_origin(&self, url: &str) {
        self.init("personal", &[]);
        self.ok(&["git", "set-remote", "personal", url]);
    }
}

#[test]
fn auth_login_stores_a_token_and_status_reports_it() {
    let env = Env::new();
    let r = env.auth(&["auth", "login", "--host", "github.com"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("stored a token for github.com"),
        "{}",
        r.stdout
    );

    let path = env.token_path("github.com");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let r = env.ok(&["auth", "status", "--host", "github.com"]);
    assert_eq!(r.stdout, "github.com  configured\n");
    let r = env.ok(&["auth", "status"]);
    assert_eq!(r.stdout, "github.com  configured\n");
}

#[test]
fn auth_logout_forgets_the_token() {
    let env = Env::new();
    assert_eq!(
        env.auth(&["auth", "login", "--host", "github.com"]).code,
        Code::Success
    );
    let r = env.ok(&["auth", "logout", "--host", "github.com"]);
    assert!(
        r.stdout.contains("forgot the token for github.com"),
        "{}",
        r.stdout
    );
    assert!(!env.token_path("github.com").exists());

    let r = env.ok(&["auth", "status", "--host", "github.com"]);
    assert!(r.stdout.contains("no token"), "{}", r.stdout);
    assert!(
        r.stdout.contains("gage auth login --host github.com"),
        "{}",
        r.stdout
    );
    // Twice is not a failure.
    env.ok(&["auth", "logout", "--host", "github.com"]);
}

#[test]
fn host_defaults_to_the_current_vaults_origin() {
    let env = Env::new();
    env.vault_with_origin("https://git.example.com/me/vault.git");
    let r = env.auth(&["auth", "login"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("git.example.com"), "{}", r.stdout);
    assert!(env.token_path("git.example.com").exists());

    env.ok(&["auth", "logout"]);
    assert!(!env.token_path("git.example.com").exists());
}

#[test]
fn auth_on_a_local_remote_says_no_token_is_needed() {
    let env = Env::new();
    let remote = gittest::new_bare_remote();
    env.vault_with_origin(remote.path().to_str().unwrap());
    let r = env.auth(&["auth", "login"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("local path"), "{}", r.stderr);
    assert!(!env.dirs.state.join("tokens").exists());
}

#[test]
fn auth_status_with_no_tokens_and_no_vault_says_so() {
    let env = Env::new();
    let r = env.ok(&["auth", "status"]);
    assert_eq!(r.stdout, "gage: no tokens are stored on this machine\n");
}

#[test]
fn auth_status_with_no_tokens_falls_back_to_the_current_vaults_host() {
    let env = Env::new();
    env.vault_with_origin("https://git.example.com/me/vault.git");
    let r = env.ok(&["auth", "status"]);
    for want in ["git.example.com", "no token", "gage auth login"] {
        assert!(r.stdout.contains(want), "{want}: {}", r.stdout);
    }
}

// A
// `git remote set-url` run outside gage moves the real origin without
// touching the global config; the repository is the authority.
#[test]
fn auth_login_reads_the_origin_from_the_repository_when_config_lags() {
    let env = Env::new();
    let path = env.init("personal", &[]);
    gitrepo::set_remote(&path, "https://git.example.com/me/vault.git").unwrap();
    assert_eq!(env.entry("personal").git.origin, None, "config lags");

    let r = env.auth(&["auth", "login"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("git.example.com"), "{}", r.stdout);
}

#[test]
fn auth_login_on_a_vault_with_no_remote_names_the_vault() {
    let env = Env::new();
    env.init("personal", &[]);
    let r = env.auth(&["auth", "login"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    for want in ["personal", "no remote", "--host"] {
        assert!(r.stderr.contains(want), "{want}: {}", r.stderr);
    }
}

// Auth half: the registry tags them
// both-mode, and they run in a session. The sync/pull/push/clone half is
// M8a2's, which adds those commands.
#[test]
fn auth_commands_work_in_both_modes() {
    let reg = registry();
    for name in ["auth login", "auth status", "auth logout"] {
        let info = reg
            .find(name)
            .unwrap_or_else(|| panic!("{name} unregistered"));
        assert_eq!(info.availability, Availability::Both, "{name}");
    }
    let env = Env::new();
    let r = env.session_ok(&[
        "auth login --host github.com",
        TOKEN,
        "auth status",
        "auth logout --host github.com",
        "exit",
    ]);
    assert!(
        r.stdout.contains("stored a token for github.com"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("github.com  configured"), "{}", r.stdout);
    assert!(r.stdout.contains("forgot the token"), "{}", r.stdout);
}

// Stdout, stderr and the
// session's history file. The token is prompted, never an argument, so no
// command line carries it to a history file in the first place.
#[test]
fn no_token_value_ever_appears_in_output() {
    let env = Env::new();
    for args in [
        &["auth", "login", "--host", "github.com"][..],
        &["auth", "status", "--host", "github.com"],
        &["auth", "status"],
        &["auth", "logout", "--host", "github.com"],
        &["auth", "status", "--host", "github.com"],
    ] {
        let r = env.auth(args);
        assert!(!r.stdout.contains(TOKEN), "{args:?}: {}", r.stdout);
        assert!(!r.stderr.contains(TOKEN), "{args:?}: {}", r.stderr);
    }
    let r = env.session_ok(&["auth login --host github.com", TOKEN, "auth status", "exit"]);
    assert!(!r.stdout.contains(TOKEN) && !r.stderr.contains(TOKEN));
    let history = std::fs::read_to_string(env.dirs.state.join("history")).unwrap();
    assert!(history.contains("auth login"), "{history}");
    assert!(!history.contains(TOKEN), "{history}");

    let tree = crate::cli::build(&registry(), &crate::BuildInfo::stamped());
    let auth = tree.find_subcommand("auth").unwrap();
    for sub in auth.get_subcommands() {
        assert!(
            sub.get_positionals().next().is_none(),
            "auth {} takes an argument; the token must be prompted",
            sub.get_name()
        );
    }
}

// Asserted at the
// source: a real expiry needs a server to refuse us. The library's wire
// tests (`push_after_logout_names_the_host_and_the_command`) prove the hint
// reaches a real refusal.
#[test]
fn expired_token_error_names_the_host_and_the_command() {
    let hint = remoteauth::auth_hint("github.com");
    for want in ["github.com", "gage auth login", "expired"] {
        assert!(hint.contains(want), "{want}: {hint}");
    }
}

// Auth's failures.
#[test]
fn no_auth_failure_doubles_gages_own_prefix() {
    let env = Env::new();
    let doubled = format!("{0}{0}", crate::error::ERROR_PREFIX);
    let failing: &[&[&str]] = &[
        &["auth", "login"],
        &["auth", "logout"],
        &["auth", "login", "--host", "../x"],
        &["auth", "logout", "--host", "a/b"],
        &["auth", "status", "extra"],
    ];
    for args in failing {
        let r = env.auth(args);
        assert_ne!(r.code, Code::Success, "{args:?}");
        assert!(r.stderr.starts_with("gage: "), "{args:?}: {}", r.stderr);
        assert!(!r.stderr.contains(&doubled), "{args:?}: {}", r.stderr);
    }
    // An empty token, from the prompter.
    let r = env.run_with(
        &["auth", "login", "--host", "github.com"],
        &FakePrompter::new().with_value("  "),
    );
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(!r.stderr.contains(&doubled), "{}", r.stderr);
}
