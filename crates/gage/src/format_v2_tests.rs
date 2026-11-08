//! The golden format-v2 vault (`tests/fixtures/format-v2/`), as this build
//! meets it: refused. Format v3 authenticated entries and keeps no
//! unauthenticated read path (ADR-0065), so a v2 vault is an older format
//! this gage no longer reads. What still holds is that the fixture itself
//! has not changed and its history has the shape v2 specified.
//!
//! These tests are a format guard. A change that needs one of them edited
//! to pass is a format change, which takes a `format_version` bump, a new
//! fixture and an ADR (`doc/format/README.md`). The current format's
//! guard is `format_v3_tests`.

use std::path::Path;

use uuid::Uuid;

use crate::error::{ErrorKind, HasKind};
use crate::storage::config;
use crate::storage::paths;
use crate::storage::vaultconfig::{self, VersionSkew};
use crate::sync::gitrepo;
use crate::testsupport::format_v2::{self as fx, Installed};
use crate::vault::{ResolveError, Vault};

/// The installed vault, resolved the way a command resolves it.
fn resolve(i: &Installed) -> Result<Vault, ResolveError> {
    let global = config::load(&paths::global_config_file(&i.dirs)).unwrap();
    Vault::from_registration(&i.dirs, fx::VAULT_NAME, &global.vaults[fx::VAULT_NAME])
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The committed config is refused at load, as an older format, with
/// advice that says so; and so is every way a command reaches the vault.
#[test]
fn format_v2_vault_is_refused_as_an_older_format() {
    let i = fx::install();
    let err = vaultconfig::load(&i.vault.join(".gage/config.toml")).unwrap_err();
    assert!(
        matches!(
            err,
            vaultconfig::Error::UnsupportedFormatVersion {
                found: 2,
                skew: VersionSkew::Older,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let msg = err.to_string();
    assert!(msg.contains("format_version 2"), "{msg}");
    assert!(msg.contains("older format"), "{msg}");

    let err = resolve(&i).unwrap_err();
    assert!(
        matches!(
            err,
            ResolveError::Config(vaultconfig::Error::UnsupportedFormatVersion { found: 2, .. })
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
}

#[test]
fn format_v2_commit_layout() {
    let i = fx::install();
    let repo = git2::Repository::open(&i.vault).unwrap();

    // What this build writes with is what the fixture was written with.
    assert_eq!(gitrepo::INITIAL_BRANCH, "main");
    assert_eq!(gitrepo::ORIGIN, "origin");
    assert_eq!(gitrepo::AUTHOR_NAME, "gage");
    assert_eq!(gitrepo::AUTHOR_EMAIL, "gage@localhost");
    assert_eq!(crate::vault::INIT_COMMIT_MESSAGE, "gage: initialize vault");
    assert_eq!(
        crate::sync::remotesync::MERGE_COMMIT_MESSAGE,
        "gage: merge origin"
    );

    assert_eq!(repo.head().unwrap().name().ok(), Some("refs/heads/main"));
    let origin = repo.find_remote("origin").unwrap();
    assert_eq!(origin.url().ok(), i.remote.to_str());

    let mut walk = repo.revwalk().unwrap();
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
        .unwrap();
    walk.push_head().unwrap();

    let mut kinds: Vec<&str> = Vec::new();
    for id in walk {
        let commit = repo.find_commit(id.unwrap()).unwrap();
        for sig in [commit.author(), commit.committer()] {
            assert_eq!(sig.name().ok(), Some("gage"));
            assert_eq!(sig.email().ok(), Some("gage@localhost"));
        }
        let message = commit.message().unwrap();
        let tree = commit.tree().unwrap();
        let changed = |commit: &git2::Commit| -> Vec<String> {
            let parent = commit.parent(0).unwrap().tree().unwrap();
            let diff = repo
                .diff_tree_to_tree(Some(&parent), Some(&tree), None)
                .unwrap();
            diff.deltas()
                .map(|d| d.new_file().path().unwrap().to_str().unwrap().to_string())
                .collect()
        };
        match commit.parent_count() {
            0 => {
                assert_eq!(message, "gage: initialize vault");
                let mut paths = Vec::new();
                tree.walk(git2::TreeWalkMode::PreOrder, |dir, e| {
                    if e.kind() == Some(git2::ObjectType::Blob) {
                        paths.push(format!("{dir}{}", e.name().unwrap()));
                    }
                    git2::TreeWalkResult::Ok
                })
                .unwrap();
                paths.sort();
                assert_eq!(
                    paths,
                    [
                        ".age-recipients",
                        ".gage/config.toml",
                        ".gitattributes",
                        ".gitignore"
                    ]
                );
                kinds.push("init");
            }
            2 => {
                assert_eq!(message, "gage: merge origin");
                kinds.push("merge");
            }
            1 if message == "gage: enrollment request" => {
                let changed = changed(&commit);
                assert_eq!(changed.len(), 1, "{changed:?}");
                assert!(
                    changed[0].starts_with(&format!(".gage/pending/{}-", fx::REQUEST_ID)),
                    "{changed:?}"
                );
                kinds.push("enroll");
            }
            1 => {
                // An entry write's message is the entry's id, and it
                // touches that entry's file and nothing else.
                let id = Uuid::try_parse(message)
                    .unwrap_or_else(|_| panic!("unexpected commit message {message:?}"));
                assert_eq!(id.hyphenated().to_string(), message);
                assert_eq!(changed(&commit), [format!("entries/{message}.age")]);
                kinds.push("entry");
            }
            n => panic!("a commit with {n} parents"),
        }
    }
    kinds.sort();
    assert_eq!(
        kinds,
        ["enroll", "entry", "entry", "entry", "init", "merge"],
        "the fixture's history"
    );
}

#[test]
fn format_v2_fixture_is_byte_stable() {
    let dir = fx::fixture_dir();
    let want = text(&dir.join(fx::MANIFEST_FILE));
    assert_eq!(
        fx::manifest(&dir),
        want,
        "tests/fixtures/format-v2 changed. It is never edited: a format change adds format-v3/ beside it (doc/format/README.md)"
    );
}

// The installed copy is what the tests above read, never the fixture, and
// a refused vault is left as it was found.
#[test]
fn format_v2_install_leaves_the_fixture_alone() {
    let before = fx::manifest(&fx::fixture_dir());
    let i = fx::install();
    assert!(!i.vault.starts_with(fx::fixture_dir()));
    assert!(!i.remote.starts_with(fx::fixture_dir()));
    let head = gitrepo::head_commit(&i.vault).unwrap();
    assert!(resolve(&i).is_err());
    assert_eq!(gitrepo::head_commit(&i.vault).unwrap(), head);
    assert!(gitrepo::is_clean(&i.vault).unwrap());
    assert_eq!(fx::manifest(&fx::fixture_dir()), before);
}
