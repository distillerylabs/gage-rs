//! The small slice of git a vault needs: init plus the first commit, the
//! `origin` remote, clean/dirty state, and (from P4) fetch, push, compare,
//! fast-forward, file-level merge, reset and clone. Everything goes through
//! the git crate in-process (Q-GIT-CRATE); gage never shells out to a `git`
//! binary and has no generic passthrough. See "Git-specific commands" in
//! the design doc.

use std::path::{Component, Path, PathBuf};
use std::sync::Once;

use crate::exitcode::{Code, HasCode};
use crate::{remoteauth, syncerr};

mod history;
mod merge;
mod remote;

pub use history::{Revision, log};
pub use merge::{MergeResult, MergeSide, PendingMerge, merge_remote, prepare_merge};
pub use remote::{
    REMOTE_OP_TIMEOUT, RemoteOptions, RemoteState, ahead_count, clone, compare, fast_forward,
    fetch, push,
};

/// The one remote name a git-type vault uses.
pub const ORIGIN: &str = "origin";

/// The branch a new vault starts on. Set explicitly so the user's
/// `init.defaultBranch` never applies; Q-COMPAT frees it from go-git's
/// `master`.
pub const INITIAL_BRANCH: &str = "main";

/// gage's own commit identity, never read from the user's git config,
/// which may not exist and would make a vault's history depend on the
/// machine that wrote it.
pub const AUTHOR_NAME: &str = "gage";
pub const AUTHOR_EMAIL: &str = "gage@localhost";

/// How many paths a dirty-tree refusal names. A vault mid-restore can have
/// thousands, and an error message is a sentence, not a report.
pub const MAX_NAMED_DIRTY_PATHS: usize = 5;

/// What went wrong, as variants a caller can match: the layer above turns
/// a dirty tree or a classified remote failure into its own exit code and
/// message, and everything else into an internal fault.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A libgit2 call failed for a reason in none of the variants below.
    #[error("gitrepo: {op} {path}: {source}")]
    Git {
        op: &'static str,
        path: PathBuf,
        source: git2::Error,
    },
    /// The working tree has uncommitted changes, so an operation that would
    /// move it declined. One variant for fast-forward and merge alike, so
    /// the layer above matches it in one place.
    ///
    /// gage commits every write immediately, so a dirty tree means
    /// something outside gage is mid-edit. A fast-forward's hard reset
    /// would discard that work; a merge's commit would publish it under a
    /// message nobody wrote it for. Neither is gage's call to make.
    #[error(
        "gitrepo: the working tree has uncommitted changes ({}), so gage is not {remedy}; commit or discard them first",
        name_paths(paths)
    )]
    DirtyWorkTree {
        paths: Vec<String>,
        remedy: &'static str,
    },
    /// A fetch or push failed as unreachable, auth or diverged.
    #[error(transparent)]
    Sync(#[from] syncerr::Error),
    /// The remote's auth method couldn't be resolved (an `~/.ssh/config`
    /// alias, no ssh-agent, a token file others can read).
    #[error(transparent)]
    RemoteAuth(#[from] remoteauth::Error),
    /// A path from a git tree would land outside the vault. Tree paths come
    /// from whoever can push to the remote.
    #[error("gitrepo: refusing a remote path that escapes the vault: {0:?}")]
    UnsafePath(String),
    /// A remote to clone has no branches: an empty repository, not a vault.
    #[error("gitrepo: {url} has no branches; it is an empty repository, not a vault")]
    NoBranches { url: String },
    /// A remote to clone has several branches and neither `main` nor
    /// `master` among them.
    #[error(
        "gitrepo: {url} has several branches ({}) and none is main or master; gage cannot tell which one holds the vault",
        branches.join(", ")
    )]
    AmbiguousBranch { url: String, branches: Vec<String> },
    /// A merge was asked for with nothing fetched to merge.
    #[error("gitrepo: {} has nothing fetched to merge", path.display())]
    NothingFetched { path: PathBuf },
    /// A merge commit was asked for while paths still conflict.
    #[error(
        "gitrepo: refusing to merge with {0} unanswered: every conflicting path needs a choice"
    )]
    Unanswered(String),
    /// A merge was asked to add a file at a path that exists, or that the
    /// merge itself writes. Adds create; they never replace.
    #[error("gitrepo: refusing to add {0}: it already exists")]
    AddWouldOverwrite(String),
    /// A filesystem step of a merge or reset failed.
    #[error("gitrepo: {op} {path}: {source}")]
    Io {
        op: &'static str,
        path: String,
        source: std::io::Error,
    },
    /// A merge failed partway and rolling it back failed too.
    #[error("{cause} (and rolling the partial merge back failed: {rollback})")]
    RollbackFailed {
        cause: Box<Error>,
        rollback: Box<Error>,
    },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        match self {
            Error::DirtyWorkTree { .. } => Code::Conflict,
            Error::Sync(e) => e.code(),
            Error::RemoteAuth(e) => e.code(),
            Error::RollbackFailed { cause, .. } => cause.code(),
            Error::Git { .. }
            | Error::UnsafePath(_)
            | Error::NoBranches { .. }
            | Error::AmbiguousBranch { .. }
            | Error::NothingFetched { .. }
            | Error::Unanswered(_)
            | Error::AddWouldOverwrite(_)
            | Error::Io { .. } => Code::Internal,
        }
    }
}

