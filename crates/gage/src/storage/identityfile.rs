//! This device's wrapped identity files: `$GAGE_DATA/identities/<vault-id>/
//! <device>.age`, each an age-keygen-format key file encrypted to a
//! passphrase alone, plus the plaintext marker naming the vault. See "Local
//! identity storage" in the design doc: the sole-recipient invariant (a
//! passphrase-only recipient) and keying by vault id (ADR-0006), with the marker.
//!
//! Nothing here touches a terminal: every passphrase arrives through a
//! [`Prompter`], and how many tries a human gets is the frontend's call.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::crypto::crypt;
use crate::crypto::identity::{KeyLine, SECRET_KEY_PREFIX};
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::{
    Kind, PromptError, Prompter, Purpose, UnlockRequest, UnlockResponse, Warning,
};
use crate::storage::atomicfile;
use crate::storage::config;
use crate::storage::paths;
use crate::storage::vaultconfig::VaultId;
use crate::storage::xdgpaths::Dirs;

/// The plaintext marker in each identities directory, naming the vault it
/// belongs to (ADR-0006). The directory is named by an opaque UUID, and the
/// design doc permits backing a wrapped identity file up by hand, so the
/// directory says what it is. Advisory only: nothing reads it to make a
/// decision, and deleting it changes nothing but the label.
pub const MARKER_FILE_NAME: &str = "vault-name.txt";

/// The extension every wrapped identity file carries, and what tells one
/// apart from the marker.
pub const IDENTITY_FILE_EXTENSION: &str = "age";

/// A runaway guard, not a retry policy. Retry policy is the frontend's,
/// expressed by its `Prompter` refusing to answer; this ceiling only exists
/// so a prompter that never stops (a test fake, a buggy script) fails
/// instead of spinning forever.
const MAX_UNLOCK_ATTEMPTS: u32 = 100;

