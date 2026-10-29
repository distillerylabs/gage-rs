//! `identity add` and `identity list`: this device's own identities for a
//! vault, kept apart from `recipient`, which is the vault's list of who may
//! decrypt it ("`identity` vs `recipient` stay separate"). Adding an
//! identity grants nothing: the printed key still has to be added as a
//! recipient from a device that already has access.

use std::path::PathBuf;

use crate::config;
use crate::devicename;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::identityfile::{self, IDENTITY_FILE_EXTENSION, UnlockError};
use crate::paths;
use crate::prompter::Prompter;
use crate::recovery::{ReservedLabel, check_device_label_free};
use crate::vault::Vault;
use crate::vaultconfig;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityAddError {
    #[error("device name {0:?} is invalid")]
    InvalidDeviceName(String),
    #[error(transparent)]
    ReservedDeviceLabel(#[from] ReservedLabel),
    /// The vault already lists a recipient under this label, so a second
    /// device answering to it would be indistinguishable from the first in
    /// every recipient listing.
    #[error("device name {device:?} is already listed as a recipient of {vault:?}")]
    DeviceNameTaken { device: String, vault: String },
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Identity(#[from] UnlockError),
}

impl HasKind for IdentityAddError {
    fn kind(&self) -> ErrorKind {
        match self {
            IdentityAddError::InvalidDeviceName(_) => ErrorKind::Usage,
            IdentityAddError::ReservedDeviceLabel(e) => e.kind(),
            IdentityAddError::DeviceNameTaken { .. } => ErrorKind::Conflict,
            IdentityAddError::Config(e) => e.kind(),
            IdentityAddError::Identity(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            IdentityAddError::DeviceNameTaken { .. } => Some(Remedy::ChooseDevice),
            IdentityAddError::ReservedDeviceLabel(e) => e.remedy(),
            IdentityAddError::Config(e) => e.remedy(),
            IdentityAddError::Identity(e) => e.remedy(),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ListIdentitiesError {
    #[error("reading {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl HasKind for ListIdentitiesError {
    fn kind(&self) -> ErrorKind {
        match self {
            ListIdentitiesError::Io { .. } => ErrorKind::Internal,
        }
    }
}

/// One wrapped identity file this machine holds for a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalIdentity {
    pub device: String,
    pub path: PathBuf,
    /// Whether this is the identity the vault's registration on this
    /// machine currently uses.
    pub current: bool,
}

impl Vault {
    /// Gets or creates this device's identity for the vault and returns its
    /// public key. A name another recipient already carries is refused
    /// before anything is written.
    ///
    /// Recording the device, method and key in global config is the
    /// caller's: this never writes it, as with `init`.
    pub fn add_identity(
        &self,
        device: &str,
        prompter: &dyn Prompter,
    ) -> Result<String, IdentityAddError> {
        if !devicename::valid(device) {
            return Err(IdentityAddError::InvalidDeviceName(device.to_string()));
        }
        check_device_label_free(device)?;
        let file = vaultconfig::load(&self.config_path())?;
        if file.recipients.iter().any(|r| r.device == device) {
            return Err(IdentityAddError::DeviceNameTaken {
                device: device.to_string(),
                vault: self.name.clone(),
            });
        }
        Ok(identityfile::create(
            self.dirs(),
            &self.id,
            &self.name,
            device,
            prompter,
        )?)
    }

    /// The identity files this machine holds for the vault, by device name.
    /// Subdirectories, the marker, and files whose names aren't valid
    /// device names are not identities and are skipped.
    pub fn list_identities(&self) -> Result<Vec<LocalIdentity>, ListIdentitiesError> {
        let dir = paths::identities_dir(self.dirs(), &self.id);
        let io = |source| ListIdentitiesError::Io {
            path: dir.clone(),
            source,
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(e)),
        };
        let current = self.registered_device();
        let mut found = Vec::new();
        for entry in entries {
            let path = entry.map_err(io)?.path();
            if !path.is_file()
                || path
                    .extension()
                    .is_none_or(|x| x != IDENTITY_FILE_EXTENSION)
            {
                continue;
            }
            let Some(device) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| devicename::valid(s))
            else {
                continue;
            };
            found.push(LocalIdentity {
                device: device.to_string(),
                current: current.as_deref() == Some(device),
                path: path.clone(),
            });
        }
        found.sort_by(|a, b| a.device.cmp(&b.device));
        Ok(found)
    }

    /// The device name this machine's registration of the vault records, if
    /// there is one. A listing is informational, so a missing or unreadable
    /// global config just means nothing is marked current.
    fn registered_device(&self) -> Option<String> {
        let global = config::load(&paths::global_config_file(self.dirs())).ok()?;
        let device = &global.vaults.get(&self.name)?.device;
        (!device.is_empty()).then(|| device.clone())
    }
}
