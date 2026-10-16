//! Entry CRUD on a vault: insert, remove, list, and resolving a query to
//! one entry. Every write is one git commit whose message is the entry's
//! UUID alone (no title, no verb: the history of a public remote must not
//! say what a secret is called), authored as gage, and holds the vault's
//! write lock across the whole read-modify-commit sequence. Reads take no
//! lock. See "Entry CRUD" and "Concurrent processes and the vault lock" in
//! the design doc.
//!
//! After the commit, still under the lock, the write is pushed (P4 ·
//! M8a2). A failed push warns through the prompter the caller passes and
//! never fails the write: the commit is already durable locally.

use std::collections::BTreeMap;
use std::time::Duration;

use uuid::Uuid;

use super::{Entry, Error, Extra, Timestamp, new_id};
use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;
use crate::prompter::Prompter;
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
    /// Keys this build doesn't recognize, kept as given (`insert -e` lets
    /// someone type one into the template).
    pub extra: BTreeMap<String, Extra>,
}

/// Secrets stay out of logs: the value and field values are redacted.
impl std::fmt::Debug for NewEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewEntry")
            .field("title", &self.title)
            .field("description", &self.description)
            .field("value", &"<redacted>")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("extra", &self.extra.keys().collect::<Vec<_>>())
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

/// Resolves `query` against `(id, title)` pairs, in stages: exact title,
/// then title substring (case-insensitive), then exact UUID (any spelling
/// `uuid` parses: hyphenated, simple, braced, URN), then UUID substring
/// (hyphens and case ignored on both sides, so a prefix is one, and a query
/// may span a hyphen). The first stage with any match decides: one match
/// is the answer, more than one is [`ResolveError::Ambiguous`] listing that
/// stage's matches. Every title stage runs before any UUID stage on purpose
/// (see "Addressing entries" in the design doc): a title that happens to
/// look like part of another entry's id still names its own entry.
///
/// Substring matching is title-only: descriptions and bodies are `search`'s
/// job, not addressing's. A query that isn't hex simply matches nothing at
/// the UUID stages.
///
/// A pure function over what is already decrypted, so a cached index can
/// reuse it.
pub fn resolve_query(query: &str, entries: &[(Uuid, String)]) -> Result<Uuid, ResolveError> {
    let lower_query = query.to_lowercase();
    let stripped_query = strip_uuid(query);
    let stages: [Stage; 4] = [
        &|_, title| title == query,
        &|_, title| title.to_lowercase().contains(&lower_query),
        &|id, _| Uuid::try_parse(query).is_ok_and(|wanted| *id == wanted),
        &|id, _| strip_uuid(&id.to_string()).contains(&stripped_query),
    ];
    for matches in stages {
        let hits: Vec<&(Uuid, String)> = entries.iter().filter(|(id, t)| matches(id, t)).collect();
        match hits.as_slice() {
            [] => continue,
            [(id, _)] => return Ok(*id),
            many => return Err(ambiguous(query, many.iter().copied())),
        }
    }
    Err(ResolveError::NotFound {
        query: query.to_string(),
    })
}

/// One resolution stage: whether an `(id, title)` pair matches.
type Stage<'a> = &'a dyn Fn(&Uuid, &str) -> bool;

