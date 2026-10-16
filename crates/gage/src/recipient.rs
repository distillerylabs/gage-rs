//! The read side of `recipient`: who the vault lists, and whether its two
//! copies of that list agree. Neither needs an identity: both files are
//! plaintext and committed, so anyone with the vault can inspect them.

use crate::exitcode::{Code, HasCode};
use crate::recipients;
use crate::vault::Vault;
use crate::vaultconfig;

/// Where `.gage/config.toml` and `.age-recipients` differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientVerification {
    pub in_sync: bool,
    /// Keys `.age-recipients` carries that no `[[recipients]]` lists:
    /// entries are encrypted to someone the config doesn't name.
    pub only_in_recipients_file: Vec<String>,
    /// Keys a `[[recipients]]` lists that `.age-recipients` lacks: someone
    /// the config names who can't decrypt new entries.
    pub only_in_config: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VerifyError {
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Recipients(#[from] recipients::Error),
}

impl HasCode for VerifyError {
    fn code(&self) -> Code {
        match self {
            VerifyError::Config(e) => e.code(),
            VerifyError::Recipients(e) => e.code(),
        }
    }
}

impl Vault {
    /// Every recipient, with its device label, exactly as `.gage/config.toml`
    /// lists them.
    pub fn recipients(&self) -> Result<Vec<vaultconfig::Recipient>, vaultconfig::Error> {
        Ok(vaultconfig::read(&self.config_path())?.recipients)
    }

    /// Compares the keys in `.age-recipients` (what entries are encrypted
    /// to) with those in `.gage/config.toml` (what the vault says it
    /// grants). Order and repeats don't matter: a key is either on both
    /// sides or it isn't.
    ///
    /// Takes no identity and no prompter: it reads two plaintext files.
    pub fn verify_recipients(&self) -> Result<RecipientVerification, VerifyError> {
        let config = self.recipients()?;
        let file = recipients::read(&self.path.join(".age-recipients"))?;
        let only_in_recipients_file = missing_from(&file, config.iter().map(|r| &r.pubkey));
        let only_in_config = missing_from(config.iter().map(|r| &r.pubkey), &file);
        Ok(RecipientVerification {
            in_sync: only_in_recipients_file.is_empty() && only_in_config.is_empty(),
            only_in_recipients_file,
            only_in_config,
        })
    }
}

/// The keys of `ours` that `theirs` lacks, in first-seen order, once each.
fn missing_from<'a, 'b>(
    ours: impl IntoIterator<Item = &'a String>,
    theirs: impl IntoIterator<Item = &'b String>,
) -> Vec<String> {
    let theirs: std::collections::HashSet<&String> = theirs.into_iter().collect();
    let mut seen = std::collections::HashSet::new();
    ours.into_iter()
        .filter(|k| !theirs.contains(k) && seen.insert(*k))
        .cloned()
        .collect()
}
