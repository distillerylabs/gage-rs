//! Everything that talks to a remote or compares two histories: fetch,
//! push, clone, and the four-way comparison every sync decision is made
//! from. Nothing here runs on its own; `Vault` decides when (M8a2).

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::{Duration, Instant};

use super::{Error, ORIGIN, fail, isolate, open, require_clean_work_tree};
use crate::remoteauth::{self, Method};
use crate::syncerr::{self, Kind};
use crate::xdgpaths::Dirs;

/// How long one fetch, push or clone may take before it is abandoned as
/// unreachable (Q-REMOTE-TIMEOUT). libgit2's connect and read timeouts are
/// set to it once, in [`isolate`]; the whole operation is bounded by a
/// deadline its progress callbacks check.
pub const REMOTE_OP_TIMEOUT: Duration = Duration::from_secs(30);

/// What a remote operation needs beyond the repository: where tokens are
/// stored, and how long it may take. Every operation starts its own
/// deadline from `timeout`.
#[derive(Debug, Clone)]
pub struct RemoteOptions {
    dirs: Dirs,
    timeout: Duration,
}

impl RemoteOptions {
    pub fn new(dirs: &Dirs) -> RemoteOptions {
        RemoteOptions {
            dirs: dirs.clone(),
            timeout: REMOTE_OP_TIMEOUT,
        }
    }

    /// The same options with another per-operation timeout. Tests use
    /// zero to prove the deadline aborts, never real time.
    pub fn with_timeout(mut self, timeout: Duration) -> RemoteOptions {
        self.timeout = timeout;
        self
    }
}

/// How the local branch relates to the last known state of its remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteState {
    /// No remote-tracking ref to compare against: no origin, or nothing
    /// ever fetched. A local-only vault sits here permanently.
    None,
    /// Both sides are on the same commit.
    InSync,
    /// The local branch has commits the remote lacks: a push fast-forwards.
    Ahead,
    /// The remote has commits the local branch lacks: a pull
    /// fast-forwards.
    Behind,
    /// Both sides have commits the other lacks.
    Diverged,
}

impl std::fmt::Display for RemoteState {
    /// The words sync messages and tests refer to a state by.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RemoteState::None => "no-remote",
            RemoteState::InSync => "in-sync",
            RemoteState::Ahead => "ahead",
            RemoteState::Behind => "behind",
            RemoteState::Diverged => "diverged",
        })
    }
}

/// Updates the remote-tracking refs from origin with the default refspec,
/// reporting whether anything moved, so a caller can tell "caught up" from
/// "already current". Every remote failure comes back as
/// [`Error::Sync`] (unreachable or auth); anything else is passed through.
pub fn fetch(dir: &Path, opts: &RemoteOptions) -> Result<bool, Error> {
    let repo = open(dir)?;
    let mut remote = repo
        .find_remote(ORIGIN)
        .map_err(fail("reading origin in", dir))?;
    let url = remote.url().unwrap_or_default().to_string();
    let method = remoteauth::method(&opts.dirs, &url)?;
    let op = Op::new(&method, &url, opts.timeout);
    let mut fetch = git2::FetchOptions::new();
    fetch
        .remote_callbacks(op.callbacks())
        .follow_redirects(git2::RemoteRedirect::Initial)
        .download_tags(git2::AutotagOption::None);
    match remote.fetch::<&str>(&[], Some(&mut fetch), None) {
        Ok(()) => Ok(op.updated.get()),
        Err(e) => Err(op.fail(e, Direction::Fetch, "fetching origin into", dir)),
    }
}

/// Publishes the current branch to the same branch on origin, reporting
/// whether anything was sent. A remote that has moved on is
/// [`Error::Sync`] with [`Kind::Diverged`], and so is a ref the server
/// refused: libgit2 reports that through `push_update_reference` while
/// `push` itself can succeed, so a refused push is never reported as one
/// that landed.
pub fn push(dir: &Path, opts: &RemoteOptions) -> Result<bool, Error> {
    let repo = open(dir)?;
    let head = repo.head().map_err(fail("reading HEAD of", dir))?;
    let branch = head.name().unwrap_or_default().to_string();
    let mut remote = repo
        .find_remote(ORIGIN)
        .map_err(fail("reading origin in", dir))?;
    let url = remote.url().unwrap_or_default().to_string();
    let method = remoteauth::method(&opts.dirs, &url)?;
    let op = Op::new(&method, &url, opts.timeout);
    let mut push = git2::PushOptions::new();
    push.remote_callbacks(op.callbacks())
        .follow_redirects(git2::RemoteRedirect::Initial);
    if let Err(e) = remote.push(&[format!("{branch}:{branch}")], Some(&mut push)) {
        return Err(op.fail(e, Direction::Push, "pushing", dir));
    }
    if let Some((refname, status)) = op.rejected.borrow().first() {
        return Err(syncerr::Error::new(
            Kind::Diverged,
            format!("the remote refused {refname}: {status}"),
        )
        .into());
    }
    Ok(op.sent.get())
}

