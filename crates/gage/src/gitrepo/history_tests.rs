//! `log`: Go's `history_test` rows, and the merge cases they leave out.
//! The merge rows are what catch a walk that diffs each commit against its
//! neighbour instead of its parents (see the module doc of `history`).

use std::collections::BTreeMap;
use std::path::Path;

use super::remote_tests::{commit_file, published_repo};
use super::worktree_tests::write;
use super::*;
use crate::gittest::{self, Device};

fn fetch_now(dir: &Path) {
    fetch(dir, &gittest::remote_options()).unwrap();
}

fn contents(revisions: &[Revision]) -> Vec<Option<String>> {
    revisions
        .iter()
        .map(|r| {
            r.content
                .as_ref()
                .map(|c| String::from_utf8(c.clone()).unwrap())
        })
        .collect()
}

fn messages(revisions: &[Revision]) -> Vec<&str> {
    revisions.iter().map(|r| r.message.as_str()).collect()
}

/// A two-parent commit on HEAD whose tree is HEAD's with `rel` set to
/// `content`: a merge that resolved to a blob neither parent has, which
/// `merge_remote` never produces on its own.
fn merge_with_new_blob(dir: &Path, rel: &str, content: &str) {
    let repo = git2::Repository::open(dir).unwrap();
    let local = repo.head().unwrap().peel_to_commit().unwrap();
    let remote = repo
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let mut index = repo.index().unwrap();
    let blob = repo.blob(content.as_bytes()).unwrap();
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: content.len() as u32,
            id: blob,
            flags: 0,
            flags_extended: 0,
            path: rel.as_bytes().to_vec(),
        })
        .unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now(AUTHOR_NAME, AUTHOR_EMAIL).unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "resolved to neither",
        &tree,
        &[&local, &remote],
    )
    .unwrap();
}

// Go: TestLogReportsEachRealChangeOnce
#[test]
fn log_reports_each_real_change_once() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "one");
    write(dir.path(), "b.txt", "other");
    init_and_commit(dir.path(), "first").unwrap();
    commit_file(dir.path(), "b.txt", "other, edited"); // not a.txt
    commit_file(dir.path(), "a.txt", "two");
    commit_all(dir.path(), "empty commit").unwrap(); // nothing changed at all

    let log = log(dir.path(), "a.txt").unwrap();
    assert_eq!(
        contents(&log),
        [Some("two".into()), Some("one".into())],
        "newest first, one per real change"
    );
    assert_eq!(messages(&log), ["commit a.txt", "first"]);
    assert!(log.iter().all(|r| r.id.len() == 40));
}

// Go: TestLogReportsADeletionOnce
#[test]
fn log_reports_a_deletion_once() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "one");
    write(dir.path(), "keep.txt", "keep");
    init_and_commit(dir.path(), "first").unwrap();
    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    commit_all(dir.path(), "deleted").unwrap();
    commit_file(dir.path(), "keep.txt", "kept, edited"); // path still absent

    let log = log(dir.path(), "a.txt").unwrap();
    assert_eq!(contents(&log), [None, Some("one".into())]);
    assert_eq!(messages(&log), ["deleted", "first"]);
}

// Go: TestLogOnAPathGitHasNeverSeenIsEmpty
#[test]
fn log_on_a_path_git_has_never_seen_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "one");
    init_and_commit(dir.path(), "first").unwrap();
    assert!(log(dir.path(), "entries/nope.age").unwrap().is_empty());
}

// Go: TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet, the Log half.
#[test]
fn log_on_a_repo_with_no_commits_yet_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    git2::Repository::init(dir.path()).unwrap();
    assert!(log(dir.path(), "a.txt").unwrap().is_empty());
}

// Go: TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet, the
// HeadCommitterTime half.
#[test]
fn head_committer_time_on_a_repo_with_no_commits_yet() {
    let dir = tempfile::tempdir().unwrap();
    git2::Repository::init(dir.path()).unwrap();
    assert_eq!(head_committer_time(dir.path()).unwrap(), None);

    write(dir.path(), "a.txt", "one");
    init_and_commit(dir.path(), "first").unwrap();
    let got = head_committer_time(dir.path()).unwrap().expect("a commit");
    let drift = (jiff::Timestamp::now().as_second() - got.as_second()).abs();
    assert!(drift < 60, "{got}");
}