/// The first [`MAX_NAMED_DIRTY_PATHS`] paths, and how many more.
fn name_paths(paths: &[String]) -> String {
    let named = paths[..paths.len().min(MAX_NAMED_DIRTY_PATHS)].join(", ");
    match paths.len().saturating_sub(MAX_NAMED_DIRTY_PATHS) {
        0 => named,
        more => format!("{named} and {more} more"),
    }
}

/// Process-wide libgit2 setup, run before every repository open
/// (Q-GIT-ISOLATION, Q-REMOTE-TIMEOUT). Idempotent and cheap after the
/// first call.
///
/// libgit2 reads the user's global config (and the system's, and XDG's,
/// and ProgramData's) and honors `url.<base>.insteadOf` from it, so
/// without this a user's config could redirect a vault's push. An empty
/// search path at every level hides them all. gage opens repositories with
/// `Repository::open`, never "from env", so `GIT_CONFIG_*` are never read
/// either. The server timeouts bound a blackholed connect and a stalled
/// read; the per-operation deadline is `RemoteOptions`'.
pub fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let timeout_ms = i32::try_from(REMOTE_OP_TIMEOUT.as_millis()).unwrap_or(i32::MAX);
        #[allow(unsafe_code)]
        // SAFETY: these set process-global libgit2 options. The `Once`
        // runs them exactly once, before any repository this process opens
        // through gage, and nothing else in gage writes them.
        let result = unsafe {
            [
                git2::ConfigLevel::System,
                git2::ConfigLevel::XDG,
                git2::ConfigLevel::Global,
                git2::ConfigLevel::ProgramData,
            ]
            .into_iter()
            .try_for_each(|level| git2::opts::set_search_path(level, ""))
            .and_then(|()| git2::opts::set_server_connect_timeout_in_milliseconds(timeout_ms))
            .and_then(|()| git2::opts::set_server_timeout_in_milliseconds(timeout_ms))
        };
        // These fail only on allocation failure or an invalid level; a
        // vault that silently ran unisolated would be worse than stopping.
        result.expect("gitrepo: isolating libgit2 from the user's git config");
    });
}

/// Initializes a repository at `dir` (already holding the files to
/// commit) and makes one commit of every file in the working tree, on
/// [`INITIAL_BRANCH`], as gage. Returns the commit's id.
pub fn init_and_commit(dir: &Path, message: &str) -> Result<String, Error> {
    isolate();
    let repo = git2::Repository::init_opts(
        dir,
        git2::RepositoryInitOptions::new().initial_head(INITIAL_BRANCH),
    )
    .map_err(fail("initializing", dir))?;
    let mut index = repo.index().map_err(fail("opening the index of", dir))?;
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .and_then(|()| index.write())
        .map_err(fail("staging files in", dir))?;
    let tree_id = index
        .write_tree()
        .map_err(fail("writing the tree of", dir))?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(fail("reading the tree of", dir))?;
    let sig = git2::Signature::now(AUTHOR_NAME, AUTHOR_EMAIL).map_err(fail("signing in", dir))?;
    let id = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
        .map_err(fail("committing in", dir))?;
    Ok(id.to_string())
}

