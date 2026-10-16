//! gage's three-way merge of the local branch and its remote-tracking ref
//! (Q-FILE-MERGE). File-level, never `git2::merge_trees`.
//!
//! Entries are age ciphertext, opaque blobs with no line structure, so two
//! edits to one entry can never be reconciled by a text merge. A path is
//! taken from whichever side changed it, and a path both sides changed
//! differently is a conflict, full stop. That same coarseness keeps two
//! devices that each add a recipient from merging `.age-recipients` into a
//! union neither wrote, without gage ever reading `.gitattributes`: a
//! line-level merge would union them, and only that committed file's
//! `-merge` would stop it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::remote::head_and_tracking;
use super::{
    AUTHOR_EMAIL, AUTHOR_NAME, Error, fail, open, remove_if_present, require_clean_work_tree,
    safe_join,
};

/// What a merge did: committed, or found the paths that stop it (sorted),
/// having changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Merged { commit: String },
    Conflicts(Vec<String>),
}

/// One of the two versions a merge reconciles: which side's content to
/// read, and which side's version to keep for a path both changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeSide {
    /// This device's version: what HEAD has.
    Local,
    /// The fetched version: what the remote-tracking ref has.
    Remote,
}

/// A three-way merge computed but not yet applied. Splitting the two is
/// what lets a caller (M8b's resolution) read both sides of what
/// conflicted ([`PendingMerge::content`]) and decide before anything is
/// written; one that decides not to proceed never calls
/// [`PendingMerge::commit`], and the vault is untouched.
#[derive(Debug)]
pub struct PendingMerge {
    dir: PathBuf,
    local: git2::Oid,
    remote: git2::Oid,
    /// HEAD's paths, for rollback: which written paths the hard reset
    /// restores and which were new and are removed by name.
    local_paths: HashSet<String>,
    /// Paths both sides changed differently, sorted.
    conflicts: Vec<String>,
    /// Paths only the remote changed: taken without asking anyone.
    take_remote: Vec<String>,
}

/// A path's content at one commit: blob id and mode, so a chmod counts as
/// a change, as git treats it. `None` is a deletion.
type Version = Option<(git2::Oid, u32)>;

/// Merges the remote-tracking ref into the local branch as a real
/// two-parent commit, or reports the conflicts and writes nothing.
/// [`prepare_merge`] then [`PendingMerge::commit`], with no choices: the
/// one path, so the automatic merge and a resolved one can't drift.
pub fn merge_remote(dir: &Path, message: &str) -> Result<MergeResult, Error> {
    let merge = prepare_merge(dir)?;
    if !merge.conflicts.is_empty() {
        return Ok(MergeResult::Conflicts(merge.conflicts));
    }
    merge.commit(message, &BTreeMap::new(), &BTreeMap::new())
}