#[test]
fn head_committer_time_of_a_directory_that_is_not_a_repository_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(head_committer_time(dir.path()).is_err());
}

#[test]
fn log_of_a_directory_that_is_not_a_repository_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        log(dir.path(), "a.txt"),
        Err(Error::Git { op: "opening", .. })
    ));
}

/// An empty file is present, and absent is not: a file emptied is a
/// change, and a file deleted after being emptied is another.
#[test]
fn log_tells_an_empty_file_from_an_absent_one() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "other.txt", "x");
    init_and_commit(dir.path(), "no a yet").unwrap();
    commit_file(dir.path(), "a.txt", ""); // appears, empty
    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    commit_all(dir.path(), "deleted").unwrap();

    let log = log(dir.path(), "a.txt").unwrap();
    assert_eq!(contents(&log), [None, Some(String::new())]);
}

// Go: TestLogAcrossAMergeOfUnrelatedEdits ⚑
#[test]
fn log_across_a_merge_of_unrelated_edits() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("theirs.txt", "theirs", "their commit");
    commit_file(dir.path(), "mine.txt", "mine");
    fetch_now(dir.path());
    let MergeResult::Merged { .. } = merge_remote(dir.path(), "gage: merge origin").unwrap() else {
        panic!("disjoint changes conflicted");
    };

    // Each side's file has exactly its own commit: the merge is no change
    // for either, and the other side's commit is not reported against it.
    assert_eq!(
        messages(&log(dir.path(), "mine.txt").unwrap()),
        ["commit mine.txt"]
    );
    assert_eq!(
        messages(&log(dir.path(), "theirs.txt").unwrap()),
        ["their commit"]
    );
    assert_eq!(
        messages(&log(dir.path(), "seed.txt").unwrap()),
        ["initial commit"]
    );
}

// Go: TestLogAcrossAConflictResolvedByTakingASide ⚑
#[test]
fn log_across_a_conflict_resolved_by_taking_a_side() {
    for (side, winner) in [(MergeSide::Remote, "theirs"), (MergeSide::Local, "ours")] {
        let (dir, remote) = published_repo();
        Device::clone(remote.path()).write_commit_push("seed.txt", "theirs", "their edit");
        commit_file(dir.path(), "seed.txt", "ours");
        fetch_now(dir.path());
        let pending = prepare_merge(dir.path()).unwrap();
        let choices = BTreeMap::from([("seed.txt".to_string(), side)]);
        pending
            .commit("gage: merge origin", &choices, &BTreeMap::new())
            .unwrap();

        let log = log(dir.path(), "seed.txt").unwrap();
        // The merge matches one parent, so it is not a change; the walk
        // sees the three real writes.
        let mut seen = contents(&log);
        assert_eq!(seen.len(), 3, "taking {winner}: {seen:?}");
        assert_eq!(seen.pop().unwrap().as_deref(), Some("seed"));
        seen.sort();
        assert_eq!(seen, [Some("ours".into()), Some("theirs".into())]);
        assert!(!messages(&log).contains(&"gage: merge origin"));
    }
}

/// A merge whose result equals neither parent is a change: the case the
/// every-parent rule exists to keep.
#[test]
fn log_reports_a_merge_that_resolved_to_a_blob_neither_parent_has() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("seed.txt", "theirs", "their edit");
    commit_file(dir.path(), "seed.txt", "ours");
    fetch_now(dir.path());
    merge_with_new_blob(dir.path(), "seed.txt", "both, by hand");

    let log = log(dir.path(), "seed.txt").unwrap();
    assert_eq!(log.len(), 4);
    assert_eq!(log[0].message, "resolved to neither");
    assert_eq!(log[0].content.as_deref(), Some(&b"both, by hand"[..]));
}