/// Stages every change in the working tree, deletions included, and makes
/// one commit on HEAD as gage. Returns the commit's id. The message is the
/// caller's; the author and committer are always gage's own.
///
/// An unchanged tree still commits (an empty commit): every write a caller
/// makes changes something, so "nothing to commit" would be a bug to
/// surface, not to hide.
pub fn commit_all(dir: &Path, message: &str) -> Result<String, Error> {
    let repo = open(dir)?;
    let mut index = repo.index().map_err(fail("opening the index of", dir))?;
    // add_all picks up new and modified files; update_all is what notices
    // a path that is gone.
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .and_then(|()| index.update_all(["*"], None))
        .and_then(|()| index.write())
        .map_err(fail("staging files in", dir))?;
    let tree_id = index
        .write_tree()
        .map_err(fail("writing the tree of", dir))?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(fail("reading the tree of", dir))?;
    let parent = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(fail("reading HEAD of", dir))?;
    let sig = git2::Signature::now(AUTHOR_NAME, AUTHOR_EMAIL).map_err(fail("signing in", dir))?;
    let id = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .map_err(fail("committing in", dir))?;
    Ok(id.to_string())
}

/// Points `origin` at `url`, adding the remote if there is none: `git
/// remote add` or `git remote set-url`, whichever applies.
pub fn set_remote(dir: &Path, url: &str) -> Result<(), Error> {
    let repo = open(dir)?;
    // set-url writes the config key whether or not the remote exists, so
    // one call covers "add" and "change", and repairs an origin left with
    // a fetch refspec but no URL. An origin it just created has no
    // refspec yet; give it `git remote add`'s default.
    repo.remote_set_url(ORIGIN, url)
        .map_err(fail("setting origin in", dir))?;
    let remote = repo
        .find_remote(ORIGIN)
        .map_err(fail("reading origin in", dir))?;
    if remote
        .fetch_refspecs()
        .map_err(fail("reading origin in", dir))?
        .is_empty()
    {
        repo.remote_add_fetch(ORIGIN, &format!("+refs/heads/*:refs/remotes/{ORIGIN}/*"))
            .map_err(fail("setting origin in", dir))?;
    }
    Ok(())
}

/// `origin`'s URL, or `None` when there is no `origin` or it has no URL.
pub fn remote_url(dir: &Path) -> Result<Option<String>, Error> {
    origin_url(&open(dir)?, dir)
}

fn origin_url(repo: &git2::Repository, dir: &Path) -> Result<Option<String>, Error> {
    match repo.find_remote(ORIGIN) {
        Ok(remote) => {
            let url = remote.url().map_err(fail("reading origin in", dir))?;
            Ok((!url.is_empty()).then(|| url.to_string()))
        }
        // libgit2 reports an origin with no URL as absent, too.
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(fail("reading origin in", dir)(e)),
    }
}

/// Whether `dir` has an `origin` with a URL to sync with. A directory
/// that isn't a repository answers `false` rather than failing: "nothing
/// here to sync" is the honest answer for a vault whose files are missing,
/// and the caller's own command produces the clearer failure.
pub fn has_remote(dir: &Path) -> Result<bool, Error> {
    isolate();
    let repo = match git2::Repository::open(dir) {
        Ok(repo) => repo,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(false),
        Err(e) => return Err(fail("opening", dir)(e)),
    };
    Ok(origin_url(&repo, dir)?.is_some())
}

/// Whether the working tree has no staged, unstaged or untracked changes.
/// Untracked files count: "clean" has to imply "everything was committed".
pub fn is_clean(dir: &Path) -> Result<bool, Error> {
    let repo = open(dir)?;
    let statuses = repo
        .statuses(Some(
            git2::StatusOptions::new()
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false),
        ))
        .map_err(fail("reading the status of", dir))?;
    Ok(statuses.is_empty())
}

