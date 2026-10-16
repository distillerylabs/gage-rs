//! Test-only git fixtures: ephemeral local bare repositories that stand in
//! for a real remote, so sync-related tests exercise the git crate's real
//! code paths with no network. First used by M1's `git set-remote` tests;
//! M8a1 adds the second half of the sync harness: a [`Device`] playing
//! "another machine", and an [`HttpRecorder`] that observes what libgit2
//! sends over HTTP on a loopback port.
//!
//! Compiled only for the library's own tests and behind the `test-support`
//! feature (which `gage-cli` enables as a dev-dependency), never into the
//! shipped binary. Excluded from the coverage gate.

use std::path::{Path, PathBuf};

use crate::gitrepo::{self, RemoteOptions};
use crate::xdgpaths::Dirs;

mod recorder;

pub use recorder::{Answer, HttpRecorder, Request, basic};

/// A bare repository in its own temp directory, deleted on drop. Two calls
/// never collide, even within one test.
#[derive(Debug)]
pub struct BareRemote {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl BareRemote {
    /// The repository's path, usable as a remote URL.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Creates an empty bare repository whose HEAD names `main`, independent of
/// the user's `init.defaultBranch`. Panics on failure: it is a fixture.
pub fn new_bare_remote() -> BareRemote {
    gitrepo::isolate();
    let dir = tempfile::tempdir().expect("gittest: creating temp dir");
    let path = dir.path().join("remote.git");
    git2::Repository::init_opts(
        &path,
        git2::RepositoryInitOptions::new()
            .bare(true)
            .initial_head("main"),
    )
    .unwrap_or_else(|e| {
        panic!(
            "gittest: initializing bare remote at {}: {e}",
            path.display()
        )
    });
    BareRemote { _dir: dir, path }
}

/// Options for a remote operation against a local-path remote, which
/// never reads a token: the token store points at a directory nothing
/// creates.
pub fn remote_options() -> RemoteOptions {
    let nowhere = std::env::temp_dir().join("gage-gittest-no-local-state");
    RemoteOptions::new(&Dirs {
        config: nowhere.join("config"),
        data: nowhere.join("data"),
        state: nowhere.join("state"),
    })
}

/// An `http://127.0.0.1:<port>/...` URL nothing listens on: a port that
/// was free a moment ago, so a connection is refused at once. How a test
/// is offline with no network at all.
pub fn refused_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("gittest: binding a port");
    let port = listener.local_addr().expect("gittest: port").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/me/vault.git")
}

/// A throwaway clone of a remote standing in for another device: the
/// states that exist only because two machines share a vault (remote
/// commits to pull, and with a local commit, real divergence). Not a gage
/// vault: it never unlocks or reads `.gage/config.toml`, and it uses git2
/// directly rather than `gitrepo`, so it moves commits the way a second
/// machine would without depending on the code under test.
#[derive(Debug)]
pub struct Device {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Device {
    /// Clones `remote` (a local path) into a fresh temp directory.
    pub fn clone(remote: &Path) -> Device {
        gitrepo::isolate();
        let dir = tempfile::tempdir().expect("gittest: creating temp dir");
        let path = dir.path().join("device");
        git2::Repository::clone(&remote.to_string_lossy(), &path)
            .unwrap_or_else(|e| panic!("gittest: cloning {}: {e}", remote.display()));
        Device { _dir: dir, path }
    }

    /// The clone's working tree.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn repo(&self) -> git2::Repository {
        git2::Repository::open(&self.path).expect("gittest: opening the device")
    }

    /// Creates or replaces a file; `rel` is slash-separated, like a git
    /// path.
    pub fn write(&self, rel: &str, content: &str) {
        let full = self.path.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).expect("gittest: mkdir");
        std::fs::write(&full, content).unwrap_or_else(|e| panic!("gittest: writing {rel}: {e}"));
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path.join(rel))
            .unwrap_or_else(|e| panic!("gittest: removing {rel}: {e}"));
    }

    /// A file's contents: what reached this device.
    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path.join(rel))
            .unwrap_or_else(|e| panic!("gittest: reading {rel}: {e}"))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path.join(rel).exists()
    }

    /// Stages everything, deletions included, and commits.
    pub fn commit(&self, message: &str) {
        let repo = self.repo();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .and_then(|()| index.update_all(["*"], None))
            .and_then(|()| index.write())
            .expect("gittest: staging");
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("gittest", "gittest@localhost").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .expect("gittest: committing");
    }

    /// Publishes the current branch to the remote.
    pub fn push(&self) {
        let repo = self.repo();
        let branch = repo.head().unwrap().name().unwrap().to_string();
        repo.find_remote("origin")
            .unwrap()
            .push(&[format!("{branch}:{branch}")], None)
            .expect("gittest: pushing");
    }

    /// Fast-forwards to whatever the remote has now, so a test can assert
    /// what a second device would see.
    pub fn pull(&self) {
        let repo = self.repo();
        let head = repo.head().unwrap();
        let branch = head.shorthand().unwrap().to_string();
        repo.find_remote("origin")
            .unwrap()
            .fetch::<&str>(&[], None, None)
            .expect("gittest: fetching");
        let target = repo
            .find_reference(&format!("refs/remotes/origin/{branch}"))
            .unwrap()
            .peel(git2::ObjectType::Commit)
            .unwrap();
        repo.reset(&target, git2::ResetType::Hard, None)
            .expect("gittest: fast-forwarding");
    }

    /// The whole "another device changed something" gesture.
    pub fn write_commit_push(&self, rel: &str, content: &str, message: &str) {
        self.write(rel, content);
        self.commit(message);
        self.push();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go: TestNewBareRemoteIsABareRepo
    #[test]
    fn new_bare_remote_is_a_bare_repo() {
        let remote = new_bare_remote();
        assert!(remote.path().is_dir());
        let repo = git2::Repository::open(remote.path()).unwrap();
        assert!(repo.is_bare());
        assert_eq!(
            repo.find_reference("HEAD")
                .unwrap()
                .symbolic_target()
                .unwrap(),
            Some("refs/heads/main")
        );
    }

    // Go: TestNewBareRemoteProducesIndependentRepos
    #[test]
    fn new_bare_remote_produces_independent_repos() {
        let (first, second) = (new_bare_remote(), new_bare_remote());
        assert_ne!(first.path(), second.path());
        for remote in [&first, &second] {
            git2::Repository::open_bare(remote.path()).unwrap();
        }
    }

    #[test]
    fn dropping_the_remote_deletes_it() {
        let remote = new_bare_remote();
        let path = remote.path().to_path_buf();
        drop(remote);
        assert!(!path.exists());
    }
}
