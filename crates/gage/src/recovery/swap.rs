//! The recovery key's one power inside gage, and the swap it shares with
//! `recovery rotate`: one atomic change to the recipient list, some labels
//! removed and some recipients added, re-encrypted and committed once.
//!
//! A recovery-scoped [`Identity`] exists only inside this module. It is
//! built by the private `recovery_identity`, and `Identity` can't be
//! constructed outside the crate, so there is no way to obtain one from a
//! frontend and carry it into ordinary CRUD. That is the enforcement, not
//! a convention.

use zeroize::Zeroizing;

use super::{RECOVERY_DEVICE_LABEL, RecoveryError, check_device_label_free};
use crate::crypto::identity::{Identity, KeyLine, PageLock, Unlocked};
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::prompter::{Prompter, Warning};
use crate::storage::vaultconfig::Recipient;
use crate::trust::recipient::{RecipientError, RecipientWrite};
use crate::vault::{Vault, warn_page_lock_failed};
use crate::{crypto::agekey, devicename};

/// The change [`Vault::recover_device`] makes to a vault's recipient list,
/// in one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverSpec {
    /// The replacement device being enrolled. The caller has already
    /// generated its identity; this is its label and public half.
    pub device: String,
    pub pubkey: String,
    /// A recipient to remove in the same commit: the lost device, whose
    /// wrapped key may be in someone else's hands. It may not name the
    /// recovery key, which is retired unconditionally.
    pub replaces: Option<String>,
    /// The replacement recovery key, registered under the same fixed
    /// label. `None` leaves the vault with no recovery recipient, which a
    /// caller may ask for and never gets by default.
    pub new_recovery_pubkey: Option<String>,
}

/// What [`Vault::recover_device`] did, for a frontend to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverResult {
    pub device: String,
    pub pubkey: String,
    /// The public half of the recovery key that authorized the swap and
    /// was removed by it.
    pub retired_recovery: String,
    pub new_recovery_pubkey: Option<String>,
    /// The device removed through [`RecoverSpec::replaces`].
    pub replaced: Option<String>,
    /// The one commit's id.
    pub commit: String,
    /// Always the whole vault.
    pub reencrypted: usize,
}

/// What [`Vault::rotate_recovery_key`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateResult {
    /// The recovery key replaced, or `None` when the vault had none.
    pub retired: Option<String>,
    /// True when there was nothing to replace, so this added the first.
    pub created: bool,
    /// The new recovery key's public half.
    pub pubkey: String,
    pub commit: String,
    pub reencrypted: usize,
}

/// Checked by [`RecipientSwap::inspect`] against the list as read under
/// the lock, before the final list is computed.
type Inspect<'a> = Box<dyn FnMut(&[Recipient]) -> Result<(), RecoveryError> + 'a>;

/// One atomic change to the recipient list: some labels removed, some
/// recipients added, in one commit.
#[derive(Default)]
struct RecipientSwap<'a> {
    add: Vec<Recipient>,
    /// Labels to remove. One that names nobody is an error, so a typo in
    /// `--replaces` can't silently leave the lost device a recipient.
    remove: Vec<String>,
    /// Labels removed when held and skipped when not: rotate retires a
    /// recovery recipient a `--no-recovery-key` vault never had.
    remove_if_present: Vec<String>,
    message: String,
    /// Called with the current list under the lock. An error aborts the
    /// swap with nothing written.
    inspect: Option<Inspect<'a>>,
}

