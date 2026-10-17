//! `[shell]` from global config, resolved for a session: the prompt
//! template and its tokens, the idle timeout, and where history lives.
//! CLI-terminal policy a GUI frontend wouldn't share, so it lives here
//! rather than in the library (`config::Shell` is only the raw table).

use std::path::PathBuf;
use std::time::Duration;

use gage::config::{self, Shell};
use gage::exitcode::Code;
use gage::xdgpaths::Dirs;
use gage::{gitrepo, paths, vault};

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
    let prompt = if shell.prompt.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        shell.prompt.clone()
    };
    let idle_timeout = if shell.idle_timeout.is_empty() {
        Some(DEFAULT_IDLE_TIMEOUT)
    } else {
        let raw = &shell.idle_timeout;
        match parse_duration(raw) {
            Ok(d) if d.is_zero() => None,
            Ok(d) => Some(d),
            Err(DurationError::Negative) => {
                return Err(CliError::new(
                    Code::Usage,
                    format!(
                        "gage: [shell].idle_timeout {raw:?} is negative; use \"0\" to disable the idle re-lock"
                    ),
                ));
            }
            Err(DurationError::Malformed) => {
                return Err(CliError::new(
                    Code::Usage,
                    format!("gage: [shell].idle_timeout {raw:?} is not a duration (e.g. \"10m\")"),
                ));
            }
        }
    };
    // Refused rather than clamped, and with no "0 disables it": a clear
    // that silently never fired, or one rounded up from what the human
    // chose, would weaken the one guarantee -c makes. Turning the clear
    // off is left un-takeable, as in Go; adding it later is additive.
    let clipboard_timeout = if shell.clipboard_timeout.is_empty() {
        DEFAULT_CLIPBOARD_TIMEOUT
    } else {
        let raw = &shell.clipboard_timeout;
        match parse_duration(raw) {
            Ok(d) if d >= MIN_CLIPBOARD_TIMEOUT => d,
            Ok(_) | Err(DurationError::Negative) => {
                return Err(CliError::new(
                    Code::Usage,
                    format!(
                        "gage: [shell].clipboard_timeout {raw:?} is below the {} minimum",
                        go_duration_string(MIN_CLIPBOARD_TIMEOUT)
                    ),
                ));
            }
            Err(DurationError::Malformed) => {
                return Err(CliError::new(
                    Code::Usage,
                    format!(
                        "gage: [shell].clipboard_timeout {raw:?} is not a duration (e.g. \"45s\")"
                    ),
                ));
            }
        }
    };
    // The design doc writes the path with `$GAGE_STATE` in it, so the
    // default and a configured value go through the same expansion.
    let history_file = if shell.history_file.is_empty() {
        dirs.state.join("history")
    } else {
        expand_gage_path(&shell.history_file, dirs, home)?
    };
    Ok(ShellSettings {
        prompt,
        idle_timeout,
        history_file,
        clipboard_timeout,
    })
}

