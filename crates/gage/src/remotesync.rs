//! Sync between a vault and its `origin`: the automatic pull on every
//! unlock, the automatic push after every write, and the manual `pull`,
//! `push` and `sync` verbs (Go's `remotesync.go`). See "Sync model" in the
//! design doc.
//!
//! Local durability and network risk are kept apart. The automatic paths
//! never fail anything: being offline costs one warning, a divergence one
//! more, and the local copy carries on. Only a verb a human asked for
//! reports a remote failure as one.
//!
//! The lock rules are the point of this module:
//!
//! - the automatic pull tries the write lock and skips if it is busy, so a
//!   read never queues behind a write;
//! - the automatic push runs inside the write's own lock, as the unlocked
//!   form, so a write never waits on itself;
//! - the manual verbs wait for the lock, since a fast-forward or a merge
//!   moves the working tree just as a write does.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::entry::{self, LockError, WRITE_LOCK_TIMEOUT};
use crate::exitcode::{Code, HasCode};
use crate::gitrepo::{self, MergeResult, RemoteOptions, RemoteState};
use crate::identityfile::UnlockError;
use crate::prompter::{PromptError, Prompter};
use crate::syncerr::Kind;
use crate::vault::Vault;
use crate::xdgpaths::Dirs;

pub use crate::gitrepo::REMOTE_OP_TIMEOUT;

/// What an automatic merge commits under. Like every commit message gage
/// writes, it names no entry.
pub const MERGE_COMMIT_MESSAGE: &str = "gage: merge origin";

/// What an unpublished write says it cost (Go's `unpushedWriteClause`).
/// Later writes that need a more specific clause (M9) reuse the sentence
/// rather than inventing their own.
pub const UNPUSHED_WRITE_CLAUSE: &str = "the write is committed locally but not pushed";

/// The two files whose divergence is more serious than an entry's: they
/// define who can read the vault at all.
pub(crate) const RECIPIENTS_FILE: &str = ".age-recipients";
pub(crate) const VAULT_CONFIG_FILE: &str = ".gage/config.toml";

/// The two operations of sync that touch the network. Everything else
/// (comparing, fast-forwarding, merging) is local git work that runs for
/// real in every test. This exists so a test can inject the one thing a
/// local repository can't produce: a network that isn't there.
///
/// Each returns whether anything moved: a fetch that updated a
/// remote-tracking ref, a push that sent something. Failures are
/// `gitrepo`'s, so a remote failure arrives as [`gitrepo::Error::Sync`]
/// in one of `syncerr`'s buckets.
pub trait RemoteSyncer: Send + Sync {
    fn fetch(&self, deadline: Instant) -> Result<bool, gitrepo::Error>;
    fn push(&self, deadline: Instant) -> Result<bool, gitrepo::Error>;
}

/// The real [`RemoteSyncer`]: libgit2 against the vault's own origin.
pub struct GitSyncer {
    dir: PathBuf,
    dirs: Dirs,
}

impl GitSyncer {
    /// The options for one operation, bounded by what is left of
    /// `deadline`: one deadline covers a whole sync, fetch and push alike.
    fn options(&self, deadline: Instant) -> RemoteOptions {
        RemoteOptions::new(&self.dirs)
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
    }
}

impl RemoteSyncer for GitSyncer {
    fn fetch(&self, deadline: Instant) -> Result<bool, gitrepo::Error> {
        gitrepo::fetch(&self.dir, &self.options(deadline))
    }

    fn push(&self, deadline: Instant) -> Result<bool, gitrepo::Error> {
        gitrepo::push(&self.dir, &self.options(deadline))
    }
}

/// The injection seam: `None` is the real [`GitSyncer`], so nothing
/// outside a test ever sets it.
#[derive(Clone, Default)]
pub(crate) struct SyncerSeam(Option<Arc<dyn RemoteSyncer>>);

// Two vaults are the same vault whichever syncer they'd use; an injected
// one only ever equals itself.
impl PartialEq for SyncerSeam {
    fn eq(&self, other: &SyncerSeam) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for SyncerSeam {}

impl std::fmt::Debug for SyncerSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "injected" } else { "git" })
    }
}

