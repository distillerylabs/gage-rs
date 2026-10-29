//! `[shell]` from global config, resolved for a session: the prompt
//! template and its tokens, the idle timeout, and where history lives.
//! CLI-terminal policy a GUI frontend wouldn't share, so it lives here
//! rather than in the library (`config::Shell` is only the raw table).

use std::path::PathBuf;
use std::time::Duration;

use crate::exitcode::ExitCode;
use gage::config::{self, Shell};
use gage::vaultconfig::VaultType;
use gage::xdgpaths::Dirs;
use gage::{gitrepo, paths};

use crate::duration;
use crate::error::CliError;

/// Renders as a bare `gage> ` until a vault is selected and
/// `personal🔓 gage> ` once one is, as in the design doc's transcript. The
/// tokens collapsing to nothing is why [`render_prompt`] trims leading
/// spaces rather than the template carrying two spellings of itself.
pub const DEFAULT_PROMPT: &str = "{vault}{lock} gage> ";

/// The design doc's own `[shell]` example. A re-lock, not a logout: the
/// process stays and the next command re-prompts.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long `-c` leaves a secret on the clipboard. pass's long-standing
/// default (`PASSWORD_STORE_CLIP_TIME`), so the number people coming from
/// it already have a feel for.
pub const DEFAULT_CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(45);

/// Below a second the clear races the paste it exists to allow, so a
/// shorter `clipboard_timeout` is a config error, not a brisk setting.
pub const MIN_CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(1);

pub const UNLOCKED_GLYPH: &str = "🔓";
pub const LOCKED_GLYPH: &str = "🔒";

/// `[shell]`, resolved: defaults filled in, the timeouts parsed, the
/// history path expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSettings {
    pub prompt: String,
    /// `None` disables the idle re-lock.
    pub idle_timeout: Option<Duration>,
    pub history_file: PathBuf,
    pub clipboard_timeout: Duration,
}

/// Resolves the raw table. A malformed `idle_timeout` is a usage error,
/// not an ignored field: a typo'd `10min` that quietly meant "never
/// re-lock" would weaken the protection the setting exists for. `home`
/// expands a leading `~`, and is only asked when there is one.
pub fn resolve(
    shell: &Shell,
    dirs: &Dirs,
    home: &dyn Fn() -> Option<PathBuf>,
) -> Result<ShellSettings, CliError> {
    let prompt = shell
        .prompt
        .clone()
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let idle_timeout = match &shell.idle_timeout {
        None => Some(DEFAULT_IDLE_TIMEOUT),
        Some(raw) => match duration::parse(raw) {
            Ok(d) if d.is_zero() => None,
            Ok(d) => Some(d),
            Err(duration::DurationError::Negative) => {
                return Err(CliError::new(
                    ExitCode::Usage,
                    format!(
                        "[shell].idle_timeout {raw:?} is negative; use \"0\" to disable the idle re-lock"
                    ),
                ));
            }
            Err(duration::DurationError::Malformed) => {
                return Err(CliError::new(
                    ExitCode::Usage,
                    format!("[shell].idle_timeout {raw:?} is not a duration (e.g. \"10m\")"),
                ));
            }
        },
    };
    // Refused rather than clamped, and with no "0 disables it": a clear
    // that silently never fired, or one rounded up from what the human
    // chose, would weaken the one guarantee -c makes. Turning the clear
    // off is left un-takeable; adding it later is additive.
    let clipboard_timeout = match &shell.clipboard_timeout {
        None => DEFAULT_CLIPBOARD_TIMEOUT,
        Some(raw) => match duration::parse(raw) {
            Ok(d) if d >= MIN_CLIPBOARD_TIMEOUT => d,
            Ok(_) | Err(duration::DurationError::Negative) => {
                return Err(CliError::new(
                    ExitCode::Usage,
                    format!(
                        "[shell].clipboard_timeout {raw:?} is below the {} minimum",
                        duration::format(MIN_CLIPBOARD_TIMEOUT)
                    ),
                ));
            }
            Err(duration::DurationError::Malformed) => {
                return Err(CliError::new(
                    ExitCode::Usage,
                    format!("[shell].clipboard_timeout {raw:?} is not a duration (e.g. \"45s\")"),
                ));
            }
        },
    };
    // The design doc writes the path with `$GAGE_STATE` in it, so the
    // default and a configured value go through the same expansion.
    let history_file = match &shell.history_file {
        None => dirs.state.join("history"),
        Some(path) => expand_gage_path(path, dirs, home)?,
    };
    Ok(ShellSettings {
        prompt,
        idle_timeout,
        history_file,
        clipboard_timeout,
    })
}

