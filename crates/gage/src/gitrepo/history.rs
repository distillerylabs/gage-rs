//! One path's history: the commits that changed it, newest first.
//!
//! A commit is a change when its version of the path differs from *every*
//! parent's. Diffing against the previous commit in the walk instead would
//! invent changes and deletions on any vault that has synced, because a
//! walk interleaves both sides of every merge (the bug Go's first `Log`
//! shipped with). "Absent" and "present but empty" are different versions,
//! so presence is compared along with content.

use std::collections::HashMap;
use std::path::Path;

use super::{Error, fail, open};

/// One commit that changed a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The commit id, in full.
    pub id: String,
    /// The author time, as Go's `c.Author.When`.
    pub when: jiff::Timestamp,
    /// The commit message. gage's are an entry's UUID and nothing else.
    pub message: String,
    /// The path's bytes at this commit; `None` when the commit deleted it.
    pub content: Option<Vec<u8>>,
}

/// What a path is at one commit: the blob there, or `None` when absent.
type Version = Option<git2::Oid>;

/// Every commit reachable from HEAD that changed `path` (slash-separated,
/// relative to the repository root), newest first. A path git has never
/// seen, and a repository with no commits yet, give an empty list.
pub fn log(dir: &Path, path: &str) -> Result<Vec<Revision>, Error> {
    let repo = open(dir)?;
    match repo.head() {
        Ok(_) => {}
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(Vec::new()),
        Err(e) => return Err(fail("reading HEAD of", dir)(e)),
    }
    let mut walk = repo.revwalk().map_err(fail("walking", dir))?;
    // Time order, with children kept ahead of their parents when
    // timestamps tie (a vault commits several times a second).
    walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .and_then(|()| walk.push_head())
        .map_err(fail("walking", dir))?;

    let mut versions: HashMap<git2::Oid, Version> = HashMap::new();
    let mut version_at = |id: git2::Oid| -> Result<Version, Error> {
        if let Some(v) = versions.get(&id) {
            return Ok(*v);
        }
        let tree = repo
            .find_commit(id)
            .and_then(|c| c.tree())
            .map_err(fail("reading a commit of", dir))?;
        let v = match tree.get_path(Path::new(path)) {
            Ok(entry) => Some(entry.id()),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(fail("reading a tree of", dir)(e)),
        };
        versions.insert(id, v);
        Ok(v)
    };

    let mut out = Vec::new();
    for id in walk {
        let id = id.map_err(fail("walking", dir))?;
        let commit = repo
            .find_commit(id)
            .map_err(fail("reading a commit of", dir))?;
        let here = version_at(id)?;
        let changed = if commit.parent_count() == 0 {
            here.is_some()
        } else {
            let mut changed = true;
            for parent in commit.parent_ids() {
                if version_at(parent)? == here {
                    changed = false;
                    break;
                }
            }
            changed
        };
        if !changed {
            continue;
        }
        let content = match here {
            Some(blob) => Some(
                repo.find_blob(blob)
                    .map_err(fail("reading a blob of", dir))?
                    .content()
                    .to_vec(),
            ),
            None => None,
        };
        out.push(Revision {
            id: id.to_string(),
            when: jiff::Timestamp::from_second(commit.author().when().seconds())
                .unwrap_or(jiff::Timestamp::UNIX_EPOCH),
            message: String::from_utf8_lossy(commit.message_bytes()).into_owned(),
            content,
        });
    }
    Ok(out)
}