/// A UUID (or a fragment of one) as the substring stage compares it:
/// hyphens dropped, lowercase.
fn strip_uuid(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase()
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

/// `ls` order: title, then id, so entries sharing a title (`-f` allows
/// it) come out the same way in both modes.
pub(crate) fn sort_list(rows: &mut [ListEntry]) {
    rows.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
}

impl Vault {
    /// Every entry's listing row, sorted by title then id. It decrypts
    /// every entry, so it needs the identity; it takes no lock.
    pub fn list(&self, identity: &Identity) -> Result<Vec<ListEntry>, Error> {
        let mut rows: Vec<ListEntry> = self
            .decrypt_all(identity)?
            .into_iter()
            .map(|(id, e)| ListEntry {
                id,
                title: e.title,
                description: e.description,
                created: e.created,
                updated: e.updated,
                updated_by: e.updated_by,
            })
            .collect();
        sort_list(&mut rows);
        Ok(rows)
    }

    /// Every entry, decrypted: one pass over the vault. Callers drop what
    /// they don't keep (a listing keeps no value or field).
    pub(crate) fn decrypt_all(&self, identity: &Identity) -> Result<Vec<(Uuid, Entry)>, Error> {
        self.entry_ids()?
            .into_iter()
            .map(|id| Ok((id, self.read_entry(id, identity)?)))
            .collect()
    }

    /// The one entry `query` names, decrypted, or a typed reason it names
    /// none. Reads only, so it takes no lock: a caller that goes on to
    /// write resolves again under the lock (see [`Vault::remove`]).
    pub fn resolve(&self, identity: &Identity, query: &str) -> Result<(Uuid, Entry), ResolveError> {
        let id = resolve_query(query, &self.titles(identity)?)?;
        Ok((id, self.read_entry(id, identity)?))
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
    /// can't interleave their commits. The push follows the commit inside
    /// the same lock; a push that fails is reported through `prompter`.
    pub fn insert(
        &self,
        identity: &Identity,
        new: NewEntry,
        force: bool,
        prompter: &dyn Prompter,
    ) -> Result<Uuid, WriteError> {
        self.insert_within(WRITE_LOCK_TIMEOUT, identity, new, force, prompter)
    }

    pub(crate) fn insert_within(
        &self,
        timeout: Duration,
        identity: &Identity,
        new: NewEntry,
        force: bool,
        prompter: &dyn Prompter,
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
            let now = Timestamp::now();
            let entry = Entry {
                title: new.title,
                description: new.description,
                created: Some(now),
                updated: Some(now),
                updated_by: identity.device().to_string(),
                value: new.value,
                fields: new.fields,
                extra: new.extra,
            };
            let id = new_id();
            self.write_entry(id, &entry)?;
            // A failed commit leaves the entry file in the working tree, as
            // Go: the next write's commit picks it up.
            gitrepo::commit_all(&self.path, &id.to_string())?;
            self.push_after_write(prompter);
            Ok(id)
        })
    }

    /// Deletes the entry `query` names and commits the deletion, under the
    /// write lock. The query is resolved inside the lock: resolving first
    /// and locking after would let a concurrent write change what the
    /// query means.
    pub fn remove(
        &self,
        identity: &Identity,
        query: &str,
        prompter: &dyn Prompter,
    ) -> Result<Uuid, WriteError> {
        self.remove_within(WRITE_LOCK_TIMEOUT, identity, query, prompter)
    }

    pub(crate) fn remove_within(
        &self,
        timeout: Duration,
        identity: &Identity,
        query: &str,
        prompter: &dyn Prompter,
    ) -> Result<Uuid, WriteError> {
        self.with_write_lock(timeout, || {
            let id = resolve_query(query, &self.titles(identity)?)?;
            let path = self.entry_path(id);
            std::fs::remove_file(&path).map_err(|source| WriteError::Io {
                action: "removing",
                path,
                source,
            })?;
            gitrepo::commit_all(&self.path, &id.to_string())?;
            self.push_after_write(prompter);
            Ok(id)
        })
    }

    /// Rewrites entry `id` as `e` (same id, fresh ciphertext) and commits,
    /// under the write lock. It stamps nothing itself: the caller hands it
    /// the final entry (`edit` re-stamps `updated`/`updated_by` and keeps
    /// the original `created`), so it can rewrite an entry as-is.
    ///
    /// Unlike Go's `Update` it takes no identity: the entry is encrypted to
    /// the vault's recipients, not to the writer. Go's other use for the
    /// identity, reporting the automatic push, is `prompter` here.
    pub fn update(&self, id: Uuid, e: &Entry, prompter: &dyn Prompter) -> Result<(), WriteError> {
        self.with_write_lock(WRITE_LOCK_TIMEOUT, || {
            self.write_entry(id, e)?;
            gitrepo::commit_all(&self.path, &id.to_string())?;
            self.push_after_write(prompter);
            Ok(())
        })
    }

    /// Changes only the title of the entry `query` names, re-stamping
    /// `updated` and `updated_by`; value, fields and `created` stay as they
    /// were. A title another entry already has is refused unless `force`,
    /// as on insert; the entry's own current title is always allowed, since
    /// renaming to it changes nothing. Resolved and checked under the write
    /// lock, as [`Vault::remove`] is.
    pub fn rename(
        &self,
        identity: &Identity,
        query: &str,
        new_title: &str,
        force: bool,
        prompter: &dyn Prompter,
    ) -> Result<Uuid, WriteError> {
        self.with_write_lock(WRITE_LOCK_TIMEOUT, || {
            let titles = self.titles(identity)?;
            let id = resolve_query(query, &titles)?;
            if !force
                && let Some((other, _)) = titles
                    .iter()
                    .find(|(other, t)| *other != id && t == new_title)
            {
                return Err(WriteError::DuplicateTitle {
                    title: new_title.to_string(),
                    id: *other,
                });
            }
            let mut e = self.read_entry(id, identity)?;
            e.title = new_title.to_string();
            e.updated = Some(Timestamp::now());
            e.updated_by = identity.device().to_string();
            self.write_entry(id, &e)?;
            gitrepo::commit_all(&self.path, &id.to_string())?;
            self.push_after_write(prompter);
            Ok(id)
        })
    }

    /// Runs `f` holding the vault's write lock, waiting up to `timeout` for
    /// it. The lock is released on every path out, including an error.
    /// Every write takes it, and so does every sync verb, since a
    /// fast-forward or a merge moves the working tree as a write does.
    pub(crate) fn with_write_lock<T, E: From<LockError>>(
        &self,
        timeout: Duration,
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let lock = vaultlock::Lock::acquire(&self.prepare_lock_file()?, timeout)
            .map_err(LockError::from)?;
        let result = f();
        match (result, lock.release()) {
            (Ok(v), Ok(())) => Ok(v),
            (Err(e), _) => Err(e),
            (Ok(_), Err(e)) => Err(LockError::from(e).into()),
        }
    }

    /// The write lock if it is free right now, `None` if another process
    /// holds it. Never waits: for pull on unlock, which skips its catch-up
    /// rather than make a read queue behind a write.
    pub(crate) fn try_write_lock(&self) -> Result<Option<vaultlock::Lock>, LockError> {
        Ok(vaultlock::Lock::try_acquire(&self.prepare_lock_file()?)?)
    }

    /// The lock file's path, with its directory created. The path (and
    /// with it the vault id) is validated before anything is created.
    fn prepare_lock_file(&self) -> Result<std::path::PathBuf, LockError> {
        let path = self.lock_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| LockError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Ok(path)
    }
}