impl Vault {
    /// The recovery key's one power inside gage: enrolls a replacement
    /// device and retires itself, in a single commit.
    ///
    /// `identity add` can't touch the recipient list (a device that lost
    /// its key can't authorize itself), and on a machine whose identity
    /// file is gone the recovery key is the only thing left that can say
    /// "admit this key". Retiring it in the same commit is not tidiness: by
    /// now the secret has been typed into a terminal, and it is a full-read
    /// credential. Leaving it live would trade a lost identity file for a
    /// permanent one.
    ///
    /// `secret` is taken by value and dropped once parsed, on every path
    /// (ADR-0027).
    pub fn recover_device(
        &self,
        spec: &RecoverSpec,
        secret: Zeroizing<String>,
        prompter: &dyn Prompter,
    ) -> Result<RecoverResult, RecoveryError> {
        // Before anything is read, as unlock does: the list the key is
        // checked against, and the entries about to be rewritten, should
        // be the current ones. This path skips unlock, so without it a
        // recovery could re-encrypt on a stale tip, or refuse a key a
        // rotation elsewhere has already made current.
        self.sync_on_unlock(prompter);

        let identity = self.recovery_identity(secret, prompter)?;
        let retired = identity.recipient().to_string();
        // The retired key's own public half, because "retired" has to mean
        // retired: see check_recover_spec.
        check_recover_spec(spec, &retired)?;

        let mut add = vec![Recipient {
            device: spec.device.clone(),
            pubkey: spec.pubkey.clone(),
        }];
        if let Some(pubkey) = &spec.new_recovery_pubkey {
            add.push(Recipient {
                device: RECOVERY_DEVICE_LABEL.into(),
                pubkey: pubkey.clone(),
            });
        }
        let mut remove = vec![RECOVERY_DEVICE_LABEL.to_string()];
        remove.extend(spec.replaces.clone());
        let (reencrypted, commit) = self.swap_recipients(
            RecipientSwap {
                add,
                remove,
                message: format!(
                    "gage: recovery enroll {}, retire {RECOVERY_DEVICE_LABEL} (reencrypt)",
                    spec.device
                ),
                ..RecipientSwap::default()
            },
            &identity,
            prompter,
        )?;
        Ok(RecoverResult {
            device: spec.device.clone(),
            pubkey: spec.pubkey.clone(),
            retired_recovery: retired,
            new_recovery_pubkey: spec.new_recovery_pubkey.clone(),
            replaced: spec.replaces.clone(),
            commit,
            reencrypted,
        })
    }

