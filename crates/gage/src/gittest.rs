//! Test-only git fixtures: ephemeral local bare repositories that stand in
//! for a real remote, so sync-related tests exercise the git crate's real
//! code paths with no socket and no network. First used by M1's
//! `git set-remote` tests; grows into the sync harness in M8a.
//!
//! Compiled only for the library's own tests and behind the `test-support`
//! feature (which `gage-cli` enables as a dev-dependency), never into the
//! shipped binary. Excluded from the coverage gate.

use std::path::{Path, PathBuf};

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
