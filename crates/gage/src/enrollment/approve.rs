//! The approving device: the collision pre-check that runs before anyone
//! is asked anything, approval itself, and deny. See "The flows", the
//! approving device, "The approver unlocks, and the unlock comes late" and
//! "Approval fetches before it commits".
//!
//! As on the joining side, every step's position is load-bearing; the
//! comments on [`Vault::approve_enrollments`] say what each one is for.

use std::time::Instant;

use jiff::Timestamp;
use zeroize::Zeroizing;

use super::code::normalize_code;
use super::name::parse_pending_file_name;
use super::seal::Attempt;
use super::{EnrollmentError, OpenedRequest, PENDING_DIR, PendingRequest, read_pending_file};
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::gitrepo::{self, REMOTE_OP_TIMEOUT};
use crate::identity::Identity;
use crate::prompter::Prompter;
use crate::recipient::RecipientWrite;
use crate::recovery::check_device_label_free;
use crate::remotesync::offline_or_error;
use crate::vault::Vault;
use crate::vaultconfig::Recipient;

/// What an unpublished approval says it cost. The generic "committed
/// locally but not pushed" is far too thin here: locally the vault is
/// re-encrypted, the device listed and its request gone; on the remote
/// none of it happened, and the joining device is locked out with no way
/// to tell, since `gage sync` reports nothing wrong on its side.
pub(crate) const APPROVAL_PUSH_CLAUSE: &str = "the approval is committed locally but not pushed, so it has not taken effect for anyone else: the request is still pending on the remote and the joining device is still locked out — run `gage push` to finish it";

/// [`APPROVAL_PUSH_CLAUSE`] for an approval that granted nothing: nobody is
/// locked out, and the only casualty is a stale request on the remote.
pub(crate) const CLEARED_PUSH_CLAUSE: &str = "the request was cleared locally but the removal was not pushed, so it is still pending for every other device";

/// What an unpublished deny says it cost: not a late write but a request
/// still live for every other device holding the code. A silent failed
/// refusal is indistinguishable from a successful one.
pub(crate) const DENIED_PUSH_CLAUSE: &str = "the request was removed locally but the removal was not pushed, so it is still pending for every other device";

/// The commits that resolve a request without adding anyone. Like every
/// commit gage writes, they name no device: someone turned away is as
/// entitled to the filename scheme's anonymity as someone let in.
const DENY_COMMIT_MESSAGE: &str = "gage: enrollment request denied";
const CLEARED_COMMIT_MESSAGE: &str = "gage: enrollment request cleared (already a recipient)";

/// An opened request and the name the approver chose for it. `None` keeps
/// the sealed name; a label overrides it, which is how a device-name
/// collision is resolved without a new round trip (D-ENROLL-COLLISIONS).
///
/// The label rides here, not on [`OpenedRequest`], because every field of
/// that type is authenticated and a label the approver typed is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub request: OpenedRequest,
    pub label: Option<String>,
}

impl Approval {
    /// Approve under the name the request claims.
    pub fn new(request: OpenedRequest) -> Approval {
        Approval {
            request,
            label: None,
        }
    }

    /// Approve under `label` instead (`--device`).
    pub fn relabeled(request: OpenedRequest, label: &str) -> Approval {
        Approval {
            request,
            label: Some(label.to_string()),
        }
    }

    /// The name this approval records.
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.request.device)
    }
}

/// What happened to one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOutcome {
    pub request: OpenedRequest,
    /// The name in the recipient list: the approval's label, or for a
    /// no-op the name the key was already listed under.
    pub label: String,
    /// False when the key was already a recipient: a success that did no
    /// work, the approving side of enroll's `published == false`.
    pub added: bool,
}

/// A whole batch, which shares one re-encryption pass and one commit.
/// `RecipientChange` isn't reused: it names one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalResult {
    pub outcomes: Vec<ApprovalOutcome>,
    pub reencrypted: usize,
    pub commit: String,
}

