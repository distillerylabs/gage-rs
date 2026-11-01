//! How gage authenticates to a git remote, and the tokens it does that
//! with (ADR-0020, ADR-0015).
//!
//! The supported path is HTTPS with a token the user issued themselves,
//! held per *host* (two vaults on one host share one token) at
//! `$GAGE_STATE/tokens/<host>`, mode 0600. SSH remotes are best-effort
//! through ssh-agent only: libssh2 never reads `~/.ssh/config`, so a
//! remote that depends on a host alias is refused with the real reason
//! instead of failing later as a confusing connection error.
//!
//! Nothing here touches the network. [`method`] decides what a fetch or
//! push will offer; `gitrepo` offers it through libgit2's `credentials`
//! callback, only to the host the token was stored for.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::error::{ErrorKind, HasKind, Remedy};
use crate::storage::paths;
use crate::storage::xdgpaths::Dirs;

/// The loosest mode a stored token may have, and the mode it is written
/// with.
const TOKEN_FILE_MODE: u32 = 0o600;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The host isn't a single safe path component.
    #[error(transparent)]
    Path(#[from] paths::Error),
    #[error("an empty token is not accepted")]
    EmptyToken,
    #[error("no token is stored for {host}")]
    NoToken { host: String },
    /// A stored token is readable by more than its owner. A token is a
    /// bearer credential, so a loosened mode is refused rather than used,
    /// the posture identity files take.
    #[error(
        "token file permissions are too permissive: {} is {mode:04o}, want 0600",
        path.display()
    )]
    InsecureToken {
        path: PathBuf,
        mode: u32,
        host: String,
    },
    #[error(transparent)]
    Write(#[from] crate::storage::atomicfile::Error),
    #[error("{op} {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse remote {url:?}: {reason}")]
    BadRemote { url: String, reason: String },
    /// An SSH remote whose host only `~/.ssh/config` could resolve.
    #[error(
        "this remote depends on a ~/.ssh/config host alias, which gage does not read: {host:?} is not a resolvable host name (re-spelling the remote with the host's real name also works)"
    )]
    SshConfigAlias { host: String },
    /// An SSH remote on a build without SSH (iOS, Android; ADR-0062).
    #[error(
        "this build of gage has no SSH support, so it cannot use {remote:?}: use the repository's HTTPS URL instead"
    )]
    SshUnsupported { remote: String },
    /// An SSH remote, and no ssh-agent to authenticate through.
    #[error(
        "this remote is SSH-spelled and gage could not reach ssh-agent ({reason}). SSH support is best-effort"
    )]
    SshAgent { reason: String },
}

impl HasKind for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::Path(e) => e.kind(),
            Error::EmptyToken
            | Error::BadRemote { .. }
            | Error::SshConfigAlias { .. }
            | Error::SshUnsupported { .. } => ErrorKind::Usage,
            Error::NoToken { .. } | Error::InsecureToken { .. } | Error::SshAgent { .. } => {
                ErrorKind::LockedOrAuth
            }
            Error::Write(_) | Error::Io { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            Error::InsecureToken { host, .. } => {
                Some(Remedy::AuthLoginAgain { host: host.clone() })
            }
            Error::SshConfigAlias { .. }
            | Error::SshAgent { .. }
            | Error::SshUnsupported { .. } => Some(Remedy::UseHttps),
            Error::Path(e) => e.remedy(),
            _ => None,
        }
    }
}

/// Stores `host`'s token at 0600, creating `$GAGE_STATE/tokens` if this is
/// the first one on this machine. Surrounding whitespace (a pasted
/// newline) is not part of a token.
pub fn save(dirs: &Dirs, host: &str, token: &str) -> Result<(), Error> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::EmptyToken);
    }
    let path = paths::token_file_path(dirs, host)?;
    let dir = paths::tokens_dir(dirs);
    create_private_dir(&dir).map_err(|source| Error::Io {
        op: "creating",
        path: dir.clone(),
        source,
    })?;
    Ok(crate::storage::atomicfile::write(
        &path,
        token.as_bytes(),
        TOKEN_FILE_MODE,
    )?)
}

