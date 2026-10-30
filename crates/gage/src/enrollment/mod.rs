//! Device enrollment, library side: the code, the seal, the filename
//! scheme, listing, opening, resolution and pruning, and the
//! joining device's `enroll`, and the approving device's `approve`
//! and `deny`, through the same primitives.
//!
//! Listing, resolution and pruning are built from filenames alone (no git
//! history, no unlock, no code), which is what ADR-0042
//! exists to buy. Opening is keyed by the code, never an identity, which is
//! what lets `approve` unlock after the human has said yes. See "Expiry,
//! revocation, and pruning" and "The load-bearing invariant: `pending/` is
//! inert".

mod approve;
mod code;
mod enroll;
mod name;
mod seal;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use uuid::Uuid;
use zeroize::Zeroizing;

pub use approve::{Approval, ApprovalOutcome, ApprovalResult};

use crate::entry::LockError;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::recovery::ReservedLabel;
use crate::storage::vaultconfig::{self, UnlockMethod, VaultId};
use crate::sync::remotesync::SyncError;
use crate::trust::recipient::RecipientError;
use crate::vault::Vault;
use crate::{
    crypto::agekey, crypto::crypt, storage::atomicfile, storage::config, storage::identityfile,
    storage::paths, sync::gitrepo,
};
#[cfg(test)]
pub(crate) use code::normalize_code;
use name::{expiry_live, parse_pending_file_name};
use seal::Attempt;
#[cfg(test)]
pub(crate) use seal::Payload;

/// How long a request lives when nobody says otherwise: long enough for a
/// request made in one timezone and approved the next morning in another,
/// short enough that blobs don't sit in the tree for weeks (ADR-0041).
pub const DEFAULT_ENROLLMENT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The ceiling on a request's lifetime, with two jobs: it bounds the TTL at
/// creation, and it bounds the expiry gage will honour on the way back in.
/// A claim further than this beyond now, in a filename or under the seal,
/// is treated as already expired: a legitimate request can't make one, and
/// without the clamp a hostile git writer could park a request forever.
pub const MAX_ENROLLMENT_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How many requests one code-trying run will attempt: a bound on work,
/// not on how many devices may enrol. The count comes from a directory any
/// git writer can fill, and each attempt is a KDF run (ADR-0040).
/// Far above anything legitimate, and recoverable in place by naming an id.
pub const MAX_ENROLLMENT_ATTEMPTS: usize = 32;

/// How large a file in `pending/` gage will read, and how large a payload
/// it will hold after decrypting one: about fifty times the largest request
/// gage writes. Applied to both because age's plaintext isn't bounded by
/// its ciphertext.
pub const MAX_PENDING_REQUEST_BYTES: usize = 16 * 1024;

/// The vault-relative directory sealed requests live in. Created lazily by
/// the first enroll; its absence is the normal state (git tracks no empty
/// directories), including in every vault that predates enrollment.
pub const PENDING_DIR: &str = ".gage/pending";

/// What a joining device produced. `code` is the generated out-of-band
/// secret: the one field a frontend must display and the library must
/// never persist.
///
/// `published` is false in exactly one case, which `enroll` owns: this
/// device's key is already a recipient, so there was nothing to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentRequest {
    /// `None` when nothing was published, so there is no request.
    pub id: Option<String>,
    pub device: String,
    pub pubkey: String,
    pub method: UnlockMethod,
    pub created: Timestamp,
    pub expires: Timestamp,
    pub code: Zeroizing<String>,
    pub published: bool,
}

/// One unopened request, built entirely from its filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: String,
    pub path: PathBuf,
    /// Unauthenticated: parsed from the filename's epoch. Display and
    /// pruning only; approval reads the sealed copy.
    pub expires: Timestamp,
}

/// A request a code has opened. Every field is authenticated and validated
/// (authenticated says unaltered, not well-formed), all before one of
/// these exists.
///
/// There is deliberately no vault id: the sealed one is compared against
/// this vault's and refused on a mismatch, so every `OpenedRequest` is for
/// this vault, and a field would be a constant the caller could only
/// re-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedRequest {
    pub id: String,
    pub device: String,
    pub pubkey: String,
    pub method: UnlockMethod,
    pub created: Timestamp,
    pub expires: Timestamp,
}