/// A duration as Go's `Duration.String` spells one at second scale and
/// above: `45s`, `1m30s`, `1h0m0s`, `1.5s`.
pub fn go_duration_string(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if h > 0 || m > 0 {
        out.push_str(&format!("{m}m"));
    }
    out.push_str(&s.to_string());
    let nanos = d.subsec_nanos();
    if nanos > 0 {
        let frac = format!("{nanos:09}");
        out.push('.');
        out.push_str(frac.trim_end_matches('0'));
    }
    out.push('s');
    out
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
                Code::Internal,
                format!("gage: {token} ({}) is not valid UTF-8", dir.display()),
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
                CliError::new(Code::Internal, "gage: resolving ~: no home directory")
            })?;
            Ok(if rest.is_empty() {
                home
            } else {
                home.join(rest)
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    Malformed,
    Negative,
}

/// Go's `time.ParseDuration` grammar, which is what the setting is
/// documented in: an optional sign, then one or more `<decimal><unit>`
/// terms (`1h30m`, `1.5h`), with units `ns`, `us`/`µs`, `ms`, `s`, `m`,
/// `h`; or a bare `0`. `humantime` is not used because it also accepts
/// `10min`, which Go rejects.
pub fn parse_duration(s: &str) -> Result<Duration, DurationError> {
    const UNITS: [(&str, u128); 8] = [
        ("ns", 1),
        ("us", 1_000),
        ("µs", 1_000),
        ("μs", 1_000),
        ("ms", 1_000_000),
        ("s", 1_000_000_000),
        ("m", 60_000_000_000),
        ("h", 3_600_000_000_000),
    ];
    let (negative, mut rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.is_empty() {
        return Err(DurationError::Malformed);
    }
    let mut total: u128 = 0;
    while !rest.is_empty() {
        let int_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (int, after) = rest.split_at(int_len);
        let (frac, after) = match after.strip_prefix('.') {
            Some(after) => {
                let len = after.bytes().take_while(u8::is_ascii_digit).count();
                after.split_at(len)
            }
            None => ("", after),
        };
        if int.is_empty() && frac.is_empty() {
            return Err(DurationError::Malformed);
        }
        let unit_len = after
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(unit_len);
        let scale = UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, scale)| *scale)
            .ok_or(DurationError::Malformed)?;
        let whole: u128 = if int.is_empty() {
            0
        } else {
            int.parse().map_err(|_| DurationError::Malformed)?
        };
        let mut term = whole.checked_mul(scale).ok_or(DurationError::Malformed)?;
        // Digits past nanosecond precision change nothing.
        let mut place = scale;
        for digit in frac.bytes() {
            place /= 10;
            term += u128::from(digit - b'0') * place;
        }
        total = total.checked_add(term).ok_or(DurationError::Malformed)?;
        rest = after;
    }
    if negative && total > 0 {
        return Err(DurationError::Negative);
    }
    let secs = u64::try_from(total / 1_000_000_000).map_err(|_| DurationError::Malformed)?;
    Ok(Duration::new(secs, (total % 1_000_000_000) as u32))
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
    if entry.vault_type != vault::TYPE_GIT {
        return false;
    }
    gitrepo::is_clean(&entry.path).is_ok_and(|clean| !clean)
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

    // Go: TestResolveShellSettingsDefaults
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

    // Go: TestResolveShellSettingsParsesIdleTimeout
    #[test]
    fn resolve_shell_settings_parses_idle_timeout() {
        let got = resolve_with(Shell {
            idle_timeout: "45s".into(),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.idle_timeout, Some(Duration::from_secs(45)));
    }

    // Go: TestResolveShellSettingsRejectsAMalformedIdleTimeout. A typo must
    // not silently mean "never re-lock".
    #[test]
    fn resolve_shell_settings_rejects_a_malformed_idle_timeout() {
        for bad in ["10min", "later", "-5m"] {
            let err = resolve_with(Shell {
                idle_timeout: bad.into(),
                ..Shell::default()
            })
            .unwrap_err();
            assert_eq!(err.code(), Code::Usage, "{bad}: {err}");
            assert!(err.to_string().contains("idle_timeout"), "{err}");
        }
    }

    // Go: TestResolveShellSettingsClipboardTimeout. The default, a
    // configured value, and a malformed one. There is no "0 disables it":
    // anything under the minimum is refused, not clamped.
    #[test]
    fn resolve_shell_settings_clipboard_timeout() {
        let got = resolve_with(Shell::default()).unwrap();
        assert_eq!(got.clipboard_timeout, DEFAULT_CLIPBOARD_TIMEOUT);
        assert_eq!(got.clipboard_timeout, Duration::from_secs(45));

        let got = resolve_with(Shell {
            clipboard_timeout: "90s".into(),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.clipboard_timeout, Duration::from_secs(90));

        let got = resolve_with(Shell {
            clipboard_timeout: "1s".into(),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.clipboard_timeout, MIN_CLIPBOARD_TIMEOUT);

        for bad in ["45sec", "200ms", "0", "-5s"] {
            let err = resolve_with(Shell {
                clipboard_timeout: bad.into(),
                ..Shell::default()
            })
            .unwrap_err();
            assert_eq!(err.code(), Code::Usage, "{bad}: {err}");
            assert!(err.to_string().contains("clipboard_timeout"), "{err}");
        }
    }

    // The clipboard notice says how long in Go's spelling, as Go's does.
    #[test]
    fn go_duration_string_matches_gos_spelling() {
        for (d, want) in [
            (Duration::from_secs(45), "45s"),
            (Duration::from_secs(1), "1s"),
            (Duration::from_secs(90), "1m30s"),
            (Duration::from_secs(3600), "1h0m0s"),
            (Duration::from_millis(1500), "1.5s"),
            (Duration::from_millis(61_250), "1m1.25s"),
        ] {
            assert_eq!(go_duration_string(d), want, "{d:?}");
        }
    }

    // Go: TestZeroIdleTimeoutDisablesTheRelock. "0" turns it off, distinct
    // from leaving the field unset (the default).
    #[test]
    fn zero_idle_timeout_disables_the_relock() {
        let got = resolve_with(Shell {
            idle_timeout: "0".into(),
            ..Shell::default()
        })
        .unwrap();
        assert_eq!(got.idle_timeout, None);
    }

    #[test]
    fn durations_follow_gos_grammar() {
        let secs = Duration::from_secs;
        for (input, want) in [
            ("10m", secs(600)),
            ("90s", secs(90)),
            ("1h", secs(3600)),
            ("1h30m", secs(5400)),
            ("1.5h", secs(5400)),
            (".5m", secs(30)),
            ("1.m", secs(60)),
            ("+5m", secs(300)),
            ("-0", Duration::ZERO),
            ("0", Duration::ZERO),
            ("0s", Duration::ZERO),
            ("250ms", Duration::from_millis(250)),
            ("3us", Duration::from_micros(3)),
            ("3µs", Duration::from_micros(3)),
            ("7ns", Duration::from_nanos(7)),
        ] {
            assert_eq!(parse_duration(input), Ok(want), "{input:?}");
        }
        for bad in [
            "", "5", "m", ".", "10min", "1d", "1 h", "h1", "1h-5m", "--5m", "later",
        ] {
            assert_eq!(
                parse_duration(bad),
                Err(DurationError::Malformed),
                "{bad:?}"
            );
        }
        assert_eq!(parse_duration("-5m"), Err(DurationError::Negative));
    }

    // Go: TestHistoryPathExpandsGageRoots. The design doc writes
    // `$GAGE_STATE/history`, so the names resolve rather than becoming a
    // directory literally called `$GAGE_STATE`.
    #[test]
    fn history_path_expands_gage_roots() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let got = resolve(
            &Shell {
                history_file: "$GAGE_STATE/gage-history".into(),
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

    // Go: TestHistoryFileHonoursAnAbsolutePath. A named path is used
    // exactly, and opening it creates the directory it's in.
    #[test]
    fn history_file_honours_an_absolute_path() {
        let elsewhere = tempfile::tempdir().unwrap();
        let want = elsewhere.path().join("elsewhere").join("history");
        let got = resolve_with(Shell {
            history_file: want.to_str().unwrap().into(),
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

    // Go: TestHistoryDisabledWithoutAPath. A no-op history is a usable one,
    // so a session can carry on when the file can't be opened.
    #[test]
    fn history_disabled_without_a_path() {
        let mut h = History::open(None).unwrap();
        h.add("show something").unwrap();
        assert!(h.lines().is_empty());
    }

    // Go: TestExpandGagePathExpandsTilde. Home is injected rather than set
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
        assert_eq!(err.code(), Code::Internal);
    }

    // Go: TestVaultIsDirtyIsBestEffort. Anything the {dirty} token can't
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

    // Go: TestRenderPromptTokens
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