/// The vault-relative paths with staged, unstaged or untracked changes,
/// sorted. Empty means the tree is clean.
pub fn dirty_paths(dir: &Path) -> Result<Vec<String>, Error> {
    let repo = open(dir)?;
    Ok(status_paths(&repo, dir)?
        .into_iter()
        .map(|(path, _)| path)
        .collect())
}

/// Every changed path and whether it is untracked, sorted by path.
fn status_paths(repo: &git2::Repository, dir: &Path) -> Result<Vec<(String, bool)>, Error> {
    let statuses = repo
        .statuses(Some(
            git2::StatusOptions::new()
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false),
        ))
        .map_err(fail("reading the status of", dir))?;
    let mut paths: Vec<(String, bool)> = statuses
        .iter()
        .map(|e| {
            let path = String::from_utf8_lossy(e.path_bytes()).into_owned();
            (path, e.status() == git2::Status::WT_NEW)
        })
        .collect();
    paths.sort();
    Ok(paths)
}

/// The guard both tree-moving operations open with. `remedy` completes
/// "..., so gage is not <remedy>".
pub(crate) fn require_clean_work_tree(dir: &Path, remedy: &'static str) -> Result<(), Error> {
    let paths = dirty_paths(dir)?;
    if paths.is_empty() {
        Ok(())
    } else {
        Err(Error::DirtyWorkTree { paths, remedy })
    }
}

/// Discards every uncommitted change, returning the vault-relative paths
/// it discarded, sorted. A clean tree discards nothing.
///
/// Untracked files are removed explicitly: a hard reset, in libgit2 as in
/// `git`, leaves them alone, and the shape that matters is an interrupted
/// `insert`'s uncommitted `entries/<uuid>.age`, which must not be folded
/// into the next write's commit. HEAD never moves: the reset discards
/// uncommitted work, never a commit.
pub fn reset_hard(dir: &Path) -> Result<Vec<String>, Error> {
    let repo = open(dir)?;
    let changed = status_paths(&repo, dir)?;
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    for (path, _) in changed.iter().filter(|(_, untracked)| *untracked) {
        remove_if_present(&safe_join(dir, path)?, path)?;
    }
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(fail("reading HEAD of", dir))?;
    repo.reset(head.as_object(), git2::ResetType::Hard, None)
        .map_err(fail("resetting to HEAD in", dir))?;
    Ok(changed.into_iter().map(|(path, _)| path).collect())
}

/// Removes a file, treating "already gone" as done.
pub(crate) fn remove_if_present(full: &Path, path: &str) -> Result<(), Error> {
    match std::fs::remove_file(full) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::Io {
            op: "removing",
            path: path.to_string(),
            source: e,
        }),
        _ => Ok(()),
    }
}

/// Resolves a slash-separated path from a git tree against the vault
/// directory, refusing anything that could land outside it: an empty
/// path, a NUL, an absolute path or drive prefix, or any `..`. Tree paths
/// come from whoever can push to the remote, so
/// `entries/../../../etc/cron.d/x` is refused rather than resolved. Real
/// git refuses such trees too; this doesn't depend on that.
pub(crate) fn safe_join(dir: &Path, path: &str) -> Result<PathBuf, Error> {
    let refuse = || Error::UnsafePath(path.to_string());
    if path.is_empty() || path.contains('\0') {
        return Err(refuse());
    }
    let mut joined = dir.to_path_buf();
    for part in path.split('/') {
        // A leading or doubled slash: an absolute path, or a tree no git
        // writes.
        if part.is_empty() {
            return Err(refuse());
        }
        for component in Path::new(part).components() {
            match component {
                Component::Normal(c) => joined.push(c),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(refuse());
                }
            }
        }
    }
    if joined == dir {
        return Err(refuse());
    }
    Ok(joined)
}

/// The number of commits reachable from HEAD.
pub fn commit_count(dir: &Path) -> Result<usize, Error> {
    let repo = open(dir)?;
    let mut walk = repo.revwalk().map_err(fail("walking", dir))?;
    walk.push_head().map_err(fail("reading HEAD of", dir))?;
    let mut count = 0;
    for id in walk {
        id.map_err(fail("walking", dir))?;
        count += 1;
    }
    Ok(count)
}

/// What a test wants to know about a commit: who made it and what it says.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
}

