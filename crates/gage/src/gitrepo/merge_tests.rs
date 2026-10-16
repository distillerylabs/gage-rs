//! gage's file-level merge (Q-FILE-MERGE): Go's `remote_test` merge rows,
//! plus the recipient-file property `merge_trees` would have lost.

use std::collections::BTreeMap;
use std::path::Path;

use super::remote_tests::{commit_file, published_repo};
use super::worktree_tests::{head_id, read, write};
use super::*;
use crate::gittest::{self, BareRemote, Device};

fn fetch_now(dir: &std::path::Path) {
    fetch(dir, &gittest::remote_options()).unwrap();
}

// Go: TestMergeRemoteMergesDisjointChanges
#[test]
fn merge_remote_merges_disjoint_changes() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("theirs.txt", "theirs", "their commit");
    commit_file(dir.path(), "mine.txt", "mine");
    fetch_now(dir.path());
    let local = head_id(dir.path());

    let MergeResult::Merged { commit } = merge_remote(dir.path(), "gage: merge origin").unwrap()
    else {
        panic!("disjoint changes conflicted");
    };
    assert_eq!(read(dir.path(), "theirs.txt"), "theirs");
    assert_eq!(read(dir.path(), "mine.txt"), "mine");
    assert!(is_clean(dir.path()).unwrap());

    // A real merge commit: parents [local, remote], gage's message and
    // identity, naming no file.
    let repo = git2::Repository::open(dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.id().to_string(), commit);
    let parents: Vec<String> = head.parent_ids().map(|p| p.to_string()).collect();
    let tracking = repo
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .target()
        .unwrap()
        .to_string();
    assert_eq!(parents, [local, tracking]);
    assert_eq!(head.message().unwrap(), "gage: merge origin");
    assert_eq!(head.author().name().unwrap(), AUTHOR_NAME);
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::Ahead);
}

// Go: TestMergeRemoteConflictsAndChangesNothing
#[test]
fn merge_remote_conflicts_and_changes_nothing() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("seed.txt", "their version", "their edit");
    Device::clone(remote.path()); // a second clone must not matter
    commit_file(dir.path(), "seed.txt", "my version");
    fetch_now(dir.path());
    let before = head_id(dir.path());

    let result = merge_remote(dir.path(), "gage: merge origin").unwrap();
    assert_eq!(result, MergeResult::Conflicts(vec!["seed.txt".into()]));
    assert_eq!(head_id(dir.path()), before);
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestMergeRemoteTakesADeletionAndConflictsOnDeleteModify: the two
// deletion shapes of "What \"diverged\" actually means".
#[test]
fn merge_remote_takes_a_deletion_and_conflicts_on_delete_modify() {
    // A deletion we hadn't touched is taken.
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.remove("seed.txt");
    other.commit("their deletion");
    other.push();
    commit_file(dir.path(), "mine.txt", "mine");
    fetch_now(dir.path());
    let merged = merge_remote(dir.path(), "gage: merge origin").unwrap();
    assert!(matches!(merged, MergeResult::Merged { .. }), "{merged:?}");
    assert!(!dir.path().join("seed.txt").exists());
    assert!(is_clean(dir.path()).unwrap());

    // Delete on one side, modify on the other: a conflict.
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.remove("seed.txt");
    other.commit("their deletion");
    other.push();
    commit_file(dir.path(), "seed.txt", "my edit of the file they deleted");
    fetch_now(dir.path());
    assert_eq!(
        merge_remote(dir.path(), "gage: merge origin").unwrap(),
        MergeResult::Conflicts(vec!["seed.txt".into()])
    );
    assert_eq!(
        read(dir.path(), "seed.txt"),
        "my edit of the file they deleted"
    );
}

#[test]
fn identical_changes_on_both_sides_are_not_a_conflict() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("same.txt", "same", "theirs");
    commit_file(dir.path(), "same.txt", "same");
    fetch_now(dir.path());
    let merged = merge_remote(dir.path(), "gage: merge origin").unwrap();
    assert!(matches!(merged, MergeResult::Merged { .. }), "{merged:?}");
}