/// Why this device couldn't open, create or remove an identity. Each
/// outcome a caller decides differently on is its own variant: a wrong
/// passphrase re-prompts (in the frontend), a corrupt file never does, and
/// a missing identity means "this device isn't a recipient yet".
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnlockError {
    /// No identity is registered for this device: no global config, no
    /// registration for the vault, no device or method recorded, or no
    /// file where the registration points.
    #[error("no local identity is registered for this device: {0}")]
    NoLocalIdentity(String),
    /// The registration is complete but no identity file is where it
    /// points: a device that hasn't joined the vault yet, most likely.
    #[error(
        "no local identity is registered for this device: expected a wrapped identity at {}",
        .0.display()
    )]
    NoIdentityFile(PathBuf),
    /// This device's locally recorded method is one this build can't
    /// perform.
    #[error(
        "this device unlocks {vault:?} with method {method:?}, which this build cannot perform"
    )]
    UnsupportedMethod { vault: String, method: String },
    /// A device name or vault id that would have become a path component
    /// failed validation (ADR-0010, ADR-0006).
    #[error(transparent)]
    UnsafePath(#[from] paths::Error),
    /// At least one answer was a wrong passphrase, and the prompter then
    /// stopped answering; its reason is the source.
    #[error("wrong passphrase: {0}")]
    WrongPassphrase(#[source] PromptError),
    /// The prompter gave no answer before any passphrase was tried.
    #[error(transparent)]
    Prompter(PromptError),
    /// An empty answer is a prompter bug or an empty stdin, never a real
    /// passphrase; deriving a key from it would be worse than failing.
    #[error("an empty passphrase is not accepted")]
    EmptyPassphrase,
    /// The file isn't an intact passphrase-wrapped identity: not an age
    /// file, wrapped to something other than a passphrase, or decrypting to
    /// something with no usable key line.
    #[error("identity file is corrupt: {0}")]
    CorruptIdentityFile(String),
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Write(#[from] atomicfile::Error),
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error("wrapping the new identity: {0}")]
    Wrap(crypt::Error),
}

impl HasKind for UnlockError {
    fn kind(&self) -> ErrorKind {
        use UnlockError::*;
        match self {
            NoLocalIdentity(_)
            | NoIdentityFile(_)
            | UnsupportedMethod { .. }
            | WrongPassphrase(_)
            | Prompter(_)
            | EmptyPassphrase => ErrorKind::LockedOrAuth,
            UnsafePath(e) => e.kind(),
            CorruptIdentityFile(_) => ErrorKind::Conflict,
            Io { .. } | Write(_) | Config(_) | Wrap(_) => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        use UnlockError::*;
        match self {
            NoIdentityFile(_) => Some(Remedy::EnrollOrRecover),
            UnsafePath(e) => e.remedy(),
            _ => None,
        }
    }
}

/// Get-or-create this device's identity for a vault, returning its public
/// key: the only part that ever leaves this machine.
///
/// If a wrapped file already sits at the path, it is opened (an ordinary
/// unlock exchange, wrong-passphrase retries included) and its existing key
/// returned, with a warning: the file is the only copy of a private key, so
/// regenerating over it would silently destroy access nothing could
/// restore. Otherwise a fresh X25519 keypair is generated, its private half
/// wrapped to a new passphrase asked for through `p` (purpose
/// [`Purpose::Create`]), and written: directory 0700, file 0600, plus the
/// marker naming `vault_name`.
///
/// `vault_id` keys the path; `vault_name` is only what a human is shown.
/// The two have different types, so they can't be passed the wrong way
/// round.
pub fn create(
    dirs: &Dirs,
    vault_id: &VaultId,
    vault_name: &str,
    device: &str,
    p: &dyn Prompter,
) -> Result<String, UnlockError> {
    let path = paths::identity_file_path(dirs, vault_id, device)?;
    if let Some(wrapped) = load(&path)? {
        let key = open(vault_name, device, &wrapped, p)?;
        p.warn(&Warning::from(format!(
            "reusing the existing local identity file for {vault_name:?} (device {device}) rather than generating a new one"
        )));
        return Ok(key.recipient().to_string());
    }

    let passphrase = request_passphrase(p, Purpose::Create, vault_name, device, 1)?;
    let key = age::x25519::Identity::generate();
    let recipient = key.to_public().to_string();
    let plaintext = format_identity_file(&key, &recipient);
    let wrapped = crypt::encrypt(&plaintext, &[crypt::Recipient::passphrase(&passphrase)])
        .map_err(UnlockError::Wrap)?;
    // The plaintext key leaves memory as soon as it is wrapped.
    drop(plaintext);

    let dir = path
        .parent()
        .expect("an identity file path has a directory");
    make_private_dir(dir)?;
    atomicfile::write(&path, &wrapped, 0o600)?;
    // Rewritten rather than written once: nothing reads it, so refreshing
    // a renamed vault's label is free.
    atomicfile::write(
        &dir.join(MARKER_FILE_NAME),
        format!("{vault_name}\n").as_bytes(),
        0o600,
    )?;
    Ok(recipient)
}

/// Deletes this device's wrapped identity file for a vault; success if
/// there was nothing to delete. The marker goes with the last identity, and
/// the directory with it once empty.
///
/// Narrow on purpose: deleting an identity a vault still lists loses access
/// to its ciphertext. Its callers (rolling back a failed `init`, and later
/// `vault remove` pruning a key the vault no longer lists) establish first
/// that nothing is lost; this does not check.
pub fn remove(dirs: &Dirs, vault_id: &VaultId, device: &str) -> Result<(), UnlockError> {
    let path = paths::identity_file_path(dirs, vault_id, device)?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(UnlockError::Io {
                action: "removing",
                path,
                source,
            });
        }
    }
    let dir = path
        .parent()
        .expect("an identity file path has a directory");
    // A marker would otherwise keep the directory from ever being empty,
    // turning the prune below into a no-op and leaving one labelled,
    // keyless directory behind per removed vault.
    if holds_no_identities(dir) {
        let _ = std::fs::remove_file(dir.join(MARKER_FILE_NAME));
    }
    // Succeeds only on an empty directory, which is the wanted semantics:
    // another device's identity keeps it. A leftover empty directory is not
    // worth failing a rollback over.
    let _ = std::fs::remove_dir(dir);
    Ok(())
}

