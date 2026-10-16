//! The dirty-tree guard and `reset_hard`: Go's `gitrepo_test` (1 row),
//! `reset_test` and `worktree_state_test`.

use std::path::Path;

use super::remote_tests::{diverge_from, published_repo};
use super::*;
use crate::gittest;

pub(super) fn write(dir: &Path, rel: &str, content: &str) {
    let full = dir.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
}

pub(super) fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

// Go: TestRequireCleanWorkTreeTruncatesTheNamedPaths
#[test]
fn require_clean_work_tree_truncates_the_named_paths() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "seed.txt", "seed");
    init_and_commit(dir.path(), "initial commit").unwrap();
    for i in 0..=MAX_NAMED_DIRTY_PATHS {
        write(dir.path(), &format!("f{i}.txt"), "dirty");
    }

    let err = require_clean_work_tree(dir.path(), "testing").unwrap_err();
    let Error::DirtyWorkTree { paths, .. } = &err else {
        panic!("{err}");
    };
    assert_eq!(paths.len(), MAX_NAMED_DIRTY_PATHS + 1, "all paths kept");
    let msg = err.to_string();
    for i in 0..MAX_NAMED_DIRTY_PATHS {
        assert!(msg.contains(&format!("f{i}.txt")), "{msg}");
    }
    assert!(
        !msg.contains(&format!("f{MAX_NAMED_DIRTY_PATHS}.txt")),
        "{msg}"
    );
    assert!(msg.contains("and 1 more"), "{msg}");
    assert!(msg.contains("so gage is not testing"), "{msg}");
    assert_eq!(err.code(), Code::Conflict);
}

#[test]
fn dirty_paths_lists_modified_and_untracked_files_sorted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "b.txt", "b");
    init_and_commit(dir.path(), "seed").unwrap();
    assert!(dirty_paths(dir.path()).unwrap().is_empty());
    write(dir.path(), "b.txt", "changed");
    write(dir.path(), "entries/a.age", "new");
    assert_eq!(dirty_paths(dir.path()).unwrap(), ["b.txt", "entries/a.age"]);
}

// Go: TestMergeRemoteRefusesADirtyWorkTree. Go's correction: a merge
// stages the whole tree, so without this guard a human's half-finished
// edit was swept into "gage: merge origin" and pushed everywhere.
#[test]
fn merge_remote_refuses_a_dirty_work_tree() {
    let (dir, remote) = published_repo();
    diverge_from(dir.path(), &remote, "theirs.txt", "ours.txt");
    write(dir.path(), "human-scratch.txt", "half-typed note");
    write(dir.path(), "seed.txt", "edited by hand");
    let before = head_id(dir.path());

    let err = merge_remote(dir.path(), "gage: merge origin").unwrap_err();
    assert!(matches!(err, Error::DirtyWorkTree { .. }), "{err}");
    for path in ["human-scratch.txt", "seed.txt"] {
        assert!(err.to_string().contains(path), "{err}");
    }
    assert_eq!(read(dir.path(), "seed.txt"), "edited by hand");
    assert!(!dir.path().join("theirs.txt").exists());
    assert_eq!(head_id(dir.path()), before);
}

// Go: TestFastForwardAndMergeShareOneDirtyTreeSentinel. One variant, so
// the layer above matches either refusal in one place.
#[test]
fn fast_forward_and_merge_share_one_dirty_tree_sentinel() {
    let (dir, remote) = published_repo();
    let other = gittest::Device::clone(remote.path());
    other.write_commit_push("theirs.txt", "theirs", "their commit");
    fetch(dir.path(), &gittest::remote_options()).unwrap();
    write(dir.path(), "seed.txt", "edited by hand");

    let ff = fast_forward(dir.path()).unwrap_err();
    assert!(matches!(ff, Error::DirtyWorkTree { .. }), "{ff}");
    let merge = merge_remote(dir.path(), "m").unwrap_err();
    assert!(matches!(merge, Error::DirtyWorkTree { .. }), "{merge}");
    assert_eq!(ff.code(), merge.code());
}

pub(super) fn head_id(dir: &Path) -> String {
    git2::Repository::open(dir)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap()
        .to_string()
}

/// A repository with one commit and no remote: all `reset_hard` looks at.
/// It holds a vault's root recipient file too, since an interrupted
/// recipient write is the one case that dirties only that.
fn seeded_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "entries/tracked.age", "committed");
    write(dir.path(), ".age-recipients", "age1committed\n");
    init_and_commit(dir.path(), "seed").unwrap();
    dir
}

// Go: TestResetHardDiscardsModifiedAndUntrackedFiles. The untracked half
// is the one worth pinning: a hard reset leaves untracked files alone, so
// an interrupted insert's orphan must be removed explicitly or it is
// folded into the next write's commit.
#[test]
fn reset_hard_discards_modified_and_untracked_files() {
    let dir = seeded_repo();
    write(dir.path(), "entries/tracked.age", "half-written ciphertext");
    write(
        dir.path(),
        "entries/orphan.age",
        "an insert that never committed",
    );

    let discarded = reset_hard(dir.path()).unwrap();

    assert_eq!(discarded, ["entries/orphan.age", "entries/tracked.age"]);
    assert_eq!(read(dir.path(), "entries/tracked.age"), "committed");
    assert!(!dir.path().join("entries/orphan.age").exists());
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestResetHardOnACleanTreeDiscardsNothing
#[test]
fn reset_hard_on_a_clean_tree_discards_nothing() {
    let dir = seeded_repo();
    assert!(reset_hard(dir.path()).unwrap().is_empty());
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestResetHardLeavesHEADWhereItWas
#[test]
fn reset_hard_leaves_head_where_it_was() {
    let dir = seeded_repo();
    let before = head_id(dir.path());
    write(dir.path(), "entries/tracked.age", "half-written");
    reset_hard(dir.path()).unwrap();
    assert_eq!(head_id(dir.path()), before);
    assert_eq!(commit_count(dir.path()).unwrap(), 1);
}

// Go: TestResetHardDiscardsChangesOutsideEntries. An interrupted
// `recipient add` dirties only the recipient pair; a reset keyed off
// entries/ would find nothing and let the next write commit an abandoned
// recipient list.
#[test]
fn reset_hard_discards_changes_outside_entries() {
    let dir = seeded_repo();
    write(
        dir.path(),
        ".age-recipients",
        "age1committed\nage1abandoned\n",
    );
    assert_eq!(reset_hard(dir.path()).unwrap(), [".age-recipients"]);
    assert_eq!(read(dir.path(), ".age-recipients"), "age1committed\n");
    assert!(is_clean(dir.path()).unwrap());
}

// Go: TestSafeJoinRefusesPathsThatEscapeTheVault
#[test]
fn safe_join_refuses_paths_that_escape_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "../outside.txt",
        "entries/../../outside.txt",
        "..",
        "",
        "/etc/passwd",
        "a\0b",
        ".",
    ] {
        let err = safe_join(dir.path(), path).unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)), "{path:?}: {err}");
    }
    #[cfg(windows)]
    for path in [r"..\outside.txt", r"C:\Windows\x", r"entries\..\..\x"] {
        assert!(safe_join(dir.path(), path).is_err(), "{path:?}");
    }
    assert_eq!(
        safe_join(dir.path(), "entries/abc.age").unwrap(),
        dir.path().join("entries").join("abc.age")
    );
}
