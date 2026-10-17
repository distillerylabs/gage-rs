//! The recovery key: a second, paper-held recipient registered at `init`
//! under a fixed label, and the reservation that keeps any device from
//! taking that label. See "Recovery key at `init`" in the design doc.
//!
//! RK1 is the library half of creation: the keypair, the reserved label,
//! and checking a pasted secret against a vault's recipients.

#[cfg(test)]
mod create_tests;
mod key;
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod reservation_tests;
#[cfg(test)]
mod verify_tests;

pub use key::{RecoveryKey, new_recovery_key};

use crate::exitcode::{Code, HasCode};

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
    #[error(transparent)]
    Config(#[from] crate::vaultconfig::Error),
}

impl HasCode for RecoveryError {
    fn code(&self) -> Code {
        match self {
            RecoveryError::MalformedRecoveryKey => Code::Usage,
            RecoveryError::NotARecipient { .. } => Code::LockedOrAuth,
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
