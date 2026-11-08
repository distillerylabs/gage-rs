//! Hostile-checkout regressions (issue #195): a git writer holds no vault
//! key but controls every tree the remote serves. A vault only ever contains
//! regular files ([ADR-0064]); these prove gage upholds that when it
//! materializes a remote's tree.
//!
//! Three guarantees:
//!   - the file-level merge writer never follows a symlink already on disk
//!     (so a committed symlink can't redirect a merged write outside the
//!     vault), even for a symlink gage itself would never have written;
//!   - a merge refuses a tree entry that is a symlink or gitlink;
//!   - `clone` and `fast_forward` never lay a traversable symlink on disk.

use std::path::Path;

use super::remote_tests::{commit_file, published_repo};
use super::*;
use crate::gittest::{self, BareRemote};

fn opts() -> RemoteOptions {
    gittest::remote_options()
}

fn url(remote: &BareRemote) -> &str {
    remote.path().to_str().unwrap()
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Adds `name` as a symlink blob pointing at `target` on top of the bare
/// remote's current `main`, the way a git writer with no key would. `name`
/// is a single path component (no nesting needed to exercise the guards).
fn push_symlink_entry(remote: &BareRemote, name: &str, target: &str) {
    let repo = git2::Repository::open_bare(remote.path()).unwrap();
    let parent = repo
        .find_reference("refs/heads/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let blob = repo.blob(target.as_bytes()).unwrap();
    let mut tb = repo.treebuilder(Some(&parent.tree().unwrap())).unwrap();
    // 0o120000 is git's symlink filemode.
    tb.insert(name, blob, 0o120_000).unwrap();
    let tree = repo.find_tree(tb.write().unwrap()).unwrap();
    let sig = git2::Signature::now("attacker", "attacker@evil.example").unwrap();
    let oid = repo
        .commit(None, &sig, &sig, "attacker: symlink", &tree, &[&parent])
        .unwrap();
    repo.reference("refs/heads/main", oid, true, "attacker: symlink")
        .unwrap();
}

/// A merge refuses a remote tree entry that is a symlink, writing nothing.
#[test]
fn merge_refuses_a_symlink_tree_entry() {
    let (dir, remote) = published_repo();
    push_symlink_entry(&remote, "link", "/etc/passwd");
    // A divergent local commit, so the pull merges rather than fast-forwards.
    commit_file(dir.path(), "mine.txt", "mine");
    fetch(dir.path(), &opts()).unwrap();

    let err = merge_remote(dir.path(), "gage: merge origin").unwrap_err();
    assert!(
        matches!(&err, Error::UnsafeEntry { path, .. } if path == "link"),
        "{err}"
    );
    assert!(!dir.path().join("link").exists(), "nothing was written");
    assert!(is_clean(dir.path()).unwrap(), "the tree is left clean");
}

/// A `clone` of a vault whose tree carries a symlink blob leaves a regular
/// file on disk, never a traversable symlink.
#[test]
fn clone_does_not_lay_down_a_traversable_symlink() {
    let (_dir, remote) = published_repo();
    push_symlink_entry(&remote, "link", "/etc/passwd");

    let into = tempfile::tempdir().unwrap();
    let vault = into.path().join("vault");
    clone(url(&remote), &vault, &opts()).unwrap();

    let link = vault.join("link");
    assert!(link.exists(), "the entry was checked out");
    assert!(!is_symlink(&link), "it must not be a traversable symlink");
}

/// A `fast_forward` that pulls in a symlink blob also leaves a regular file.
#[test]
fn fast_forward_does_not_lay_down_a_traversable_symlink() {
    let (dir, remote) = published_repo();
    push_symlink_entry(&remote, "link", "/etc/passwd");
    fetch(dir.path(), &opts()).unwrap();

    assert!(fast_forward(dir.path()).unwrap(), "behind, so it moves");
    let link = dir.path().join("link");
    assert!(link.exists());
    assert!(!is_symlink(&link), "it must not be a traversable symlink");
}

/// The direct F12 regression: even when a symlink is already on disk at a
/// path (however it got there — an older gage, an external tool), the merge
/// writer replaces it rather than writing through it, so a merged write
/// cannot reach the symlink's target outside the vault.
///
/// The local repo is set up with raw git2 (and `core.symlinks=true`) so the
/// symlink is genuinely committed and on disk in a clean tree — the state
/// gage's own checkout no longer produces, but the writer must still be safe
/// against.
#[cfg(unix)]
#[test]
fn merge_does_not_write_through_a_committed_symlink() {
    use crate::gittest::Device;
    use std::os::unix::fs::symlink;

    // A file outside the vault the attacker wants overwritten.
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("escaped");
    std::fs::write(&sentinel, b"UNTOUCHED").unwrap();

    // Local vault, built with git2 so `pwn` is a real on-disk symlink
    // committed in a clean tree.
    let dir = tempfile::tempdir().unwrap();
    {
        let repo = git2::Repository::init_opts(
            dir.path(),
            git2::RepositoryInitOptions::new().initial_head(INITIAL_BRANCH),
        )
        .unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.symlinks", true)
            .unwrap();
        std::fs::write(dir.path().join("seed.txt"), b"seed").unwrap();
        symlink(&sentinel, dir.path().join("pwn")).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now(AUTHOR_NAME, AUTHOR_EMAIL).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "base with symlink", &tree, &[])
            .unwrap();
    }
    assert!(
        is_symlink(&dir.path().join("pwn")),
        "precondition: symlink on disk"
    );

    let remote = gittest::new_bare_remote();
    set_remote(dir.path(), url(&remote)).unwrap();
    assert!(push(dir.path(), &opts()).unwrap());

    // The attacker, from another clone, turns `pwn` into a regular file with
    // chosen bytes and pushes.
    let other = Device::clone(remote.path());
    std::fs::remove_file(other.path().join("pwn")).unwrap();
    std::fs::write(other.path().join("pwn"), b"ATTACKER").unwrap();
    other.commit("replace symlink with a regular file");
    other.push();

    // This side makes a divergent change, then pulls and merges.
    commit_file(dir.path(), "mine.txt", "mine");
    fetch(dir.path(), &opts()).unwrap();
    let _ = merge_remote(dir.path(), "gage: merge origin");

    // The merge must not have written through the symlink.
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"UNTOUCHED",
        "the merge followed the symlink and wrote outside the vault"
    );
    assert!(
        !is_symlink(&dir.path().join("pwn")),
        "the symlink should have been replaced, not followed"
    );
}