/// "The working tree moved": set by a fast-forward or a merge, and read
/// (and cleared) by whoever caches what the tree held, which today is a
/// session's metadata index. Set where the tree moves rather than by each
/// caller, so no caller can forget (Go's `notePulled`).
#[derive(Clone, Default)]
pub(crate) struct PullObserver(Option<Arc<AtomicBool>>);

impl PullObserver {
    pub(crate) fn note(&self) {
        if let Some(flag) = &self.0 {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

// Observation, not identity.
impl PartialEq for PullObserver {
    fn eq(&self, _: &PullObserver) -> bool {
        true
    }
}

impl Eq for PullObserver {}

impl std::fmt::Debug for PullObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PullObserver")
    }
}

/// What a divergence conflicted on. The two cases want different words in
/// front of a human, and M8b resolves only the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    None,
    /// Entries changed on both sides: resolved through `gage sync`.
    Entries,
    /// A file that defines who can read the vault diverged. Never merged:
    /// the result would be an access list neither device wrote.
    Recipients,
}

/// What one sync attempt did. The default report is "nothing to do",
/// which is what a vault with no remote produces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// A fast-forward moved the local branch.
    pub pulled: bool,
    /// A divergence was reconciled into a merge commit.
    pub merged: bool,
    /// Commits were sent to the remote. A push with nothing to send
    /// leaves it false (Q-SYNC-PUSHED).
    pub pushed: bool,
    /// Both sides had moved on. It stays true after a merge: it says what
    /// was found, not what was left.
    pub diverged: bool,
    /// The vault-relative paths that could not be merged, sorted.
    pub conflicts: Vec<String>,
    /// How many local commits the remote lacks, as of the report.
    pub ahead: usize,
    /// How many conflicting entries a human's choice was applied to.
    /// Zero on every automatic path. `u32`, as is `skipped`: every sync
    /// result carries this report, and keeping it small keeps them cheap.
    pub resolved: u32,
    /// How many conflicts were left unanswered on purpose. Any skip means
    /// nothing was applied and nothing pushed: a partial resolution would
    /// turn a skipped entry's local version into a silent choice.
    pub skipped: u32,
    /// The human quit the resolution, which changes nothing.
    pub aborted: bool,
}

impl SyncReport {
    /// What the conflicts are, with a recipient-file conflict outranking
    /// an entry conflict when both are present.
    pub fn conflict_kind(&self) -> ConflictKind {
        let mut kind = ConflictKind::None;
        for path in &self.conflicts {
            if path == RECIPIENTS_FILE || path == VAULT_CONFIG_FILE {
                return ConflictKind::Recipients;
            }
            kind = ConflictKind::Entries;
        }
        kind
    }

    /// The divergence in one line, in the design doc's own words ("origin
    /// has diverged — 2 local commits pending"), plus what conflicted and
    /// what to run.
    pub fn summary(&self) -> String {
        match self.conflict_kind() {
            ConflictKind::Recipients => format!(
                "origin has diverged on the files that define who can read this vault ({}) — \
                 {} local commit(s) pending. These are never merged automatically: \
                 review both sides and run `gage sync`",
                self.conflicts.join(", "),
                self.ahead
            ),
            ConflictKind::Entries => format!(
                "origin has diverged — {} local commit(s) pending, and {} entr(y/ies) changed on both sides; run `gage sync`",
                self.ahead,
                self.conflicts.len()
            ),
            ConflictKind::None => format!(
                "origin has diverged — {} local commit(s) pending; run `gage sync`",
                self.ahead
            ),
        }
    }
}

