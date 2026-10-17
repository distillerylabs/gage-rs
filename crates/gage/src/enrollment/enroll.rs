//! The joining device: publish a request to join, sealed to a code the
//! human carries to an approving device. See "The flows", the joining
//! device, and `D-ENROLL-REMOTE`.

use std::time::{Duration, Instant};

use jiff::Timestamp;
use zeroize::Zeroizing;

use super::{EnrollmentError, EnrollmentRequest, MAX_ENROLLMENT_TTL, name::pending_file_name};
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::gitrepo::REMOTE_OP_TIMEOUT;
use crate::prompter::Prompter;
use crate::syncerr::Kind;
use crate::vault::Vault;
use crate::{atomicfile, config, devicename, gitrepo, identityfile, paths, vaultconfig};

impl Vault {
    /// Asks to join this vault: creates or reuses this device's identity,
    /// seals a request to a freshly generated code, publishes it, and
    /// returns the code for the caller to show. Nothing it publishes grants
    /// anything; only an approving device holding the code does.
    ///
    /// The order is the contract (`D-ENROLL-REMOTE`): the arguments are
    /// checked, then the remote, then the lock and the dirty-tree reset,
    /// then a fetch and fast-forward, then the two collision checks, and
    /// only then the identity. So an unreachable remote, a divergence or a
    /// name clash is found before a passphrase is asked for or a key
    /// written. Everything remote goes through the lock-held sync
    /// functions: the public `pull` and `push` take the lock themselves,
    /// and it is not re-entrant.
    ///
    /// A device whose recorded key is already a recipient gets a success
    /// that did nothing (`published == false`), checked before the name so
    /// it is never mistaken for a clash with itself.
    ///
    /// Recording the device, method and key in global config is the
    /// caller's, as with `add_identity`.
    pub fn enroll(
        &self,
        device: &str,
        ttl: Duration,
        prompter: &dyn Prompter,
    ) -> Result<EnrollmentRequest, EnrollmentError> {
        if ttl.is_zero() || ttl > MAX_ENROLLMENT_TTL {
            return Err(EnrollmentError::InvalidTtl { ttl });
        }
        if !devicename::valid(device) {
            return Err(EnrollmentError::InvalidDevice {
                device: device.to_string(),
            });
        }
        if !gitrepo::has_remote(&self.path)? {
            return Err(EnrollmentError::NoRemote {
                vault: self.name.clone(),
            });
        }

        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            self.enroll_locked(device, ttl, prompter)
        })
    }

    fn enroll_locked(
        &self,
        device: &str,
        ttl: Duration,
        prompter: &dyn Prompter,
    ) -> Result<EnrollmentRequest, EnrollmentError> {
        let deadline = Instant::now() + REMOTE_OP_TIMEOUT;
        let pulled = self.pull_locked(deadline).map_err(|e| {
            if e.remote_kind() == Some(Kind::Unreachable) {
                EnrollmentError::RemoteUnreachable(e)
            } else {
                EnrollmentError::Sync(e)
            }
        })?;
        // Found by the report, not by an error: a pull that cannot
        // fast-forward still succeeds.
        if pulled.diverged {
            return Err(EnrollmentError::Diverged {
                vault: self.name.clone(),
            });
        }
        self.warn_if_clock_is_behind_head(prompter);

        let file = vaultconfig::read(&self.config_path())?;
        let recorded = self.recorded_registration();
        if !recorded.pubkey.is_empty()
            && file.recipients.iter().any(|r| r.pubkey == recorded.pubkey)
        {
            let now = Timestamp::now();
            return Ok(EnrollmentRequest {
                id: String::new(),
                device: recorded.device,
                pubkey: recorded.pubkey,
                method: recorded.method,
                created: now,
                expires: now,
                code: Zeroizing::default(),
                published: false,
            });
        }
        if file.recipients.iter().any(|r| r.device == device) {
            return Err(EnrollmentError::DeviceNameTaken {
                device: device.to_string(),
                vault: self.name.clone(),
            });
        }

        let pubkey = identityfile::create(self.dirs(), &self.id, &self.name, device, prompter)?;
        let method = if recorded.method.is_empty() {
            file.method.default.clone()
        } else {
            recorded.method
        };
        let (mut request, sealed) = self.seal_enrollment(device, &pubkey, &method, ttl)?;

        let dir = self.pending_dir();
        std::fs::create_dir_all(&dir).map_err(|source| EnrollmentError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = pending_file_name(&request.id, request.expires);
        atomicfile::write(&dir.join(name), &sealed, 0o600)?;
        gitrepo::commit_all(&self.path, &format!("enrollment request {}", request.id))?;

        self.push_locked(Instant::now() + REMOTE_OP_TIMEOUT)
            .map_err(EnrollmentError::Push)?;
        request.published = true;
        Ok(request)
    }

    /// Warns when this machine's clock reads earlier than the newest commit
    /// the vault has: a request sealed now would carry an expiry computed
    /// from a clock the approver may judge wrong. It proceeds regardless.
    fn warn_if_clock_is_behind_head(&self, prompter: &dyn Prompter) {
        let Ok(Some(head)) = gitrepo::head_committer_time(&self.path) else {
            return;
        };
        let now = Timestamp::now();
        if now < head {
            prompter.warn(&format!(
                "gage: this machine's clock ({now}) is behind the vault's newest commit ({head}); a request sealed now may look expired or skewed to the approving device, so check the clock"
            ));
        }
    }

    /// What this machine's registration records for the vault, or blanks
    /// when there is none: a missing or unreadable one skips the checks it
    /// feeds and is never an error here.
    fn recorded_registration(&self) -> Recorded {
        let Ok(global) = config::read(&paths::global_config_file(self.dirs())) else {
            return Recorded::default();
        };
        global
            .vaults
            .get(&self.name)
            .map(|v| Recorded {
                device: v.device.clone(),
                pubkey: v.pubkey.clone(),
                method: v.method.clone(),
            })
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct Recorded {
    device: String,
    pubkey: String,
    method: String,
}