/// Clones `url` into `dir`: `gage clone`'s git half.
///
/// The branch is chosen explicitly rather than by following the remote's
/// HEAD, which routinely points nowhere: a repository made with `git init
/// --bare` names `main` before any commit exists, whatever branch is later
/// pushed into it. Asking the remote what it has costs one round trip on a
/// once-per-vault operation.
pub fn clone(url: &str, dir: &Path, opts: &RemoteOptions) -> Result<(), Error> {
    isolate();
    let method = remoteauth::method(&opts.dirs, url)?;
    let branch = remote_branch(url, &method, opts)?;
    let op = Op::new(&method, url, opts.timeout);
    let mut fetch = git2::FetchOptions::new();
    fetch
        .remote_callbacks(op.callbacks())
        .follow_redirects(git2::RemoteRedirect::Initial)
        .download_tags(git2::AutotagOption::None);
    git2::build::RepoBuilder::new()
        .branch(&branch)
        .fetch_options(fetch)
        .clone(url, dir)
        .map(drop)
        .map_err(|e| op.fail(e, Direction::Fetch, "cloning into", dir))
}

/// Which branch of `url` to clone: the only one, else `main`, else
/// `master`. Anything else is refused by name rather than guessed at:
/// picking the wrong branch of a secrets vault would silently produce a
/// vault missing entries.
pub(crate) fn remote_branch(
    url: &str,
    method: &Method,
    opts: &RemoteOptions,
) -> Result<String, Error> {
    isolate();
    let op = Op::new(method, url, opts.timeout);
    let classify = |e| {
        op.fail(
            e,
            Direction::Fetch,
            "listing the branches of",
            Path::new(url),
        )
    };
    let mut remote = git2::Remote::create_detached(url).map_err(classify)?;
    let connection = remote
        .connect_auth(git2::Direction::Fetch, Some(op.callbacks()), None)
        .map_err(classify)?;
    let mut heads: Vec<String> = connection
        .list()
        .map_err(classify)?
        .iter()
        .filter_map(|h| h.name().strip_prefix("refs/heads/").map(str::to_string))
        .collect();
    heads.sort();
    match heads.len() {
        0 => Err(Error::NoBranches {
            url: url.to_string(),
        }),
        1 => Ok(heads.remove(0)),
        _ => ["main", "master"]
            .into_iter()
            .find(|preferred| heads.iter().any(|h| h == preferred))
            .map(str::to_string)
            .ok_or(Error::AmbiguousBranch {
                url: url.to_string(),
                branches: heads,
            }),
    }
}

/// How the current branch relates to its remote-tracking ref. No network:
/// it answers from what the last fetch left, which is what makes "fetch,
/// then decide" two separable steps.
pub fn compare(dir: &Path) -> Result<RemoteState, Error> {
    let repo = open(dir)?;
    let (local, Some(remote)) = head_and_tracking(&repo, dir)? else {
        return Ok(RemoteState::None);
    };
    let (ahead, behind) = repo
        .graph_ahead_behind(local, remote)
        .map_err(fail("comparing with the remote in", dir))?;
    Ok(match (ahead, behind) {
        (0, 0) => RemoteState::InSync,
        (_, 0) => RemoteState::Ahead,
        (0, _) => RemoteState::Behind,
        _ => RemoteState::Diverged,
    })
}