/// A sync that failed, with what it had done by then: a fast-forward
/// before a failed push still happened, and a conflict's paths are in the
/// report.
#[derive(Debug)]
pub struct SyncError {
    pub report: SyncReport,
    pub cause: SyncCause,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum SyncCause {
    /// A fetch or push failed: unreachable, auth, a remote that moved
    /// again, or a failure in none of `syncerr`'s buckets.
    Remote(gitrepo::Error),
    /// Both sides changed the same paths (the report's `conflicts`).
    /// Nothing was merged or changed.
    Conflict,
    /// The local half failed: comparing, fast-forwarding, merging or
    /// counting. A dirty working tree is the expected case.
    Local(gitrepo::Error),
    /// The vault's write lock.
    Lock(LockError),
    /// The frontend gave no answer to a conflict: it has nobody to ask
    /// ([`PromptError::NotInteractive`]), or asking failed. Nothing was
    /// applied.
    Prompt(PromptError),
    /// The lazy unlock a conflict forced failed. Nothing was applied.
    /// Boxed, as is `Entry`: both are large, and every sync result
    /// carries this type.
    Unlock(Box<UnlockError>),
    /// A path that is not an entry changed on both sides, so there is no
    /// version of it to choose. Refused before anything was unlocked.
    NotAnEntry(String),
    /// Decrypting a side of a conflict, or encrypting keep-both's copy,
    /// failed. Nothing was applied.
    Entry(Box<entry::Error>),
    /// Conflicts were skipped (the report's `skipped` of `total`), so
    /// nothing was applied or pushed.
    Unresolved { total: usize },
    /// The human quit the resolution; nothing was applied or pushed.
    Aborted,
}

impl SyncError {
    pub(crate) fn new(cause: SyncCause) -> SyncError {
        SyncError {
            report: SyncReport::default(),
            cause,
        }
    }

    pub(crate) fn with_report(mut self, report: SyncReport) -> SyncError {
        self.report = report;
        self
    }

    /// Which `syncerr` bucket the failure is in, if any. A real conflict
    /// is [`Kind::Diverged`] too, so one question covers both "the push
    /// was rejected" and "the rejection turned out to be a conflict"
    /// (Go's `errors.Is(err, syncerr.ErrDiverged)`).
    ///
    /// A resolution that was skipped or aborted is still diverged: the
    /// divergence it declined to resolve is there, as it was before anyone
    /// was asked (Go's `resolutionRefusal`).
    pub fn remote_kind(&self) -> Option<Kind> {
        match &self.cause {
            SyncCause::Remote(gitrepo::Error::Sync(e)) => Some(e.kind()),
            SyncCause::Conflict | SyncCause::Unresolved { .. } | SyncCause::Aborted => {
                Some(Kind::Diverged)
            }
            _ => None,
        }
    }

    /// Whether this is uncommitted changes in the working tree, which a
    /// human resolves.
    pub fn is_dirty_work_tree(&self) -> bool {
        matches!(
            self.cause,
            SyncCause::Local(gitrepo::Error::DirtyWorkTree { .. })
        )
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.cause {
            SyncCause::Conflict => write!(f, "gage: {}", self.report.summary()),
            // gitrepo's own wording, said as gage: a dirty tree is neither
            // unexpected nor a fault, so it reads as gage's message.
            SyncCause::Local(e @ gitrepo::Error::DirtyWorkTree { .. }) => {
                let msg = e.to_string();
                write!(f, "gage: {}", msg.strip_prefix("gitrepo: ").unwrap_or(&msg))
            }
            SyncCause::Remote(e) | SyncCause::Local(e) => e.fmt(f),
            SyncCause::Lock(e) => e.fmt(f),
            SyncCause::Prompt(e) => e.fmt(f),
            SyncCause::Unlock(e) => write!(f, "gage: unlocking to resolve a conflict: {e}"),
            SyncCause::Entry(e) => e.fmt(f),
            SyncCause::NotAnEntry(path) => write!(
                f,
                "gage: {path:?} changed on both sides and is not an entry, so there is no version of it to choose; \
                 the vault is an ordinary git repository"
            ),
            SyncCause::Unresolved { total } => write!(
                f,
                "gage: {} of {total} conflicting entries left unresolved, so nothing was applied and \
                 nothing was pushed; run `gage sync` again to finish",
                self.report.skipped
            ),
            SyncCause::Aborted => {
                f.write_str("gage: resolution aborted; nothing was changed and nothing was pushed")
            }
        }
    }
}

// Transparent: the cause's own source, so nobody renders a message twice.
impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.cause {
            SyncCause::Remote(e) | SyncCause::Local(e) => e.source(),
            SyncCause::Lock(e) => e.source(),
            SyncCause::Prompt(e) => e.source(),
            SyncCause::Unlock(e) => Some(e.as_ref()),
            SyncCause::Entry(e) => e.source(),
            SyncCause::Conflict
            | SyncCause::NotAnEntry(_)
            | SyncCause::Unresolved { .. }
            | SyncCause::Aborted => None,
        }
    }
}