/// Reads `host`'s token, refusing a file that others can read. On Windows
/// the mode isn't checked: Windows doesn't model those bits, and a file
/// there reads back as writable by everyone whatever its ACL, so enforcing
/// them would refuse every token for a property it can't express. That is
/// how gage's other 0600 checks treat Windows too.
pub fn load(dirs: &Dirs, host: &str) -> Result<Zeroizing<String>, Error> {
    let path = paths::token_file_path(dirs, host)?;
    let io = |op, source| Error::Io {
        op,
        path: path.clone(),
        source,
    };
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NoToken {
                host: host.to_string(),
            });
        }
        Err(e) => return Err(io("reading", e)),
    };
    if let Some(mode) = loose_mode(&meta) {
        return Err(Error::InsecureToken {
            path,
            mode,
            host: host.to_string(),
        });
    }
    let data = Zeroizing::new(std::fs::read(&path).map_err(|e| io("reading", e))?);
    let token = Zeroizing::new(String::from_utf8_lossy(&data).trim().to_string());
    if token.is_empty() {
        return Err(Error::NoToken {
            host: host.to_string(),
        });
    }
    Ok(token)
}

/// Forgets `host`'s token. Forgetting one that isn't there is not an
/// error: `gage auth logout` twice must not fail the second time.
pub fn remove(dirs: &Dirs, host: &str) -> Result<(), Error> {
    let path = paths::token_file_path(dirs, host)?;
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::Io {
            op: "removing",
            path,
            source: e,
        }),
        _ => Ok(()),
    }
}

/// Every host with a stored token, sorted: what `gage auth status` lists.
pub fn hosts(dirs: &Dirs) -> Result<Vec<String>, Error> {
    let dir = paths::tokens_dir(dirs);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                op: "listing",
                path: dir,
                source,
            });
        }
    };
    let mut hosts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            op: "listing",
            path: dir.clone(),
            source,
        })?;
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        let name = entry.file_name().to_string_lossy().into_owned();
        // atomicfile's temp files are dot-prefixed, as is anything a
        // human parks there; neither is a host.
        if is_file && !name.starts_with('.') {
            hosts.push(name);
        }
    }
    hosts.sort();
    Ok(hosts)
}

/// How a remote URL is spelled.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Remote {
    /// A path, or a `file://` URL: no host.
    Local,
    /// `scheme://[user@]host[:port]/path`.
    Url {
        scheme: String,
        host: String,
        user: Option<String>,
    },
    /// scp-like `[user@]host:path`, which git treats as SSH.
    Scp { host: String, user: Option<String> },
}

fn parse(remote: &str) -> Result<Remote, Error> {
    let bad = |reason: &str| Error::BadRemote {
        url: remote.to_string(),
        reason: reason.to_string(),
    };
    if remote.contains("://") {
        let url = url::Url::parse(remote).map_err(|e| bad(&e.to_string()))?;
        if url.scheme() == "file" {
            return Ok(Remote::Local);
        }
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| bad("it names no host"))?;
        return Ok(Remote::Url {
            scheme: url.scheme().to_string(),
            host: host.to_string(),
            user: (!url.username().is_empty()).then(|| url.username().to_string()),
        });
    }
    // git's own rule: a colon before any slash makes it scp-like, except a
    // single letter before it, which is a Windows drive (`C:\vaults\x`).
    if let Some(colon) = remote.find(':') {
        let before = &remote[..colon];
        let is_drive = before.len() == 1 && before.chars().all(|c| c.is_ascii_alphabetic());
        if !before.contains(['/', '\\']) && !is_drive {
            let (user, host) = match before.rsplit_once('@') {
                Some((user, host)) => (Some(user.to_string()), host),
                None => (None, before),
            };
            if host.is_empty() {
                return Err(bad("it names no host"));
            }
            return Ok(Remote::Scp {
                host: host.to_string(),
                user,
            });
        }
    }
    Ok(Remote::Local)
}

/// The host a remote URL names: the key its token is stored under. Both
/// remote spellings work: `https://github.com/u/r.git` and
/// `git@github.com:u/r.git`. A local path (what tests and a USB-drive
/// vault use) has no host and reports `""`. The port is not part of it.
pub fn host(remote: &str) -> Result<String, Error> {
    Ok(match parse(remote)? {
        Remote::Local => String::new(),
        Remote::Url { host, .. } | Remote::Scp { host, .. } => host,
    })
}

/// The host a remote would authenticate a *token* to, or `""` if it isn't
/// a token candidate: a local path has no host, and an SSH remote uses
/// ssh-agent. Mirrors [`method`]'s dispatch without loading anything, so a
/// caller can decide to ask for a token before connecting (`gage init
/// --remote`).
pub fn token_host(remote: &str) -> Result<String, Error> {
    Ok(match parse(remote)? {
        Remote::Url { scheme, host, .. } if is_http(&scheme) => host,
        _ => String::new(),
    })
}

fn is_http(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")
}