/// Expands the `$GAGE_CONFIG`, `$GAGE_DATA` and `$GAGE_STATE` names the
/// design doc uses for gage's three roots, then a leading `~` (alone, or
/// followed by a separator). A `~` anywhere else is part of a file name.
pub fn expand_gage_path(
    path: &str,
    dirs: &Dirs,
    home: &dyn Fn() -> Option<PathBuf>,
) -> Result<PathBuf, CliError> {
    let mut path = path.to_string();
    for (token, dir) in [
        ("$GAGE_CONFIG", &dirs.config),
        ("$GAGE_DATA", &dirs.data),
        ("$GAGE_STATE", &dirs.state),
    ] {
        if !path.contains(token) {
            continue;
        }
        let dir = dir.to_str().ok_or_else(|| {
            CliError::new(
                ExitCode::Internal,
                format!("{token} ({}) is not valid UTF-8", dir.display()),
            )
        })?;
        path = path.replace(token, dir);
    }
    let rest = match path.strip_prefix('~') {
        Some("") => Some(""),
        Some(rest) if rest.starts_with('/') || rest.starts_with(std::path::MAIN_SEPARATOR) => {
            Some(rest.trim_start_matches(['/', std::path::MAIN_SEPARATOR]))
        }
        _ => None,
    };
    match rest {
        None => Ok(PathBuf::from(path)),
        Some(rest) => {
            let home = home().ok_or_else(|| {
                CliError::new(ExitCode::Internal, "resolving ~: no home directory")
            })?;
            Ok(if rest.is_empty() {
                home
            } else {
                home.join(rest)
            })
        }
    }
}

/// Everything the prompt template can show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptState {
    /// The current vault, if any.
    pub vault: Option<String>,
    pub unlocked: bool,
    pub dirty: bool,
}

/// Substitutes `{vault}`, `{lock}` and `{dirty}`. `{lock}` renders only
/// when a vault is current. Leading spaces are trimmed afterwards, so the
/// default template reads as a bare `gage> ` with no vault, while a
/// template with literal text before its tokens is unaffected.
pub fn render_prompt(template: &str, st: &PromptState) -> String {
    let lock = match (&st.vault, st.unlocked) {
        (None, _) => "",
        (Some(_), true) => UNLOCKED_GLYPH,
        (Some(_), false) => LOCKED_GLYPH,
    };
    let dirty = if st.dirty { "*" } else { "" };
    template
        .replace("{vault}", st.vault.as_deref().unwrap_or(""))
        .replace("{lock}", lock)
        .replace("{dirty}", dirty)
        .trim_start_matches(' ')
        .to_string()
}

/// Whether a template shows `{dirty}`, so the prompt only pays for a git
/// status when it has somewhere to show one.
pub fn wants_dirty(template: &str) -> bool {
    template.contains("{dirty}")
}