// Q-FILE-MERGE, beyond Go's rows: two devices that each add a recipient
// line conflict on `.age-recipients`, with no `.gitattributes` in the
// repository at all. A line-level merge (`git2::merge_trees`) would union
// them into a list neither device wrote, held back only by that file's
// `-merge`.
#[test]
fn two_added_recipients_conflict_without_gitattributes() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".age-recipients", "age1first\nage1second\n");
    init_and_commit(dir.path(), "seed").unwrap();
    assert!(!dir.path().join(".gitattributes").exists());
    let remote = gittest::new_bare_remote();
    set_remote(dir.path(), remote.path().to_str().unwrap()).unwrap();
    push(dir.path(), &gittest::remote_options()).unwrap();

    Device::clone(remote.path()).write_commit_push(
        ".age-recipients",
        "age1first\nage1second\nage1theirs\n",
        "their recipient",
    );
    commit_file(
        dir.path(),
        ".age-recipients",
        "age1ours\nage1first\nage1second\n",
    );
    fetch_now(dir.path());

    assert_eq!(
        merge_remote(dir.path(), "gage: merge origin").unwrap(),
        MergeResult::Conflicts(vec![".age-recipients".into()])
    );
    assert_eq!(
        read(dir.path(), ".age-recipients"),
        "age1ours\nage1first\nage1second\n"
    );
}

// Q-FILE-MERGE: a merge that fails partway rolls back, removing what it
// created and reaching the hard reset even past a path it never managed
// to write. Here the remote adds `a.txt` (written first) and `x/y.txt`,
// while this side committed a *file* `x`: creating the directory fails.
#[test]
fn a_partial_merge_failure_rolls_back() {
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write("a.txt", "theirs");
    other.write("x/y.txt", "theirs");
    other.write("seed.txt", "their edit of a tracked file");
    other.commit("theirs");
    other.push();
    commit_file(dir.path(), "x", "a file where they have a directory");
    fetch_now(dir.path());
    let before = head_id(dir.path());

    let err = merge_remote(dir.path(), "gage: merge origin").unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err}");
    assert_eq!(head_id(dir.path()), before);
    assert!(
        !dir.path().join("a.txt").exists(),
        "created file left behind"
    );
    assert_eq!(
        read(dir.path(), "seed.txt"),
        "seed",
        "tracked file not restored"
    );
    assert!(
        is_clean(dir.path()).unwrap(),
        "{:?}",
        dirty_paths(dir.path())
    );
}

#[test]
fn merging_with_nothing_fetched_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "seed.txt", "seed");
    init_and_commit(dir.path(), "seed").unwrap();
    let err = merge_remote(dir.path(), "m").unwrap_err();
    assert!(matches!(err, Error::NothingFetched { .. }), "{err}");
}

#[test]
fn a_pending_merge_with_conflicts_refuses_to_commit() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("seed.txt", "theirs", "t");
    commit_file(dir.path(), "seed.txt", "ours");
    fetch_now(dir.path());
    let pending = prepare_merge(dir.path()).unwrap();
    assert_eq!(pending.conflicts(), ["seed.txt"]);
    let err = pending
        .commit("m", &BTreeMap::new(), &BTreeMap::new())
        .unwrap_err();
    assert!(
        matches!(err, Error::Unanswered(ref p) if p == "seed.txt"),
        "{err}"
    );
}

/// `dir` diverged from its remote on `seed.txt` (both sides edited it),
/// fetched, with the merge computed and nothing written: the seam M8b's
/// resolution lives in.
fn prepared_conflict() -> (tempfile::TempDir, BareRemote, PendingMerge) {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("seed.txt", "their version", "their edit");
    commit_file(dir.path(), "seed.txt", "my version");
    fetch_now(dir.path());
    let merge = prepare_merge(dir.path()).unwrap();
    (dir, remote, merge)
}

fn choose(path: &str, side: MergeSide) -> BTreeMap<String, MergeSide> {
    BTreeMap::from([(path.to_string(), side)])
}

fn add(path: &str, content: &str) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([(path.to_string(), content.as_bytes().to_vec())])
}