#[cfg(any(test, feature = "test-support"))]
fn commit_info(c: &git2::Commit<'_>) -> CommitInfo {
    let text = |r: Result<&str, git2::Error>| r.unwrap_or_default().to_string();
    CommitInfo {
        message: text(c.message()),
        author_name: text(c.author().name()),
        author_email: text(c.author().email()),
        committer_name: text(c.committer().name()),
        committer_email: text(c.committer().email()),
    }
}

/// The commit at HEAD. A test fixture helper, never compiled into the
/// shipped binary.
#[cfg(any(test, feature = "test-support"))]
pub fn head_commit(dir: &Path) -> Result<CommitInfo, Error> {
    let repo = open(dir)?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(fail("reading HEAD of", dir))?;
    Ok(commit_info(&head))
}

/// Every commit reachable from HEAD, newest first. A test fixture helper.
#[cfg(any(test, feature = "test-support"))]
pub fn commit_log(dir: &Path) -> Result<Vec<CommitInfo>, Error> {
    let repo = open(dir)?;
    let mut walk = repo.revwalk().map_err(fail("walking", dir))?;
    walk.push_head().map_err(fail("reading HEAD of", dir))?;
    let mut log = Vec::new();
    for id in walk {
        let id = id.map_err(fail("walking", dir))?;
        let commit = repo
            .find_commit(id)
            .map_err(fail("reading a commit of", dir))?;
        log.push(commit_info(&commit));
    }
    Ok(log)
}

fn open(dir: &Path) -> Result<git2::Repository, Error> {
    isolate();
    // open, not discover: a vault's path is its repository's root, and a
    // vault directory inside some other repository must not be mistaken
    // for that repository.
    git2::Repository::open(dir).map_err(fail("opening", dir))
}

fn fail(op: &'static str, dir: &Path) -> impl Fn(git2::Error) -> Error + use<> {
    let path = dir.to_path_buf();
    move |source| Error::Git {
        op,
        path: path.clone(),
        source,
    }
}

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod isolation_tests;
#[cfg(test)]
mod merge_tests;
#[cfg(test)]
mod remote_tests;
#[cfg(test)]
mod worktree_tests;

#[cfg(test)]
#[allow(clippy::disallowed_macros)]
mod tests {
    use super::*;
    use crate::gittest;

