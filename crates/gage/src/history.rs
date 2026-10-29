//! An entry's history: when it changed (`log`, no identity needed) and
//! what it held (`history`, decrypting every revision). Both walk the one
//! path `entries/<id>.age` through [`gitrepo::log`]. Reads: neither takes
//! the vault lock.

use uuid::Uuid;

use crate::entry::{ENTRIES_DIR_NAME, ENTRY_FILE_EXT, Entry, Timestamp};
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::gitrepo;
use crate::identity::Identity;
use crate::vault::Vault;

/// How many characters of a commit hash an error names.
const SHORT_HASH_LEN: usize = 7;

/// One change to an entry, as `log` reports it. There is no message field:
/// gage's commit messages are the entry's UUID and nothing else, and
/// printing one would only make `log` look like it shows message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub hash: String,
    pub when: Timestamp,
    /// This commit removed the entry.
    pub deleted: bool,
}

/// One past version of an entry, decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub hash: String,
    pub when: Timestamp,
    pub deleted: bool,
    /// `None` exactly when `deleted`.
    pub entry: Option<Entry>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoryError {
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    /// A revision this identity can't read. Never skipped: a history with
    /// a silent gap would look complete.
    #[error("decrypting revision {hash} of {id}: {source}")]
    Decrypt {
        hash: String,
        id: Uuid,
        source: crate::entry::Error,
    },
}

impl HasKind for HistoryError {
    fn kind(&self) -> ErrorKind {
        match self {
            HistoryError::Git(e) => e.kind(),
            HistoryError::Decrypt { source, .. } => source.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            HistoryError::Git(e) => e.remedy(),
            HistoryError::Decrypt { source, .. } => source.remedy(),
        }
    }
}

/// The path git knows an entry by. Always forward slashes, whatever the
/// platform: git tree paths are slash-separated, and a `Path::join` would
/// match nothing on Windows.
fn entry_file_path(id: Uuid) -> String {
    format!("{ENTRIES_DIR_NAME}/{id}{ENTRY_FILE_EXT}")
}

impl Vault {
    /// When entry `id` changed, newest first. Takes no identity: a caller
    /// holding an id shouldn't have to unlock. An id git has never seen
    /// gives an empty list.
    pub fn log(&self, id: Uuid) -> Result<Vec<LogEntry>, HistoryError> {
        Ok(gitrepo::log(&self.path, &entry_file_path(id))?
            .into_iter()
            .map(|r| LogEntry {
                deleted: r.content.is_none(),
                hash: r.id,
                when: Timestamp::new(r.when),
            })
            .collect())
    }

    /// Every past revision of entry `id`, decrypted, oldest first. A
    /// revision `identity` can't decrypt fails the whole call.
    pub fn history(&self, id: Uuid, identity: &Identity) -> Result<Vec<Revision>, HistoryError> {
        let mut revisions = gitrepo::log(&self.path, &entry_file_path(id))?;
        revisions.reverse();
        revisions
            .into_iter()
            .map(|r| {
                let entry = match &r.content {
                    Some(ciphertext) => {
                        Some(self.decrypt_entry(ciphertext, identity).map_err(|source| {
                            HistoryError::Decrypt {
                                hash: r.id.chars().take(SHORT_HASH_LEN).collect(),
                                id,
                                source,
                            }
                        })?)
                    }
                    None => None,
                };
                Ok(Revision {
                    deleted: entry.is_none(),
                    hash: r.id,
                    when: Timestamp::new(r.when),
                    entry,
                })
            })
            .collect()
    }
}