/// How many local commits the remote doesn't have: the "2 local commits
/// pending" of a divergence message. Nothing fetched yet reports 0, since
/// "pending" means nothing without a remote to be pending against.
///
/// It counts what the remote-tracking ref can't reach, by hiding that ref
/// in the walk. Stopping at the merge-base *commit* instead is wrong once
/// a merge sits between the local head and the base: the walk descends
/// the merge's other parent into shared history and counts the vault's
/// whole history as pending (Go's correction,
/// `TestAheadCountIsNotFooledByAMergeCommit`).
pub fn ahead_count(dir: &Path) -> Result<usize, Error> {
    let repo = open(dir)?;
    let (local, Some(remote)) = head_and_tracking(&repo, dir)? else {
        return Ok(0);
    };
    let mut walk = repo.revwalk().map_err(fail("walking", dir))?;
    walk.push(local)
        .and_then(|()| walk.hide(remote))
        .map_err(fail("walking", dir))?;
    let mut count = 0;
    for id in walk {
        id.map_err(fail("walking", dir))?;
        count += 1;
    }
    Ok(count)
}

/// Moves the local branch to the remote-tracking ref when that is a
/// fast-forward, reporting whether it moved. The only way this module
/// moves HEAD on a pull: any other state leaves everything untouched
/// ("Sync model"'s fast-forward-only rule), and a dirty working tree is
/// refused, since the hard reset would discard someone's editing.
pub fn fast_forward(dir: &Path) -> Result<bool, Error> {
    if compare(dir)? != RemoteState::Behind {
        return Ok(false);
    }
    require_clean_work_tree(dir, "fast-forwarding over them")?;
    let repo = open(dir)?;
    let (_, Some(remote)) = head_and_tracking(&repo, dir)? else {
        return Ok(false);
    };
    let target = repo
        .find_object(remote, Some(git2::ObjectType::Commit))
        .map_err(fail("reading the remote's commit in", dir))?;
    // Hard is right because this is a fast-forward: the local branch is an
    // ancestor of the target, so no local commit is discarded, and the
    // tree check above ruled out uncommitted work.
    repo.reset(&target, git2::ResetType::Hard, None)
        .map_err(fail("fast-forwarding", dir))?;
    Ok(true)
}