    /// A repository with one committed file, `a.txt`.
    fn committed_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        init_and_commit(dir.path(), "initial commit").unwrap();
        dir
    }

    fn path_str(p: &Path) -> &str {
        p.to_str().unwrap()
    }

    // Go: TestInitAndCommitCreatesExactlyOneCommit
    #[test]
    fn init_and_commit_creates_exactly_one_commit() {
        let dir = committed_repo();
        assert_eq!(commit_count(dir.path()).unwrap(), 1);
        assert!(is_clean(dir.path()).unwrap());
    }

    #[test]
    fn init_and_commit_returns_head_and_commits_every_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "b").unwrap();

        let id = init_and_commit(dir.path(), "gage: initialize vault").unwrap();

        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id().to_string(), id);
        assert_eq!(head.message().unwrap(), "gage: initialize vault");
        assert_eq!(head.parent_count(), 0);
        let mut files = Vec::new();
        head.tree()
            .unwrap()
            .walk(git2::TreeWalkMode::PreOrder, |root, entry| {
                if entry.kind() == Some(git2::ObjectType::Blob) {
                    files.push(format!("{root}{}", entry.name().unwrap()));
                }
                git2::TreeWalkResult::Ok
            })
            .unwrap();
        assert_eq!(files, ["a.txt", "sub/b.txt"]);
    }

    /// Asserts HEAD is gage's commit on `main`, whatever the environment's
    /// git config says.
    fn assert_gage_defaults(dir: &Path) {
        let repo = git2::Repository::open(dir).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(head.name().unwrap(), "refs/heads/main");
        let commit = head.peel_to_commit().unwrap();
        for sig in [commit.author(), commit.committer()] {
            assert_eq!(sig.name().unwrap(), AUTHOR_NAME);
            assert_eq!(sig.email().unwrap(), AUTHOR_EMAIL);
        }
    }

    #[test]
    fn init_and_commit_uses_gages_branch_and_signature() {
        let dir = committed_repo();
        assert_gage_defaults(dir.path());
    }

    const USER_CONFIG_ENV: &str = "GAGE_GITREPO_TEST_USER_CONFIG";

    /// The child side of the test below: runs with HOME (and the XDG and
    /// Windows equivalents) pointing at a directory whose `.gitconfig`
    /// sets a different default branch and identity.
    #[test]
    fn user_git_config_trampoline() {
        let Some(home) = std::env::var_os(USER_CONFIG_ENV) else {
            return;
        };
        let dir = Path::new(&home).join("vault");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        init_and_commit(&dir, "initial commit").unwrap();
        assert_gage_defaults(&dir);
        eprintln!("CHECKED");
    }

    // The plan's git-defaults test: the commit's author, committer and
    // branch are gage's even when the user's git config says otherwise.
    // A child process, because HOME is process-wide and libgit2 reads it
    // once: setting it here would race every other test.
    #[test]
    fn init_and_commit_ignores_the_users_git_config() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            "[init]\n\tdefaultBranch = trunk\n[user]\n\tname = Someone Else\n\temail = else@example.com\n",
        )
        .unwrap();
        // The child's own libgit2 reads this file; the parent proves it
        // would have applied, so the test can't pass on a config that was
        // never found.
        let config = git2::Config::open(&home.path().join(".gitconfig")).unwrap();
        assert_eq!(config.get_string("init.defaultBranch").unwrap(), "trunk");

        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "gitrepo::tests::user_git_config_trampoline",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(USER_CONFIG_ENV, home.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path().join(".config"))
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_COMMITTER_NAME")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "child failed:\n{stderr}");
        assert!(stderr.contains("CHECKED"), "child ran no check:\n{stderr}");
    }

    // Go: TestSetRemoteThenChangeIt
    #[test]
    fn set_remote_then_change_it() {
        let dir = committed_repo();
        let (first, second) = (gittest::new_bare_remote(), gittest::new_bare_remote());

        set_remote(dir.path(), path_str(first.path())).unwrap();
        assert_eq!(
            remote_url(dir.path()).unwrap().as_deref(),
            Some(path_str(first.path()))
        );

        set_remote(dir.path(), path_str(second.path())).unwrap();
        assert_eq!(
            remote_url(dir.path()).unwrap().as_deref(),
            Some(path_str(second.path()))
        );
        assert!(has_remote(dir.path()).unwrap());
    }

    // A remote added by set_remote is an ordinary one: it fetches every
    // branch into refs/remotes/origin, as `git remote add` sets up.
    #[test]
    fn set_remote_configures_the_default_fetch_refspec() {
        let dir = committed_repo();
        set_remote(dir.path(), "/elsewhere.git").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let remote = repo.find_remote(ORIGIN).unwrap();
        let refspecs: Vec<String> = remote
            .fetch_refspecs()
            .unwrap()
            .iter()
            .map(|s| s.unwrap().unwrap().to_string())
            .collect();
        assert_eq!(refspecs, ["+refs/heads/*:refs/remotes/origin/*"]);
    }

    // Go: TestRemoteURLEmptyWhenUnset
    #[test]
    fn remote_url_empty_when_unset() {
        let dir = committed_repo();
        assert_eq!(remote_url(dir.path()).unwrap(), None);
    }

    /// Adds an `origin` with a fetch refspec but no URL, as a hand edit of
    /// `.git/config` could leave it.
    fn add_url_less_origin(dir: &Path) {
        let path = dir.join(".git").join("config");
        let mut config = std::fs::read_to_string(&path).unwrap();
        config.push_str("[remote \"origin\"]\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
        std::fs::write(&path, config).unwrap();
    }

    // Go: TestRemoteURLAndHasRemoteWithNoConfiguredURLs
    #[test]
    fn remote_url_and_has_remote_with_no_configured_urls() {
        let dir = committed_repo();
        add_url_less_origin(dir.path());

        assert_eq!(remote_url(dir.path()).unwrap(), None);
        assert!(!has_remote(dir.path()).unwrap());
    }

    // set_remote repairs a URL-less origin rather than failing on it.
    #[test]
    fn set_remote_fills_in_a_url_less_origin() {
        let dir = committed_repo();
        add_url_less_origin(dir.path());

        set_remote(dir.path(), "/elsewhere.git").unwrap();

        assert_eq!(
            remote_url(dir.path()).unwrap().as_deref(),
            Some("/elsewhere.git")
        );
    }

    // Go: TestHasRemoteOnARepoWithNoOriginConfigured
    #[test]
    fn has_remote_on_a_repo_with_no_origin_configured() {
        let dir = committed_repo();
        assert!(!has_remote(dir.path()).unwrap());
    }

    #[test]
    fn has_remote_is_false_for_a_directory_that_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_remote(dir.path()).unwrap());
        assert!(!has_remote(&dir.path().join("gone")).unwrap());
    }

    #[test]
    fn the_other_operations_fail_on_a_directory_that_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = remote_url(dir.path()).unwrap_err();
        assert_eq!(err.code(), Code::Internal);
        assert!(err.to_string().starts_with("gitrepo: opening"), "{err}");
        assert!(is_clean(dir.path()).is_err());
        assert!(commit_count(dir.path()).is_err());
        assert!(set_remote(dir.path(), "/x.git").is_err());
    }

    // Go: TestIsCleanDetectsDirtyWorkingTree
    #[test]
    fn is_clean_detects_dirty_working_tree() {
        let dir = committed_repo();
        std::fs::write(dir.path().join("a.txt"), "changed").unwrap();
        assert!(!is_clean(dir.path()).unwrap());
    }

    // Go: TestIsCleanCountsUntrackedFilesAsDirty
    #[test]
    fn is_clean_counts_untracked_files_as_dirty() {
        let dir = committed_repo();
        std::fs::write(dir.path().join("never-committed.txt"), "stray").unwrap();
        assert!(!is_clean(dir.path()).unwrap());
    }

    // An untracked file inside an untracked directory is still untracked
    // (git reports only the directory unless asked to recurse).
    #[test]
    fn is_clean_counts_an_untracked_file_in_a_new_directory() {
        let dir = committed_repo();
        std::fs::create_dir(dir.path().join("entries")).unwrap();
        assert!(
            is_clean(dir.path()).unwrap(),
            "an empty directory is not a change"
        );
        std::fs::write(dir.path().join("entries/stray.age"), "x").unwrap();
        assert!(!is_clean(dir.path()).unwrap());
    }

    #[test]
    fn is_clean_ignores_gitignored_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), ".DS_Store\n").unwrap();
        init_and_commit(dir.path(), "initial commit").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "finder").unwrap();
        assert!(is_clean(dir.path()).unwrap());
    }

    #[test]
    fn commit_all_stages_additions_modifications_and_deletions() {
        let dir = committed_repo();
        std::fs::write(dir.path().join("b.txt"), "new").unwrap();
        std::fs::write(dir.path().join("a.txt"), "changed").unwrap();
        let id = commit_all(dir.path(), "first").unwrap();
        assert!(is_clean(dir.path()).unwrap());
        assert_eq!(commit_count(dir.path()).unwrap(), 2);

        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        let del = commit_all(dir.path(), "second").unwrap();
        assert_ne!(id, del);
        assert!(is_clean(dir.path()).unwrap(), "the deletion was committed");
        assert_eq!(commit_count(dir.path()).unwrap(), 3);

        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        assert!(tree.get_name("a.txt").is_none());
        assert!(tree.get_name("b.txt").is_some());
        assert_eq!(head.message().unwrap(), "second");
        assert_eq!(head.author().name().unwrap(), AUTHOR_NAME);
        assert_eq!(head.author().email().unwrap(), AUTHOR_EMAIL);
        assert_eq!(head.committer().email().unwrap(), AUTHOR_EMAIL);
    }

    #[test]
    fn commit_all_commits_files_in_subdirectories() {
        let dir = committed_repo();
        std::fs::create_dir(dir.path().join("entries")).unwrap();
        std::fs::write(dir.path().join("entries/x.age"), "x").unwrap();
        commit_all(dir.path(), "m").unwrap();
        assert!(is_clean(dir.path()).unwrap());
    }
}