/// One approval as the copy under the lock states it, with the file it was
/// verified against. The file travels with it because that is the path
/// the commit deletes, and it can't be rebuilt from the request: the
/// filename's epoch is a hint, not an authenticated field.
struct Verified {
    approval: Approval,
    rel_path: String,
}

impl Vault {
    /// The device-name collision pre-check: does any approval's label
    /// already belong to a different recipient?
    ///
    /// It reads only the plaintext recipient list and holds no identity,
    /// which is what lets a frontend run it after the open and before the
    /// render, the confirmation and any unlock. It is here rather than in
    /// a frontend so a second one inherits it.
    ///
    /// UX, not correctness: the authoritative check runs inside the
    /// recipient write, under the lock, and returns `RecipientExists`.
    /// This one exists so the ordinary case names `--device`. A request
    /// whose key is already listed is skipped: that is the no-op, and
    /// calling it a clash would advise minting a second label for a key
    /// that already has access.
    pub fn check_enrollment_labels(&self, approvals: &[Approval]) -> Result<(), EnrollmentError> {
        if approvals.is_empty() {
            return Ok(());
        }
        let current = self.recipients()?;
        for a in approvals {
            if current.iter().any(|r| r.pubkey == a.request.pubkey) {
                continue;
            }
            check_device_label_free(a.label())?;
            if let Some(taken) = current.iter().find(|r| r.device == a.label()) {
                return Err(EnrollmentError::NameTaken {
                    device: a.label().to_string(),
                    pubkey: taken.pubkey.clone(),
                    vault: self.name.clone(),
                });
            }
        }
        Ok(())
    }

