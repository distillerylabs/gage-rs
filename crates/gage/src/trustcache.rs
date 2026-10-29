//! The local trust cache: this device's record of the recipient list it
//! last confirmed for each vault, and the checks that consult it. A
//! blocking one runs before anything is encrypted, and an opportunistic one
//! on unlock and on sync. See "Trust boundaries", "Local trust cache" and
//! "Cache regeneration isn't one-size-fits-all" in the design doc.
//!
//! The record is a verbatim copy of `.gage/config.toml` plus the SHA-256 of
//! `.age-recipients`, under `$GAGE_STATE/<vault-id>/`. Both halves matter:
//! the config copy is what gets diffed and shown, because device names make
//! a legible warning; the hash is what catches someone editing only the
//! file encryption actually reads.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::entry::Timestamp;
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::gitrepo;
use crate::prompter::{PromptError, Prompter, Warning};
use crate::recipient::{RecipientError, RecipientVerification, VerifyError};
use crate::serde_opt::empty_as_none;
use crate::vault::Vault;
use crate::vaultconfig::{self, Recipient, VaultId};
use crate::xdgpaths::Dirs;
use crate::{atomicfile, paths};

/// The verbatim copy's name: the file the plan and the design doc both
/// name, and the one a human comparing caches by hand reaches for.
pub const KNOWN_CONFIG_FILE: &str = "known-config.toml";

/// The hash half, with this record's metadata.
pub const TRUST_FILE: &str = "trust.toml";

/// What the library hands a frontend when the committed recipient list no
/// longer matches what this device last confirmed: the diff to show,
/// whether the two recipient files still agree with each other, and who
/// moved.
///
/// A value, not printed text, for the same reason a candidate list is one:
/// `gage-cli` renders it as the design doc's diff and `[y/N]`, a GUI as a
/// dialog, and neither rendering belongs in the library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientChangeWarning {
    /// The vault whose list changed, named because a session holds
    /// several.
    pub vault: String,
    /// The unified diff of the cached `known-config.toml` against the
    /// committed `.gage/config.toml`. Empty when only `.age-recipients`
    /// changed, which `verification` catches instead.
    pub diff: String,
    /// `verify_recipients` now. Out of sync is the tampering signature, and
    /// is what makes this a mismatched change rather than a routine one.
    pub verification: RecipientVerification,
    /// Recipients `config.toml` gained and lost since the cached copy, so a
    /// frontend can summarize without parsing the diff.
    pub added: Vec<Recipient>,
    pub removed: Vec<Recipient>,
    /// When this device last confirmed a list for this vault: the date the
    /// design doc's diff header carries.
    pub last_confirmed: Option<Timestamp>,
}

impl RecipientChangeWarning {
    /// The severe case: `.age-recipients` and `config.toml` disagree with
    /// each other, not merely with the cache. Confirming past one does not
    /// regenerate the cache.
    pub fn mismatched(&self) -> bool {
        !self.verification.in_sync
    }

    /// The one line the opportunistic checks warn with, on unlock and on
    /// sync, where nothing blocks and nobody is asked. With
    /// [`RecipientChangeWarning::summary_remedy`] it says what to do next:
    /// the warning rides along with a read, so what it interrupts is not
    /// what it is about.
    pub fn summary(&self) -> String {
        if self.mismatched() {
            return format!(
                "{:?}'s recipient list changed since this device last confirmed it, and its two \
                 recipient files now disagree",
                self.vault
            );
        }
        format!(
            "{:?}'s recipient list changed since this device last confirmed it ({}); \
             the next write here will ask you to review it.",
            self.vault,
            self.delta()
        )
    }

    /// What the summary's reader does next: check the files when they
    /// disagree. Otherwise the next write asks, and nothing needs running.
    pub fn summary_remedy(&self) -> Option<Remedy> {
        self.mismatched().then_some(Remedy::RecipientVerify)
    }

    /// The summary's parenthetical: what moved, counted rather than listed,
    /// since one line has no room for keys.
    pub(crate) fn delta(&self) -> String {
        match (self.added.len(), self.removed.len()) {
            (0, 0) => {
                // config.toml shows no recipient change, so there is
                // nothing to count: the case with the least to say and the
                // most to worry about.
                ".age-recipients was edited on its own".to_string()
            }
            (a, 0) => format!("{a} added"),
            (0, r) => format!("{r} removed"),
            (a, r) => format!("{a} added, {r} removed"),
        }
    }
}