/// The `{dirty}` token's answer. Best-effort: a vault that isn't
/// registered, isn't git, or can't be read renders clean, since a prompt
/// has nowhere to report an error and the next real command will.
pub fn vault_is_dirty(name: &str, dirs: &Dirs) -> bool {
    let Ok(global) = config::load(&paths::global_config_file(dirs)) else {
        return false;
    };
    let Ok(entry) = global.entry(name) else {
        return false;
    };
    match entry.vault_type {
        VaultType::Git => gitrepo::is_clean(&entry.path).is_ok_and(|clean| !clean),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::history::History;
    use crate::lifecycle_tests::Env;

    fn dirs_in(root: &Path) -> Dirs {
        Dirs {
            config: root.join("config"),
            data: root.join("data"),
            state: root.join("state"),
        }
    }

    fn no_home() -> Option<PathBuf> {
        panic!("home asked for a path with no ~")
    }

    fn resolve_with(shell: Shell) -> Result<ShellSettings, CliError> {
        let root = tempfile::tempdir().unwrap();
        resolve(&shell, &dirs_in(root.path()), &no_home)
    }

    #[test]
    fn resolve_shell_settings_defaults() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let got = resolve(&Shell::default(), &dirs, &no_home).unwrap();
        assert_eq!(got.prompt, DEFAULT_PROMPT);
        assert_eq!(got.idle_timeout, Some(DEFAULT_IDLE_TIMEOUT));
        assert_eq!(got.idle_timeout, Some(Duration::from_secs(600)));
        assert_eq!(got.history_file, dirs.state.join("history"));
    }

    #[test]
    fn resolve_shell_settings_parses_idle_timeout() {
        let got = resolve_with(Shell {
            idle_timeout: Some("45s".into()),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.idle_timeout, Some(Duration::from_secs(45)));
    }

    // A typo must
    // not silently mean "never re-lock".
    #[test]
    fn resolve_shell_settings_rejects_a_malformed_idle_timeout() {
        for bad in ["10min", "later", "-5m"] {
            let err = resolve_with(Shell {
                idle_timeout: Some(bad.into()),
                ..Shell::default()
            })
            .unwrap_err();
            assert_eq!(err.code(), ExitCode::Usage, "{bad}: {err}");
            assert!(err.to_string().contains("idle_timeout"), "{err}");
        }
    }

    // The default, a
    // configured value, and a malformed one. There is no "0 disables it":
    // anything under the minimum is refused, not clamped.
    #[test]
    fn resolve_shell_settings_clipboard_timeout() {
        let got = resolve_with(Shell::default()).unwrap();
        assert_eq!(got.clipboard_timeout, DEFAULT_CLIPBOARD_TIMEOUT);
        assert_eq!(got.clipboard_timeout, Duration::from_secs(45));

        let got = resolve_with(Shell {
            clipboard_timeout: Some("90s".into()),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.clipboard_timeout, Duration::from_secs(90));

        let got = resolve_with(Shell {
            clipboard_timeout: Some("1s".into()),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.clipboard_timeout, MIN_CLIPBOARD_TIMEOUT);

        for bad in ["45sec", "200ms", "0", "-5s"] {
            let err = resolve_with(Shell {
                clipboard_timeout: Some(bad.into()),
                ..Shell::default()
            })
            .unwrap_err();
            assert_eq!(err.code(), ExitCode::Usage, "{bad}: {err}");
            assert!(err.to_string().contains("clipboard_timeout"), "{err}");
        }
    }

    // "0" turns it off, distinct
    // from leaving the field unset (the default).
    #[test]
    fn zero_idle_timeout_disables_the_relock() {
        let got = resolve_with(Shell {
            idle_timeout: Some("0".into()),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.idle_timeout, None);
    }

    // The design doc writes
    // `$GAGE_STATE/history`, so the names resolve rather than becoming a
    // directory literally called `$GAGE_STATE`.
    #[test]
    fn history_path_expands_gage_roots() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let got = resolve(
            &Shell {
                history_file: Some("$GAGE_STATE/gage-history".into()),
                ..Shell::default()
            },
            &dirs,
            &no_home,
        )
        .unwrap();
        assert_eq!(got.history_file, dirs.state.join("gage-history"));
        for (token, dir) in [("$GAGE_CONFIG", &dirs.config), ("$GAGE_DATA", &dirs.data)] {
            let path = expand_gage_path(&format!("{token}/h"), &dirs, &no_home).unwrap();
            assert_eq!(path, dir.join("h"));
        }
    }

    // A named path is used
    // exactly, and opening it creates the directory it's in.
    #[test]
    fn history_file_honours_an_absolute_path() {
        let elsewhere = tempfile::tempdir().unwrap();
        let want = elsewhere.path().join("elsewhere").join("history");
        let got = resolve_with(Shell {
            history_file: Some(want.to_str().unwrap().into()),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.history_file, want);

        let mut h = History::open(Some(&got.history_file)).unwrap();
        h.add("show something").unwrap();
        drop(h);
        let reread = History::open(Some(&want)).unwrap();
        assert_eq!(reread.lines(), ["show something"]);
    }

    // A no-op history is a usable one,
    // so a session can carry on when the file can't be opened.
    #[test]
    fn history_disabled_without_a_path() {
        let mut h = History::open(None).unwrap();
        h.add("show something").unwrap();
        assert!(h.lines().is_empty());
    }

    // Home is injected rather than set
    // through $HOME, so the test touches no process state.
    #[test]
    fn expand_gage_path_expands_tilde() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let home = root.path().join("home");
        let home_fn = || Some(home.clone());

        assert_eq!(expand_gage_path("~", &dirs, &home_fn).unwrap(), home);
        assert_eq!(
            expand_gage_path("~/notes/history", &dirs, &home_fn).unwrap(),
            home.join("notes").join("history")
        );
        // Only a leading ~ means home; "a~b" and "~user" are names.
        let inside = Path::new("some").join("a~b");
        assert_eq!(
            expand_gage_path(inside.to_str().unwrap(), &dirs, &no_home).unwrap(),
            inside
        );
        assert_eq!(
            expand_gage_path("~user/x", &dirs, &no_home).unwrap(),
            PathBuf::from("~user/x")
        );
        let err = expand_gage_path("~/x", &dirs, &|| None).unwrap_err();
        assert_eq!(err.code(), ExitCode::Internal);
    }

    // Anything the {dirty} token can't
    // answer reads as clean.
    #[test]
    fn vault_is_dirty_is_best_effort() {
        let env = Env::new();
        assert!(!vault_is_dirty("never-registered", &env.dirs));

        env.init("personal", &[]);
        assert!(!vault_is_dirty("personal", &env.dirs), "a fresh vault");

        // And the positive case, so the checks above aren't vacuous.
        let path = env.entry("personal").path;
        std::fs::write(path.join("stray.txt"), "x").unwrap();
        assert!(vault_is_dirty("personal", &env.dirs));

        std::fs::remove_dir_all(&path).unwrap();
        assert!(!vault_is_dirty("personal", &env.dirs), "files gone");
    }

    #[test]
    fn render_prompt_tokens() {
        let vault = |unlocked, dirty| PromptState {
            vault: Some("personal".into()),
            unlocked,
            dirty,
        };
        for (template, state, want) in [
            (DEFAULT_PROMPT, PromptState::default(), "gage> ".to_string()),
            (
                DEFAULT_PROMPT,
                vault(true, false),
                format!("personal{UNLOCKED_GLYPH} gage> "),
            ),
            (
                "[{vault}{lock}] gage> ",
                vault(false, false),
                format!("[personal{LOCKED_GLYPH}] gage> "),
            ),
            ("{vault}{dirty}> ", vault(true, true), "personal*> ".into()),
            ("{vault}{dirty}> ", vault(true, false), "personal> ".into()),
        ] {
            assert_eq!(render_prompt(template, &state), want, "{template:?}");
        }
        assert!(wants_dirty("{vault}{dirty}> "));
        assert!(!wants_dirty(DEFAULT_PROMPT));
    }
}
