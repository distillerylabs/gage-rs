//! Entry CRUD on a vault: insert, remove, list, and resolving a query to
//! one entry. Every write is one git commit whose message is the entry's
//! UUID alone (no title, no verb: the history of a public remote must not
//! say what a secret is called), authored as gage, and holds the vault's
//! write lock across the whole read-modify-commit sequence. Reads take no
//! lock. See "Entry CRUD" and "Concurrent processes and the vault lock" in
//! the design doc.
//!
//! The commit is local; pushing is P4's.

use std::collections::BTreeMap;
use std::time::Duration;

use uuid::Uuid;

use super::{Entry, Error, Timestamp, new_id};
use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;
use crate::vault::Vault;
use crate::{gitrepo, paths, vaultlock};

/// How long a write waits for another process's write to finish before
/// giving up (Go's `vaultLockTimeout`): long enough for any normal commit,
/// short enough that a wedged holder doesn't hang a terminal.
pub const WRITE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// What the caller chooses for a new entry. The id and the timestamps are
/// the library's: `created` and `updated` are now, `updated_by` is the
/// unlocked identity's device.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NewEntry {
    pub title: String,
    pub description: String,
    pub value: String,
    pub fields: BTreeMap<String, String>,
}

/// Secrets stay out of logs: the value and field values are redacted.
impl std::fmt::Debug for NewEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewEntry")
            .field("title", &self.title)
            .field("description", &self.description)
            .field("value", &"<redacted>")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// One row of a listing: everything about an entry except its secrets.
/// `ls` renders from this, and a later metadata index can serve the same
/// rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub created: Option<Timestamp>,
    pub updated: Option<Timestamp>,
    pub updated_by: String,
}

/// One entry a query could have meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: Uuid,
    pub title: String,
}

/// The entries an ambiguous query matched, sorted by title then id. It is
/// data, not text: a terminal prints it, a GUI shows a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateList {
    pub query: String,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    #[error("gage: no entry matches {query:?}")]
    NotFound { query: String },
    #[error("gage: {:?} is ambiguous: it matches {} entries", .0.query, .0.candidates.len())]
    Ambiguous(CandidateList),
    /// Reading the vault's entries to resolve against failed.
    #[error(transparent)]
    Entry(#[from] Error),
}