/// A wrapped identity file held in memory while something replaces it, so
/// a failure part-way through can put back exactly what was there
/// (recovery depends on it). The file is usually unopenable, since a
/// forgotten passphrase is why anyone replaces one, but not always:
/// rebuilding a machine under its own name is legitimate with a good key on
/// disk.
///
/// Holding one decides nothing. Whether a file may be replaced at all is
/// the caller's policy.
pub struct IdentityBackup {
    path: std::path::PathBuf,
    /// Still wrapped, so not a secret in itself, but zeroed anyway: it is
    /// the only copy of a private key.
    wrapped: Zeroizing<Vec<u8>>,
    /// The marker as it was, if there was one.
    marker: Option<Vec<u8>>,
}

// Hand-written so the bytes never reach a log.
impl std::fmt::Debug for IdentityBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityBackup")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Reads this device's wrapped identity file for a vault into memory,
/// with the marker beside it; `None` if there is no file. Only reads.
pub fn backup(
    dirs: &Dirs,
    vault_id: &VaultId,
    device: &str,
) -> Result<Option<IdentityBackup>, UnlockError> {
    let path = paths::identity_file_path(dirs, vault_id, device)?;
    let Some(wrapped) = load(&path)? else {
        return Ok(None);
    };
    let marker_path = path
        .parent()
        .expect("an identity file path has a directory")
        .join(MARKER_FILE_NAME);
    let marker = load(&marker_path)?;
    Ok(Some(IdentityBackup {
        path,
        wrapped: Zeroizing::new(wrapped),
        marker,
    }))
}

impl IdentityBackup {
    /// Puts the file back as it was. The directory is remade first:
    /// [`remove`] prunes it, with the marker, when the file was the
    /// vault's last identity, and a write beside a missing directory would
    /// fail and lose the file this exists to keep.
    /// Takes `&self`, so a caller whose restore failed can still report
    /// where the bytes belonged.
    pub fn restore(&self) -> Result<(), UnlockError> {
        let dir = self
            .path
            .parent()
            .expect("an identity file path has a directory");
        make_private_dir(dir)?;
        atomicfile::write(&self.path, &self.wrapped, 0o600)?;
        if let Some(marker) = &self.marker {
            atomicfile::write(&dir.join(MARKER_FILE_NAME), marker, 0o600)?;
        }
        Ok(())
    }

