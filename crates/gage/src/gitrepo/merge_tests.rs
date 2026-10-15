//! gage's file-level merge (Q-FILE-MERGE): Go's `remote_test` merge rows,
//! plus the recipient-file property `merge_trees` would have lost.

use super::remote_tests::{commit_file, published_repo};
use super::worktree_tests::{head_id, read, write};
use super::*;
use crate::gittest::{self, Device};

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
    let err = pending.commit("m").unwrap_err();
    assert!(
        matches!(err, Error::Unanswered(ref p) if p == "seed.txt"),
        "{err}"
    );
}
