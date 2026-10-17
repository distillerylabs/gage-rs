//! The recovery key: a second, paper-held recipient registered at `init`
//! under a fixed label, and the reservation that keeps any device from
//! taking that label. See "Recovery key at `init`" in the design doc.
//!
//! RK1 is the library half of creation: the keypair, the reserved label,
//! and checking a pasted secret against a vault's recipients. RK2 adds the
//! key's one power inside gage, enrolling a replacement device and
//! retiring itself, and replacing it from an unlocked device: both one
//! atomic swap of the recipient list (`swap`).

#[cfg(test)]
mod create_tests;
mod key;
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod reservation_tests;
mod swap;
#[cfg(test)]
mod verify_tests;

pub use key::{RecoveryKey, new_recovery_key};
pub use swap::{RecoverResult, RecoverSpec, RotateResult};

use crate::entry::LockError;
use crate::exitcode::{Code, HasCode};
use crate::gitrepo;
use crate::recipient::RecipientError;

/// The label the recovery recipient is registered under. Fixed, so a
/// recovery key is recognisable in any vault's recipient listing, and
/// reserved (see [`check_device_label_free`]) so no device can take it.
pub const RECOVERY_DEVICE_LABEL: &str = "recovery-paper-key";

/// Why a recovery key could not be made or checked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecoveryError {
    /// The text is not an age secret key. Never contains the input, and the
    /// parser's own error is dropped rather than wrapped: either could echo
    /// part of a secret.
    #[error("that is not a recovery key: expected an AGE-SECRET-KEY-1... line")]
    MalformedRecoveryKey,
    /// A well-formed key that is not one of this vault's recipients.
    #[error("that key ({pubkey}) is not a recipient of {vault:?}")]
    NotARecipient { pubkey: String, vault: String },
    /// A key that parses, and may even be a recipient, but is not the one
    /// labelled [`RECOVERY_DEVICE_LABEL`]. The distinction is the feature's
    /// boundary: a device's key is wrapped and passphrase-protected and
    /// reaches gage through `unlock`. Accepting one here would make this a
    /// general "unlock from a pasted key" path, so a device key is refused
    /// even though it would decrypt everything.
    #[error("gage: that key is not this vault's recovery key: {}", not_recovery(.device, .pubkey, .vault))]
    NotRecoveryKey {
        /// The label the key is listed under, when it is a device's.
        device: Option<String>,
        pubkey: String,
        vault: String,
    },
    /// `replaces` named the recovery label. The used key is retired
    /// whether or not it is named, and naming it would read as if not
    /// naming it were a way to keep it.
    #[error(
        "gage: the recovery key is retired by this operation whether or not it is named; --replaces is for the device that was lost"
    )]
    ReplacesRecoveryKey,
    /// The key being retired, offered back by the swap that retires it:
    /// as the enrolled device's key (a bare, passphrase-less secret made a
    /// permanent device), or as its own replacement (a retirement that
    /// changes nothing). The swap removes the label before it checks the
    /// additions, so this is the one collision it cannot see by itself
    /// (Q-RECOVERY-ERRORS).
    #[error("gage: {pubkey} is the recovery key this operation is retiring, so it cannot also be {} — generate a new key", retired_as(.as_device))]
    RetiredKeyReused { pubkey: String, as_device: String },
    /// Rotate's backstop under the label reservation: the key this vault
    /// was unlocked with is the one registered under the recovery label,
    /// so rotating would lock out the device running it.
    #[error(
        "gage: the key {vault:?} was unlocked with is the one registered as \"recovery-paper-key\", so rotating would remove this device's own access; re-register this device under its own name first"
    )]
    WouldEvictActingDevice { vault: String },
    /// Every refusal and failure the recipient machinery shares, so a
    /// caller matching `RecipientExists` sees the same variant from every
    /// recipient verb.
    #[error(transparent)]
    Recipient(#[from] RecipientError),
    #[error(transparent)]
    Config(#[from] crate::vaultconfig::Error),
}

fn not_recovery(device: &Option<String>, pubkey: &str, vault: &str) -> String {
    match device {
        Some(device) => format!(
            "it is {device:?}'s key, and only the {RECOVERY_DEVICE_LABEL:?} key can enroll a device this way"
        ),
        None => format!("its public key {pubkey} is not a recipient of vault {vault:?}"),
    }
}

fn retired_as(device: &str) -> String {
    if device == RECOVERY_DEVICE_LABEL {
        "its own replacement".to_string()
    } else {
        format!("enrolled as {device:?}")
    }
}

// What `with_vault_write` needs of its error type, through the recipient
// machinery's own conversions.
impl From<LockError> for RecoveryError {
    fn from(err: LockError) -> RecoveryError {
        RecoveryError::Recipient(err.into())
    }
}

impl From<gitrepo::Error> for RecoveryError {
    fn from(err: gitrepo::Error) -> RecoveryError {
        RecoveryError::Recipient(err.into())
    }
}

impl HasCode for RecoveryError {
    fn code(&self) -> Code {
        match self {
            RecoveryError::MalformedRecoveryKey => Code::Usage,
            RecoveryError::NotARecipient { .. } | RecoveryError::NotRecoveryKey { .. } => {
                Code::LockedOrAuth
            }
            RecoveryError::ReplacesRecoveryKey => Code::Usage,
            RecoveryError::RetiredKeyReused { .. }
            | RecoveryError::WouldEvictActingDevice { .. } => Code::Conflict,
            RecoveryError::Recipient(e) => e.code(),
            RecoveryError::Config(e) => e.code(),
        }
    }
}

/// A device tried to name itself [`RECOVERY_DEVICE_LABEL`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "device name {device:?} is reserved for the vault's recovery key; choose another with --device"
)]
pub struct ReservedLabel {
    pub device: String,
}

impl HasCode for ReservedLabel {
    fn code(&self) -> Code {
        Code::Usage
    }
}

/// Refuses [`RECOVERY_DEVICE_LABEL`] as a device's name.
///
/// Deliberately not part of `devicename::valid`: the recovery recipient
/// itself is written under this label (by `create`'s extras, and by the
/// swap), and none of those writers go through the paths that call this.
pub fn check_device_label_free(device: &str) -> Result<(), ReservedLabel> {
    if device == RECOVERY_DEVICE_LABEL {
        return Err(ReservedLabel {
            device: device.to_string(),
        });
    }
    Ok(())
}
