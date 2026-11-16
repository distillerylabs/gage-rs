//! The joining device, after its request: has it been approved yet? Asked
//! of the recipient list, so it needs no passphrase (ADR-0081).

use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::Prompter;
use crate::storage::{config, paths, vaultconfig};
use crate::sync::remotesync::SyncError;
use crate::vault::Vault;

/// Why [`Vault::check_approved`] has no answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalCheckError {
    /// This device's registration records no key of its own for the vault:
    /// it has not asked to join from here, so there is nothing to wait for.
    #[error("this device has not asked to join {vault:?}")]
    NotEnrolled { vault: String },
    /// The pull failed: an unreachable or refusing remote among them.
    #[error(transparent)]
    Sync(#[from] SyncError),
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error(transparent)]
    Registry(#[from] config::RegistryError),
    #[error(transparent)]
    VaultConfig(#[from] vaultconfig::Error),
}

impl HasKind for ApprovalCheckError {
    fn kind(&self) -> ErrorKind {
        match self {
            ApprovalCheckError::NotEnrolled { .. } => ErrorKind::Usage,
            ApprovalCheckError::Sync(e) => e.kind(),
            ApprovalCheckError::Config(e) => e.kind(),
            ApprovalCheckError::Registry(e) => e.kind(),
            ApprovalCheckError::VaultConfig(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            ApprovalCheckError::NotEnrolled { .. } => Some(Remedy::EnrollOrRecover),
            ApprovalCheckError::Sync(e) => e.remedy(),
            ApprovalCheckError::Config(e) => e.remedy(),
            ApprovalCheckError::Registry(e) => e.remedy(),
            ApprovalCheckError::VaultConfig(e) => e.remedy(),
        }
    }
}

impl Vault {
    /// Pulls, then says whether the key this device's registration records
    /// as its own is on the committed recipient list: whether it has been
    /// approved. No identity is unlocked and nothing is asked; `prompter`
    /// only hears the pull's recipient-change warning, as with
    /// [`Vault::pull_checked`].
    ///
    /// The list is a claim, not proof: the first read is what shows this
    /// device can decrypt. A pull that finds a divergence still answers,
    /// from the local branch (ADR-0081).
    pub fn check_approved(&self, prompter: &dyn Prompter) -> Result<bool, ApprovalCheckError> {
        // Before the pull, so a device with nothing to wait for is told so
        // without a trip to the remote.
        let own = self.own_recorded_key()?;
        self.pull_checked(prompter)?;
        let file = vaultconfig::load(&self.config_path())?;
        Ok(file.recipients.iter().any(|r| r.pubkey == own))
    }

    /// The key the registration records as this device's own. Unlike
    /// [`Vault::recorded_registration`], an unreadable or missing
    /// registration is an error here: it is the whole answer.
    fn own_recorded_key(&self) -> Result<String, ApprovalCheckError> {
        let global = config::load(&paths::global_config_file(self.dirs()))?;
        global
            .entry(&self.name)?
            .pubkey
            .clone()
            .ok_or_else(|| ApprovalCheckError::NotEnrolled {
                vault: self.name.clone(),
            })
    }
}