impl HasCode for SyncError {
    fn code(&self) -> Code {
        match &self.cause {
            // Unreachable 7, auth 5, diverged 1 (Q-SYNC-CLASSIFY).
            SyncCause::Remote(e) => e.code(),
            // "A state a human must resolve."
            SyncCause::Conflict => Code::Conflict,
            // A dirty tree is Conflict; anything else here is Internal.
            SyncCause::Local(e) => e.code(),
            SyncCause::Lock(e) => e.code(),
            // Nobody to ask is Conflict; a broken terminal is Internal.
            SyncCause::Prompt(e) => e.code(),
            SyncCause::Unlock(e) => e.code(),
            SyncCause::Entry(e) => e.code(),
            SyncCause::NotAnEntry(_) | SyncCause::Unresolved { .. } | SyncCause::Aborted => {
                Code::Conflict
            }
        }
    }
}

impl From<LockError> for SyncError {
    fn from(err: LockError) -> SyncError {
        SyncError::new(SyncCause::Lock(err))
    }
}

pub(crate) fn remote(err: gitrepo::Error) -> SyncError {
    SyncError::new(SyncCause::Remote(err))
}

pub(crate) fn local(err: gitrepo::Error) -> SyncError {
    SyncError::new(SyncCause::Local(err))
}

impl Vault {
    /// Fetches and fast-forwards, and never does anything else: a
    /// divergence is reported in the report, not merged ("Sync model"'s
    /// fast-forward-only rule). Waits for the write lock.
    pub fn pull(&self) -> Result<SyncReport, SyncError> {
        self.under_write_lock(|deadline| self.pull_locked(deadline))
    }

    /// Publishes local commits. A rejected push fetches, merges a
    /// divergence whose two sides touched different paths, and pushes
    /// again; a real conflict is reported with nothing changed. Waits for
    /// the write lock.
    pub fn push(&self) -> Result<SyncReport, SyncError> {
        self.under_write_lock(|deadline| self.push_locked(deadline))
    }

    /// Catches up, then publishes, under one lock acquisition, so nothing
    /// can land between the two. The push is what merges a divergence;
    /// a real conflict is reported, with nothing changed. Resolving one
    /// is [`Vault::sync_resolving`]'s.
    pub fn sync(&self) -> Result<SyncReport, SyncError> {
        self.under_write_lock(|deadline| self.sync_locked(deadline))
    }

    /// The sync, with the write lock already held.
    pub(crate) fn sync_locked(&self, deadline: Instant) -> Result<SyncReport, SyncError> {
        let pulled = self.pull_locked(deadline)?;
        // The pull's findings survive into the combined report: a
        // fast-forward before a failed push still happened.
        let merge = |mut report: SyncReport| {
            report.pulled |= pulled.pulled;
            report.diverged |= pulled.diverged;
            report
        };
        match self.push_locked(deadline) {
            Ok(report) => Ok(merge(report)),
            Err(mut err) => {
                err.report = merge(err.report);
                Err(err)
            }
        }
    }

