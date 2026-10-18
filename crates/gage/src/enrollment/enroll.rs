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
use crate::recovery::check_device_label_free;
use crate::syncerr::Kind;
use crate::vault::{METHOD_PASSPHRASE, Vault};
use crate::{atomicfile, config, devicename, gitrepo, identityfile, paths, vaultconfig};

/// What a published request commits under. Like every commit gage writes,
/// it names no device and no request: the filename scheme exists so a
/// reader of the repository learns nothing about who is enrolling.
const ENROLLMENT_COMMIT_MESSAGE: &str = "gage: enrollment request";

/// How far HEAD's committer time has to be ahead of this machine's clock
/// before enroll says so: comfortably past ordinary drift and well short of
/// any TTL, so the warning means something (Go's
/// `enrollmentClockSkewThreshold`).
const CLOCK_SKEW_THRESHOLD: jiff::SignedDuration = jiff::SignedDuration::from_hours(1);

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
        check_device_label_free(device)?;
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
        self.catch_up_for_enroll()?;
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
        // Passphrase, not what global config or the vault's default says:
        // the payload's method describes how this device stores its key,
        // and `create` has just written (or unlocked) a passphrase-wrapped
        // file. A build that can produce another kind of identity is the
        // build that changes this line.
        let (mut request, sealed) =
            self.seal_enrollment(device, &pubkey, METHOD_PASSPHRASE, ttl)?;

        let dir = self.pending_dir();
        std::fs::create_dir_all(&dir).map_err(|source| EnrollmentError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = pending_file_name(&request.id, request.expires);
        atomicfile::write(&dir.join(name), &sealed, 0o600)?;
        gitrepo::commit_all(&self.path, ENROLLMENT_COMMIT_MESSAGE)?;

        self.push_locked(Instant::now() + REMOTE_OP_TIMEOUT)
            .map_err(EnrollmentError::Push)?;
        request.published = true;
        Ok(request)
    }

    /// Step 3: fetch and fast-forward with the lock held, refusing a
    /// divergence. The test hook fires here, inside the same function as
    /// the pull, so the two can't be separated: a catch-up moved outside
    /// the lock takes the hook (and its probe) with it, and one that goes
    /// around this function never fires the hook at all, which the lock
    /// row asserts.
    fn catch_up_for_enroll(&self) -> Result<(), EnrollmentError> {
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
        self.enroll_hook.after_pull();
        Ok(())
    }

    /// Warns when this machine's clock reads at least
    /// [`CLOCK_SKEW_THRESHOLD`] earlier than the newest commit the vault
    /// has: a request sealed now would carry an expiry computed from a
    /// clock the approver may judge wrong. It proceeds regardless, and a
    /// failure to read the time is silent: a heuristic that could fail the
    /// command would be worse than the skew it detects.
    fn warn_if_clock_is_behind_head(&self, prompter: &dyn Prompter) {
        let Ok(Some(head)) = gitrepo::head_committer_time(&self.path) else {
            return;
        };
        let now = Timestamp::now();
        if head.duration_since(now) >= CLOCK_SKEW_THRESHOLD {
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