    /// Where the file belongs.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether `dir` holds no wrapped identity files: "was that the last one".
/// Asked about identity files, not emptiness, because the marker is what
/// stops the directory being empty. An unreadable directory holds nothing
/// this can see, and the marker removal it gates is best-effort anyway.
fn holds_no_identities(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    !entries.flatten().any(|e| {
        let path = e.path();
        path.is_file()
            && path
                .extension()
                .is_some_and(|x| x == IDENTITY_FILE_EXTENSION)
    })
}

/// The wrapped file at `path`, or `None` if there is none.
pub(crate) fn load(path: &Path) -> Result<Option<Vec<u8>>, UnlockError> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(UnlockError::Io {
            action: "reading",
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Runs the passphrase exchange until `wrapped` opens, the prompter stops
/// answering, or a failure no passphrase can fix, then pulls the key line
/// out of the decrypted file.
///
/// Takes a vault name rather than a `Vault` because `create`'s reuse path
/// opens a file before any `Vault` exists.
pub(crate) fn open(
    vault: &str,
    device: &str,
    wrapped: &[u8],
    p: &dyn Prompter,
) -> Result<KeyLine, UnlockError> {
    // Whether any answer so far was a genuinely wrong passphrase, so that
    // when the prompter stops answering the error says why it was being
    // asked again rather than only that it gave up.
    let mut wrong_so_far = false;
    for attempt in 1..=MAX_UNLOCK_ATTEMPTS {
        let passphrase = match request_passphrase(p, Purpose::Unlock, vault, device, attempt) {
            Ok(passphrase) => passphrase,
            Err(UnlockError::Prompter(reason)) if wrong_so_far => {
                return Err(UnlockError::WrongPassphrase(reason));
            }
            Err(e) => return Err(e),
        };
        match crypt::decrypt_with(wrapped, &crypt::scrypt_identity(&passphrase)) {
            Ok(plaintext) => return key_line(&Zeroizing::new(plaintext)),
            Err(failure) if is_wrong_passphrase(&failure) => wrong_so_far = true,
            Err(failure) => {
                return Err(UnlockError::CorruptIdentityFile(failure.cause.to_string()));
            }
        }
    }
    // The runaway guard: a prompter that never stops answering.
    Err(UnlockError::WrongPassphrase(PromptError::TooManyAttempts))
}

/// Whether a failed decrypt means "the human typed the wrong thing" rather
/// than "this file is not what it should be" (ADR-0022).
///
/// Rust's age reports a wrong passphrase as the scrypt stanza's AEAD
/// failing (`DecryptionFailed`). That is only a passphrase problem when the
/// header is scrypt-only, as an identity file is by construction: a
/// file wrapped to anything else fails with `NoMatchingKeys` instead, and
/// re-prompting for it would loop forever. Every other failure (a header
/// that doesn't parse, a bad MAC, an excessive work factor, a damaged
/// payload) is a corrupt file. Accepted: a damaged scrypt stanza
/// body is indistinguishable from a wrong passphrase, because the AEAD
/// can't tell them apart.
fn is_wrong_passphrase(failure: &crypt::DecryptFailure) -> bool {
    failure.scrypt_only
        && matches!(
            failure.cause,
            crypt::Cause::Header(age::DecryptError::DecryptionFailed)
        )
}

/// The first `AGE-SECRET-KEY-1…` line of a decrypted identity file, copied
/// into its own pages and validated. Lines are split in place, so the only
/// copy made of the key is the one the caller will own.
fn key_line(plaintext: &[u8]) -> Result<KeyLine, UnlockError> {
    let line = plaintext
        .split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii)
        .find(|line| line.starts_with(SECRET_KEY_PREFIX.as_bytes()))
        .ok_or_else(|| {
            UnlockError::CorruptIdentityFile(format!(
                "it contains no {SECRET_KEY_PREFIX}\u{2026} line"
            ))
        })?;
    KeyLine::parse(line)
        .map_err(|e| UnlockError::CorruptIdentityFile(format!("its key line does not parse: {e}")))
}

/// One prompter exchange, with the answer's shape checked, so no caller has
/// to re-check that a passphrase response carries a passphrase.
fn request_passphrase(
    p: &dyn Prompter,
    purpose: Purpose,
    vault: &str,
    device: &str,
    attempt: u32,
) -> Result<Zeroizing<String>, UnlockError> {
    let req = UnlockRequest {
        kind: Kind::Passphrase,
        purpose,
        vault: vault.to_string(),
        device: device.to_string(),
        attempt,
    };
    // One variant per Kind, and this asked for a passphrase: the pattern is
    // irrefutable until a second Kind exists, at which point the compiler
    // demands the mismatch arm.
    let UnlockResponse::Passphrase(passphrase) = p.unlock(&req).map_err(UnlockError::Prompter)?;
    if passphrase.is_empty() {
        return Err(UnlockError::EmptyPassphrase);
    }
    Ok(passphrase)
}

/// age-keygen's own file format: two comments and the key, so the stock
/// age CLI can open a gage identity file directly. Built in a buffer sized
/// up front, so it never reallocates and leaves a stray copy of the key.
fn format_identity_file(key: &age::x25519::Identity, recipient: &str) -> Zeroizing<Vec<u8>> {
    use age::secrecy::ExposeSecret;
    let created = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    let secret = key.to_string();
    let parts: [&str; 7] = [
        "# created: ",
        &created,
        "\n# public key: ",
        recipient,
        "\n",
        secret.expose_secret(),
        "\n",
    ];
    let mut out = Zeroizing::new(Vec::with_capacity(parts.iter().map(|p| p.len()).sum()));
    for part in parts {
        out.extend_from_slice(part.as_bytes());
    }
    out
}

/// Creates `dir` (and parents) 0700, and tightens it to 0700 if it already
/// existed looser: an identities directory made before this rule, or under
/// a wide umask, is not trusted as it stands.
fn make_private_dir(dir: &Path) -> Result<(), UnlockError> {
    let fail = |source| UnlockError::Io {
        action: "creating",
        path: dir.to_path_buf(),
        source,
    };
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir).map_err(fail)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(fail)
    }
    #[cfg(not(unix))]
    {
        builder.create(dir).map_err(fail)
    }
}