// Go: TestPrepareMergeDecidesWithoutWriting
#[test]
fn prepare_merge_decides_without_writing() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("seed.txt", "their version", "their edit");
    commit_file(dir.path(), "seed.txt", "my version");
    fetch_now(dir.path());
    let before = head_id(dir.path());

    let merge = prepare_merge(dir.path()).unwrap();
    assert_eq!(merge.conflicts(), ["seed.txt"]);
    assert_eq!(head_id(dir.path()), before);
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    assert!(is_clean(dir.path()).unwrap());
    // Dropping it unapplied leaves nothing behind: what makes abort free.
    drop(merge);
    assert_eq!(head_id(dir.path()), before);
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestPendingMergeContentReadsBothSidesFromTheObjects
#[test]
fn pending_merge_content_reads_both_sides_from_the_objects() {
    let (dir, _remote, merge) = prepared_conflict();
    assert_eq!(
        merge
            .content(MergeSide::Local, "seed.txt")
            .unwrap()
            .as_deref(),
        Some(&b"my version"[..])
    );
    assert_eq!(
        merge
            .content(MergeSide::Remote, "seed.txt")
            .unwrap()
            .as_deref(),
        Some(&b"their version"[..])
    );
    // Reading the remote's version put it nowhere a later command could
    // mistake for a merged result.
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestPendingMergeContentReportsAnAbsentSide
#[test]
fn pending_merge_content_reports_an_absent_side() {
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.remove("seed.txt");
    other.commit("their deletion");
    other.push();
    commit_file(dir.path(), "seed.txt", "my edit of the file they deleted");
    fetch_now(dir.path());

    let merge = prepare_merge(dir.path()).unwrap();
    assert_eq!(merge.conflicts(), ["seed.txt"]);
    assert_eq!(merge.content(MergeSide::Remote, "seed.txt").unwrap(), None);
    assert_eq!(
        merge
            .content(MergeSide::Local, "seed.txt")
            .unwrap()
            .as_deref(),
        Some(&b"my edit of the file they deleted"[..])
    );
}

// Go: TestCommitRefusesAnUnansweredConflict
#[test]
fn commit_refuses_an_unanswered_conflict() {
    let (dir, _remote, merge) = prepared_conflict();
    let before = head_id(dir.path());
    // An answer for some other path is not an answer for this one.
    let err = merge
        .commit(
            "gage: merge origin",
            &choose("other.txt", MergeSide::Local),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::Unanswered(ref p) if p == "seed.txt"),
        "{err}"
    );
    assert!(err.to_string().contains("seed.txt"), "{err}");
    assert_eq!(head_id(dir.path()), before);
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestCommitAppliesEachChoice
#[test]
fn commit_applies_each_choice() {
    for (side, want) in [
        (MergeSide::Local, "my version"),
        (MergeSide::Remote, "their version"),
    ] {
        let (dir, _remote, merge) = prepared_conflict();
        let local = head_id(dir.path());
        let result = merge
            .commit(
                "gage: merge origin",
                &choose("seed.txt", side),
                &BTreeMap::new(),
            )
            .unwrap();
        let MergeResult::Merged { commit } = result else {
            panic!("{side:?}: not merged: {result:?}");
        };
        assert_eq!(read(dir.path(), "seed.txt"), want, "{side:?}");
        assert!(is_clean(dir.path()).unwrap(), "{side:?}");
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id().to_string(), commit);
        assert_eq!(head.parent_count(), 2, "{side:?}");
        assert_eq!(head.parent_id(0).unwrap().to_string(), local);
    }
}

// Go: TestCommitCreatesTheAddsItIsGiven
#[test]
fn commit_creates_the_adds_it_is_given() {
    let (dir, _remote, merge) = prepared_conflict();
    merge
        .commit(
            "gage: merge origin",
            &choose("seed.txt", MergeSide::Local),
            &add("entries/new.age", "the losing version"),
        )
        .unwrap();
    assert_eq!(read(dir.path(), "entries/new.age"), "the losing version");
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    // Committed, not merely written: an untracked add would leave every
    // later sync refusing over a dirty tree.
    assert!(
        is_clean(dir.path()).unwrap(),
        "{:?}",
        dirty_paths(dir.path())
    );
    let repo = git2::Repository::open(dir.path()).unwrap();
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    assert!(tree.get_path(Path::new("entries/new.age")).is_ok());
}