/// Computes the merge [`merge_remote`] would make, writing nothing.
///
/// A dirty working tree is refused up front, as [`super::fast_forward`]
/// refuses one: the merge commit stages the whole tree, so a human's
/// half-finished editing would be swept into "gage: merge origin" and
/// pushed to every other device (Go's correction,
/// `TestMergeRemoteRefusesADirtyWorkTree`).
pub fn prepare_merge(dir: &Path) -> Result<PendingMerge, Error> {
    require_clean_work_tree(dir, "merging origin's changes on top of them")?;
    let repo = open(dir)?;
    let (local, Some(remote)) = head_and_tracking(&repo, dir)? else {
        return Err(Error::NothingFetched {
            path: dir.to_path_buf(),
        });
    };
    // Two histories with no common ancestor diff from an empty tree, so
    // every path both have is an add/add pair: a conflict unless
    // identical, which is right, as gage has no basis to prefer a side.
    let base = match repo.merge_base(local, remote) {
        Ok(base) => Some(base),
        Err(e) if e.code() == git2::ErrorCode::NotFound => None,
        Err(e) => return Err(fail("finding the merge base in", dir)(e)),
    };
    let tree = |id: git2::Oid| {
        repo.find_commit(id)
            .and_then(|c| c.tree())
            .map_err(fail("reading a tree of", dir))
    };
    let base_tree = base.map(tree).transpose()?;
    let (local_tree, remote_tree) = (tree(local)?, tree(remote)?);
    let ours = changes(&repo, dir, base_tree.as_ref(), &local_tree)?;
    let theirs = changes(&repo, dir, base_tree.as_ref(), &remote_tree)?;

    let (mut conflicts, mut take_remote) = (Vec::new(), Vec::new());
    for (path, their_version) in theirs {
        match ours.get(&path) {
            // Only the remote touched it: new content or a deletion.
            None => take_remote.push(path),
            // Both sides agree, including both having deleted it.
            Some(our_version) if *our_version == their_version => {}
            Some(_) => conflicts.push(path),
        }
    }
    let mut local_paths = HashSet::new();
    local_tree
        .walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() != Some(git2::ObjectType::Tree) {
                local_paths.insert(format!("{root}{}", entry.name().unwrap_or_default()));
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(fail("reading HEAD's tree in", dir))?;
    Ok(PendingMerge {
        dir: dir.to_path_buf(),
        local,
        remote,
        local_paths,
        conflicts,
        take_remote,
    })
}

/// Every path that differs between `base` (or an empty tree) and `tree`,
/// with its version in `tree`. Sorted, so a merge visits paths in the same
/// order on every platform.
fn changes(
    repo: &git2::Repository,
    dir: &Path,
    base: Option<&git2::Tree<'_>>,
    tree: &git2::Tree<'_>,
) -> Result<BTreeMap<String, Version>, Error> {
    let diff = repo
        .diff_tree_to_tree(
            base,
            Some(tree),
            Some(git2::DiffOptions::new().include_typechange(true)),
        )
        .map_err(fail("comparing trees in", dir))?;
    let mut out = BTreeMap::new();
    for delta in diff.deltas() {
        let (file, version) = if delta.status() == git2::Delta::Deleted {
            (delta.old_file(), None)
        } else {
            let file = delta.new_file();
            let version = Some((file.id(), u32::from(file.mode())));
            (file, version)
        };
        let path = file
            .path_bytes()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_default();
        out.insert(path, version);
    }
    Ok(out)
}

/// What any file this merge creates is written as. 0600 rather than the
/// 0644 git records for every blob: everything a vault merge writes is an
/// entry's ciphertext or a file saying who can read it, and an entry gage
/// writes directly is 0600, so how an entry arrived must not decide who
/// can read its file. An overwrite keeps the file's existing mode.
const MERGED_FILE_MODE: u32 = 0o600;

impl PendingMerge {
    /// The paths both sides changed differently, sorted. Empty means
    /// [`PendingMerge::commit`] can proceed.
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// One side's version of `path`, read from the git objects, or `None`
    /// if that side doesn't have it: a deletion, which is what lets a
    /// delete/modify conflict be shown as "one side removed this". The
    /// remote's version of a conflicted path was never written to disk,
    /// and reading the local one from the same place keeps one source of
    /// truth for what "local" means.
    pub fn content(&self, side: MergeSide, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let commit = match side {
            MergeSide::Local => self.local,
            MergeSide::Remote => self.remote,
        };
        let repo = open(&self.dir)?;
        let tree = repo
            .find_commit(commit)
            .and_then(|c| c.tree())
            .map_err(fail("reading a tree of", &self.dir))?;
        let entry = match tree.get_path(Path::new(path)) {
            Ok(entry) => entry,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(fail("reading a tree of", &self.dir)(e)),
        };
        let blob = entry
            .to_object(&repo)
            .and_then(|o| o.peel_to_blob())
            .map_err(fail("reading a merged file in", &self.dir))?;
        Ok(Some(blob.content().to_vec()))
    }

    /// Applies the merge and records it as a commit with parents [local,
    /// remote] and `message`.
    ///
    /// `choices` answers every path [`PendingMerge::conflicts`] named:
    /// [`MergeSide::Local`] leaves this device's version, and
    /// [`MergeSide::Remote`] replaces it with the fetched one, a deletion
    /// included. A missing answer is refused rather than defaulted:
    /// silently keeping the local version is the last-write-wins outcome
    /// the sync model exists to refuse, and a caller forgetting a path
    /// must not reach it.
    ///
    /// `adds` are new files the merge also creates: how keep-both writes
    /// the losing version under a fresh id without this layer knowing what
    /// an entry is. They never replace a file, so one naming a path either
    /// side has, or the merge touches, is refused.
    ///
    /// A failure partway rolls back to the tree as it was. Without that a
    /// failed merge would leave a dirty tree that every later sync
    /// operation refuses over, wedging the vault until a human cleaned up.
    pub fn commit(
        self,
        message: &str,
        choices: &BTreeMap<String, MergeSide>,
        adds: &BTreeMap<String, Vec<u8>>,
    ) -> Result<MergeResult, Error> {
        if let Some(path) = self.conflicts.iter().find(|p| !choices.contains_key(*p)) {
            return Err(Error::Unanswered(path.clone()));
        }
        let touched = |p: &String| {
            self.local_paths.contains(p)
                || self.take_remote.contains(p)
                || self.conflicts.contains(p)
        };
        if let Some(path) = adds.keys().find(|p| touched(p)) {
            return Err(Error::AddWouldOverwrite(path.clone()));
        }
        let repo = open(&self.dir)?;
        let remote_tree = repo
            .find_commit(self.remote)
            .and_then(|c| c.tree())
            .map_err(fail("reading the remote's tree in", &self.dir))?;
        let take = self.take_remote.iter().chain(
            self.conflicts
                .iter()
                .filter(|p| choices.get(*p) == Some(&MergeSide::Remote)),
        );
        // A path joins `applied` before its write starts, so one that
        // failed halfway is still rolled back.
        let mut applied: Vec<&str> =
            Vec::with_capacity(self.take_remote.len() + self.conflicts.len() + adds.len());
        for path in take {
            applied.push(path);
            if let Err(e) = self.apply_remote_version(&repo, &remote_tree, path) {
                return Err(self.rollback(&repo, &applied, e));
            }
        }
        // A BTreeMap: written in the same order on every platform.
        for (path, content) in adds {
            applied.push(path);
            if let Err(e) = self.write_add(path, content) {
                return Err(self.rollback(&repo, &applied, e));
            }
        }
        match self.commit_merge(&repo, &applied, message) {
            Ok(commit) => Ok(MergeResult::Merged { commit }),
            Err(e) => Err(self.rollback(&repo, &applied, e)),
        }
    }

    /// Writes (or deletes) one path in the working tree to match the
    /// remote's version.
    fn apply_remote_version(
        &self,
        repo: &git2::Repository,
        remote_tree: &git2::Tree<'_>,
        path: &str,
    ) -> Result<(), Error> {
        let full = safe_join(&self.dir, path)?;
        let entry = match remote_tree.get_path(Path::new(path)) {
            Ok(entry) => entry,
            // Not in the remote's tree: the remote deleted it and we
            // hadn't touched it.
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                return remove_if_present(&full, path);
            }
            Err(e) => return Err(fail("reading the remote's tree in", &self.dir)(e)),
        };
        let blob = entry
            .to_object(repo)
            .and_then(|o| o.peel_to_blob())
            .map_err(fail("reading a remote file in", &self.dir))?;
        let io = |op| {
            move |source| Error::Io {
                op,
                path: path.to_string(),
                source,
            }
        };
        if let Some(parent) = full.parent() {
            create_private_dirs(parent).map_err(io("creating the directory of"))?;
        }
        write_file(&full, blob.content()).map_err(io("writing"))
    }

    /// Creates one of [`PendingMerge::commit`]'s adds.
    fn write_add(&self, path: &str, content: &[u8]) -> Result<(), Error> {
        let full = safe_join(&self.dir, path)?;
        let io = |op| {
            move |source| Error::Io {
                op,
                path: path.to_string(),
                source,
            }
        };
        if let Some(parent) = full.parent() {
            create_private_dirs(parent).map_err(io("creating the directory of"))?;
        }
        write_file(&full, content).map_err(io("writing"))
    }

    /// Stages what the merge wrote and commits it with both parents.
    fn commit_merge(
        &self,
        repo: &git2::Repository,
        applied: &[&str],
        message: &str,
    ) -> Result<String, Error> {
        let dir = &self.dir;
        let mut index = repo.index().map_err(fail("opening the index of", dir))?;
        for path in applied {
            let present = safe_join(dir, path)?.symlink_metadata().is_ok();
            if present {
                index.add_path(Path::new(path))
            } else {
                index.remove_path(Path::new(path))
            }
            .map_err(fail("staging the merge in", dir))?;
        }
        index.write().map_err(fail("staging the merge in", dir))?;
        let tree_id = index
            .write_tree()
            .map_err(fail("writing the merged tree of", dir))?;
        let tree = repo
            .find_tree(tree_id)
            .map_err(fail("reading the merged tree of", dir))?;
        let parents = [self.local, self.remote].map(|id| {
            repo.find_commit(id)
                .map_err(fail("reading a parent in", dir))
        });
        let [local, remote] = parents;
        let (local, remote) = (local?, remote?);
        let sig =
            git2::Signature::now(AUTHOR_NAME, AUTHOR_EMAIL).map_err(fail("signing in", dir))?;
        // A merge whose result equals the local tree is still recorded: it
        // marks the remote's history as incorporated, without which the
        // next push is rejected all over again.
        let id = repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&local, &remote])
            .map_err(fail("committing the merge in", dir))?;
        Ok(id.to_string())
    }

    /// Undoes a partly applied merge and returns `cause`. Discarding is
    /// safe here and only here: [`prepare_merge`] verified the tree was
    /// clean, so everything undone is this merge's own work.
    ///
    /// The hard reset restores tracked files; paths the merge created are
    /// untracked and invisible to it, so they are removed by name first.
    /// The reset runs whichever way those removals go: `applied` routinely
    /// names a path that was never created (the failed write is why we are
    /// here), and abandoning the rollback over it would leave every
    /// tracked file the merge already overwrote.
    fn rollback(&self, repo: &git2::Repository, applied: &[&str], cause: Error) -> Error {
        let mut failure = None;
        for path in applied.iter().filter(|p| !self.local_paths.contains(**p)) {
            let removed = safe_join(&self.dir, path).and_then(|full| {
                // Absent, however it reports it (ENOENT, or ENOTDIR under
                // a path that turned out to be a file): nothing to undo.
                if full.symlink_metadata().is_err() {
                    return Ok(());
                }
                remove_if_present(&full, path)
            });
            if let Err(e) = removed {
                failure.get_or_insert(e);
            }
        }
        let reset = repo
            .find_object(self.local, Some(git2::ObjectType::Commit))
            .and_then(|head| repo.reset(&head, git2::ResetType::Hard, None))
            .map_err(fail("rolling back the merge in", &self.dir));
        match reset.err().or(failure) {
            None => cause,
            Some(rollback) => Error::RollbackFailed {
                cause: Box::new(cause),
                rollback: Box::new(rollback),
            },
        }
    }
}

#[cfg(unix)]
fn create_private_dirs(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dirs(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn write_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, MERGED_FILE_MODE);
    #[cfg(not(unix))]
    let _ = MERGED_FILE_MODE;
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()
}
