//! Whether this device's identity file for a vault is an orphan once the
//! vault is forgotten (`gage vault remove`): the key it holds is no longer
//! one of the vault's recipients, so nothing in the vault can need it.
//!
//! The library only assesses. The identity file is the only copy of a
//! private key, so deleting it is a human's call, made with the path in
//! front of them: [`assess`] returns [`OrphanCheck::Offer`] and the
//! frontend asks. Every other outcome keeps the file, and says why.
//!
//! The comparison is by public key, never by device name. A name match is
//! wrong in both directions: a vault may label an active recipient's key
//! differently from local config (`recipient approve --device` does so on
//! purpose), and another device may since have taken a stale key's old
//! label (ADR-0013). The check lives
//! here and the question does not: the library reports, the frontend asks.

use std::path::PathBuf;

use crate::config::VaultEntry;
use crate::paths;
use crate::vault::Vault;
use crate::xdgpaths::Dirs;

/// What to do about this device's identity file for a vault being forgotten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanCheck {
    /// There is no identity file to speak of: no device or id recorded, or
    /// no file on disk. Nothing to delete and nothing to say.
    Nothing,
    /// Leave the file, and tell the human why.
    Keep { path: PathBuf, reason: KeepReason },
    /// The key is no longer a recipient. Ask before deleting `path`; the
    /// answer must default to no.
    Offer { path: PathBuf, device: String },
}

/// Why an identity file is kept. Each one is a case where guessing would
/// strand access to the vault's ciphertext for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepReason {
    /// Global config records no public key for this device (an older
    /// registration, a hand-edited config, or a clone that never ran
    /// `identity add`), so there is no key to compare.
    NoPublicKeyRecorded { device: String },
    /// The registration points at a different vault than the one on disk,
    /// or at one whose config can't be used. The identity would be judged
    /// against the wrong vault's recipients.
    RegistrationDoesNotMatch { detail: String },
    /// The vault's recipient list could not be read (a moved or deleted
    /// store, a remote-only one, filesystem trouble).
    RecipientsUnreadable { device: String, detail: String },
    /// The key is still a recipient of the vault, listed as `label`.
    StillARecipient { label: String },
}