// Go: TestCommitRefusesAnAddThatWouldOverwrite
#[test]
fn commit_refuses_an_add_that_would_overwrite() {
    let (dir, _remote, merge) = prepared_conflict();
    let before = head_id(dir.path());
    let err = merge
        .commit(
            "gage: merge origin",
            &choose("seed.txt", MergeSide::Local),
            &add("seed.txt", "clobber"),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::AddWouldOverwrite(ref p) if p == "seed.txt"),
        "{err}"
    );
    assert_eq!(read(dir.path(), "seed.txt"), "my version");
    assert_eq!(head_id(dir.path()), before);

    // Nor a path the merge itself is bringing in from the remote.
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write("seed.txt", "their version");
    other.write("theirs.txt", "theirs");
    other.commit("theirs");
    other.push();
    commit_file(dir.path(), "seed.txt", "my version");
    fetch_now(dir.path());
    let err = prepare_merge(dir.path())
        .unwrap()
        .commit(
            "gage: merge origin",
            &choose("seed.txt", MergeSide::Local),
            &add("theirs.txt", "clobber"),
        )
        .unwrap_err();
    assert!(matches!(err, Error::AddWouldOverwrite(_)), "{err}");
    assert!(!dir.path().join("theirs.txt").exists());
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestMergeCreatedFilesAre0600
#[cfg(unix)]
#[test]
fn merge_created_files_are_0600() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write("seed.txt", "their version");
    other.write("entries/theirs.age", "an entry this device has never seen");
    other.commit("their edit and a new entry");
    other.push();
    commit_file(dir.path(), "seed.txt", "my version");
    fetch_now(dir.path());

    prepare_merge(dir.path())
        .unwrap()
        .commit(
            "gage: merge origin",
            &choose("seed.txt", MergeSide::Local),
            &add("entries/mine.age", "the losing version"),
        )
        .unwrap();
    for path in ["entries/theirs.age", "entries/mine.age"] {
        let mode = std::fs::metadata(dir.path().join(path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{path}: {:o}", mode & 0o777);
    }
}

// Go: TestCommitRollsBackAPartialFailure. The add lands under a path that
// is a *file*, after the remote's own new file and its edit of a tracked
// file have been applied: the rollback removes the first by name and
// restores the second with the hard reset, even though the add's own path
// can't be statted.
#[test]
fn commit_rolls_back_a_partial_failure() {
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write("seed.txt", "their version");
    other.write("theirs.txt", "only they touched this");
    other.commit("their edit and their new file");
    other.push();
    commit_file(dir.path(), "blocker", "a file, not a directory");
    fetch_now(dir.path());

    let merge = prepare_merge(dir.path()).unwrap();
    assert!(merge.conflicts().is_empty(), "{:?}", merge.conflicts());
    let before = head_id(dir.path());
    let err = merge
        .commit(
            "gage: merge origin",
            &BTreeMap::new(),
            &add("blocker/impossible.age", "nowhere to put this"),
        )
        .unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err}");
    assert_eq!(head_id(dir.path()), before);
    assert!(!dir.path().join("theirs.txt").exists(), "created file left");
    assert_eq!(read(dir.path(), "seed.txt"), "seed", "reset skipped");
    assert_eq!(read(dir.path(), "blocker"), "a file, not a directory");
    assert!(
        is_clean(dir.path()).unwrap(),
        "{:?}",
        dirty_paths(dir.path())
    );
}

// Go: TestMergeRemoteAndPrepareMergeCannotDrift
#[test]
fn merge_remote_and_prepare_merge_cannot_drift() {
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write("seed.txt", "their version");
    other.write("b.txt", "theirs");
    other.write("only-theirs.txt", "theirs");
    other.commit("theirs");
    other.push();
    write(dir.path(), "seed.txt", "my version");
    write(dir.path(), "b.txt", "mine");
    write(dir.path(), "only-mine.txt", "mine");
    commit_all(dir.path(), "mine").unwrap();
    fetch_now(dir.path());

    let prepared = prepare_merge(dir.path()).unwrap().conflicts().to_vec();
    assert_eq!(prepared, ["b.txt", "seed.txt"]);
    assert_eq!(
        merge_remote(dir.path(), "gage: merge origin").unwrap(),
        MergeResult::Conflicts(prepared)
    );
}
