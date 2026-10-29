//! Resolves the three directory roots gage owns: config, data and state. On
//! Linux and macOS these follow the XDG Base Directory spec (macOS uses the
//! Unix layout, not `~/Library`); on Windows they map onto the per-user
//! directories Windows already provides. See the design doc's "Global
//! config" section.
//!
//! Resolution is a pure function of (OS, environment, home directory), so
//! every platform's rules are tested on every platform, and so a caller
//! (the CLI's tests above all) can resolve against an injected environment
//! instead of the process's.

use std::path::{Path, PathBuf};

use crate::exitcode::{Code, HasCode};

/// The subdirectory every gage-owned path lives under, inside whichever
/// role root is resolved.
const APP_DIR: &str = "gage";

/// One of the three directory roots gage owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Config,
    Data,
    State,
}

/// Which platform's layout to resolve. macOS is `Unix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Unix,
    Windows,
}

impl Os {
    /// The platform this binary was built for.
    pub const fn current() -> Os {
        if cfg!(windows) { Os::Windows } else { Os::Unix }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("xdgpaths: home directory is empty")]
    EmptyHome,
    #[error("xdgpaths: could not resolve the home directory")]
    NoHome,
}

impl HasCode for Error {
    fn code(&self) -> Code {
        Code::Internal
    }
}

/// Resolves one role root, `<root>/gage`. `env` looks up an environment
/// variable; an empty value counts as unset.
pub fn resolve(
    os: Os,
    env: &dyn Fn(&str) -> Option<String>,
    home: &Path,
    role: Role,
) -> Result<PathBuf, Error> {
    if home.as_os_str().is_empty() {
        return Err(Error::EmptyHome);
    }
    let var = |key: &str| env(key).filter(|v| !v.is_empty());

    // An explicit XDG_* variable wins on any OS, Windows included: see the
    // design doc's "Global config" table.
    if let Some(v) = var(xdg_var(role)) {
        return Ok(Path::new(&v).join(APP_DIR));
    }
    Ok(match os {
        Os::Unix => match role {
            Role::Config => home.join(".config").join(APP_DIR),
            Role::Data => home.join(".local").join("share").join(APP_DIR),
            Role::State => home.join(".local").join("state").join(APP_DIR),
        },
        Os::Windows => {
            let app_data = var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Roaming"));
            let local_app_data = var("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Local"));
            match role {
                Role::Config => app_data.join(APP_DIR),
                Role::Data => local_app_data.join(APP_DIR),
                Role::State => local_app_data.join(APP_DIR).join("state"),
            }
        }
    })
}

fn xdg_var(role: Role) -> &'static str {
    match role {
        Role::Config => "XDG_CONFIG_HOME",
        Role::Data => "XDG_DATA_HOME",
        Role::State => "XDG_STATE_HOME",
    }
}

/// All three resolved roots, each already ending in `gage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
}

impl Dirs {
    pub fn resolve(
        os: Os,
        env: &dyn Fn(&str) -> Option<String>,
        home: &Path,
    ) -> Result<Dirs, Error> {
        Ok(Dirs {
            config: resolve(os, env, home, Role::Config)?,
            data: resolve(os, env, home, Role::Data)?,
            state: resolve(os, env, home, Role::State)?,
        })
    }

    /// The real roots for this process: its OS, environment and home.
    pub fn from_process() -> Result<Dirs, Error> {
        let home = std::env::home_dir().ok_or(Error::NoHome)?;
        Dirs::resolve(Os::current(), &|k| std::env::var(k).ok(), &home)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn dirs(os: Os, pairs: &[(&str, &str)], home: &str) -> Dirs {
        Dirs::resolve(os, &env(pairs), Path::new(home)).unwrap()
    }

    // Linux and macOS share the layout.
    #[test]
    fn unix_defaults() {
        let home = Path::new("/home/testuser");
        let d = dirs(Os::Unix, &[], "/home/testuser");
        assert_eq!(d.config, home.join(".config").join("gage"));
        assert_eq!(d.data, home.join(".local").join("share").join("gage"));
        assert_eq!(d.state, home.join(".local").join("state").join("gage"));
    }

    #[test]
    fn unix_xdg_env_override() {
        let d = dirs(
            Os::Unix,
            &[
                ("XDG_CONFIG_HOME", "/custom/config"),
                ("XDG_DATA_HOME", "/custom/data"),
                ("XDG_STATE_HOME", "/custom/state"),
            ],
            "/home/testuser",
        );
        assert_eq!(d.config, Path::new("/custom/config").join("gage"));
        assert_eq!(d.data, Path::new("/custom/data").join("gage"));
        assert_eq!(d.state, Path::new("/custom/state").join("gage"));
    }

    #[test]
    fn an_empty_xdg_variable_counts_as_unset() {
        let d = dirs(Os::Unix, &[("XDG_CONFIG_HOME", "")], "/home/testuser");
        assert_eq!(d.config, Path::new("/home/testuser/.config").join("gage"));
    }

    #[test]
    fn windows_defaults() {
        let roaming = r"C:\Users\testuser\AppData\Roaming";
        let local = r"C:\Users\testuser\AppData\Local";
        let d = dirs(
            Os::Windows,
            &[("APPDATA", roaming), ("LOCALAPPDATA", local)],
            r"C:\Users\testuser",
        );
        assert_eq!(d.config, Path::new(roaming).join("gage"));
        assert_eq!(d.data, Path::new(local).join("gage"));
        assert_eq!(d.state, Path::new(local).join("gage").join("state"));
    }

    #[test]
    fn windows_missing_env_falls_back_to_home() {
        let home = Path::new(r"C:\Users\testuser");
        let d = dirs(Os::Windows, &[], r"C:\Users\testuser");
        assert_eq!(d.config, home.join("AppData").join("Roaming").join("gage"));
        assert_eq!(d.data, home.join("AppData").join("Local").join("gage"));
        assert_eq!(
            d.state,
            home.join("AppData")
                .join("Local")
                .join("gage")
                .join("state")
        );
    }

    #[test]
    fn windows_xdg_env_override() {
        let d = dirs(
            Os::Windows,
            &[("XDG_CONFIG_HOME", r"D:\dotfiles\config")],
            r"C:\Users\testuser",
        );
        assert_eq!(d.config, Path::new(r"D:\dotfiles\config").join("gage"));
    }

    #[test]
    fn empty_home_dir_is_an_error() {
        let err = resolve(Os::Unix, &env(&[]), Path::new(""), Role::Config).unwrap_err();
        assert!(matches!(err, Error::EmptyHome));
    }

    // A smoke test: the real, current-process
    // roots resolve on whatever platform runs the suite.
    #[test]
    fn real_accessors_resolve() {
        let d = Dirs::from_process().unwrap();
        for p in [&d.config, &d.data, &d.state] {
            assert!(p.is_absolute(), "{}", p.display());
        }
    }

    // An unknown role value is not tested: Role is
    // a closed enum, so an unknown role can't be constructed and the
    // matches above are exhaustive at compile time.
}
