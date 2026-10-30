//! Sorts the ways a fetch or a push can fail into the three buckets gage
//! behaves differently on: unreachable, auth and diverged
//! (ADR-0017). Neither `Vault` nor `gage-cli` inspects a raw
//! `git2::Error`: they match on [`Kind`], and the classification lives
//! here, in one place.
//!
//! An [`Error`] carries its bucket and a message with any URL credentials
//! already stripped. It never holds the `git2::Error` itself, so nothing
//! downstream can render a libgit2 message that echoes a URL with a token
//! in it. A frontend's fake `RemoteSyncer` builds these directly, without
//! touching git2.
//!
//! # What libgit2 1.9 actually returns
//!
//! Observed on libgit2 1.9.7 (git2 0.21), by the tests in this module and
//! in `gitrepo`, each triggering the case for real:
//!
//! | Case | Class | ErrorKind | Bucket |
//! |---|---|---|---|
//! | loopback port refusing connections | `Net` (`Os` under WinHTTP) | `GenericError` | unreachable |
//! | `.invalid` host (RFC 6761, fails without a network) | `Net` (`Os` under WinHTTP) | `GenericError` | unreachable |
//! | gage's own deadline abort | (none: gage's flag) | | unreachable |
//! | HTTP 401, no credentials offered | `Http` | `Auth` (`GenericError` under WinHTTP) | auth |
//! | HTTP 401 after the stored token, refused again | (none: gage's flag) | | auth |
//! | HTTP 403, HTTP 404 | `Http` | `GenericError` | auth |
//! | local path that doesn't exist | `Net` ("unsupported URL protocol") | `GenericError` | unreachable |
//! | local path that isn't a repository | `Repository` | `GenericError` | auth |
//! | non-fast-forward push | `Reference` | `NotFastForward` | diverged |
//! | ref refused by the server | (`push_update_reference` status) | | diverged |
//!
//! Three of these depart from the plan's table, and are recorded in
//! ADR-0017. libgit2 takes a remote for a local path only when the
//! path exists, so a missing one (an unmounted drive) is unreachable rather
//! than auth; gitrepo names the path in the message. libgit2 types at
//! most a 401, and only in its own HTTP client: the WinHTTP backend
//! (Windows) reports even that one generically. Both backends report every
//! other non-200 status as class `Http` with `GenericError` and the status
//! only in the message text. So the whole
//! `Http` class is auth. That covers 403 and 404 (a repository
//! that isn't there is auth too), and also a 5xx or a page that isn't a
//! git repository, which the message names. Nothing here reads message
//! text.

use crate::error::{ErrorKind, HasKind, Remedy};

/// Which of the three buckets a remote failure is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The remote could not be reached at all: DNS, a refused connection,
    /// a timeout. Says nothing about either side's history, so gage's
    /// automatic sync paths warn once and carry on with the local copy;
    /// only an explicit sync verb surfaces it, as its own exit code.
    Unreachable,
    /// The remote refused access: bad, expired or missing credentials, or
    /// a repository reported as missing, which most hosts answer for a
    /// private repository the caller can't see.
    Auth,
    /// The remote has commits this vault doesn't, so a push can't
    /// fast-forward. What diverged is decided above, after a fetch.
    Diverged,
}

impl Kind {
    fn sentence(self) -> &'static str {
        match self {
            Kind::Unreachable => "could not reach the remote",
            Kind::Auth => "the remote refused access",
            Kind::Diverged => "the remote has diverged from this vault",
        }
    }
}

/// A classified remote failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {detail}{}", kind.sentence(), note.as_deref().map(|h| format!("; {h}")).unwrap_or_default())]
pub struct Error {
    kind: Kind,
    detail: String,
    /// Advice that names no command: what to check about a local path.
    note: Option<String>,
    remedy: Option<Remedy>,
}

impl Error {
    /// A failure already known to be in `kind`'s bucket. `detail` has any
    /// URL credentials stripped before it is kept.
    pub fn new(kind: Kind, detail: impl AsRef<str>) -> Error {
        Error {
            kind,
            detail: redact_userinfo(detail.as_ref()),
            note: None,
            remedy: None,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Gives an auth failure what fixes it: a token for `host`, named so
    /// an expired or absent one doesn't surface as a bare 403, or for a
    /// remote with no host (a local path, where no token exists to be
    /// wrong) a look at the path. Every other bucket is returned
    /// untouched: a divergence must never acquire a "check your token"
    /// suggestion.
    pub fn annotate_auth(mut self, host: &str) -> Error {
        if self.kind != Kind::Auth {
            return self;
        }
        if host.is_empty() {
            self.note =
                Some("check that the repository exists at that path and is readable".into());
        } else {
            self.remedy = Some(Remedy::AuthLogin { host: host.into() });
        }
        self
    }
}

impl HasKind for Error {
    fn kind(&self) -> ErrorKind {
        match self.kind {
            // Deliberately not Conflict: "retry when the network is back"
            // and "a human must reconcile two histories" stay
            // distinguishable to a script, not only to a reader.
            Kind::Unreachable => ErrorKind::Unreachable,
            Kind::Auth => ErrorKind::LockedOrAuth,
            Kind::Diverged => ErrorKind::Conflict,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        self.remedy.clone()
    }
}

/// Classifies a failed push. Unreachable and auth are detected first, then
/// divergence positively (`NotFastForward`); anything left is divergence by
/// elimination. A vault's remote is a dedicated, hook-free
/// repository, so there is no other reason for it to refuse a push.
pub fn classify_push(err: &git2::Error) -> Error {
    // Positively, then by elimination: both land in the same bucket, but
    // the typed rejection is what a push to a vault actually produces.
    classify_fetch(err).unwrap_or_else(|| Error::new(Kind::Diverged, err.message()))
}

/// Classifies a failed fetch into unreachable or auth, or `None` to pass it
/// through. A fetch only adds objects and moves remote-tracking refs, so it
/// is never rejected for divergence, and an unrecognized fetch failure is a
/// broken repository or transport, not a divergence.
pub fn classify_fetch(err: &git2::Error) -> Option<Error> {
    git_kind(err).map(|kind| Error::new(kind, err.message()))
}

/// The bucket a libgit2 error is in, by class and code, never by its
/// message. `None` is a failure in no bucket.
fn git_kind(err: &git2::Error) -> Option<Kind> {
    use git2::{ErrorClass as Class, ErrorCode as C};
    match err.code() {
        C::Auth => return Some(Kind::Auth),
        // A TLS or SSH host key that doesn't check out: nothing was
        // exchanged, so nothing is known about either side's history.
        C::Certificate => return Some(Kind::Unreachable),
        _ => {}
    }
    match err.class() {
        Class::Net | Class::Os | Class::Ssl | Class::Ssh => Some(Kind::Unreachable),
        // See the table above: libgit2 types only a 401.
        Class::Http => Some(Kind::Auth),
        // During a fetch or push the remote is the only repository being
        // opened: a local path with no repository at it. A
        // missing repository is auth, and the hint for a hostless
        // remote says to check the path.
        Class::Repository => Some(Kind::Auth),
        _ => None,
    }
}

/// Removes the `user:password@` part of every URL in `s`. gage never puts a
/// token in a URL, but a remote spelled with one would otherwise carry it
/// into an error message.
fn redact_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| c == '/' || c == '\'' || c == '"' || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(userinfo_end) => out.push_str(&authority[userinfo_end + 1..]),
            None => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
#[path = "syncerr_tests.rs"]
mod tests;
