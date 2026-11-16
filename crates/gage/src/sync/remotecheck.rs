//! Testing a remote token before relying on it (#232, ADR-0080): does the
//! host accept it for this repository, and with write access? A frontend
//! asks before saving a token, so a bad one is refused on the screen that
//! took it rather than at the next sync, and before a join, so a token
//! that can read but not push is caught before `enroll`.
//!
//! The check changes nothing: no vault's repository is opened, nothing is
//! fetched or pushed, and a token passed in is offered and dropped, never
//! stored. The token goes in and never comes out (ADR-0063).

use zeroize::Zeroizing;

use crate::error::{ErrorKind, HasKind, Remedy};
use crate::storage::xdgpaths::Dirs;
use crate::sync::gitrepo::{self, RemoteOptions};
use crate::sync::remoteauth::{self, Method};
use crate::vault::Vault;

pub use crate::sync::gitrepo::Access;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckError {
    /// The vault has no remote to check.
    #[error("{vault:?} has no remote to check a token against")]
    NoRemote { vault: String },
    /// A token was passed for a remote that is never offered one: a local
    /// path, SSH, or cleartext `http://` (ADR-0075). `remote` has any
    /// credentials removed.
    #[error("{remote:?} is never sent a token: gage offers one only to an https:// remote")]
    NotTokenRemote { remote: String },
    /// The check itself: a refused token is [`gitrepo::Error::Sync`] with
    /// `syncerr::Kind::Auth`, an absent host `Unreachable`.
    #[error(transparent)]
    Remote(#[from] gitrepo::Error),
}

impl From<remoteauth::Error> for CheckError {
    fn from(err: remoteauth::Error) -> CheckError {
        CheckError::Remote(err.into())
    }
}

impl HasKind for CheckError {
    fn kind(&self) -> ErrorKind {
        match self {
            CheckError::NoRemote { .. } | CheckError::NotTokenRemote { .. } => ErrorKind::Usage,
            CheckError::Remote(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            CheckError::NoRemote { .. } => Some(Remedy::SetRemote),
            CheckError::NotTokenRemote { .. } => None,
            CheckError::Remote(e) => e.remedy(),
        }
    }
}

/// What `url` lets a token do. With `token`, that token is tested in place
/// of any stored for the host, and is not stored; surrounding whitespace is
/// not part of it, as [`remoteauth::save`] has it. Without one, whatever a
/// fetch would offer is: the stored token, or nothing.
pub fn check_remote(dirs: &Dirs, url: &str, token: Option<&str>) -> Result<Access, CheckError> {
    remoteauth::check_new_remote(url)?;
    let method = match token {
        Some(token) => candidate(url, token)?,
        None => remoteauth::method(dirs, url)?,
    };
    Ok(gitrepo::check_access(
        url,
        &method,
        &RemoteOptions::new(dirs),
    )?)
}

/// `token` as the method a fetch to `url` would use if it were stored.
fn candidate(url: &str, token: &str) -> Result<Method, CheckError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(remoteauth::Error::EmptyToken.into());
    }
    let host = remoteauth::token_host(url)?;
    if host.is_empty() {
        return Err(CheckError::NotTokenRemote {
            remote: remoteauth::redacted(url),
        });
    }
    Ok(Method::Token {
        host,
        token: Zeroizing::new(token.to_string()),
    })
}

impl Vault {
    /// [`check_remote`] against this vault's origin. Takes no lock and
    /// reads nothing but the origin's URL, so it can run beside a write.
    pub fn check_remote(&self, token: Option<&str>) -> Result<Access, CheckError> {
        let url = gitrepo::remote_url(&self.path)?.ok_or_else(|| CheckError::NoRemote {
            vault: self.name.clone(),
        })?;
        check_remote(self.dirs(), &url, token)
    }
}

#[cfg(test)]
#[path = "remotecheck_tests.rs"]
mod tests;