impl HasCode for ResolveError {
    fn code(&self) -> Code {
        match self {
            ResolveError::NotFound { .. } => Code::NotFound,
            ResolveError::Ambiguous(_) => Code::Ambiguous,
            ResolveError::Entry(e) => e.code(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// Another entry already has this exact title, and `force` was not set.
    #[error(
        "gage: an entry titled {title:?} already exists ({id}); use --force to insert a duplicate"
    )]
    DuplicateTitle { title: String, id: Uuid },
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Entry(#[from] Error),
    #[error(transparent)]
    Path(#[from] paths::Error),
    #[error(transparent)]
    Lock(#[from] vaultlock::Error),
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    #[error("gage: {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

impl HasCode for WriteError {
    fn code(&self) -> Code {
        match self {
            WriteError::DuplicateTitle { .. } => Code::Conflict,
            WriteError::Resolve(e) => e.code(),
            WriteError::Entry(e) => e.code(),
            WriteError::Path(e) => e.code(),
            WriteError::Lock(e) => e.code(),
            WriteError::Git(e) => e.code(),
            WriteError::Io { .. } => Code::Internal,
        }
    }
}

/// Resolves `query` against `(id, title)` pairs: exact title, then exact
/// UUID (any spelling `uuid` parses: hyphenated, simple, braced, URN). More
/// than one match at a stage is [`ResolveError::Ambiguous`]. The title
/// comes first on purpose (see "Addressing entries" in the design doc): a
/// title that happens to look like a UUID is still the title.
///
/// A pure function over what is already decrypted, so a cached index can
/// reuse it. Substring stages slot in as stages two and four.
pub fn resolve_query(query: &str, entries: &[(Uuid, String)]) -> Result<Uuid, ResolveError> {
    let exact_title: Vec<&(Uuid, String)> = entries.iter().filter(|(_, t)| t == query).collect();
    match exact_title.as_slice() {
        [] => {}
        [(id, _)] => return Ok(*id),
        many => return Err(ambiguous(query, many.iter().copied())),
    }
    if let Ok(wanted) = Uuid::try_parse(query)
        && entries.iter().any(|(id, _)| *id == wanted)
    {
        return Ok(wanted);
    }
    Err(ResolveError::NotFound {
        query: query.to_string(),
    })
}

fn ambiguous<'a>(query: &str, matches: impl Iterator<Item = &'a (Uuid, String)>) -> ResolveError {
    let mut candidates: Vec<Candidate> = matches
        .map(|(id, title)| Candidate {
            id: *id,
            title: title.clone(),
        })
        .collect();
    candidates.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
    ResolveError::Ambiguous(CandidateList {
        query: query.to_string(),
        candidates,
    })
}

impl Vault {
    /// Every entry's listing row, sorted by title then id. It decrypts
    /// every entry, so it needs the identity; it takes no lock.
    pub fn list(&self, identity: &Identity) -> Result<Vec<ListEntry>, Error> {
        let mut rows = Vec::new();
        for id in self.entry_ids()? {
            let e = self.read_entry(id, identity)?;
            rows.push(ListEntry {
                id,
                title: e.title,
                description: e.description,
                created: e.created,
                updated: e.updated,
                updated_by: e.updated_by,
            });
        }
        rows.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Ok(rows)
    }

    /// The one entry `query` names, or a typed reason it names none. Reads
    /// only, so it takes no lock: a caller that goes on to write resolves
    /// again under the lock (see [`Vault::remove`]).
    pub fn resolve(&self, identity: &Identity, query: &str) -> Result<Uuid, ResolveError> {
        resolve_query(query, &self.titles(identity)?)
    }

    fn titles(&self, identity: &Identity) -> Result<Vec<(Uuid, String)>, Error> {
        Ok(self
            .list(identity)?
            .into_iter()
            .map(|r| (r.id, r.title))
            .collect())
    }

    /// Adds an entry: encrypts it to every recipient, writes it, and makes
    /// one commit named by its id. A title another entry already has is
    /// refused unless `force`. The lock is held from before that check to
    /// after the commit, so two processes can't both pass the check, and
    /// can't interleave their commits.
    pub fn insert(
        &self,
        identity: &Identity,
        new: NewEntry,
        force: bool,
    ) -> Result<Uuid, WriteError> {
        self.insert_within(WRITE_LOCK_TIMEOUT, identity, new, force)
    }

    pub(crate) fn insert_within(
        &self,
        timeout: Duration,
        identity: &Identity,
        new: NewEntry,
        force: bool,
    ) -> Result<Uuid, WriteError> {
        self.with_write_lock(timeout, || {
            if !force
                && let Some((id, _)) = self
                    .titles(identity)?
                    .into_iter()
                    .find(|(_, t)| *t == new.title)
            {
                return Err(WriteError::DuplicateTitle {
                    title: new.title,
                    id,
                });
            }
            let now = Timestamp::new(jiff::Timestamp::now());
            let entry = Entry {
                title: new.title,
                description: new.description,
                created: Some(now),
                updated: Some(now),
                updated_by: identity.device().to_string(),
                value: new.value,
                fields: new.fields,
                ..Entry::default()
            };
            let id = new_id();
            self.write_entry(id, &entry)?;
            // A failed commit leaves the entry file in the working tree, as
            // Go: the next write's commit picks it up.
            gitrepo::commit_all(&self.path, &id.to_string())?;
            Ok(id)
        })
    }

    /// Deletes the entry `query` names and commits the deletion, under the
    /// write lock. The query is resolved inside the lock: resolving first
    /// and locking after would let a concurrent write change what the
    /// query means.
    pub fn remove(&self, identity: &Identity, query: &str) -> Result<Uuid, WriteError> {
        self.remove_within(WRITE_LOCK_TIMEOUT, identity, query)
    }

    pub(crate) fn remove_within(
        &self,
        timeout: Duration,
        identity: &Identity,
        query: &str,
    ) -> Result<Uuid, WriteError> {
        self.with_write_lock(timeout, || {
            let id = self.resolve(identity, query)?;
            let path = self.entry_path(id);
            std::fs::remove_file(&path).map_err(|source| WriteError::Io {
                action: "removing",
                path,
                source,
            })?;
            gitrepo::commit_all(&self.path, &id.to_string())?;
            Ok(id)
        })
    }

    /// Runs `f` holding the vault's write lock, waiting up to `timeout` for
    /// it. The lock is released on every path out, including an error.
    fn with_write_lock<T>(
        &self,
        timeout: Duration,
        f: impl FnOnce() -> Result<T, WriteError>,
    ) -> Result<T, WriteError> {
        // The path (and with it the vault id) is validated before anything
        // is created or locked.
        let path = self.lock_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WriteError::Io {
                action: "creating",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let lock = vaultlock::Lock::acquire(&path, timeout)?;
        let result = f();
        match (result, lock.release()) {
            (Ok(v), Ok(())) => Ok(v),
            (Err(e), _) => Err(e),
            (Ok(_), Err(e)) => Err(e.into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
mod tests {
    use std::io::{BufRead, BufReader, Read};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::time::Instant;

    use super::*;
    use crate::testsupport::fast_scrypt;
    use crate::vault::CreateSpec;
    use crate::xdgpaths::Dirs;
    use crate::{vaultconfig, vaultlock};

    /// A real vault (git repository, recipients file) and the identity
    /// that can read it.
    struct Fixture {
        _root: tempfile::TempDir,
        dirs: Dirs,
        vault: Vault,
        key: age::x25519::Identity,
    }

    fn dirs_in(root: &Path) -> Dirs {
        Dirs {
            config: root.join("config"),
            data: root.join("data"),
            state: root.join("state"),
        }
    }

    impl Fixture {
        fn new() -> Fixture {
            fast_scrypt();
            let root = tempfile::tempdir().unwrap();
            let dirs = dirs_in(root.path());
            let key = age::x25519::Identity::generate();
            let vault = Vault::create(
                &dirs,
                &CreateSpec {
                    name: "work".into(),
                    id: vaultconfig::new_id(),
                    path: root.path().join("vault"),
                    vault_type: "git".into(),
                    method: "passphrase".into(),
                    device: "laptop-1".into(),
                    recipients: vec![key.to_public().to_string()],
                    remote: None,
                },
            )
            .unwrap();
            Fixture {
                _root: root,
                dirs,
                vault,
                key,
            }
        }

        fn identity(&self) -> Identity {
            Identity::for_tests(&self.key)
        }

        fn commits(&self) -> usize {
            gitrepo::commit_count(&self.vault.path).unwrap()
        }

        fn head(&self) -> Head {
            let repo = git2::Repository::open(&self.vault.path).unwrap();
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            Head {
                message: commit.message().unwrap().to_string(),
                author_name: commit.author().name().unwrap().to_string(),
                author_email: commit.author().email().unwrap().to_string(),
                committer_email: commit.committer().email().unwrap().to_string(),
            }
        }
    }

    /// The tip commit, as plain data.
    struct Head {
        message: String,
        author_name: String,
        author_email: String,
        committer_email: String,
    }

    fn new_entry(title: &str, value: &str) -> NewEntry {
        NewEntry {
            title: title.into(),
            value: value.into(),
            ..NewEntry::default()
        }
    }

    // Go: TestInsertRoundTripsThroughReadEntry
    #[test]
    fn insert_round_trips_through_read_entry() {
        let f = Fixture::new();
        let id = f
            .vault
            .insert(
                &f.identity(),
                NewEntry {
                    title: "ProtonMail".into(),
                    description: "Personal email".into(),
                    value: "correct horse".into(),
                    fields: [("username".to_string(), "me@proton.me".to_string())].into(),
                },
                false,
            )
            .unwrap();
        let e = f.vault.read_entry(id, &f.identity()).unwrap();
        assert_eq!(e.title, "ProtonMail");
        assert_eq!(e.description, "Personal email");
        assert_eq!(e.value, "correct horse");
        assert_eq!(e.field("username").unwrap(), "me@proton.me");
        assert!(e.created.is_some() && e.created == e.updated);
        assert_eq!(e.updated_by, "test-device");
        assert_eq!(f.vault.entry_ids().unwrap(), vec![id]);
    }

    // Go: TestInsertCommitsExactlyOnceWithUUIDMessageAndFixedAuthor
    #[test]
    fn insert_commits_exactly_once_with_uuid_message_and_fixed_author() {
        let f = Fixture::new();
        let before = f.commits();
        let id = f
            .vault
            .insert(&f.identity(), new_entry("a title", "v"), false)
            .unwrap();
        assert_eq!(f.commits(), before + 1);
        let head = f.head();
        assert_eq!(head.message, id.to_string());
        assert_eq!(head.author_name, gitrepo::AUTHOR_NAME);
        assert_eq!(head.author_email, gitrepo::AUTHOR_EMAIL);
        assert_eq!(head.committer_email, gitrepo::AUTHOR_EMAIL);
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
    }

    // Go: TestInsertDuplicateTitleRejectedWithoutForce
    #[test]
    fn insert_duplicate_title_rejected_without_force() {
        let f = Fixture::new();
        let first = f
            .vault
            .insert(&f.identity(), new_entry("dup", "1"), false)
            .unwrap();
        let before = f.commits();
        let err = f
            .vault
            .insert(&f.identity(), new_entry("dup", "2"), false)
            .unwrap_err();
        assert!(
            matches!(&err, WriteError::DuplicateTitle { id, .. } if *id == first),
            "{err}"
        );
        assert_eq!(err.code(), Code::Conflict);
        assert_eq!(f.vault.entry_ids().unwrap().len(), 1);
        assert_eq!(f.commits(), before, "a refused insert commits nothing");
    }

    // Go: TestInsertDuplicateTitleAllowedWithForce
    #[test]
    fn insert_duplicate_title_allowed_with_force() {
        let f = Fixture::new();
        f.vault
            .insert(&f.identity(), new_entry("dup", "1"), false)
            .unwrap();
        f.vault
            .insert(&f.identity(), new_entry("dup", "2"), true)
            .unwrap();
        assert_eq!(f.vault.entry_ids().unwrap().len(), 2);
    }

    // Go: TestRemoveDeletesFileAndCommits. By UUID, as Go: the title path
    // is the CLI's `rm` test.
    #[test]
    fn remove_deletes_file_and_commits() {
        let f = Fixture::new();
        let id = f
            .vault
            .insert(&f.identity(), new_entry("gone soon", "v"), false)
            .unwrap();
        let before = f.commits();

        let removed = f.vault.remove(&f.identity(), &id.to_string()).unwrap();

        assert_eq!(removed, id);
        assert!(!f.vault.entry_path(id).exists());
        assert!(f.vault.entry_ids().unwrap().is_empty());
        assert_eq!(f.commits(), before + 1);
        let head = f.head();
        assert_eq!(head.message, id.to_string());
        assert_eq!(head.author_name, gitrepo::AUTHOR_NAME);
        assert_eq!(head.author_email, gitrepo::AUTHOR_EMAIL);
        assert_eq!(head.committer_email, gitrepo::AUTHOR_EMAIL);
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
        let err = f.vault.read_entry(id, &f.identity()).unwrap_err();
        assert!(
            matches!(&err, Error::EntryNotFound { id: got } if *got == id),
            "{err}"
        );
    }

    // Not a Go row, but Go's behavior: a failed commit fails the insert and
    // leaves the entry file uncommitted in the working tree (no rollback),
    // and the next successful write's commit records it.
    #[test]
    fn an_insert_whose_commit_fails_leaves_the_entry_uncommitted() {
        let f = Fixture::new();
        let before = f.commits();
        // A held index lock is what a concurrent git process looks like:
        // staging can't write the index, so the commit fails for real.
        let index_lock = f.vault.path.join(".git").join("index.lock");
        std::fs::write(&index_lock, "").unwrap();

        let err = f
            .vault
            .insert(&f.identity(), new_entry("x", "v"), false)
            .unwrap_err();

        assert!(matches!(err, WriteError::Git(_)), "{err}");
        assert_eq!(err.code(), Code::Internal);
        assert_eq!(f.commits(), before);
        assert_eq!(f.vault.entry_ids().unwrap().len(), 1, "the file stays");
        std::fs::remove_file(&index_lock).unwrap();
        assert!(!gitrepo::is_clean(&f.vault.path).unwrap());

        f.vault
            .insert(&f.identity(), new_entry("y", "v"), false)
            .unwrap();
        assert_eq!(f.commits(), before + 1);
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
        assert_eq!(f.vault.list(&f.identity()).unwrap().len(), 2);
    }

    // Go: TestRemoveUnknownQueryFails
    #[test]
    fn remove_unknown_query_fails() {
        let f = Fixture::new();
        f.vault
            .insert(&f.identity(), new_entry("keep", "v"), false)
            .unwrap();
        let before = f.commits();
        let err = f.vault.remove(&f.identity(), "nope").unwrap_err();
        assert!(
            matches!(&err, WriteError::Resolve(ResolveError::NotFound { query }) if query == "nope"),
            "{err}"
        );
        assert_eq!(err.code(), Code::NotFound);
        assert_eq!(f.vault.entry_ids().unwrap().len(), 1);
        assert_eq!(f.commits(), before);
    }

    fn pairs(titles: &[&str]) -> Vec<(Uuid, String)> {
        titles.iter().map(|t| (new_id(), t.to_string())).collect()
    }

    // Go: TestResolveByExactTitle
    #[test]
    fn resolve_by_exact_title() {
        let entries = pairs(&["ProtonMail", "Proton", "other"]);
        assert_eq!(resolve_query("Proton", &entries).unwrap(), entries[1].0);
        assert_eq!(resolve_query("ProtonMail", &entries).unwrap(), entries[0].0);
        // Exact means exact: no case folding, no trimming.
        assert!(resolve_query("proton", &entries).is_err());
    }

    // Go: TestResolveByUUID
    #[test]
    fn resolve_by_uuid_in_every_spelling() {
        let entries = pairs(&["a", "b"]);
        let id = entries[1].0;
        for q in [
            id.hyphenated().to_string(),
            id.simple().to_string(),
            id.braced().to_string(),
            id.urn().to_string(),
            id.hyphenated().to_string().to_uppercase(),
        ] {
            assert_eq!(resolve_query(&q, &entries).unwrap(), id, "{q}");
        }
        // A well-formed UUID of no entry is not found.
        let err = resolve_query(&new_id().to_string(), &entries).unwrap_err();
        assert!(matches!(err, ResolveError::NotFound { .. }));
    }

    #[test]
    fn a_title_beats_a_uuid_of_the_same_spelling() {
        let mut entries = pairs(&["x"]);
        let other = new_id();
        entries.push((other, entries[0].0.to_string()));
        assert_eq!(
            resolve_query(&entries[0].0.to_string(), &entries).unwrap(),
            other,
            "the title match wins over the other entry's id"
        );
    }

    // Go: TestResolveAmbiguousTitleFails
    #[test]
    fn resolve_ambiguous_title_fails_with_a_typed_candidate_list() {
        let mut entries = pairs(&["twin", "twin", "zzz"]);
        entries.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        let err = resolve_query("twin", &entries).unwrap_err();
        assert_eq!(err.code(), Code::Ambiguous);
        let ResolveError::Ambiguous(list) = err else {
            panic!("want Ambiguous");
        };
        assert_eq!(list.query, "twin");
        assert_eq!(list.candidates.len(), 2);
        assert!(list.candidates.iter().all(|c| c.title == "twin"));
        assert!(
            list.candidates[0].id < list.candidates[1].id,
            "sorted by title, then id"
        );
    }

    // Go: TestResolveUnknownQueryFails
    #[test]
    fn resolve_unknown_query_fails() {
        let err = resolve_query("nothing", &pairs(&["a"])).unwrap_err();
        assert!(matches!(&err, ResolveError::NotFound { query } if query == "nothing"));
        assert_eq!(err.code(), Code::NotFound);
        assert!(matches!(
            resolve_query("x", &[]).unwrap_err(),
            ResolveError::NotFound { .. }
        ));
    }

    #[test]
    fn vault_resolve_reads_the_real_entries() {
        let f = Fixture::new();
        let id = f
            .vault
            .insert(&f.identity(), new_entry("findme", "v"), false)
            .unwrap();
        assert_eq!(f.vault.resolve(&f.identity(), "findme").unwrap(), id);
        assert_eq!(
            f.vault
                .resolve(&f.identity(), &id.simple().to_string())
                .unwrap(),
            id
        );
    }

    #[test]
    fn list_is_sorted_by_title_then_id_and_carries_the_metadata() {
        let f = Fixture::new();
        let id = f.identity();
        assert!(f.vault.list(&id).unwrap().is_empty());
        for t in ["b", "a", "b"] {
            f.vault.insert(&id, new_entry(t, "v"), true).unwrap();
        }
        let rows = f.vault.list(&id).unwrap();
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["a", "b", "b"]);
        assert!(rows[1].id < rows[2].id);
        assert!(rows.iter().all(|r| r.created.is_some()));
        assert!(rows.iter().all(|r| r.updated_by == "test-device"));
    }

    #[test]
    fn a_vault_with_an_unsafe_id_fails_insert_before_any_path_is_built() {
        let f = Fixture::new();
        let mut vault = f.vault.clone();
        vault.id = String::new();
        let err = vault
            .insert(&f.identity(), new_entry("x", "v"), false)
            .unwrap_err();
        assert!(matches!(err, WriteError::Path(_)), "{err}");
        assert!(!f.dirs.state.exists(), "no lock directory was created");
        assert!(vault.entry_ids().unwrap().is_empty());
    }

    // --- process-level lock tests ----------------------------------------
    //
    // The lock is held by a real, separate OS process, never a thread: file
    // locks belong to the open file description, so a thread-based test
    // would pass even if the lock were an in-process mutex. The child is
    // this test binary re-executed with CHILD_ENV set, running only
    // `crud_child_trampoline` (as vaultlock's tests do).

    const CHILD_ENV: &str = "GAGE_CRUD_TEST_CHILD";

    /// The child side. `hold`: take the vault's write lock, announce it on
    /// stderr, hold it until stdin closes. `insert`: insert one entry.
    #[test]
    fn crud_child_trampoline() {
        let Ok(mode) = std::env::var(CHILD_ENV) else {
            return;
        };
        let var = |k: &str| std::env::var(k).unwrap();
        let root = PathBuf::from(var("GAGE_CRUD_TEST_ROOT"));
        let vault = child_vault(&root);
        match mode.as_str() {
            "hold" => {
                let path = vault.lock_file_path().unwrap();
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let lock = vaultlock::Lock::acquire(&path, Duration::from_secs(5)).unwrap();
                eprintln!("ACQUIRED");
                let _ = std::io::stdin().read(&mut [0u8]);
                lock.release().unwrap();
            }
            "insert" => {
                fast_scrypt();
                let key: age::x25519::Identity = var("GAGE_CRUD_TEST_KEY").parse().unwrap();
                let identity = Identity::for_tests(&key);
                vault
                    .insert(
                        &identity,
                        new_entry(&var("GAGE_CRUD_TEST_TITLE"), "v"),
                        false,
                    )
                    .unwrap();
                eprintln!("INSERTED");
            }
            other => panic!("unknown child mode {other}"),
        }
    }

    /// The fixture's vault as a child sees it: same dirs, path and id.
    fn child_vault(root: &Path) -> Vault {
        let id = std::env::var("GAGE_CRUD_TEST_VAULT_ID").unwrap();
        let mut v = Vault::for_tests(&dirs_in(root), "work", &id);
        v.path = root.join("vault");
        v
    }

    fn spawn_child(f: &Fixture, mode: &str, title: &str, stdin: Stdio) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "entry::crud::tests::crud_child_trampoline",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, mode)
            .env("GAGE_CRUD_TEST_ROOT", f._root.path())
            .env("GAGE_CRUD_TEST_VAULT_ID", &f.vault.id)
            .env("GAGE_CRUD_TEST_TITLE", title)
            .env(
                "GAGE_CRUD_TEST_KEY",
                age::secrecy::ExposeSecret::expose_secret(&f.key.to_string()),
            )
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    struct Holder {
        child: Child,
        stdin: Option<ChildStdin>,
    }

    impl Holder {
        /// Starts a process holding the vault's write lock and returns once
        /// it has it.
        fn spawn(f: &Fixture) -> Holder {
            let mut child = spawn_child(f, "hold", "", Stdio::piped());
            let stderr = BufReader::new(child.stderr.take().unwrap());
            let acquired = stderr.lines().any(|l| l.unwrap() == "ACQUIRED");
            if !acquired {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the holder exited without acquiring the lock");
            }
            let stdin = child.stdin.take();
            Holder { child, stdin }
        }

        fn release(&mut self) {
            drop(self.stdin.take());
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            self.release();
            let _ = self.child.wait();
        }
    }

    // Go: TestInsertBlocksOnAConcurrentlyHeldWriteLock
    #[test]
    fn insert_blocks_on_a_concurrently_held_write_lock() {
        let f = Fixture::new();
        let mut holder = Holder::spawn(&f);

        // While the other process holds it, a short wait times out as a
        // conflict and writes nothing.
        let before = f.commits();
        let err = f
            .vault
            .insert_within(
                Duration::from_millis(200),
                &f.identity(),
                new_entry("x", "v"),
                false,
            )
            .unwrap_err();
        assert!(
            matches!(err, WriteError::Lock(vaultlock::Error::Contended { .. })),
            "{err}"
        );
        assert_eq!(err.code(), Code::Conflict);
        assert!(f.vault.entry_ids().unwrap().is_empty());
        assert_eq!(f.commits(), before);

        // A longer wait proceeds once the holder lets go, and not before.
        let (tx, rx) = std::sync::mpsc::channel();
        let vault = f.vault.clone();
        let key = f.key.clone();
        std::thread::spawn(move || {
            let r = vault.insert_within(
                Duration::from_secs(10),
                &Identity::for_tests(&key),
                new_entry("waited", "v"),
                false,
            );
            let _ = tx.send(r.is_ok());
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "insert returned while the other process still held the lock"
        );
        holder.release();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
        assert_eq!(f.vault.entry_ids().unwrap().len(), 1);
    }

    // Go: TestReadDoesNotBlockOnAHeldWriteLock
    #[test]
    fn read_does_not_block_on_a_held_write_lock() {
        let f = Fixture::new();
        let id = f
            .vault
            .insert(&f.identity(), new_entry("readable", "v"), false)
            .unwrap();
        let _holder = Holder::spawn(&f);

        let start = Instant::now();
        assert_eq!(f.vault.list(&f.identity()).unwrap().len(), 1);
        assert_eq!(
            f.vault.read_entry(id, &f.identity()).unwrap().title,
            "readable"
        );
        assert_eq!(f.vault.resolve(&f.identity(), "readable").unwrap(), id);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "reads waited on the write lock"
        );
    }

    // Go: TestWriteLockSerializesConcurrentInsertsAndLosesNoCommit
    #[test]
    fn write_lock_serializes_concurrent_inserts_and_loses_no_commit() {
        let f = Fixture::new();
        let before = f.commits();
        const N: usize = 6;

        // Started together, so the children really do contend: every one
        // is spawned before any is waited on.
        let children: Vec<Child> = (0..N)
            .map(|i| spawn_child(&f, "insert", &format!("entry-{i}"), Stdio::null()))
            .collect();
        for mut c in children {
            let mut err = String::new();
            c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
            assert!(c.wait().unwrap().success(), "a child failed:\n{err}");
            assert!(err.contains("INSERTED"), "{err}");
        }

        assert_eq!(f.commits(), before + N, "no commit was lost");
        let rows = f.vault.list(&f.identity()).unwrap();
        assert_eq!(rows.len(), N);
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
        // One commit per entry, each named by that entry's id.
        let repo = git2::Repository::open(&f.vault.path).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        let mut messages: Vec<String> = walk
            .map(|o| {
                repo.find_commit(o.unwrap())
                    .unwrap()
                    .message()
                    .unwrap()
                    .to_string()
            })
            .filter(|m| Uuid::try_parse(m).is_ok())
            .collect();
        messages.sort();
        let mut ids: Vec<String> = rows.iter().map(|r| r.id.to_string()).collect();
        ids.sort();
        assert_eq!(messages, ids);
    }
}