/// The requests an id matched when it matched more than one: data a
/// frontend renders, never a guess, because deny deletes
/// (ADR-0043).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestCandidates {
    pub id: String,
    pub matches: Vec<PendingRequest>,
}

/// Every way enrollment fails, at the exit code the TDD's "Library
/// surface" table gives it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnrollmentError {
    /// No request opened: the code is well-formed and opens nothing, or it
    /// failed length or alphabet before any decryption. Deliberately one
    /// variant: a frontend's retry loop keys on it, and a typo is the case
    /// that loop exists for. Validation buys cost, not a different answer.
    #[error("no pending request opened with that code")]
    CodeWrong,
    /// The authenticated expiry has passed, or is further beyond now than
    /// gage would write: the same claim from the other direction.
    #[error("this enrollment request has expired (it expired at {expires})")]
    Expired { expires: Timestamp },
    /// Expired before it was created: a wrong clock, not a stale request,
    /// and the fixes share nothing.
    #[error(
        "this enrollment request expired before it was created (sealed at {created}, expiring at {expires}), so the requesting device's clock is wrong"
    )]
    ClockSkew {
        created: Timestamp,
        expires: Timestamp,
    },
    /// The sealed id disagrees with the filename's: a blob renamed onto
    /// another slot or replayed into a fresh one.
    #[error(
        "this request's sealed id does not match its filename: sealed as {sealed}, filed as {filed}"
    )]
    IdMismatch { sealed: String, filed: String },
    /// The code opened a payload gage could not have written. Never
    /// `CodeWrong`: the code worked, and a retry loop would re-prompt for a
    /// code nothing can satisfy.
    #[error("this enrollment request's sealed contents are malformed: {reason}")]
    MalformedRequest { reason: String },
    /// A well-formed request sealed for another vault. Neither `CodeWrong`
    /// (the code worked) nor `MalformedRequest` (gage wrote it). `vault`
    /// names the vault it is for.
    #[error("this enrollment request is for {vault}, not {this:?}")]
    WrongVault { vault: String, this: String },
    /// A broad run over more live requests than one run tries, refused
    /// before any decryption.
    #[error(
        "too many pending enrollment requests to try a code against them all ({pending} pending, limit {limit}); approve or deny by id"
    )]
    TooManyPending { pending: usize, limit: usize },
    #[error("no pending enrollment request with id {id:?}")]
    NoSuchRequest { id: String },
    #[error("{:?} matches {} pending enrollment requests", .0.id, .0.matches.len())]
    Ambiguous(RequestCandidates),
    #[error(
        "an enrollment request must live more than zero and at most {} hours, not {ttl:?}",
        MAX_ENROLLMENT_TTL.as_secs() / 3600
    )]
    InvalidTtl { ttl: Duration },
    #[error("device name {device:?} is invalid")]
    InvalidDevice { device: String },
    #[error(transparent)]
    ReservedDeviceLabel(#[from] ReservedLabel),
    #[error(transparent)]
    InvalidPubkey(agekey::Error),
    #[error(
        "unknown method {method:?}; accepted: {:?}",
        UnlockMethod::SUPPORTED.iter().map(UnlockMethod::as_str).collect::<Vec<_>>()
    )]
    UnknownMethod { method: String },
    #[error("generating an enrollment code: {0}")]
    Random(rand_core::Error),
    #[error("generated an enrollment code its own validation rejects")]
    CodeMismatch,
    #[error("encoding an enrollment request: {0}")]
    Encode(String),
    #[error("sealing an enrollment request: {0}")]
    Seal(crypt::Error),
    #[error("{}: {source}", .path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Enrollment publishes a request through the remote, so a vault with
    /// none cannot be joined this way.
    #[error("{vault:?} has no remote, and enrollment publishes its request through one")]
    NoRemote { vault: String },
    /// The first thing enroll does with the network failed, before any
    /// identity was created or anything was asked.
    #[error("{0}")]
    RemoteUnreachable(SyncError),
    /// The remote has commits this vault doesn't and this vault has
    /// commits the remote doesn't. A device that cannot decrypt anything
    /// cannot resolve that, so the advice is for the other side.
    #[error(
        "{vault:?} has diverged from its remote and nothing was changed; this device cannot reconcile it"
    )]
    Diverged { vault: String },
    /// A recipient already carries this device name with a different key.
    #[error("{vault:?} already has a recipient named {device:?} with a different key")]
    DeviceNameTaken { device: String, vault: String },
    /// A fetch that failed for a reason other than being unreachable.
    #[error("{0}")]
    Sync(SyncError),
    /// The request was written and committed locally but not published. The
    /// identity and the commit are kept; running enroll again is the way
    /// forward.
    #[error("{}", push_message(.0))]
    Push(SyncError),
    /// A request's sealed device name now labels a different recipient.
    /// Recoverable by the approver alone, since the code authenticated the
    /// key and not the label. Deliberately not `RecipientExists`, which is
    /// the race under the lock and `recipient add`'s vocabulary; this is
    /// the ordinary case, found before anything is asked, and it names the
    /// flag that fixes it.
    #[error(
        "another recipient of this vault already uses this request's device name: {device:?} already labels the key {pubkey} in vault {vault:?}"
    )]
    NameTaken {
        device: String,
        pubkey: String,
        vault: String,
    },
    #[error("no enrollment requests to approve")]
    EmptyBatch,
    /// A request that was pending when it was opened and is not now, or no
    /// longer opens to what was shown. Its own variant rather than
    /// `NoSuchRequest`, which means the id was mistyped: here it most
    /// likely means another device resolved it.
    #[error(
        "no pending enrollment request with id {id:?}: it was still pending when it was opened and is not now — another device most likely approved or denied it in the meantime. Nothing was changed here: if it was approved elsewhere the device already has access, and if it was denied it should not be given any"
    )]
    NoLongerPending { id: String },
    /// Approval's fetch found the vault diverged from its remote. Unlike
    /// enroll's `Diverged`, the advice is the ordinary `gage sync`: an
    /// approver can decrypt, so that is exactly what resolves it.
    #[error("{summary} — nothing was approved, and the request is still pending")]
    ApprovalDiverged { summary: String, remedy: Remedy },
    /// The recipient write's own refusals and failures, unchanged:
    /// `CannotGrantFullAccess`, `RecipientExists`, the trust question.
    #[error(transparent)]
    Recipient(#[from] RecipientError),
    #[error(transparent)]
    Identity(#[from] identityfile::UnlockError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    #[error(transparent)]
    VaultConfig(#[from] vaultconfig::Error),
    #[error("writing the request: {0}")]
    Write(#[from] atomicfile::Error),
}

/// What a failed publish says: the failure, then what survived it. What to
/// run is [`EnrollmentError::remedy`]'s.
fn push_message(err: &SyncError) -> String {
    format!(
        "{err}; the request was not published, but this device's identity and the local commit are kept"
    )
}

impl HasKind for EnrollmentError {
    fn kind(&self) -> ErrorKind {
        use EnrollmentError::*;
        match self {
            CodeWrong => ErrorKind::LockedOrAuth,
            Expired { .. }
            | ClockSkew { .. }
            | IdMismatch { .. }
            | MalformedRequest { .. }
            | WrongVault { .. }
            | TooManyPending { .. } => ErrorKind::Conflict,
            NoSuchRequest { .. } => ErrorKind::NotFound,
            Ambiguous(_) => ErrorKind::Ambiguous,
            InvalidTtl { .. } | InvalidDevice { .. } | InvalidPubkey(_) | UnknownMethod { .. } => {
                ErrorKind::Usage
            }
            Random(_)
            | CodeMismatch
            | Encode(_)
            | Seal(_)
            | Io { .. }
            | VaultConfig(_)
            | Write(_) => ErrorKind::Internal,
            NoRemote { .. } => ErrorKind::Usage,
            Diverged { .. }
            | DeviceNameTaken { .. }
            | NameTaken { .. }
            | ApprovalDiverged { .. } => ErrorKind::Conflict,
            EmptyBatch => ErrorKind::Usage,
            ReservedDeviceLabel(e) => e.kind(),
            NoLongerPending { .. } => ErrorKind::NotFound,
            Recipient(e) => e.kind(),
            RemoteUnreachable(e) | Sync(e) | Push(e) => e.kind(),
            Identity(e) => e.kind(),
            Lock(e) => e.kind(),
            Git(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        use EnrollmentError::*;
        match self {
            NoRemote { .. } => Some(Remedy::SetRemote),
            Diverged { vault } => Some(Remedy::Reclone {
                vault: vault.clone(),
            }),
            DeviceNameTaken { .. } => Some(Remedy::ChooseDevice),
            NameTaken { .. } => Some(Remedy::ApproveWithDevice),
            ApprovalDiverged { remedy, .. } => Some(remedy.clone()),
            // Enrolling again is the way forward, after storing a token
            // when the remote refused the one it had.
            Push(e) => Some(Remedy::EnrollAgain {
                auth_refused: e.remote_kind() == Some(crate::sync::syncerr::Kind::Auth),
                host: match e.remedy() {
                    Some(Remedy::AuthLogin { host }) => Some(host),
                    _ => None,
                },
            }),
            ReservedDeviceLabel(e) => e.remedy(),
            Recipient(e) => e.remedy(),
            RemoteUnreachable(e) | Sync(e) => e.remedy(),
            Identity(e) => e.remedy(),
            Lock(e) => e.remedy(),
            Git(e) => e.remedy(),
            _ => None,
        }
    }
}

impl Vault {
    /// `<vault>/.gage/pending`.
    pub(crate) fn pending_dir(&self) -> PathBuf {
        self.path.join(PENDING_DIR)
    }

    /// The live requests in `pending/`, by id: one whose filename epoch has
    /// passed, or lies beyond the ceiling, is filtered out.
    ///
    /// Filtered, not deleted: listing is a read, takes no lock and writes
    /// nothing, so removal is pruning's job and rides the next write. The
    /// split matters twice: a listing never shows an expired request, and
    /// the 32-request bound counts what this returns, so neglect never
    /// looks like an attack whether or not a write has pruned yet.
    ///
    /// No `pending/` is zero requests. Strays and oversized files are
    /// skipped. Nothing here reads git history, needs an unlock, or opens a
    /// file.
    pub fn pending_enrollments(&self) -> Result<Vec<PendingRequest>, EnrollmentError> {
        self.pending_enrollments_at(Timestamp::now())
    }

    /// [`Vault::pending_enrollments`] as of `now`, so a test can list a
    /// committed request at an instant inside its life.
    pub(crate) fn pending_enrollments_at(
        &self,
        now: Timestamp,
    ) -> Result<Vec<PendingRequest>, EnrollmentError> {
        let mut out = Vec::new();
        for (name, path, size) in self.pending_files()? {
            let Some((id, expires)) = parse_pending_file_name(&name) else {
                continue;
            };
            // The size bound, from the directory entry: an oversized file
            // is skipped before any read, KDF run or allocation, and left
            // in place, since it is by definition not something gage wrote.
            if size > MAX_PENDING_REQUEST_BYTES as u64 || !expiry_live(expires, now) {
                continue;
            }
            out.push(PendingRequest { id, path, expires });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// The regular files in `pending/` (name, path, size); none when the
    /// directory doesn't exist. Subdirectories, symlinks (the entry's own
    /// metadata, not followed), and names that aren't UTF-8 are skipped:
    /// none is a request gage wrote.
    fn pending_files(&self) -> Result<Vec<(String, PathBuf, u64)>, EnrollmentError> {
        let dir = self.pending_dir();
        let io = |source| EnrollmentError::Io {
            path: dir.clone(),
            source,
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io)?;
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if meta.is_file() {
                out.push((name, entry.path(), meta.len()));
            }
        }
        Ok(out)
    }

    /// Tries each code against each request it is handed, and returns the
    /// ones that opened.
    ///
    /// The scope is a parameter, not "every pending request", because that
    /// makes `TooManyPending`'s way out work by construction: a broad run
    /// passes [`Vault::pending_enrollments`] and is refused past the bound;
    /// an id-scoped run passes the one request
    /// [`Vault::resolve_enrollment`] returned, which never exceeds it.
    /// Reading the directory in here would quietly reintroduce the
    /// unbounded path.
    ///
    /// It takes no identity: opening is keyed by the code, which is what
    /// makes the approving side's late unlock possible.
    ///
    /// One bad file never takes the run down. A malformed, oversized,
    /// expired or wrong-vault request is skipped in favour of anything that
    /// did open, and its error is returned only when nothing did; otherwise
    /// any git writer would hold a denial of service on approval.
    pub fn open_enrollment(
        &self,
        requests: &[PendingRequest],
        codes: &[Zeroizing<String>],
    ) -> Result<Vec<OpenedRequest>, EnrollmentError> {
        self.open_enrollment_at(requests, codes, Timestamp::now())
    }

    /// [`Vault::open_enrollment`] as of `now`, so a test can open a
    /// committed request at an instant inside its life.
    pub(crate) fn open_enrollment_at(
        &self,
        requests: &[PendingRequest],
        codes: &[Zeroizing<String>],
        now: Timestamp,
    ) -> Result<Vec<OpenedRequest>, EnrollmentError> {
        // Before any decryption, against the scope handed in.
        if requests.len() > MAX_ENROLLMENT_ATTEMPTS {
            return Err(EnrollmentError::TooManyPending {
                pending: requests.len(),
                limit: MAX_ENROLLMENT_ATTEMPTS,
            });
        }
        // Also before any decryption: a code that can't be one of gage's is
        // dropped here, and if none survives the answer is the same
        // CodeWrong a well-formed code that opens nothing gets.
        let valid: Vec<Zeroizing<String>> = codes
            .iter()
            .filter_map(|c| code::normalize_code(c))
            .collect();
        if valid.is_empty() || requests.is_empty() {
            return Err(EnrollmentError::CodeWrong);
        }

        let mut opened = Vec::new();
        let mut refused = None;
        for request in requests {
            let Some(sealed) = read_pending_file(&request.path) else {
                continue;
            };
            for code in &valid {
                match self.open_sealed(request, &sealed, code, now) {
                    Attempt::NotOpened => continue,
                    Attempt::Opened(o) => opened.push(o),
                    Attempt::Refused(e) => {
                        refused.get_or_insert(e);
                    }
                }
                break;
            }
        }
        if !opened.is_empty() {
            return Ok(opened);
        }
        Err(refused.unwrap_or(EnrollmentError::CodeWrong))
    }

    /// A typed id as one live request: exact UUID, then substring, then
    /// every match as candidates, over the UUID half of the filename only.
    ///
    /// The epoch is parsed off before matching. Ten digits of epoch are all
    /// legal hex, so a short id could match one request's expiry and
    /// another's id, and deny deletes: searching whole filenames would
    /// delete a request the human didn't name. Approve and deny share this,
    /// so there is one resolution rule.
    pub fn resolve_enrollment(&self, id: &str) -> Result<PendingRequest, EnrollmentError> {
        let pending = self.pending_enrollments()?;

        // Exact, accepting any spelling the uuid crate parses on the typed
        // side, so a braced or uppercased id resolves exactly rather than
        // falling through to a substring search it wouldn't match.
        let exact: Vec<&PendingRequest> = match Uuid::try_parse(id.trim()) {
            Ok(typed) => {
                let typed = typed.hyphenated().to_string();
                pending.iter().filter(|r| r.id == typed).collect()
            }
            Err(_) => Vec::new(),
        };
        // Substring, hyphens and case ignored on both sides: a superset of
        // prefix, so the short ids a listing shows keep working.
        let needle = id.trim().replace('-', "").to_lowercase();
        let substring = || -> Vec<&PendingRequest> {
            if needle.is_empty() {
                return Vec::new();
            }
            pending
                .iter()
                .filter(|r| r.id.replace('-', "").contains(&needle))
                .collect()
        };
        let matches = if exact.is_empty() { substring() } else { exact };
        match matches[..] {
            [] => Err(EnrollmentError::NoSuchRequest { id: id.to_string() }),
            [one] => Ok(one.clone()),
            _ => Err(EnrollmentError::Ambiguous(RequestCandidates {
                id: id.to_string(),
                matches: matches.into_iter().cloned().collect(),
            })),
        }
    }

    /// Deletes the requests gage will never honour again (expired by
    /// filename, or claiming an expiry beyond the ceiling) and returns the
    /// names removed. The filename's epoch and nothing else: no history, no
    /// unlock, no code. It rides the recipient writes, approve and deny,
    /// under their lock and in their commit, rather than taking the lock
    /// for housekeeping alone.
    ///
    /// A name that doesn't parse is left strictly alone, as is an oversized
    /// file with a parseable live name: neither is something gage wrote,
    /// and refusing to delete what it can't account for is the conservative
    /// direction. A filename lying the other way (claiming expiry on a live
    /// seal) prunes a live request: a denial of service, but not a new one,
    /// since anyone who can rename the file can delete it.
    pub(crate) fn prune_pending_enrollments(&self) -> Result<Vec<String>, EnrollmentError> {
        let now = Timestamp::now();
        let mut removed = Vec::new();
        for (name, path, _) in self.pending_files()? {
            let Some((_, expires)) = parse_pending_file_name(&name) else {
                continue;
            };
            if expiry_live(expires, now) {
                continue;
            }
            std::fs::remove_file(&path).map_err(|source| EnrollmentError::Io { path, source })?;
            removed.push(name);
        }
        removed.sort();
        Ok(removed)
    }

    /// The vault a sealed id names, the way the wrong-vault refusal needs
    /// it: by the name this device registered it under if there is one, by
    /// the bare id otherwise. A missing or unreadable registry costs the
    /// name and nothing else; failing to build a message mustn't turn a
    /// refusal into a different error.
    fn describe_vault_id(&self, id: &VaultId) -> String {
        config::load(&paths::global_config_file(self.dirs()))
            .ok()
            .and_then(|global| {
                global
                    .vaults
                    .iter()
                    .find(|(_, entry)| entry.id.as_ref() == Some(id))
                    .map(|(name, _)| format!("{name:?} (id {id})"))
            })
            .unwrap_or_else(|| format!("the vault with id {id}"))
    }
}

/// Seals and files a request whose fields are exactly as given and never
/// validated (what a git writer or a code-holder can produce), under a
/// fresh id and code, filed as expiring at `filed_expires`. Returns the id
/// and the code. Test support only: no shipped path writes a request it
/// hasn't validated.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn file_unchecked_request(
    vault: &Vault,
    device: &str,
    pubkey: &str,
    created: Timestamp,
    expires: Timestamp,
    filed_expires: Timestamp,
) -> (String, String) {
    let id = Uuid::new_v4().to_string();
    let typed = code::generate_code().expect("generating a code");
    let normalized = code::normalize_code(&typed).expect("generated codes normalize");
    let payload = seal::Payload {
        request_id: id.clone(),
        vault_id: vault.id.to_string(),
        device: device.to_string(),
        pubkey: pubkey.to_string(),
        method: UnlockMethod::Passphrase,
        created: created.to_string(),
        expires: expires.to_string(),
    };
    let sealed = seal::seal_payload(&payload, &normalized).expect("sealing");
    let dir = vault.pending_dir();
    std::fs::create_dir_all(&dir).expect("creating pending/");
    std::fs::write(
        dir.join(name::pending_file_name(&id, filed_expires)),
        sealed,
    )
    .expect("filing the request");
    (id, typed.to_string())
}

/// One sealed request's bytes, or `None` for a file that can't be read or
/// is larger than gage writes, checked before it is held in memory.
///
/// The bound is applied here as well as at listing because the open path
/// is handed a scope rather than reading the directory: a caller that
/// built a [`PendingRequest`] some other way must not route around a check
/// the listing performs. One byte past the limit, so a file exactly on it
/// is accepted and the first byte over is seen without reading the rest.
///
/// Only a regular file, not followed through a link, for the same reason:
/// a git writer can commit a symlink, and one pointing at a FIFO would
/// block the open forever (a hang a bare listing wouldn't rule out).
fn read_pending_file(path: &Path) -> Option<Vec<u8>> {
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut sealed = Vec::new();
    file.take(MAX_PENDING_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut sealed)
        .ok()?;
    (sealed.len() <= MAX_PENDING_REQUEST_BYTES).then_some(sealed)
}

#[cfg(test)]
mod approve_atomic_tests;
#[cfg(test)]
mod approve_fetch_tests;
#[cfg(test)]
mod approve_tests;
#[cfg(test)]
mod code_tests;
#[cfg(test)]
mod deny_tests;
#[cfg(test)]
mod enroll_tests;
#[cfg(test)]
mod expiry_tests;
#[cfg(test)]
mod fixtures_tests;
#[cfg(test)]
mod inert_tests;
#[cfg(test)]
mod listing_tests;
#[cfg(test)]
mod name_tests;
#[cfg(test)]
mod payload_tests;
#[cfg(test)]
mod reserved_tests;
#[cfg(test)]
mod seal_tests;