/// Assesses `entry`, the registration being forgotten as `name`. Reads the
/// vault's recipients fresh from disk and deletes nothing.
pub fn assess(dirs: &Dirs, name: &str, entry: &VaultEntry) -> OrphanCheck {
    let Some(id) = &entry.id else {
        return OrphanCheck::Nothing;
    };
    if entry.device.is_empty() {
        return OrphanCheck::Nothing;
    }
    // The device name comes from a config file this process did not write:
    // an unsafe one builds no path at all. The id was checked as it was
    // read.
    let Ok(path) = paths::identity_file_path(dirs, id, &entry.device) else {
        return OrphanCheck::Nothing;
    };
    if !path.is_file() {
        return OrphanCheck::Nothing;
    }
    let keep = |reason| OrphanCheck::Keep {
        path: path.clone(),
        reason,
    };

    let Some(pubkey) = &entry.pubkey else {
        return keep(KeepReason::NoPublicKeyRecorded {
            device: entry.device.clone(),
        });
    };
    let vault = match Vault::from_registration(dirs, name, entry) {
        Ok(vault) => vault,
        Err(e) => {
            return keep(KeepReason::RegistrationDoesNotMatch {
                detail: e.to_string(),
            });
        }
    };
    let recipients = match vault.recipients() {
        Ok(recipients) => recipients,
        Err(e) => {
            return keep(KeepReason::RecipientsUnreadable {
                device: entry.device.clone(),
                detail: e.to_string(),
            });
        }
    };
    if let Some(listed) = recipients.iter().find(|r| r.pubkey == *pubkey) {
        return keep(KeepReason::StillARecipient {
            label: listed.device.clone(),
        });
    }
    OrphanCheck::Offer {
        path,
        device: entry.device.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identityfile;
    use crate::prompter::FakePrompter;
    use crate::recipient_tests::{DEVICE, Fixture};
    use crate::testsupport::fast_scrypt;
    use crate::vaultconfig;

    const PASSPHRASE: &str = "correct horse battery staple";

    /// A vault, this device's identity file for it, and the registration
    /// that names both. The identity's key is *not* a recipient (the
    /// fixture's own key is), so by default the check offers.
    struct Case {
        fx: Fixture,
        entry: VaultEntry,
    }

    impl Case {
        fn new() -> Case {
            fast_scrypt();
            let fx = Fixture::new();
            let dirs = fx.vault.dirs().clone();
            let pubkey = identityfile::create(
                &dirs,
                &fx.vault.id,
                "personal",
                DEVICE,
                &FakePrompter::new().with_passphrase(PASSPHRASE),
            )
            .unwrap();
            let entry = VaultEntry {
                path: fx.vault.path.clone(),
                id: Some(fx.vault.id.clone()),
                device: DEVICE.into(),
                pubkey: Some(pubkey),
                ..VaultEntry::default()
            };
            Case { fx, entry }
        }

        fn assess(&self) -> OrphanCheck {
            assess(self.fx.vault.dirs(), "personal", &self.entry)
        }

        fn identity_path(&self) -> PathBuf {
            paths::identity_file_path(self.fx.vault.dirs(), &self.fx.vault.id, DEVICE).unwrap()
        }

        fn kept(&self) -> KeepReason {
            match self.assess() {
                OrphanCheck::Keep { path, reason } => {
                    assert_eq!(path, self.identity_path());
                    reason
                }
                other => panic!("want Keep, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_key_the_vault_does_not_list_is_offered_for_deletion_not_deleted() {
        let c = Case::new();
        assert_eq!(
            c.assess(),
            OrphanCheck::Offer {
                path: c.identity_path(),
                device: DEVICE.into()
            }
        );
        assert!(c.identity_path().is_file(), "assessing deletes nothing");
    }

    #[test]
    fn no_device_or_no_id_recorded_is_nothing() {
        let mut c = Case::new();
        c.entry.device.clear();
        assert_eq!(c.assess(), OrphanCheck::Nothing);
        let mut c = Case::new();
        c.entry.id = None;
        assert_eq!(c.assess(), OrphanCheck::Nothing);
    }

    #[test]
    fn no_identity_file_is_nothing() {
        let c = Case::new();
        std::fs::remove_file(c.identity_path()).unwrap();
        assert_eq!(c.assess(), OrphanCheck::Nothing);
    }

    // An unsafe id can't reach here: the global config refuses it as it is
    // read (ADR-0052).
    #[test]
    fn an_unsafe_device_builds_no_path_and_is_nothing() {
        let mut c = Case::new();
        c.entry.device = "../escape".into();
        assert_eq!(c.assess(), OrphanCheck::Nothing);
    }

    #[test]
    fn no_recorded_public_key_keeps_the_file() {
        let mut c = Case::new();
        c.entry.pubkey = None;
        assert_eq!(
            c.kept(),
            KeepReason::NoPublicKeyRecorded {
                device: DEVICE.into()
            }
        );
    }

    #[test]
    fn a_key_still_listed_is_kept_under_whatever_label_the_vault_gives_it() {
        let mut c = Case::new();
        // The vault lists the fixture's key as DEVICE; the registration
        // holds that key, but the vault might label it anything.
        c.entry.pubkey = Some(c.fx.pubkey());
        assert_eq!(
            c.kept(),
            KeepReason::StillARecipient {
                label: DEVICE.into()
            }
        );

        // Relabelled in the vault: the key matches, the name does not.
        let config_path = c.fx.vault.path.join(".gage/config.toml");
        let mut file = vaultconfig::read(&config_path).unwrap();
        file.recipients[0].device = "phone-9".into();
        vaultconfig::write(&config_path, &file).unwrap();
        assert_eq!(
            c.kept(),
            KeepReason::StillARecipient {
                label: "phone-9".into()
            }
        );
    }

    #[test]
    fn a_stale_key_whose_name_now_labels_another_is_still_offered() {
        let c = Case::new();
        // The vault lists a *different* key under this device's name.
        let config = c.fx.vault.recipients().unwrap();
        assert_eq!(config[0].device, DEVICE);
        assert_ne!(Some(&config[0].pubkey), c.entry.pubkey.as_ref());
        assert!(matches!(c.assess(), OrphanCheck::Offer { .. }));
    }

    #[test]
    fn unreadable_recipients_keep_the_file_and_are_not_read_as_not_a_recipient() {
        let c = Case::new();
        std::fs::remove_dir_all(&c.fx.vault.path).unwrap();
        match c.kept() {
            KeepReason::RecipientsUnreadable { device, detail } => {
                assert_eq!(device, DEVICE);
                assert!(!detail.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_registration_for_a_different_vault_keeps_the_file() {
        let mut c = Case::new();
        // Both ids are valid; only the one on disk is this vault's. The
        // identity sits under the registration's id, as it would have.
        let other = vaultconfig::VaultId::new();
        let dirs = c.fx.vault.dirs().clone();
        identityfile::create(
            &dirs,
            &other,
            "other",
            DEVICE,
            &FakePrompter::new().with_passphrase(PASSPHRASE),
        )
        .unwrap();
        c.entry.id = Some(other.clone());
        let path = paths::identity_file_path(&dirs, &other, DEVICE).unwrap();
        match c.assess() {
            OrphanCheck::Keep {
                path: kept,
                reason: KeepReason::RegistrationDoesNotMatch { detail },
            } => {
                assert_eq!(kept, path);
                assert!(detail.contains(other.as_str()), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }
}