    /// Admits the devices behind a batch of opened requests: adds each as
    /// a recipient, re-encrypts every entry so they can read the whole
    /// vault, and removes every approved request's file, in one commit.
    ///
    /// It takes the codes because it opens each seal again under the
    /// write lock and writes what that open says, not what was displayed
    /// a moment earlier; `open_enrollment` doesn't say which code opened
    /// which request.
    ///
    /// The order, and what each position is for:
    ///
    /// 1. The full-access pre-flight, before the lock. An actor who can't
    ///    read the whole vault can't grant it. It needs the unlocked
    ///    identity, which is the documented cost of the late unlock.
    /// 2. The lock, with the dirty-tree reset.
    /// 3. A fetch and fast-forward inside it, through the lock-held pull.
    ///    Approval rewrites every entry, so one made onto a stale tip
    ///    diverges in every entry rather than one.
    /// 4. The re-verification, under the lock.
    /// 5. Keys already listed are dropped: a success that does no work.
    /// 6. One E1b write: one trust question, one pass, one commit. Not
    ///    `add_recipient`, which would take N locks and make N commits.
    pub fn approve_enrollments(
        &self,
        approvals: &[Approval],
        codes: &[Zeroizing<String>],
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<ApprovalResult, EnrollmentError> {
        if approvals.is_empty() {
            return Err(EnrollmentError::EmptyBatch);
        }
        // On a clean tree only, as in `add_recipient`: the pre-flight reads
        // entries off disk, and on a tree an interrupted re-encryption
        // left dirty it would judge ciphertext the reset is about to
        // discard. There it moves to just after the reset, still before
        // the first byte and before M10's question.
        let clean = gitrepo::is_clean(&self.path)?;
        if clean {
            self.require_full_access(identity)?;
        }
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            if !clean {
                self.require_full_access(identity)?;
            }
            self.catch_up_before_approving(prompter)?;
            let verified = self.reverify_approvals(approvals, codes)?;
            self.write_approvals(&verified, identity, prompter)
        })
    }

    /// Fetches and fast-forwards with the lock held, and decides which
    /// failures stop the approval.
    ///
    /// Through `pull_locked`, never `pull`: that takes the lock, which is
    /// not re-entrant, so it would wait on this very process and then
    /// report contention.
    ///
    /// A divergence refuses with the ordinary advice: an approver can
    /// decrypt, so `gage sync` is exactly what resolves it. It is read
    /// from the report, not an error: a pull that can't fast-forward
    /// still succeeds. Any failure to fetch warns and proceeds, unlike
    /// enroll: an approval that lands locally is real work, while an
    /// unpublished request is none, and the failed-push warning says the
    /// rest.
    fn catch_up_before_approving(&self, prompter: &dyn Prompter) -> Result<(), EnrollmentError> {
        match self.pull_locked(Instant::now() + REMOTE_OP_TIMEOUT) {
            Err(err) => {
                prompter.warn(&format!(
                    "gage: {}; approving against the local copy of {:?}",
                    offline_or_error(&err),
                    self.name
                ));
                Ok(())
            }
            Ok(report) if report.diverged => Err(EnrollmentError::ApprovalDiverged {
                summary: report.summary(),
            }),
            Ok(_) => Ok(()),
        }
    }

    /// Re-reads each request from disk and opens its seal again, with the
    /// lock held: the whole of "what gets written is what was verified
    /// under the lock, not what was displayed".
    ///
    /// The first open ran outside the lock and before a human answered,
    /// and the fetch above makes "another device approved or denied it
    /// meanwhile" ordinary rather than rare. A file that is gone, no
    /// longer opens, or opens to something other than what the human was
    /// shown is `NoLongerPending`: consent was given for a particular
    /// device and key. A refusal from a code that did open (expired, wrong
    /// vault, malformed) is returned as itself.
    fn reverify_approvals(
        &self,
        approvals: &[Approval],
        codes: &[Zeroizing<String>],
    ) -> Result<Vec<Verified>, EnrollmentError> {
        let valid: Vec<Zeroizing<String>> =
            codes.iter().filter_map(|c| normalize_code(c)).collect();
        let now = Timestamp::now();
        approvals
            .iter()
            .map(|a| {
                let gone = || EnrollmentError::NoLongerPending {
                    id: a.request.id.clone(),
                };
                let (file, name) = self.find_pending_file(&a.request.id)?.ok_or_else(gone)?;
                let sealed = read_pending_file(&file.path).ok_or_else(gone)?;
                let fresh = self
                    .open_one_of(&file, &sealed, &valid, now)?
                    .ok_or_else(gone)?;
                if fresh != a.request {
                    return Err(gone());
                }
                Ok(Verified {
                    approval: Approval {
                        request: fresh,
                        label: a.label.clone(),
                    },
                    rel_path: format!("{PENDING_DIR}/{name}"),
                })
            })
            .collect()
    }

    /// A request's file by the UUID half of its name, with its name.
    ///
    /// Without the liveness filter on purpose: expiry is judged by the
    /// sealed copy, and filtering here would report a request that expired
    /// between the open and the lock as missing rather than expired, which
    /// send a human to different places.
    fn find_pending_file(
        &self,
        id: &str,
    ) -> Result<Option<(PendingRequest, String)>, EnrollmentError> {
        Ok(self
            .pending_files()?
            .into_iter()
            .find_map(|(name, path, _)| {
                let (found, expires) = parse_pending_file_name(&name)?;
                if found != id {
                    return None;
                }
                let request = PendingRequest {
                    id: found,
                    path,
                    expires,
                };
                Some((request, name))
            }))
    }

    /// The first of `codes` that opens `sealed`. A refusal from a code that
    /// did open is the answer: retrying another code can't change it.
    fn open_one_of(
        &self,
        file: &PendingRequest,
        sealed: &[u8],
        codes: &[Zeroizing<String>],
        now: Timestamp,
    ) -> Result<Option<OpenedRequest>, EnrollmentError> {
        for code in codes {
            match self.open_sealed(file, sealed, code, now) {
                Attempt::NotOpened => continue,
                Attempt::Opened(o) => return Ok(Some(o)),
                Attempt::Refused(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// The write, with the lock held and every request re-verified.
    fn write_approvals(
        &self,
        verified: &[Verified],
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<ApprovalResult, EnrollmentError> {
        let current = self.recipients()?;
        let mut adds = Vec::new();
        let mut clear = Vec::new();
        let mut outcomes = Vec::new();
        for v in verified {
            let request = v.approval.request.clone();
            clear.push(v.rel_path.clone());
            // The already-a-recipient case, filtered here because the
            // shared body rightly refuses it: for `recipient add` it is an
            // error, for approval a success. The authoritative duplicate
            // check stays in that body and still catches a writer who took
            // the name or the key in the window.
            if let Some(existing) = current.iter().find(|r| r.pubkey == request.pubkey) {
                outcomes.push(ApprovalOutcome {
                    request,
                    label: existing.device.clone(),
                    added: false,
                });
                continue;
            }
            let label = v.approval.label().to_string();
            adds.push(Recipient {
                device: label.clone(),
                pubkey: request.pubkey.clone(),
            });
            outcomes.push(ApprovalOutcome {
                request,
                label,
                added: true,
            });
        }

        if adds.is_empty() {
            let commit = self.commit_pending_removal(
                &clear,
                CLEARED_COMMIT_MESSAGE,
                CLEARED_PUSH_CLAUSE,
                prompter,
            )?;
            return Ok(ApprovalResult {
                outcomes,
                reencrypted: 0,
                commit,
            });
        }

        let labels: Vec<&str> = adds.iter().map(|r| r.device.as_str()).collect();
        let message = format!("gage: recipient approve {} (reencrypt)", labels.join(", "));
        let (reencrypted, commit) = self.add_recipient_locked(
            &adds,
            &RecipientWrite {
                message: &message,
                delete_paths: &clear,
                push_clause: Some(APPROVAL_PUSH_CLAUSE),
            },
            identity,
            prompter,
        )?;
        Ok(ApprovalResult {
            outcomes,
            reencrypted,
            commit,
        })
    }

    /// Removes one pending request without granting anything, and returns
    /// it.
    ///
    /// It takes a prompter and no identity: refusing access proves
    /// nothing, so it needs no code and no unlock (anyone with git write
    /// access could delete the file anyway), and without a prompter a deny
    /// whose push failed would report success while the request stayed
    /// live everywhere else. It is still a write: the lock, the reset, the
    /// commit and the push are the ordinary ones.
    ///
    /// The id is resolved inside the lock, after the reset, so what it
    /// deletes is in the state it commits.
    pub fn deny_enrollment(
        &self,
        id: &str,
        prompter: &dyn Prompter,
    ) -> Result<PendingRequest, EnrollmentError> {
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            let request = self.resolve_enrollment(id)?;
            let name = request
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("listed requests have UTF-8 names");
            let rel = format!("{PENDING_DIR}/{name}");
            self.commit_pending_removal(&[rel], DENY_COMMIT_MESSAGE, DENIED_PUSH_CLAUSE, prompter)?;
            Ok(request)
        })
    }

    /// The write behind deny and an approval that granted nothing: delete
    /// the requests, prune, commit, push. No re-encryption, no trust
    /// question and no trust-cache regeneration: all three are about a
    /// recipient list that changed, and none did. Runs with the lock held.
    fn commit_pending_removal(
        &self,
        rel_paths: &[String],
        message: &str,
        push_clause: &str,
        prompter: &dyn Prompter,
    ) -> Result<String, EnrollmentError> {
        for rel in rel_paths {
            let full = gitrepo::safe_join(&self.path, rel)?;
            gitrepo::remove_if_present(&full, rel)?;
        }
        // Already writing and holding the lock: the whole of "pruning
        // rides the next write", in this commit, never a second one.
        self.prune_pending_enrollments()?;
        let commit = gitrepo::commit_all(&self.path, message)?;
        self.push_after_write_saying(prompter, push_clause);
        Ok(commit)
    }
}
