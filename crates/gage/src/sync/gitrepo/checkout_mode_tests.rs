//! Entry-file modes after a checkout (issue #197). libgit2 honors the mode a
//! tree records, and git only ever records 0644 or 0755, so every path that
//! materializes `entries/` from a tree has to bring it back to the 0600/0700
//! the format specifies. Windows applies no Unix modes.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use super::remote_tests::{commit_file, published_repo};
use super::*;
use crate::gittest::{self, BareRemote, Device};

const ENTRY: &str = "38149e3b-0000-4000-8000-000000000000.age";
const OTHER_ENTRY: &str = "9b2f7a10-0000-4000-8000-000000000000.age";
const BLOB_MODE: i32 = 0o100_644;
const EXECUTABLE_BLOB_MODE: i32 = 0o100_755;
const TREE_MODE: i32 = 0o040_000;

fn opts() -> RemoteOptions {
    gittest::remote_options()
}

fn url(remote: &BareRemote) -> &str {
    remote.path().to_str().unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// Commits `build`'s tree on top of the bare remote's `main`.
fn push_tree(remote: &BareRemote, build: impl FnOnce(&git2::Repository, &git2::Tree) -> git2::Oid) {
    let repo = git2::Repository::open_bare(remote.path()).unwrap();
    let parent = repo
        .find_reference("refs/heads/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let tree = repo
        .find_tree(build(&repo, &parent.tree().unwrap()))
        .unwrap();
    let sig = git2::Signature::now("other", "other@example.com").unwrap();
    let oid = repo
        .commit(None, &sig, &sig, "other: push", &tree, &[&parent])
        .unwrap();
    repo.reference("refs/heads/main", oid, true, "other: push")
        .unwrap();
}

/// Adds `entries/<name>` to the remote, recorded with `filemode`.
fn push_entry(remote: &BareRemote, name: &str, filemode: i32) {
    push_tree(remote, |repo, root| {
        let blob = repo.blob(b"age ciphertext").unwrap();
        let existing = root.get_name("entries").map(|e| e.to_object(repo).unwrap());
        let mut entries = repo
            .treebuilder(existing.as_ref().and_then(|o| o.as_tree()))
            .unwrap();
        entries.insert(name, blob, filemode).unwrap();
        let mut top = repo.treebuilder(Some(root)).unwrap();
        top.insert("entries", entries.write().unwrap(), TREE_MODE)
            .unwrap();
        top.write().unwrap()
    });
}

fn cloned(remote: &BareRemote) -> (tempfile::TempDir, std::path::PathBuf) {
    let into = tempfile::tempdir().unwrap();
    let vault = into.path().join("vault");
    clone(url(remote), &vault, &opts()).unwrap();
    (into, vault)
}

#[test]
fn clone_restricts_entries_and_leaves_other_files_alone() {
    let (_dir, remote) = published_repo();
    push_entry(&remote, ENTRY, BLOB_MODE);

    let (_into, vault) = cloned(&remote);

    assert_eq!(mode(&vault.join("entries").join(ENTRY)), 0o600);
    assert_eq!(mode(&vault.join("entries")), 0o700);
    assert_ne!(
        mode(&vault.join("seed.txt")),
        0o600,
        "the format leaves files outside entries/ at the tree's mode"
    );
    assert!(is_clean(&vault).unwrap(), "a mode change is not a change");
}

#[test]
fn fast_forward_restricts_arriving_entries() {
    let (dir, remote) = published_repo();
    push_entry(&remote, ENTRY, BLOB_MODE);
    fetch(dir.path(), &opts()).unwrap();

    assert!(fast_forward(dir.path()).unwrap());

    assert_eq!(mode(&dir.path().join("entries").join(ENTRY)), 0o600);
    assert_eq!(mode(&dir.path().join("entries")), 0o700);
}

/// A vault cloned before this was enforced still holds 0644 files the
/// incoming tree doesn't touch; the next pull repairs those too.
#[test]
fn fast_forward_repairs_entries_it_did_not_change() {
    let (_dir, remote) = published_repo();
    push_entry(&remote, ENTRY, BLOB_MODE);
    let (_into, vault) = cloned(&remote);
    chmod(&vault.join("entries").join(ENTRY), 0o644);
    chmod(&vault.join("entries"), 0o755);

    push_entry(&remote, OTHER_ENTRY, BLOB_MODE);
    fetch(&vault, &opts()).unwrap();
    assert!(fast_forward(&vault).unwrap());

    assert_eq!(mode(&vault.join("entries").join(ENTRY)), 0o600);
    assert_eq!(mode(&vault.join("entries").join(OTHER_ENTRY)), 0o600);
    assert_eq!(mode(&vault.join("entries")), 0o700);
}

/// A blob recorded executable is restricted like any other, and dropping
/// its execute bit must not read as an uncommitted change: a dirty tree
/// would refuse every later pull.
#[test]
fn an_executable_entry_is_restricted_and_the_tree_stays_clean() {
    let (_dir, remote) = published_repo();
    push_entry(&remote, ENTRY, EXECUTABLE_BLOB_MODE);

    let (_into, vault) = cloned(&remote);
    assert_eq!(mode(&vault.join("entries").join(ENTRY)), 0o600);
    assert!(is_clean(&vault).unwrap(), "{:?}", dirty_paths(&vault));

    push_entry(&remote, OTHER_ENTRY, EXECUTABLE_BLOB_MODE);
    fetch(&vault, &opts()).unwrap();
    assert!(fast_forward(&vault).unwrap());
    assert_eq!(mode(&vault.join("entries").join(OTHER_ENTRY)), 0o600);
    assert!(is_clean(&vault).unwrap(), "{:?}", dirty_paths(&vault));
}

/// Discarding uncommitted work rewrites the entry from the tree, so it is a
/// checkout like any other.
#[test]
fn reset_hard_restores_a_discarded_entry_at_0600() {
    let (_dir, remote) = published_repo();
    push_entry(&remote, ENTRY, BLOB_MODE);
    let (_into, vault) = cloned(&remote);
    std::fs::write(vault.join("entries").join(ENTRY), b"interrupted").unwrap();

    let discarded = reset_hard(&vault).unwrap();

    assert_eq!(discarded, [format!("entries/{ENTRY}")]);
    assert_eq!(mode(&vault.join("entries").join(ENTRY)), 0o600);
}

/// A merge that fails partway restores what it overwrote through a hard
/// reset. Here the remote edits an entry (written first) and adds `x/y.txt`
/// while this side committed a file `x`, so creating the directory fails.
#[test]
fn a_rolled_back_merge_restores_an_entry_at_0600() {
    let (dir, remote) = published_repo();
    let path = format!("entries/{ENTRY}");
    commit_file(dir.path(), &path, "ours");
    assert!(push(dir.path(), &opts()).unwrap());
    let other = Device::clone(remote.path());
    other.write(&path, "theirs");
    other.write("x/y.txt", "theirs");
    other.commit("theirs");
    other.push();
    commit_file(dir.path(), "x", "a file where they have a directory");
    fetch(dir.path(), &opts()).unwrap();

    merge_remote(dir.path(), "gage: merge origin").unwrap_err();

    assert_eq!(std::fs::read(dir.path().join(&path)).unwrap(), b"ours");
    assert_eq!(mode(&dir.path().join(&path)), 0o600);
}

/// A tree can name `entries` as a plain file. That is for the vault's own
/// validation to reject; the checkout must not fail on it or chmod it.
#[test]
fn a_plain_file_named_entries_is_left_alone() {
    let (_dir, remote) = published_repo();
    push_tree(&remote, |repo, root| {
        let blob = repo.blob(b"not a directory").unwrap();
        let mut top = repo.treebuilder(Some(root)).unwrap();
        top.insert("entries", blob, BLOB_MODE).unwrap();
        top.write().unwrap()
    });

    let (_into, vault) = cloned(&remote);

    assert_ne!(mode(&vault.join("entries")), 0o700);
    assert_ne!(mode(&vault.join("entries")), 0o600);
}

/// A vault with no entries yet has no `entries/` to restrict.
#[test]
fn a_tree_with_no_entries_directory_is_fine() {
    let (_dir, remote) = published_repo();
    let (_into, vault) = cloned(&remote);
    assert!(!vault.join("entries").exists());
}

/// `chmod` follows symlinks. One on disk (an older gage's checkout, an
/// external tool) must not turn the normalization into a way of changing
/// the mode of a file outside the vault.
#[test]
fn restricting_does_not_follow_a_symlinked_entry() {
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("target");
    std::fs::write(&target, b"outside").unwrap();
    chmod(&target, 0o644);

    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir(vault.path().join("entries")).unwrap();
    symlink(&target, vault.path().join("entries").join(ENTRY)).unwrap();

    restrict_entries(vault.path()).unwrap();

    assert_eq!(mode(&target), 0o644, "the link's target was changed");
}

#[test]
fn restricting_does_not_follow_a_symlinked_entries_directory() {
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join(ENTRY);
    std::fs::write(&file, b"outside").unwrap();
    chmod(&file, 0o644);
    chmod(outside.path(), 0o755);

    let vault = tempfile::tempdir().unwrap();
    symlink(outside.path(), vault.path().join("entries")).unwrap();

    restrict_entries(vault.path()).unwrap();

    assert_eq!(mode(&file), 0o644, "a file through the link was changed");
    assert_eq!(mode(outside.path()), 0o755, "the link's target was changed");
}

/// A directory inside `entries/` isn't an entry and isn't made 0600, which
/// would make it untraversable.
#[test]
fn restricting_skips_a_directory_inside_entries() {
    let vault = tempfile::tempdir().unwrap();
    let nested = vault.path().join("entries").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    chmod(&nested, 0o755);

    restrict_entries(vault.path()).unwrap();

    assert_eq!(mode(&nested), 0o755);
}