    /// The same vault, fetching and pushing through `syncer`. Tests only:
    /// the real one runs everywhere else.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_remote_syncer(mut self, syncer: Arc<dyn RemoteSyncer>) -> Vault {
        self.syncer = SyncerSeam(Some(syncer));
        self
    }

    /// The same vault, raising `flag` whenever a pull or a merge moves its
    /// working tree. A session installs one per vault it holds.
    pub(crate) fn with_pull_observer(mut self, flag: Arc<AtomicBool>) -> Vault {
        self.pulled = PullObserver(Some(flag));
        self
    }

    /// The automatic catch-up on unlock, in both invocation modes: fetch
    /// and fast-forward only, if the write lock is free. It never fails
    /// anything. Being offline warns once and carries on with the local
    /// copy: refusing to show a password because the network is down is
    /// worse than showing a possibly stale one. A divergence warns too;
    /// the write that matters will say so when it is pushed.
    pub(crate) fn sync_on_unlock(&self, prompter: &dyn Prompter) {
        // Checked before the lock, so a local-only vault never touches its
        // lock file. pull_locked checks again under the lock.
        if !gitrepo::has_remote(&self.path).unwrap_or(false) {
            return;
        }
        match self.try_pull(Instant::now() + REMOTE_OP_TIMEOUT) {
            // Another process is mid-write and will push when it is done.
            // Silent: nothing is wrong, and a warning here would fire on
            // ordinary concurrency.
            Ok(None) => {}
            Ok(Some(report)) if report.diverged => {
                prompter.warn(&format!("gage: {}", report.summary()));
            }
            Ok(Some(_)) => {}
            Err(err) => prompter.warn(&format!(
                "gage: {}; continuing with the local copy of {:?}",
                offline_or_error(&err),
                self.name
            )),
        }
    }

    /// The automatic push after a write, run inside the write's own lock
    /// (the unlocked form: taking the lock again would wait on itself). The
    /// commit is already durable locally, so it never fails the write: it
    /// warns, differently for being offline ("your commit is safe but
    /// unsent") and for a divergence ("run `gage sync`").
    pub(crate) fn push_after_write(&self, prompter: &dyn Prompter) {
        match self.push_locked(Instant::now() + REMOTE_OP_TIMEOUT) {
            Ok(_) => {}
            // The divergence has its own sentence; what wasn't published
            // is already in it.
            Err(err) if err.remote_kind() == Some(Kind::Diverged) => {
                prompter.warn(&format!("gage: {}", err.report.summary()));
            }
            Err(err) => prompter.warn(&format!(
                "gage: {UNPUSHED_WRITE_CLAUSE}: {}",
                offline_or_error(&err)
            )),
        }
    }

    /// Runs a sync operation holding the write lock, with one remote
    /// deadline for the whole of it, started once the lock is held.
    pub(crate) fn under_write_lock(
        &self,
        f: impl FnOnce(Instant) -> Result<SyncReport, SyncError>,
    ) -> Result<SyncReport, SyncError> {
        self.with_write_lock(WRITE_LOCK_TIMEOUT, || f(Instant::now() + REMOTE_OP_TIMEOUT))
    }

    /// The pull, if the write lock is free right now; `None` if another
    /// process holds it.
    fn try_pull(&self, deadline: Instant) -> Result<Option<SyncReport>, SyncError> {
        let Some(lock) = self.try_write_lock()? else {
            return Ok(None);
        };
        let report = self.pull_locked(deadline);
        lock.release().map_err(LockError::from)?;
        report.map(Some)
    }

    /// The pull, with the write lock already held.
    fn pull_locked(&self, deadline: Instant) -> Result<SyncReport, SyncError> {
        if !self.has_remote()? {
            return Ok(SyncReport::default());
        }
        self.syncer().fetch(deadline).map_err(remote)?;
        self.fast_forward(SyncReport::default())
    }

    /// Advances the local branch when the last fetch left a fast-forward
    /// available, and records what it found either way. Shared by pull and
    /// by push's recovery from a rejection, so the two can't drift.
    fn fast_forward(&self, mut report: SyncReport) -> Result<SyncReport, SyncError> {
        let fail = |e, report: &SyncReport| local(e).with_report(report.clone());
        match gitrepo::compare(&self.path).map_err(|e| fail(e, &report))? {
            RemoteState::Behind => {
                let moved = gitrepo::fast_forward(&self.path).map_err(|e| fail(e, &report))?;
                report.pulled = moved;
                if moved {
                    self.pulled.note();
                }
            }
            RemoteState::Diverged => report.diverged = true,
            RemoteState::None | RemoteState::InSync | RemoteState::Ahead => {}
        }
        report.ahead = gitrepo::ahead_count(&self.path).map_err(|e| fail(e, &report))?;
        Ok(report)
    }

    /// The push, with the write lock already held. A rejected push is
    /// where a divergence is discovered, so it is classified here: fetch,
    /// then fast-forward (the rejection was a race) or merge. A merge
    /// whose two sides touched different paths needs nobody; one that
    /// conflicts stops with the paths named and nothing changed.
    pub(crate) fn push_locked(&self, deadline: Instant) -> Result<SyncReport, SyncError> {
        let mut report = SyncReport::default();
        if !self.has_remote()? {
            return Ok(report);
        }
        let syncer = self.syncer();
        let rejected = match syncer.push(deadline) {
            Ok(sent) => {
                report.pushed = sent;
                return Ok(report);
            }
            Err(e) => remote(e),
        };
        if rejected.remote_kind() != Some(Kind::Diverged) {
            return Err(rejected);
        }

        report.diverged = true;
        let with = |err: SyncError, report: &SyncReport| err.with_report(report.clone());
        syncer
            .fetch(deadline)
            .map_err(|e| with(remote(e), &report))?;
        match gitrepo::compare(&self.path).map_err(|e| with(local(e), &report))? {
            RemoteState::Behind => report = self.fast_forward(report)?,
            RemoteState::Diverged => {
                match gitrepo::merge_remote(&self.path, MERGE_COMMIT_MESSAGE)
                    .map_err(|e| with(local(e), &report))?
                {
                    MergeResult::Conflicts(paths) => {
                        report.conflicts = paths;
                        // A failure to count is not worth losing the
                        // conflict report over: the conflict is what the
                        // human acts on.
                        report.ahead = gitrepo::ahead_count(&self.path).unwrap_or(0);
                        return Err(SyncError::new(SyncCause::Conflict).with_report(report));
                    }
                    MergeResult::Merged { .. } => {
                        report.merged = true;
                        // The merge brought in the other device's writes.
                        self.pulled.note();
                        // Counted before the push, so a merge whose push
                        // then fails still says how much is pending, the
                        // merge commit included.
                        report.ahead = gitrepo::ahead_count(&self.path)
                            .map_err(|e| with(local(e), &report))?;
                    }
                }
            }
            RemoteState::None | RemoteState::InSync | RemoteState::Ahead => {}
        }

        let sent = syncer
            .push(deadline)
            .map_err(|e| with(remote(e), &report))?;
        report.pushed = sent;
        report.ahead = 0;
        Ok(report)
    }

    /// Whether there is an origin to sync with at all. A local-only vault
    /// is a first-class state, not an error: every sync path is a no-op.
    fn has_remote(&self) -> Result<bool, SyncError> {
        gitrepo::has_remote(&self.path).map_err(local)
    }

    pub(crate) fn syncer(&self) -> Arc<dyn RemoteSyncer> {
        match &self.syncer.0 {
            Some(injected) => Arc::clone(injected),
            None => Arc::new(GitSyncer {
                dir: self.path.clone(),
                dirs: self.dirs().clone(),
            }),
        }
    }
}

/// A sync failure as a clause that reads mid-sentence: being offline as the
/// expected, unalarming thing it is, and anything else as its own message
/// without the "gage: " the surrounding warning already opens with.
fn offline_or_error(err: &SyncError) -> String {
    if err.remote_kind() == Some(Kind::Unreachable) {
        return "could not reach origin".to_string();
    }
    let msg = err.to_string();
    msg.strip_prefix("gage: ").unwrap_or(&msg).to_string()
}

#[cfg(test)]
#[path = "remotesync_tests.rs"]
pub(crate) mod tests;