/// This device's record of the list it last confirmed for one vault.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TrustCache {
    /// `.gage/config.toml`'s bytes as last confirmed. Verbatim, so the diff
    /// a human reads is a diff of the change, not of gage's formatting.
    pub(crate) known_config: Vec<u8>,
    /// The hex SHA-256 of `.age-recipients` as last confirmed.
    pub(crate) recipients_hash: String,
    pub(crate) confirmed_at: Option<Timestamp>,
    /// The hash of an `.age-recipients` acknowledged while it disagreed
    /// with `config.toml`: "the mismatch was seen". It never clears the
    /// inconsistency, and neither half above moves for it.
    pub(crate) mismatch_seen: Option<String>,
}

/// `trust.toml`'s shape: everything but the config copy,
/// which is its own file so it can be read (and diffed) as the TOML it is.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TrustRecord {
    pub(crate) recipients_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) confirmed_at: Option<toml::value::Datetime>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) mismatch_seen: Option<String>,
}

/// Why the trust cache couldn't be consulted, or refused a write.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TrustError {
    /// The recipient list changed and the answer was no. Nothing was
    /// written, and the cache stays where it was, so the same question
    /// comes back next time.
    #[error("the recipient list of {vault:?} changed and was not confirmed; nothing was written")]
    ChangeDeclined { vault: String },
    /// The recipient list changed and nobody could be asked: a frontend
    /// with no one at it. An unapproved list is exactly what the check
    /// refuses to encrypt to.
    #[error(
        "the recipient list of {vault:?} changed and could not be confirmed ({}); nothing was written",
        source
    )]
    Unconfirmable { vault: String, source: PromptError },
    /// `recipient add`/`remove` refusing a vault whose two recipient files
    /// disagree. Both rebuild `.age-recipients` from `config.toml`, so
    /// running either would overwrite the stray key out of existence and
    /// commit it as an ordinary change: the evidence gone, `verify` clean,
    /// and the cache regenerated over a list nobody reviewed. It names
    /// every difference, since those are the whole content of the refusal.
    #[error(
        "the recipient files of {vault:?} disagree:\n{}",
        difference_lines(.verification)
    )]
    OutOfSync {
        vault: String,
        verification: RecipientVerification,
    },
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("encoding {TRUST_FILE}: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("creating {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("removing {path}: {source}")]
    Remove {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Write(#[from] atomicfile::Error),
    // The TOML and config errors are boxed: inline they make this enum
    // 120 bytes, and on Windows (whose `PathBuf` is larger) every error
    // carrying a `TrustError` crosses clippy's `result_large_err` limit.
    #[error(transparent)]
    Verify(Box<VerifyError>),
    #[error(transparent)]
    Config(Box<vaultconfig::Error>),
}

impl From<VerifyError> for TrustError {
    fn from(e: VerifyError) -> TrustError {
        TrustError::Verify(Box::new(e))
    }
}

impl From<vaultconfig::Error> for TrustError {
    fn from(e: vaultconfig::Error) -> TrustError {
        TrustError::Config(Box::new(e))
    }
}

impl HasKind for TrustError {
    fn kind(&self) -> ErrorKind {
        match self {
            // A human must decide, and didn't or couldn't.
            TrustError::ChangeDeclined { .. }
            | TrustError::Unconfirmable { .. }
            | TrustError::OutOfSync { .. } => ErrorKind::Conflict,
            TrustError::Write(e) => e.kind(),
            TrustError::Verify(e) => e.kind(),
            TrustError::Config(e) => e.kind(),
            TrustError::Read { .. }
            | TrustError::Parse { .. }
            | TrustError::Encode(_)
            | TrustError::CreateDir { .. }
            | TrustError::Remove { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            TrustError::Unconfirmable { .. } => Some(Remedy::Yes),
            TrustError::OutOfSync { .. } => Some(Remedy::RecipientRepair),
            TrustError::Write(e) => e.remedy(),
            TrustError::Verify(e) => e.remedy(),
            TrustError::Config(e) => e.remedy(),
            _ => None,
        }
    }
}

/// A failing verification, as `recipient verify` words it, so the refusal
/// and the diagnostic verb describe one vault in the same words.
pub fn difference_lines(v: &RecipientVerification) -> String {
    v.only_in_recipients_file
        .iter()
        .map(|k| format!("  {k} is in .age-recipients but not in .gage/config.toml"))
        .chain(
            v.only_in_config
                .iter()
                .map(|k| format!("  {k} is in .gage/config.toml but not in .age-recipients")),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

/// Deletes the vault's trust cache: what `vault remove` does alongside
/// dropping the registration, so re-adding the vault is a first use rather
/// than an inherited approval of a list nobody looked at in between.
///
/// It removes the files the cache owns, by name, and then the directory
/// only if that left it empty, rather than the whole directory: a narrow
/// delete needs no argument about what else might live there. Anything
/// added to the cache's on-disk form belongs in this list too. A missing
/// cache is not an error.
pub fn remove(dirs: &Dirs, vault_id: &VaultId) -> Result<(), TrustError> {
    let dir = paths::trust_cache_dir(dirs, vault_id);
    for name in [KNOWN_CONFIG_FILE, TRUST_FILE] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(TrustError::Remove { path, source }),
        }
    }
    // A directory something else still lives in is left alone on purpose;
    // the cache it held is already gone.
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// A file's contents, or `None` if it doesn't exist.
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, TrustError> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(TrustError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>, TrustError> {
    std::fs::read(path).map_err(|source| TrustError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Vault {
    fn trust_cache_dir(&self) -> PathBuf {
        paths::trust_cache_dir(self.dirs(), &self.id)
    }

    /// This device's record for the vault, or `None` when there is none
    /// yet: the first use, bootstrapped silently rather than warned about.
    ///
    /// Either file missing reads as no record. They are written as a pair,
    /// and half a pair is not a record this device can claim to have
    /// confirmed; treating it as a first use is the same weak-but-honest
    /// position a new device is in.
    pub(crate) fn load_trust_cache(&self) -> Result<Option<TrustCache>, TrustError> {
        let dir = self.trust_cache_dir();
        let Some(known_config) = read_optional(&dir.join(KNOWN_CONFIG_FILE))? else {
            return Ok(None);
        };
        let path = dir.join(TRUST_FILE);
        let Some(data) = read_optional(&path)? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&data);
        let record: TrustRecord = toml::from_str(&text).map_err(|source| TrustError::Parse {
            path,
            source: Box::new(source),
        })?;
        Ok(Some(TrustCache {
            known_config,
            recipients_hash: record.recipients_hash,
            confirmed_at: record.confirmed_at.and_then(|d| d.to_string().parse().ok()),
            mismatch_seen: record.mismatch_seen,
        }))
    }

    /// Writes both halves, each atomically.
    ///
    /// The config copy goes first. Two files can't land together, so the
    /// order is chosen for the safer torn state: a new config copy beside
    /// an old hash reads as a changed list and warns, which is the
    /// direction a half-finished write should fail in.
    pub(crate) fn save_trust_cache(&self, cache: &TrustCache) -> Result<(), TrustError> {
        let dir = self.trust_cache_dir();
        std::fs::create_dir_all(&dir).map_err(|source| TrustError::CreateDir {
            path: dir.clone(),
            source,
        })?;
        atomicfile::write(&dir.join(KNOWN_CONFIG_FILE), &cache.known_config, 0o600)?;
        let record = TrustRecord {
            recipients_hash: cache.recipients_hash.clone(),
            confirmed_at: cache.confirmed_at.and_then(|t| t.to_string().parse().ok()),
            mismatch_seen: cache.mismatch_seen.clone(),
        };
        let text = toml::to_string(&record)?;
        atomicfile::write(&dir.join(TRUST_FILE), text.as_bytes(), 0o600)?;
        Ok(())
    }

    /// What the cache would hold if this device confirmed the committed
    /// recipient files right now: both files' own bytes, neither
    /// re-serialized on the way through.
    fn current_trust_state(&self) -> Result<TrustCache, TrustError> {
        let known_config = read_file(&self.config_path())?;
        let keys = read_file(&self.path.join(".age-recipients"))?;
        Ok(TrustCache {
            known_config,
            recipients_hash: sha256_hex(&keys),
            confirmed_at: Some(Timestamp::now()),
            mismatch_seen: None,
        })
    }

    /// Compares the committed recipient files with this device's record.
    /// `None` means nothing to say: they match, or there was no record and
    /// this use bootstrapped one.
    ///
    /// Either half differing is enough, which is the whole reason there are
    /// two: an edit to `.age-recipients` alone leaves `config.toml`, and so
    /// the diff, completely unchanged.
    pub(crate) fn check_recipient_trust(
        &self,
    ) -> Result<Option<RecipientChangeWarning>, TrustError> {
        let current = self.current_trust_state()?;
        let Some(cache) = self.load_trust_cache()? else {
            // First use. The first list a device sees is trusted on faith:
            // inherent to trust-on-first-use, and why this is a backstop
            // rather than a guarantee.
            self.save_trust_cache(&current)?;
            return Ok(None);
        };
        if cache.known_config == current.known_config
            && cache.recipients_hash == current.recipients_hash
        {
            return Ok(None);
        }
        let diff = if cache.known_config == current.known_config {
            String::new()
        } else {
            let date = cache.confirmed_at.map_or_else(
                || "unknown".to_string(),
                |t| t.as_jiff().strftime("%Y-%m-%d").to_string(),
            );
            unified_diff(
                &format!("{KNOWN_CONFIG_FILE} (last confirmed {date})"),
                ".gage/config.toml (current)",
                &cache.known_config,
                &current.known_config,
            )
        };
        let (added, removed) = self.recipient_delta(&cache.known_config)?;
        Ok(Some(RecipientChangeWarning {
            vault: self.name.clone(),
            diff,
            verification: self.verify_recipients()?,
            added,
            removed,
            last_confirmed: cache.confirmed_at,
        }))
    }

    /// Who the committed `config.toml` gained and lost since the cached
    /// copy, each in its own file's order.
    ///
    /// The cached bytes go through the plain TOML decoder, not
    /// `vaultconfig::load`: gage wrote them from a file that was already
    /// validated, and an old copy a newer gage would refuse on
    /// `format_version` must still be describable. Refusing to describe the
    /// change would suppress the warning the refusal is about.
    fn recipient_delta(
        &self,
        cached: &[u8],
    ) -> Result<(Vec<Recipient>, Vec<Recipient>), TrustError> {
        let path = self.trust_cache_dir().join(KNOWN_CONFIG_FILE);
        let was: vaultconfig::File =
            toml::from_str(&String::from_utf8_lossy(cached)).map_err(|source| {
                TrustError::Parse {
                    path,
                    source: Box::new(source),
                }
            })?;
        let now = self.recipients()?;
        let had = |key: &str| was.recipients.iter().any(|r| r.pubkey == key);
        let has = |key: &str| now.iter().any(|r| r.pubkey == key);
        let added = now.iter().filter(|r| !had(&r.pubkey)).cloned().collect();
        let removed = was
            .recipients
            .iter()
            .filter(|r| !has(&r.pubkey))
            .cloned()
            .collect();
        Ok((added, removed))
    }

    /// The blocking pre-encrypt check, run under the vault lock before
    /// anything is written. A no refuses the write; a yes applies the
    /// two-outcome rule. The rule is decided here, from `verify`, never
    /// from who answered: that is what makes `--yes` structurally unable to
    /// buy silence for a mismatch.
    pub(crate) fn confirm_recipient_trust(
        &self,
        prompter: &dyn Prompter,
    ) -> Result<(), TrustError> {
        let Some(warning) = self.check_recipient_trust()? else {
            return Ok(());
        };
        match prompter.confirm_recipient_change(&warning) {
            Ok(true) => {}
            Ok(false) => {
                return Err(TrustError::ChangeDeclined {
                    vault: self.name.clone(),
                });
            }
            Err(source) => {
                return Err(TrustError::Unconfirmable {
                    vault: self.name.clone(),
                    source,
                });
            }
        }
        if warning.mismatched() {
            self.note_recipient_mismatch_seen()
        } else {
            self.note_recipients_reviewed()
        }
    }

    /// The opportunistic check on unlock and on sync: it never asks, never
    /// regenerates the cache, and never fails what it rides along with.
    ///
    /// A failure to read the cache or the files is swallowed on purpose:
    /// every caller has already succeeded, and an advisory must not turn a
    /// working unlock or a finished sync into an error. The blocking check
    /// before the next encrypt reports the same failure where it can be
    /// acted on.
    pub(crate) fn warn_recipient_trust(&self, prompter: &dyn Prompter) {
        if let Ok(Some(warning)) = self.check_recipient_trust() {
            prompter.warn(&Warning::from(warning.summary()).with_remedy(warning.summary_remedy()));
        }
    }

    /// Regenerates both halves against the committed state: a confirmed
    /// routine change, and the last step of a local `recipient add` or
    /// `remove`, which the operator just reviewed by typing it.
    ///
    /// The mismatch-seen mark is not carried over: both files agreeing is
    /// what settles it.
    pub(crate) fn note_recipients_reviewed(&self) -> Result<(), TrustError> {
        self.save_trust_cache(&self.current_trust_state()?)
    }

    /// The other outcome: a mismatched change was confirmed, so the write
    /// proceeds and nothing else does. Both halves stay where they were
    /// (`verify` keeps failing, the next write asks again), and only which
    /// `.age-recipients` was acknowledged is recorded.
    fn note_recipient_mismatch_seen(&self) -> Result<(), TrustError> {
        // A mismatch is only reported against a record that exists.
        // Bootstrapping one here would approve the very list it is about.
        let Some(mut cache) = self.load_trust_cache()? else {
            return Ok(());
        };
        cache.mismatch_seen = Some(self.current_trust_state()?.recipients_hash);
        self.save_trust_cache(&cache)
    }

    /// `recipient add`/`remove`'s precondition: both rebuild
    /// `.age-recipients` from `config.toml`, so neither may run over a
    /// divergence. See [`TrustError::OutOfSync`].
    pub(crate) fn require_recipients_in_sync(&self) -> Result<(), TrustError> {
        let verification = self.verify_recipients()?;
        if verification.in_sync {
            return Ok(());
        }
        Err(TrustError::OutOfSync {
            vault: self.name.clone(),
            verification,
        })
    }
}

/// What `recipient verify --repair` did: the keys it dropped from
/// `.age-recipients`, the keys it added, and the commit holding the
/// rewrite (`None` when the files already agreed and nothing was done).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientRepair {
    pub dropped: Vec<String>,
    pub added: Vec<String>,
    pub commit: Option<String>,
}

/// The confirmation names every key on both sides, because that list is
/// the whole decision: rewriting `.age-recipients` is only safe if the
/// operator agrees the keys it removes shouldn't have been there.
fn repair_prompt(vault: &str, repair: &RecipientRepair) -> String {
    let mut text = format!("Rewrite {vault:?}'s .age-recipients from .gage/config.toml?");
    for k in &repair.dropped {
        text.push_str(&format!(
            "\n  drop {k} (it can read anything written while it was listed)"
        ));
    }
    for k in &repair.added {
        text.push_str(&format!("\n  add  {k} (config.toml already lists it)"));
    }
    text
}

impl Vault {
    /// Rewrites `.age-recipients` from `.gage/config.toml`'s list and
    /// commits it as `gage: recipient repair`: the deliberate, visible form
    /// of the overwrite `recipient add` used to perform as a side effect,
    /// and the exit from [`TrustError::OutOfSync`].
    ///
    /// It confirms first through `prompter.confirm`, naming every key
    /// dropped and added; a no, or nobody to ask, writes nothing. Files
    /// that already agree are left alone with no question and no commit.
    ///
    /// It deliberately does not regenerate the trust cache: repair settles
    /// the split between the two files, not whether the list itself is one
    /// this device approves. The next encrypt still asks.
    pub fn repair_recipients(
        &self,
        prompter: &dyn Prompter,
    ) -> Result<RecipientRepair, RecipientError> {
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            // Re-read under the lock: what gets rewritten has to be what
            // was just looked at.
            let verdict = self.verify_recipients().map_err(TrustError::from)?;
            if verdict.in_sync {
                return Ok(RecipientRepair::default());
            }
            let mut repair = RecipientRepair {
                dropped: verdict.only_in_recipients_file,
                added: verdict.only_in_config,
                commit: None,
            };
            match prompter.confirm(&repair_prompt(&self.name, &repair)) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(RecipientError::RepairDeclined {
                        vault: self.name.clone(),
                    });
                }
                Err(source) => {
                    return Err(RecipientError::RepairUnconfirmed {
                        vault: self.name.clone(),
                        source,
                    });
                }
            }
            self.write_recipient_files(&self.recipients()?)?;
            repair.commit = Some(gitrepo::commit_all(&self.path, "gage: recipient repair")?);
            self.push_after_write(prompter);
            // No note_recipients_reviewed, deliberately: see above.
            Ok(repair)
        })
    }
}

// --- diffing the cached config against the committed one ------------------

/// Unchanged lines kept around each change. The files are a few dozen
/// lines of TOML, so this is about keeping a `[[recipients]]` header
/// visible above the device line that changed under it.
const DIFF_CONTEXT_LINES: usize = 3;

/// One line of an edit script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffOp {
    Keep(String),
    Remove(String),
    Add(String),
}