    /// Replaces the vault's recovery recipient with `new_pubkey`, or adds
    /// the first one, in one commit: `recover_device`'s swap, driven by an
    /// ordinary unlocked identity. For a recovery key that may have leaked,
    /// and for a vault made with `--no-recovery-key` that wants one after
    /// all. Add-then-remove can't do it: the label is fixed, so the new key
    /// can't be added while the old one holds it.
    ///
    /// Rotate can't tell a recovery key from any other key; the label is
    /// the distinction, which is why the label is reserved everywhere a
    /// device names itself.
    pub fn rotate_recovery_key(
        &self,
        new_pubkey: &str,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<RotateResult, RecoveryError> {
        agekey::validate_recipient(new_pubkey).map_err(RecipientError::from)?;

        let mut retired = None;
        let (reencrypted, commit) = self.swap_recipients(
            RecipientSwap {
                add: vec![Recipient {
                    device: RECOVERY_DEVICE_LABEL.into(),
                    pubkey: new_pubkey.to_string(),
                }],
                remove_if_present: vec![RECOVERY_DEVICE_LABEL.into()],
                message: "gage: recovery rotate (reencrypt)".into(),
                // Read under the lock, so what is reported retired is what
                // the swap actually retired, not a stale read.
                inspect: Some(Box::new(|current| {
                    let held = current.iter().find(|r| r.device == RECOVERY_DEVICE_LABEL);
                    if let Some(r) = held {
                        // The swap removes the label before it checks the
                        // additions, so the retired key would otherwise
                        // sail back in under its own name.
                        if r.pubkey == new_pubkey {
                            return Err(RecoveryError::RetiredKeyReused {
                                pubkey: r.pubkey.clone(),
                                as_device: RECOVERY_DEVICE_LABEL.into(),
                            });
                        }
                        // The backstop under the reservation: whatever
                        // slips under the label, rotate must not lock out
                        // the device running it.
                        if r.pubkey == identity.recipient() {
                            return Err(RecoveryError::WouldEvictActingDevice {
                                vault: self.name.clone(),
                            });
                        }
                    }
                    retired = held.map(|r| r.pubkey.clone());
                    Ok(())
                })),
                ..RecipientSwap::default()
            },
            identity,
            prompter,
        )?;
        Ok(RotateResult {
            created: retired.is_none(),
            retired,
            pubkey: new_pubkey.to_string(),
            commit,
            reencrypted,
        })
    }

    /// Applies a swap: remove, add, re-encrypt, commit, push, all of it or
    /// none of it. Takes the write lock.
    ///
    /// Add-then-remove is not the same operation: it would re-encrypt the
    /// vault twice in two commits, leave a window where the old and new
    /// recovery keys are both live, and couldn't add a new recovery key
    /// while the old one holds the fixed label. `remove_recipient` is the
    /// wrong shape too: its self-removal question would warn that "this
    /// device" is losing access, which is what retirement means and not
    /// what that question is for.
    ///
    /// It skips none of the other verbs' checks. Being the recovery key is
    /// not a reason to bless a list nobody reviewed.
    fn swap_recipients(
        &self,
        mut swap: RecipientSwap<'_>,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<(usize, String), RecoveryError> {
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            // The full-access pre-flight (ADR-0009). Always inside the lock, unlike add's: the
            // reset has just run, so this reads exactly what the
            // re-encryption will, and there is no clean/dirty case to get
            // wrong. Still ahead of every question and the first byte.
            self.require_full_access(identity)?;
            // The trust precondition, then its question, in add's order and for
            // add's reasons: this rebuilds .age-recipients from config.toml,
            // and a declined answer must leave nothing to undo.
            self.require_recipients_in_sync()
                .map_err(RecipientError::from)?;
            self.confirm_recipient_trust(prompter)
                .map_err(RecipientError::from)?;

            let current = self.recipients().map_err(RecipientError::from)?;
            if let Some(inspect) = swap.inspect.as_mut() {
                inspect(&current)?;
            }
            let updated = apply_swap(&current, &swap)?;

            // Said before the work: by the time the commit lands there is
            // nothing left to reconsider. Both kinds of removal, since
            // rotate retires through remove_if_present and the
            // leak-response user running it most needs to hear this; only
            // labels the list actually held.
            for label in swap.remove.iter().chain(&swap.remove_if_present) {
                if current.iter().any(|r| r.device == *label) {
                    prompter.warn(&Warning::from(format!(
                        "removing {label:?} revokes future access only — it still opens every version of every entry already in this vault's git history."
                    )));
                }
            }

            let done = self.commit_recipient_list(
                &updated,
                &RecipientWrite::message(&swap.message),
                identity,
                prompter,
            )?;
            // The operator just reviewed this list by running the command,
            // so the cache is regenerated last: otherwise their next write
            // would warn them about their own change.
            self.note_recipients_reviewed()
                .map_err(RecipientError::from)?;
            Ok(done)
        })
    }

    /// Turns a pasted recovery secret into an [`Identity`] that can decrypt
    /// this vault. Private to this module: see the module docs.
    ///
    /// The key must be the recipient labelled [`RECOVERY_DEVICE_LABEL`];
    /// see [`RecoveryError::NotRecoveryKey`] for why a device key that
    /// would decrypt just as well is refused anyway.
    ///
    /// `secret` is consumed, and dropped (so zeroed) as soon as
    /// `KeyLine::parse` has copied it into page-lockable memory, rather
    /// than staying alive across the caller's lock, re-encryption and push.
    fn recovery_identity(
        &self,
        secret: Zeroizing<String>,
        prompter: &dyn Prompter,
    ) -> Result<Identity, RecoveryError> {
        // The parser's error is dropped, not wrapped: age's parse errors
        // can quote their input. Reported as a malformed recovery key, not
        // a corrupt identity file: there is no file, and the human's next
        // move is to check what they pasted.
        let key = KeyLine::parse(secret.trim().as_bytes())
            .map_err(|_| RecoveryError::MalformedRecoveryKey);
        drop(secret);
        let key = key?;

        let pubkey = key.recipient();
        match self.recipients()?.into_iter().find(|r| r.pubkey == pubkey) {
            Some(r) if r.device == RECOVERY_DEVICE_LABEL => {}
            listed => {
                return Err(RecoveryError::NotRecoveryKey {
                    device: listed.map(|r| r.device),
                    pubkey: pubkey.to_string(),
                    vault: self.name.clone(),
                });
            }
        }
        let Unlocked {
            identity,
            page_lock,
        } = Identity::from_key(RECOVERY_DEVICE_LABEL, key, self.locker());
        if let PageLock::Failed(e) = page_lock {
            warn_page_lock_failed(prompter, RECOVERY_DEVICE_LABEL, &e);
        }
        Ok(identity)
    }
}

