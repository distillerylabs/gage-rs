//! The seal: the payload, sealing it to a code, one attempt to open it, and
//! the validation that turns an authenticated payload into a well-formed
//! one. See "The reframe: the seal is authentication, not confidentiality"
//! and "The sealed payload is untrusted input".

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::code::{generate_code, normalize_code};
use super::name::expiry_live;
use super::{
    EnrollmentError, EnrollmentRequest, MAX_ENROLLMENT_TTL, MAX_PENDING_REQUEST_BYTES,
    OpenedRequest, PendingRequest,
};
use crate::crypt::{self, Recipient};
use crate::vault::Vault;
use crate::vaultconfig::{self, InvalidVaultId, UnlockMethod, VaultId};
use crate::{agekey, devicename};

/// The plaintext under the seal, as TOML.
///
/// Every field is a string, timestamps and the vault id included, and
/// absent means empty: what arrives is untrusted, so the fields are
/// validated after parsing, not by it, and a malformed timestamp or id is
/// a request gage refuses with a reason rather than a parse error it
/// reports as corruption. The vault id becomes a [`VaultId`] in
/// [`Vault::validate_payload`]. The method is an [`UnlockMethod`], which
/// takes any string, so it can't fail the parse either.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Payload {
    pub(crate) request_id: String,
    pub(crate) vault_id: String,
    pub(crate) device: String,
    pub(crate) pubkey: String,
    pub(crate) method: UnlockMethod,
    pub(crate) created: String,
    pub(crate) expires: String,
}

/// What one code did to one request's bytes.
#[derive(Debug)]
pub(crate) enum Attempt {
    /// The code is not this request's, or the bytes aren't an intact age
    /// file. The ordinary case for every code but one, and not an error: a
    /// damaged or hostile file mustn't take the run down.
    NotOpened,
    Opened(OpenedRequest),
    /// The code opened it and gage still refuses it.
    Refused(EnrollmentError),
}

/// Encrypts `payload` to one scrypt recipient derived from `code`, at
/// enrollment's own work factor.
///
/// `code` is the normalized form: every accepted spelling has to reach the
/// same passphrase, or a correctly retyped code wouldn't open its request.
/// What this produces is an authentication token, not a secret: the
/// plaintext is a public key and metadata any reader of the vault could
/// enumerate. The seal proves whoever wrote the blob knew a code agreed out
/// of band, and that the bytes haven't changed since.
pub(crate) fn seal_payload(payload: &Payload, code: &str) -> Result<Vec<u8>, EnrollmentError> {
    let plaintext = toml::to_string(payload).map_err(|e| EnrollmentError::Encode(e.to_string()))?;
    let to = Recipient::passphrase_at(code, crypt::enrollment_scrypt_work_factor());
    crypt::encrypt(plaintext.as_bytes(), &[to]).map_err(EnrollmentError::Seal)
}

impl Vault {
    /// Seals one enrollment request for this vault and returns it with the
    /// bytes to file under `pending/`. The request carries the generated
    /// code, the one thing a caller must display and this library must
    /// never persist.
    ///
    /// The TTL is validated here because a library a GUI could call
    /// directly can't rely on a command line having been checked first.
    ///
    /// The payload names the vault it is for, which closes the replay
    /// `request_id` doesn't: a blob is readable to every reader of its
    /// vault, so moving it costs nothing, and the case needs no attacker,
    /// just one person with two vaults and two codes on screen.
    pub(crate) fn seal_enrollment(
        &self,
        device: &str,
        pubkey: &str,
        method: &UnlockMethod,
        ttl: Duration,
    ) -> Result<(EnrollmentRequest, Vec<u8>), EnrollmentError> {
        if ttl.is_zero() || ttl > MAX_ENROLLMENT_TTL {
            return Err(EnrollmentError::InvalidTtl { ttl });
        }
        if !devicename::valid(device) {
            return Err(EnrollmentError::InvalidDevice {
                device: device.to_string(),
            });
        }
        agekey::validate_recipient(pubkey).map_err(EnrollmentError::InvalidPubkey)?;
        if !method.is_supported() {
            return Err(EnrollmentError::UnknownMethod {
                method: method.to_string(),
            });
        }

        let code = generate_code().map_err(EnrollmentError::Random)?;
        // Unreachable unless generator and normalizer disagree, which
        // generated_codes_normalize_to_themselves rules out; but a silent
        // mismatch would seal a request no code opens.
        let normalized = normalize_code(&code).ok_or(EnrollmentError::CodeMismatch)?;

        let created = Timestamp::from_second(Timestamp::now().as_second())
            .expect("now is a representable second");
        let expires = SignedDuration::try_from(ttl)
            .ok()
            .and_then(|ttl| created.checked_add(ttl).ok())
            .map(|t| Timestamp::from_second(t.as_second()).expect("whole seconds"))
            .ok_or(EnrollmentError::InvalidTtl { ttl })?;
        let id = Uuid::new_v4().to_string();
        let request = EnrollmentRequest {
            id: Some(id.clone()),
            device: device.to_string(),
            pubkey: pubkey.to_string(),
            method: method.clone(),
            created,
            expires,
            code,
            published: false,
        };
        let sealed = seal_payload(
            &Payload {
                request_id: id,
                vault_id: self.id.to_string(),
                device: request.device.clone(),
                pubkey: request.pubkey.clone(),
                method: request.method.clone(),
                created: created.to_string(),
                expires: expires.to_string(),
            },
            &normalized,
        )?;
        Ok((request, sealed))
    }