/// Why the vault's write lock couldn't be taken or released. A write
/// reports it as the matching [`WriteError`] variant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LockError {
    #[error(transparent)]
    Path(#[from] paths::Error),
    #[error(transparent)]
    Lock(#[from] vaultlock::Error),
    #[error("gage: creating {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

impl HasCode for LockError {
    fn code(&self) -> Code {
        match self {
            LockError::Path(e) => e.code(),
            LockError::Lock(e) => e.code(),
            LockError::Io { .. } => Code::Internal,
        }
    }
}

impl From<LockError> for WriteError {
    fn from(err: LockError) -> WriteError {
        match err {
            LockError::Path(e) => WriteError::Path(e),
            LockError::Lock(e) => WriteError::Lock(e),
            LockError::Io { path, source } => WriteError::Io {
                action: "creating",
                path,
                source,
            },
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
    use crate::prompter::FakePrompter;
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
                    extra: [("x-future".to_string(), Extra::Plain("kept".into()))].into(),
                },
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let e = f.vault.read_entry(id, &f.identity()).unwrap();
        assert_eq!(e.extra["x-future"], Extra::Plain("kept".into()));
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
            .insert(
                &f.identity(),
                new_entry("a title", "v"),
                false,
                &FakePrompter::new(),
            )
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
            .insert(
                &f.identity(),
                new_entry("dup", "1"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let before = f.commits();
        let err = f
            .vault
            .insert(
                &f.identity(),
                new_entry("dup", "2"),
                false,
                &FakePrompter::new(),
            )
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
            .insert(
                &f.identity(),
                new_entry("dup", "1"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        f.vault
            .insert(
                &f.identity(),
                new_entry("dup", "2"),
                true,
                &FakePrompter::new(),
            )
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
            .insert(
                &f.identity(),
                new_entry("gone soon", "v"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let before = f.commits();

        let removed = f
            .vault
            .remove(&f.identity(), &id.to_string(), &FakePrompter::new())
            .unwrap();

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
            .insert(
                &f.identity(),
                new_entry("x", "v"),
                false,
                &FakePrompter::new(),
            )
            .unwrap_err();

        assert!(matches!(err, WriteError::Git(_)), "{err}");
        assert_eq!(err.code(), Code::Internal);
        assert_eq!(f.commits(), before);
        assert_eq!(f.vault.entry_ids().unwrap().len(), 1, "the file stays");
        std::fs::remove_file(&index_lock).unwrap();
        assert!(!gitrepo::is_clean(&f.vault.path).unwrap());

        f.vault
            .insert(
                &f.identity(),
                new_entry("y", "v"),
                false,
                &FakePrompter::new(),
            )
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
            .insert(
                &f.identity(),
                new_entry("keep", "v"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let before = f.commits();
        let err = f
            .vault
            .remove(&f.identity(), "nope", &FakePrompter::new())
            .unwrap_err();
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
        // The exact stage is exact: "proton" only reaches the substring
        // stage, where it matches both Proton titles.
        assert!(matches!(
            resolve_query("proton", &entries),
            Err(ResolveError::Ambiguous(_))
        ));
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
            .insert(
                &f.identity(),
                new_entry("findme", "v"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let (got, e) = f.vault.resolve(&f.identity(), "findme").unwrap();
        assert_eq!((got, e.value.as_str()), (id, "v"));
        let (got, _) = f
            .vault
            .resolve(&f.identity(), &id.simple().to_string())
            .unwrap();
        assert_eq!(got, id);
    }

    // --- the full resolution order (M5) ----------------------------------

    impl Fixture {
        /// Inserts an entry titled `title`, duplicates allowed (Go's
        /// `insertNamed`).
        fn insert_named(&self, title: &str) -> Uuid {
            self.vault
                .insert(
                    &self.identity(),
                    new_entry(title, "v"),
                    true,
                    &FakePrompter::new(),
                )
                .unwrap()
        }

        /// Writes an entry under a chosen id, so a test can pin which
        /// substrings its UUID contains.
        fn write_with_id(&self, id: &str, title: &str, value: &str) -> Uuid {
            let id = Uuid::parse_str(id).unwrap();
            let e = Entry {
                title: title.into(),
                value: value.into(),
                ..Entry::default()
            };
            self.vault.write_entry(id, &e).unwrap();
            id
        }

        fn resolve(&self, query: &str) -> Result<(Uuid, Entry), ResolveError> {
            self.vault.resolve(&self.identity(), query)
        }
    }

    fn candidate_ids(err: ResolveError) -> Vec<Uuid> {
        match err {
            ResolveError::Ambiguous(list) => list.candidates.iter().map(|c| c.id).collect(),
            other => panic!("want Ambiguous, got {other}"),
        }
    }

    // Go: TestResolveByExactUUID
    #[test]
    fn resolve_by_exact_uuid() {
        let f = Fixture::new();
        let id = f.insert_named("ProtonMail");
        let (got, e) = f.resolve(&id.to_string()).unwrap();
        assert_eq!(got, id);
        assert_eq!(e.title, "ProtonMail");
    }

    // Go: TestResolveByNonCanonicalUUIDSpellings. For a canonical id the
    // exact-UUID stage is invisible (the substring stage after it matches
    // the same entry); braced and URN spellings are what only it accepts,
    // so they are what keeps it from being deleted unnoticed.
    #[test]
    fn resolve_by_non_canonical_uuid_spellings() {
        let f = Fixture::new();
        let id = f.insert_named("ProtonMail");
        for q in [id.braced().to_string(), id.urn().to_string()] {
            let (got, e) = f.resolve(&q).unwrap();
            assert_eq!(got, id, "{q}");
            assert_eq!(e.title, "ProtonMail", "{q}");
        }
    }

    // Go: TestResolveByUUIDPrefix
    #[test]
    fn resolve_by_uuid_prefix() {
        let f = Fixture::new();
        let id = f.insert_named("ProtonMail");
        let (got, e) = f.resolve(&id.to_string()[..8]).unwrap();
        assert_eq!(got, id);
        assert_eq!(e.title, "ProtonMail");
    }

    // Go: TestResolveByUUIDMidStringSubstring. A pinned id, so the
    // substring sits at a known, non-prefix offset.
    #[test]
    fn resolve_by_uuid_mid_string_substring() {
        let f = Fixture::new();
        let id = f.write_with_id("11111111-2222-4333-8444-deadbeef0000", "ProtonMail", "v");
        let (got, e) = f.resolve("deadbeef").unwrap();
        assert_eq!(got, id);
        assert_eq!(e.title, "ProtonMail");
    }

    // Go: TestResolveByUUIDSubstringSpanningAHyphen. Hyphens are ignored on
    // both sides, so "22223333" spans the first group boundary.
    #[test]
    fn resolve_by_uuid_substring_spanning_a_hyphen() {
        let f = Fixture::new();
        let id = f.write_with_id("11112222-3333-4444-8555-666677778888", "ProtonMail", "v");
        assert_eq!(f.resolve("22223333").unwrap().0, id);
        assert_eq!(f.resolve("2222-3333").unwrap().0, id);
        assert_eq!(f.resolve("22223333-4444").unwrap().0, id);
    }

    // Go: TestResolveExactTitleWinsOverSubstringOfAnotherTitle
    #[test]
    fn resolve_exact_title_wins_over_substring_of_another_title() {
        let f = Fixture::new();
        let aws = f.insert_named("AWS");
        f.insert_named("AWS root account");
        let (got, e) = f.resolve("AWS").unwrap();
        assert_eq!(got, aws);
        assert_eq!(e.title, "AWS");
    }

    // Go: TestResolveUniqueSubstringMatch
    #[test]
    fn resolve_unique_substring_match() {
        let f = Fixture::new();
        let want = f.insert_named("AWS root account");
        f.insert_named("GitHub");
        let (got, e) = f.resolve("root").unwrap();
        assert_eq!(got, want);
        assert_eq!(e.title, "AWS root account");
    }

    // Go: TestResolveSubstringMatchIsCaseInsensitive
    #[test]
    fn resolve_substring_match_is_case_insensitive() {
        let f = Fixture::new();
        let want = f.insert_named("ProtonMail");
        assert_eq!(f.resolve("protonmail").unwrap().0, want);
        assert_eq!(f.resolve("TONMA").unwrap().0, want);
    }

    // Go: TestResolveAmbiguousSubstringListsCandidatesAsAValue
    #[test]
    fn resolve_ambiguous_substring_lists_candidates_as_a_value() {
        let f = Fixture::new();
        let a = f.insert_named("AWS root account");
        let b = f.insert_named("AWS IAM backup admin");
        let err = f.resolve("aws").unwrap_err();
        assert_eq!(err.code(), Code::Ambiguous);
        let ResolveError::Ambiguous(list) = err else {
            panic!("want Ambiguous");
        };
        assert_eq!(list.query, "aws");
        let titles: Vec<&str> = list.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["AWS IAM backup admin", "AWS root account"]);
        let ids: Vec<Uuid> = list.candidates.iter().map(|c| c.id).collect();
        assert_eq!(ids, [b, a]);
    }

    // Go: TestResolveAmbiguousUUIDSubstringListsCandidates. No title
    // matches, so the UUID substring stage is what reports the ambiguity.
    #[test]
    fn resolve_ambiguous_uuid_substring_lists_candidates() {
        let f = Fixture::new();
        let a = f.write_with_id("aaaaaaaa-1111-4111-8111-111111111111", "Something", "1");
        let b = f.write_with_id("aaaaaaaa-2222-4222-8222-222222222222", "Something", "2");
        let err = f.resolve("aaaaaaaa").unwrap_err();
        assert_eq!(err.code(), Code::Ambiguous);
        assert_eq!(candidate_ids(err), [a, b]);
    }

    // Go: TestResolveExactTitleWinsOverUUIDMatch, the regression test for
    // Go's original UUID-first order: "22" is one entry's exact title and
    // a substring of another's id, and must name the first.
    #[test]
    fn resolve_exact_title_wins_over_uuid_match() {
        let f = Fixture::new();
        f.write_with_id(
            "22ed1bfd-4b77-4bba-90be-ff40de0eab87",
            "Website Password",
            "correcthorsebattery",
        );
        let twenty_two = f.write_with_id(
            "5ebca883-6a5f-4e96-acab-766f2fceb2d4",
            "22",
            "some-other-secret",
        );
        let (got, e) = f.resolve("22").unwrap();
        assert_eq!(got, twenty_two);
        assert_eq!(
            (e.title.as_str(), e.value.as_str()),
            ("22", "some-other-secret")
        );
    }

    // Go: TestResolveNotFoundAndAmbiguousAreDistinguishable
    #[test]
    fn resolve_not_found_and_ambiguous_are_distinguishable() {
        let f = Fixture::new();
        f.insert_named("AWS root account");
        f.insert_named("AWS IAM backup admin");
        let not_found = f.resolve("no such thing at all").unwrap_err();
        let ambiguous = f.resolve("aws").unwrap_err();
        assert!(
            matches!(not_found, ResolveError::NotFound { .. }),
            "{not_found}"
        );
        assert!(
            matches!(ambiguous, ResolveError::Ambiguous(_)),
            "{ambiguous}"
        );
        assert_eq!(not_found.code(), Code::NotFound);
        assert_eq!(ambiguous.code(), Code::Ambiguous);
        assert_ne!(not_found.code(), ambiguous.code());
    }

    // --- update and rename ------------------------------------------------

    /// Rewrites `id`'s timestamps to a day ago, so a later re-stamp is
    /// observably later even within one wall-clock second.
    fn backdate(f: &Fixture, id: Uuid) -> Timestamp {
        let old = Timestamp::new(jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24));
        let mut e = f.vault.read_entry(id, &f.identity()).unwrap();
        e.created = Some(old);
        e.updated = Some(old);
        f.vault.write_entry(id, &e).unwrap();
        gitrepo::commit_all(&f.vault.path, "backdate").unwrap();
        old
    }

    // Go: TestUpdateRewritesEntryAndCommits
    #[test]
    fn update_rewrites_entry_and_commits() {
        let f = Fixture::new();
        let id = f.insert_named("ProtonMail");
        let before = f.commits();
        let mut e = f.vault.read_entry(id, &f.identity()).unwrap();
        e.value = "rotated-secret".into();

        f.vault.update(id, &e, &FakePrompter::new()).unwrap();

        assert_eq!(f.commits(), before + 1);
        assert_eq!(f.head().message, id.to_string());
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
        assert_eq!(f.vault.read_entry(id, &f.identity()).unwrap(), e);
    }

    // Go: TestRenameChangesOnlyTitleAndBumpsUpdated
    #[test]
    fn rename_changes_only_title_and_bumps_updated() {
        let f = Fixture::new();
        let id = f
            .vault
            .insert(
                &f.identity(),
                NewEntry {
                    title: "Old Title".into(),
                    description: "d".into(),
                    value: "secret".into(),
                    fields: [("user".to_string(), "me".to_string())].into(),
                    ..NewEntry::default()
                },
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let old = backdate(&f, id);
        let mut was = f.vault.read_entry(id, &f.identity()).unwrap();
        was.updated_by = "another-device".into();
        f.vault.write_entry(id, &was).unwrap();

        let got = f
            .vault
            .rename(
                &f.identity(),
                "Old Title",
                "New Title",
                false,
                &FakePrompter::new(),
            )
            .unwrap();

        assert_eq!(got, id);
        let e = f.vault.read_entry(id, &f.identity()).unwrap();
        assert_eq!(e.title, "New Title");
        assert!(e.updated.unwrap() > old, "updated bumped");
        assert_eq!(e.updated_by, "test-device", "updated_by re-stamped");
        assert_eq!(e.created, Some(old), "created untouched");
        // Everything else is as it was.
        assert_eq!(
            Entry {
                title: was.title.clone(),
                updated: was.updated,
                updated_by: was.updated_by.clone(),
                ..e
            },
            was
        );
    }

    // Go: TestRenameProducesExactlyOneCommit
    #[test]
    fn rename_produces_exactly_one_commit() {
        let f = Fixture::new();
        let id = f.insert_named("Old Title");
        let before = f.commits();
        f.vault
            .rename(
                &f.identity(),
                "Old Title",
                "New Title",
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        assert_eq!(f.commits(), before + 1);
        assert_eq!(f.head().message, id.to_string());
        assert!(gitrepo::is_clean(&f.vault.path).unwrap());
    }

    // Go: TestRenameToExistingTitleRejectedUnlessForced
    #[test]
    fn rename_to_existing_title_rejected_unless_forced() {
        let f = Fixture::new();
        let a = f.insert_named("Entry A");
        let b = f.insert_named("Entry B");
        let before = f.commits();

        let err = f
            .vault
            .rename(
                &f.identity(),
                "Entry B",
                "Entry A",
                false,
                &FakePrompter::new(),
            )
            .unwrap_err();

        assert!(
            matches!(&err, WriteError::DuplicateTitle { title, id } if title == "Entry A" && *id == a),
            "{err}"
        );
        assert_eq!(err.code(), Code::Conflict);
        assert_eq!(f.commits(), before);
        assert_eq!(f.resolve("Entry B").unwrap().0, b, "B is unrenamed");

        f.vault
            .rename(
                &f.identity(),
                "Entry B",
                "Entry A",
                true,
                &FakePrompter::new(),
            )
            .unwrap();
        assert!(matches!(
            f.resolve("Entry A"),
            Err(ResolveError::Ambiguous(_))
        ));
    }

    // Go: TestRenameToOwnTitleAlwaysAllowed
    #[test]
    fn rename_to_own_title_always_allowed() {
        let f = Fixture::new();
        let id = f.insert_named("Same");
        assert_eq!(
            f.vault
                .rename(&f.identity(), "Same", "Same", false, &FakePrompter::new())
                .unwrap(),
            id
        );
    }

    // Go: TestRenameUnknownQueryFails
    #[test]
    fn rename_unknown_query_fails() {
        let f = Fixture::new();
        f.insert_named("keep");
        let before = f.commits();
        let err = f
            .vault
            .rename(
                &f.identity(),
                "no such entry",
                "New Title",
                false,
                &FakePrompter::new(),
            )
            .unwrap_err();
        assert!(
            matches!(&err, WriteError::Resolve(ResolveError::NotFound { .. })),
            "{err}"
        );
        assert_eq!(err.code(), Code::NotFound);
        assert_eq!(f.commits(), before);
    }

    // Not a Go row: rename resolves through the full order, and an
    // ambiguous query renames nothing.
    #[test]
    fn an_ambiguous_rename_changes_nothing() {
        let f = Fixture::new();
        f.insert_named("AWS root account");
        f.insert_named("AWS IAM backup admin");
        let before = f.commits();
        let err = f
            .vault
            .rename(
                &f.identity(),
                "aws",
                "Whatever",
                false,
                &FakePrompter::new(),
            )
            .unwrap_err();
        assert_eq!(err.code(), Code::Ambiguous);
        assert_eq!(f.commits(), before);
        assert!(f.resolve("Whatever").is_err());
    }

    // A query that isn't hex matches nothing at the UUID stages rather than
    // erroring: "zz" is in no title and can't be in any id.
    #[test]
    fn a_non_hex_query_falls_through_every_stage_to_not_found() {
        let entries = pairs(&["alpha", "beta"]);
        assert!(matches!(
            resolve_query("zz", &entries),
            Err(ResolveError::NotFound { .. })
        ));
        // And the empty query is a substring of everything: ambiguous with
        // more than one entry, never a silent pick.
        assert!(matches!(
            resolve_query("", &entries),
            Err(ResolveError::Ambiguous(_))
        ));
    }

    #[test]
    fn list_is_sorted_by_title_then_id_and_carries_the_metadata() {
        let f = Fixture::new();
        let id = f.identity();
        assert!(f.vault.list(&id).unwrap().is_empty());
        for t in ["b", "a", "b"] {
            f.vault
                .insert(&id, new_entry(t, "v"), true, &FakePrompter::new())
                .unwrap();
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
            .insert(
                &f.identity(),
                new_entry("x", "v"),
                false,
                &FakePrompter::new(),
            )
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
                        &FakePrompter::new(),
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
                &FakePrompter::new(),
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
                &FakePrompter::new(),
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
            .insert(
                &f.identity(),
                new_entry("readable", "v"),
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        let _holder = Holder::spawn(&f);

        let start = Instant::now();
        assert_eq!(f.vault.list(&f.identity()).unwrap().len(), 1);
        assert_eq!(
            f.vault.read_entry(id, &f.identity()).unwrap().title,
            "readable"
        );
        assert_eq!(f.vault.resolve(&f.identity(), "readable").unwrap().0, id);
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