/// Refuses what the caller asked for, before any lock. Collisions with
/// the list are left to [`apply_swap`], which checks the final list:
/// rebuilding a machine under the name it always had is the ordinary case.
///
/// `retired` is the key authorizing the call, and the one collision the
/// swap structurally can't catch: it removes the recovery label before
/// checking the additions, so by then the retired key looks unused.
fn check_recover_spec(spec: &RecoverSpec, retired: &str) -> Result<(), RecoveryError> {
    if !devicename::valid(&spec.device) {
        return Err(RecipientError::InvalidDeviceName(spec.device.clone()).into());
    }
    check_device_label_free(&spec.device).map_err(RecipientError::from)?;
    agekey::validate_recipient(&spec.pubkey).map_err(RecipientError::from)?;
    if spec.pubkey == retired {
        return Err(RecoveryError::RetiredKeyReused {
            pubkey: retired.to_string(),
            as_device: spec.device.clone(),
        });
    }
    if let Some(pubkey) = &spec.new_recovery_pubkey {
        agekey::validate_recipient(pubkey).map_err(RecipientError::from)?;
        if pubkey == retired {
            return Err(RecoveryError::RetiredKeyReused {
                pubkey: retired.to_string(),
                as_device: RECOVERY_DEVICE_LABEL.into(),
            });
        }
    }
    if spec.replaces.as_deref() == Some(RECOVERY_DEVICE_LABEL) {
        return Err(RecoveryError::ReplacesRecoveryKey);
    }
    Ok(())
}

/// The final list, or why the list can't legally become it. Pure, so every
/// refusal comes before a byte is written.
fn apply_swap(
    current: &[Recipient],
    swap: &RecipientSwap<'_>,
) -> Result<Vec<Recipient>, RecoveryError> {
    let mut updated = Vec::with_capacity(current.len() + swap.add.len());
    let mut removed = Vec::new();
    for r in current {
        if swap.remove.contains(&r.device) || swap.remove_if_present.contains(&r.device) {
            removed.push(r.device.clone());
        } else {
            updated.push(r.clone());
        }
    }
    // A `remove` label that matched nobody: reported, so a typo is not
    // silently a no-op.
    if let Some(label) = swap.remove.iter().find(|l| !removed.contains(l)) {
        return Err(RecipientError::RecipientNotFound {
            query: label.clone(),
        }
        .into());
    }
    // Against the list as it will be, not as it is.
    for add in &swap.add {
        if !devicename::valid(&add.device) {
            return Err(RecipientError::InvalidDeviceName(add.device.clone()).into());
        }
        agekey::validate_recipient(&add.pubkey).map_err(RecipientError::from)?;
        if let Some(existing) = updated.iter().find(|r| r.pubkey == add.pubkey) {
            return Err(RecipientError::RecipientExists {
                device: existing.device.clone(),
                pubkey: Some(existing.pubkey.clone()),
            }
            .into());
        }
        if updated.iter().any(|r| r.device == add.device) {
            return Err(RecipientError::RecipientExists {
                device: add.device.clone(),
                pubkey: None,
            }
            .into());
        }
        updated.push(add.clone());
    }
    if updated.is_empty() {
        return Err(RecipientError::LastRecipient {
            device: removed.join(", "),
        }
        .into());
    }
    Ok(updated)
}

#[cfg(test)]
#[path = "swap_tests.rs"]
mod tests;