    /// One attempt to open one request's bytes with one normalized code,
    /// followed by the validation that makes an authenticated payload a
    /// well-formed one.
    ///
    /// The two halves are one function because the ordering is the
    /// security property: every field is checked before an
    /// [`OpenedRequest`] exists, so before anything in it is rendered to
    /// the human deciding whether to grant vault-wide access. The device
    /// name is printed directly above that `[y/N]`, so a name that can move
    /// the cursor could rewrite the question being answered.
    pub(crate) fn open_sealed(
        &self,
        request: &PendingRequest,
        sealed: &[u8],
        code: &str,
        now: Timestamp,
    ) -> Attempt {
        // The cap on the *claimed* factor (ADR-0026). Without
        // it, one committed file claiming 2^30 is a hang, not a slowdown.
        let identity = crypt::enrollment_scrypt_identity(code);
        self.enrollment_decrypts.bump();
        match crypt::decrypt_with_limit(sealed, &identity, Some(MAX_PENDING_REQUEST_BYTES)) {
            Err(_) => Attempt::NotOpened,
            // The code worked and the payload is larger than anything gage
            // writes. A code-holder is trusted enough to be granted access,
            // not to be handed an unbounded allocation.
            Ok(None) => Attempt::Refused(EnrollmentError::MalformedRequest {
                reason: format!(
                    "its decrypted payload is larger than {MAX_PENDING_REQUEST_BYTES} bytes"
                ),
            }),
            Ok(Some(plaintext)) => match self.validate_payload(request, &plaintext, now) {
                Ok(opened) => Attempt::Opened(opened),
                Err(e) => Attempt::Refused(e),
            },
        }
    }

    /// Everything that has to hold of an authenticated payload before it
    /// becomes an [`OpenedRequest`].
    ///
    /// Authenticated is not well-formed: two people can supply a malformed
    /// payload with no attack at all (a git writer dropping a file in, a
    /// code-holder sealing what they like). The three refusals after the
    /// field checks are distinct because they send a human to three
    /// places: damage to look at, a request to approve in another vault,
    /// a request to ask for again. None is a wrong code: the code worked.
    pub(crate) fn validate_payload(
        &self,
        request: &PendingRequest,
        plaintext: &[u8],
        now: Timestamp,
    ) -> Result<OpenedRequest, EnrollmentError> {
        let malformed = |reason: String| EnrollmentError::MalformedRequest { reason };
        let p: Payload = std::str::from_utf8(plaintext)
            .ok()
            .and_then(|text| toml::from_str(text).ok())
            .ok_or_else(|| malformed("its sealed contents are not valid TOML".into()))?;

        if !vaultconfig::valid_id(&p.request_id) {
            return Err(malformed(format!(
                "its sealed request id {:?} is not a uuid",
                p.request_id
            )));
        }
        let vault_id = VaultId::try_from(p.vault_id).map_err(|InvalidVaultId(id)| {
            malformed(format!("its sealed vault id {id:?} is not a uuid"))
        })?;
        // What keeps escape sequences and newlines out of a string shown
        // above a prompt granting vault-wide access; so here, not at
        // add_recipient two milestones and one confirmation later.
        if !devicename::valid(&p.device) {
            return Err(malformed(
                "its sealed device name is not a name gage would write".into(),
            ));
        }
        if agekey::validate_recipient(&p.pubkey).is_err() {
            return Err(malformed(
                "its sealed public key is not an age recipient".into(),
            ));
        }
        if !p.method.is_supported() {
            return Err(malformed(format!(
                "its sealed method {:?} is not one this build performs",
                p.method.as_str()
            )));
        }
        let created: Timestamp = p.created.parse().map_err(|_| {
            malformed(format!(
                "its sealed creation time {:?} is not an RFC 3339 timestamp",
                p.created
            ))
        })?;
        let expires: Timestamp = p.expires.parse().map_err(|_| {
            malformed(format!(
                "its sealed expiry {:?} is not an RFC 3339 timestamp",
                p.expires
            ))
        })?;

        // The filename is unauthenticated, the sealed id is not, so a blob
        // can't be renamed onto another slot. Only the UUID half: the epoch
        // is a hint, and holding it to the seal would refuse requests over
        // a lie that grants nothing.
        if p.request_id != request.id {
            return Err(EnrollmentError::IdMismatch {
                sealed: p.request_id,
                filed: request.id.clone(),
            });
        }
        if vault_id != self.id {
            return Err(EnrollmentError::WrongVault {
                vault: self.describe_vault_id(&vault_id),
                this: self.name.clone(),
            });
        }
        // Skew before expiry: a skewed request is also an expired one, and
        // the fixes share nothing. Sound with no reference clock: a request
        // whose expiry preceded its own creation was dead when written.
        if expires < created {
            return Err(EnrollmentError::ClockSkew { created, expires });
        }
        // The ceiling clamps the sealed expiry too, since this is the copy
        // approval enforces; and against now, not created, because both
        // sealed timestamps are the sealer's to choose.
        if !expiry_live(expires, now) {
            return Err(EnrollmentError::Expired { expires });
        }

        Ok(OpenedRequest {
            id: p.request_id,
            device: p.device,
            pubkey: p.pubkey,
            method: p.method,
            created,
            expires,
        })
    }
}