impl DiffOp {
    fn prefix(&self) -> char {
        match self {
            DiffOp::Keep(_) => ' ',
            DiffOp::Remove(_) => '-',
            DiffOp::Add(_) => '+',
        }
    }

    fn text(&self) -> &str {
        match self {
            DiffOp::Keep(s) | DiffOp::Remove(s) | DiffOp::Add(s) => s,
        }
    }
}

/// `a` against `b` as a unified diff, `old_name`/`new_name` on its
/// `---`/`+++` lines.
///
/// gage carries its own rather than a diff crate: what it diffs is one
/// small self-written TOML file against another, and the output is read by
/// a human deciding whether to trust a recipient list, which is a poor
/// place for an output shape that could change under us.
pub(crate) fn unified_diff(old_name: &str, new_name: &str, a: &[u8], b: &[u8]) -> String {
    let ops = longest_common_subsequence(&split_diff_lines(a), &split_diff_lines(b));
    let mut out = format!("--- {old_name}\n+++ {new_name}\n");
    for hunk in diff_hunks(&ops) {
        out.push_str(&hunk);
    }
    out.trim_end_matches('\n').to_string()
}

/// A file's lines, without a trailing empty one: a file ending in a
/// newline doesn't diff as having a blank last line.
pub(crate) fn split_diff_lines(data: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(data).replace("\r\n", "\n");
    let text = text.strip_suffix('\n').unwrap_or(&text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(str::to_string).collect()
}

/// The edit script turning `a` into `b`: the classic LCS table, walked
/// forwards. Quadratic, the right trade for two files of a few dozen lines.
pub(crate) fn longest_common_subsequence(a: &[String], b: &[String]) -> Vec<DiffOp> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(DiffOp::Keep(a[i].clone()));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push(DiffOp::Remove(a[i].clone()));
            i += 1;
        } else {
            ops.push(DiffOp::Add(b[j].clone()));
            j += 1;
        }
    }
    ops.extend(a[i..].iter().cloned().map(DiffOp::Remove));
    ops.extend(b[j..].iter().cloned().map(DiffOp::Add));
    ops
}