/// What a fetch or push offers a remote.
#[derive(Clone, PartialEq, Eq)]
pub enum Method {
    /// A local path: nothing to offer.
    Local,
    /// An HTTP(S) remote with no stored token: an anonymous attempt. A
    /// private remote refusing it produces the auth error with the
    /// `gage auth login` hint, which beats refusing to try.
    Anonymous { host: String },
    /// An HTTP(S) remote with a stored token, offered only to `host`.
    Token {
        host: String,
        token: Zeroizing<String>,
    },
    /// An SSH remote: a key from ssh-agent, as `user`.
    SshAgent { host: String, user: String },
}

impl std::fmt::Debug for Method {
    // Never the token: a Debug print must not be how it leaks.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Method::Local => f.write_str("Local"),
            Method::Anonymous { host } => write!(f, "Anonymous({host})"),
            Method::Token { host, .. } => write!(f, "Token({host}, <redacted>)"),
            Method::SshAgent { host, user } => write!(f, "SshAgent({user}@{host})"),
        }
    }
}

impl Method {
    /// The host this method authenticates to, `""` for a local path.
    pub fn host(&self) -> &str {
        match self {
            Method::Local => "",
            Method::Anonymous { host }
            | Method::Token { host, .. }
            | Method::SshAgent { host, .. } => host,
        }
    }
}

/// Resolves what a fetch or push to `remote` offers, without connecting to
/// anything: a stored token over HTTP(S), ssh-agent over SSH, nothing for a
/// local path.
pub fn method(dirs: &Dirs, remote: &str) -> Result<Method, Error> {
    method_for(dirs, remote, SSH_SUPPORTED)
}

/// Whether this build carries SSH. iOS and Android build git2 without its
/// `ssh` feature (ADR-0062), so an SSH remote is refused up front there.
const SSH_SUPPORTED: bool = !cfg!(any(target_os = "ios", target_os = "android"));

/// [`method`] with the target's SSH support passed in, so tests on any
/// host cover both answers.
fn method_for(dirs: &Dirs, remote: &str, ssh_supported: bool) -> Result<Method, Error> {
    let unsupported = || Error::SshUnsupported {
        remote: remote.to_string(),
    };
    match parse(remote)? {
        Remote::Local => Ok(Method::Local),
        Remote::Url { scheme, .. } if scheme.eq_ignore_ascii_case("ssh") && !ssh_supported => {
            Err(unsupported())
        }
        Remote::Scp { .. } if !ssh_supported => Err(unsupported()),
        Remote::Url { scheme, host, user } if scheme.eq_ignore_ascii_case("ssh") => {
            ssh_method(host, user)
        }
        Remote::Scp { host, user } => ssh_method(host, user),
        Remote::Url { host, .. } => match load(dirs, &host) {
            Ok(token) => Ok(Method::Token { host, token }),
            Err(Error::NoToken { .. }) => Ok(Method::Anonymous { host }),
            Err(e) => Err(e),
        },
    }
}

fn ssh_method(host: String, user: Option<String>) -> Result<Method, Error> {
    if is_ssh_config_alias(&host) {
        return Err(Error::SshConfigAlias { host });
    }
    agent_available()?;
    Ok(Method::SshAgent {
        host,
        user: user.unwrap_or_else(|| "git".into()),
    })
}

/// Whether ssh-agent can be asked at all, without asking it. On Unix that
/// is `SSH_AUTH_SOCK`; on Windows libssh2 finds the OpenSSH agent's pipe or
/// Pageant itself.
fn agent_available() -> Result<(), Error> {
    if cfg!(windows) {
        return Ok(());
    }
    match std::env::var_os("SSH_AUTH_SOCK") {
        Some(sock) if !sock.is_empty() => Ok(()),
        _ => Err(Error::SshAgent {
            reason: "SSH_AUTH_SOCK is not set".into(),
        }),
    }
}

/// Guesses whether an SSH host is a `~/.ssh/config` alias: it has no dot.
/// Every real git host is a fully qualified name (github.com,
/// git.example.internal); an alias is a bare word chosen for typing. A
/// shape check rather than a lookup: resolving would need DNS, and reading
/// `~/.ssh/config` would mean interpreting the file gage promises not to.
/// Being wrong costs a clear message on a remote that would have failed
/// anyway.
fn is_ssh_config_alias(host: &str) -> bool {
    !host.is_empty() && !host.contains('.')
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn loose_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then_some(mode)
}

#[cfg(not(unix))]
fn loose_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
#[path = "remoteauth_tests.rs"]
mod tests;
