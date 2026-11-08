//! The golden format-v2 vault (`tests/fixtures/format-v2/`) through the
//! real CLI, in-process: refused. Format v3 authenticated entries and keeps
//! no unauthenticated read path (ADR-0065), so every command that reads the
//! vault fails with the older-format advice. The global config's schema
//! outlived v2 and still parses. The format is `doc/format/v2.md`; the
//! fixture's constants and install helper are `gage::testsupport::format_v2`.
//! The generator that wrote the fixture is in the history, at the commit
//! that added it; the current format's suite is `format_v3_tests`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::exitcode::ExitCode;
use gage::prompter::FakePrompter;
use gage::testsupport::format_v2::{self as fx, Installed};

use crate::lifecycle_tests::Env;

impl Env {
    /// The machine the fixture captures: its roots are the installed ones.
    fn installed(i: Installed) -> Env {
        let mut env = Env::new();
        env.root = i.root;
        env.dirs = i.dirs;
        env
    }
}

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

/// Every command that opens the vault refuses it before asking for
/// anything, with the advice for an older format.
#[test]
fn format_v2_vault_is_refused_by_every_command() {
    let (env, i) = installed();
    for args in [
        &["ls"][..],
        &["show", fx::MINIMAL.title],
        &["cat", fx::RICH.title],
        &["recipient", "verify"],
        &["recipient", "pending"],
        &["vault", "info", fx::VAULT_NAME],
    ] {
        let p = FakePrompter::new();
        let r = env.run_with(args, &p);
        assert_eq!(r.code, ExitCode::Conflict, "{args:?}: {}", r.stderr);
        assert!(
            r.stderr.contains("format_version 2"),
            "{args:?}: {}",
            r.stderr
        );
        assert!(r.stderr.contains("older format"), "{args:?}: {}", r.stderr);
        assert!(!r.stderr.contains("upgrade gage"), "{args:?}: {}", r.stderr);
        assert!(p.unlock_requests().is_empty(), "{args:?} asked to unlock");
        assert!(r.stdout.is_empty(), "{args:?}: {}", r.stdout);
    }
    assert!(gage::sync::gitrepo::is_clean(&i.vault).unwrap());
}