/// The edit script grouped into hunks, each with [`DIFF_CONTEXT_LINES`] of
/// unchanged text either side of its changes.
fn diff_hunks(ops: &[DiffOp]) -> Vec<String> {
    let mut keep = vec![false; ops.len()];
    for (i, op) in ops.iter().enumerate() {
        if matches!(op, DiffOp::Keep(_)) {
            continue;
        }
        let lo = i.saturating_sub(DIFF_CONTEXT_LINES);
        let hi = (i + DIFF_CONTEXT_LINES).min(ops.len() - 1);
        keep[lo..=hi].iter_mut().for_each(|k| *k = true);
    }

    struct Hunk {
        old_start: usize,
        new_start: usize,
        old_count: usize,
        new_count: usize,
        body: String,
    }
    let render = |h: Hunk| {
        format!(
            "@@ -{},{} +{},{} @@\n{}",
            h.old_start, h.old_count, h.new_start, h.new_count, h.body
        )
    };

    let mut hunks = Vec::new();
    let mut open: Option<Hunk> = None;
    let (mut old_line, mut new_line) = (1, 1);
    for (op, kept) in ops.iter().zip(keep) {
        let (old, new) = match op {
            DiffOp::Keep(_) => (1, 1),
            DiffOp::Remove(_) => (1, 0),
            DiffOp::Add(_) => (0, 1),
        };
        if kept {
            let h = open.get_or_insert_with(|| Hunk {
                old_start: old_line,
                new_start: new_line,
                old_count: 0,
                new_count: 0,
                body: String::new(),
            });
            h.body.push(op.prefix());
            h.body.push_str(op.text());
            h.body.push('\n');
            h.old_count += old;
            h.new_count += new;
        } else if let Some(h) = open.take() {
            hunks.push(render(h));
        }
        old_line += old;
        new_line += new;
    }
    if let Some(h) = open {
        hunks.push(render(h));
    }
    hunks
}