/// HEAD's commit, and the commit its remote-tracking ref points at, if
/// anything has been fetched.
pub(super) fn head_and_tracking(
    repo: &git2::Repository,
    dir: &Path,
) -> Result<(git2::Oid, Option<git2::Oid>), Error> {
    let head = repo.head().map_err(fail("reading HEAD of", dir))?;
    let local = head
        .peel_to_commit()
        .map_err(fail("reading HEAD of", dir))?
        .id();
    let branch = head.shorthand().unwrap_or_default();
    match repo.find_reference(&format!("refs/remotes/{ORIGIN}/{branch}")) {
        Ok(tracking) => {
            let remote = tracking
                .peel_to_commit()
                .map_err(fail("reading the remote's commit in", dir))?
                .id();
            Ok((local, Some(remote)))
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok((local, None)),
        Err(e) => Err(fail("reading the remote-tracking ref of", dir)(e)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Fetch,
    Push,
}

/// One remote operation's state, shared by its libgit2 callbacks: the
/// deadline, what auth was offered, and what the transfer observed.
struct Op<'m> {
    method: &'m Method,
    /// The remote's URL, to name a local path that isn't there.
    url: String,
    deadline: Instant,
    timeout: Duration,
    timed_out: Cell<bool>,
    /// Why the credentials callback refused, if it did.
    auth_refused: RefCell<Option<String>>,
    /// Credentials handed out so far (username-only requests not counted).
    offered: Cell<u32>,
    /// Fetch: a remote-tracking ref moved.
    updated: Cell<bool>,
    /// Push: an update changes the remote.
    sent: Cell<bool>,
    /// Push: refs the server refused, with its status.
    rejected: RefCell<Vec<(String, String)>>,
}

impl<'m> Op<'m> {
    fn new(method: &'m Method, url: &str, timeout: Duration) -> Op<'m> {
        Op {
            method,
            url: url.to_string(),
            deadline: Instant::now() + timeout,
            timeout,
            timed_out: Cell::new(false),
            auth_refused: RefCell::new(None),
            offered: Cell::new(0),
            updated: Cell::new(false),
            sent: Cell::new(false),
            rejected: RefCell::new(Vec::new()),
        }
    }

    /// Whether the operation may continue. Past the deadline it records
    /// why, and every callback that asks aborts the operation.
    fn alive(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.timed_out.set(true);
            return false;
        }
        true
    }

    fn refuse(&self, reason: String) -> git2::Error {
        let err = git2::Error::from_str(&reason);
        *self.auth_refused.borrow_mut() = Some(reason);
        err
    }

    /// The callbacks every operation installs. Of the progress callbacks,
    /// only transfer progress, sideband progress and push negotiation can
    /// abort: git2's pack-builder and push-transfer progress callbacks
    /// return nothing, so the deadline is checked where it can act.
    fn callbacks(&self) -> git2::RemoteCallbacks<'_> {
        let mut cb = git2::RemoteCallbacks::new();
        cb.transfer_progress(|_| self.alive());
        cb.sideband_progress(|_| self.alive());
        cb.update_tips(|_, old, new| {
            if old != new {
                self.updated.set(true);
            }
            true
        });
        cb.push_negotiation(|updates| {
            if !self.alive() {
                return Err(git2::Error::from_str(
                    "gage: the remote operation timed out",
                ));
            }
            if updates.iter().any(|u| u.src() != u.dst()) {
                self.sent.set(true);
            }
            Ok(())
        });
        cb.push_update_reference(|refname, status| {
            if let Some(status) = status {
                self.rejected
                    .borrow_mut()
                    .push((refname.to_string(), status.to_string()));
            }
            Ok(())
        });
        // No credentials callback for a local or anonymous remote: libgit2
        // then reports a challenge as the typed `ErrorCode::Auth`.
        if matches!(self.method, Method::Token { .. } | Method::SshAgent { .. }) {
            cb.credentials(|url, username, allowed| self.credentials(url, username, allowed));
        }
        cb
    }

    /// Offers the stored credential once, to its own host only. libgit2
    /// asks again after a rejection; that second request fails, so a
    /// refused token ends the operation as an auth failure instead of a
    /// loop.
    fn credentials(
        &self,
        url: &str,
        username: Option<&str>,
        allowed: git2::CredentialType,
    ) -> Result<git2::Cred, git2::Error> {
        if !self.alive() {
            return Err(git2::Error::from_str(
                "gage: the remote operation timed out",
            ));
        }
        if let Method::SshAgent { user, .. } = self.method {
            let user = username.unwrap_or(user);
            if allowed.contains(git2::CredentialType::USERNAME) {
                return git2::Cred::username(user);
            }
        }
        if let Method::Token { host, .. } = self.method {
            let asking = remoteauth::host(url).unwrap_or_default();
            if !asking.eq_ignore_ascii_case(host) {
                return Err(self.refuse(format!(
                    "{asking} asked for credentials; gage offers the token stored for {host} only to {host}"
                )));
            }
        }
        if self.offered.get() > 0 {
            return Err(self.refuse(format!(
                "{} rejected the credentials gage offered",
                self.method.host()
            )));
        }
        self.offered.set(1);
        match self.method {
            Method::Token { host, token } => {
                if !allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                    return Err(
                        self.refuse(format!("{host} asked for credentials other than a token"))
                    );
                }
                // Hosts ignore the username when the password is a token,
                // but it can't be empty.
                git2::Cred::userpass_plaintext("gage", token)
            }
            Method::SshAgent { user, .. } => {
                git2::Cred::ssh_key_from_agent(username.unwrap_or(user))
            }
            Method::Local | Method::Anonymous { .. } => {
                Err(self.refuse("gage has no credentials for this remote".into()))
            }
        }
    }

    /// Turns a failed operation into gage's error: its own deadline and
    /// credential refusals first, then libgit2's error by class and code.
    /// An auth failure gains the host-naming hint.
    fn fail(&self, err: git2::Error, direction: Direction, op: &'static str, dir: &Path) -> Error {
        let classified = if self.timed_out.get() {
            Some(syncerr::Error::new(
                Kind::Unreachable,
                format!("no answer within {}s", self.timeout.as_secs()),
            ))
        } else if let Some(reason) = self.auth_refused.borrow().clone() {
            Some(syncerr::Error::new(Kind::Auth, reason))
        } else if *self.method == Method::Local && !Path::new(&self.url).exists() {
            // libgit2 calls a missing path an unsupported URL protocol.
            Some(syncerr::Error::new(
                Kind::Unreachable,
                format!("{} does not exist (a missing or unmounted path?)", self.url),
            ))
        } else if direction == Direction::Push {
            Some(syncerr::classify_push(&err))
        } else {
            syncerr::classify_fetch(&err)
        };
        match classified {
            Some(e) => e
                .annotate_auth(remoteauth::auth_hint(self.method.host()))
                .into(),
            None => fail(op, dir)(err),
        }
    }
}
