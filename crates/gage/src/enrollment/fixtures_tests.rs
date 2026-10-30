//! The fixtures every enrollment test builds on. They write into `pending/`
//! directly rather than through a command, so the primitives are proven
//! before anything is layered on them.

use std::path::PathBuf;
use std::time::Duration;

use jiff::Timestamp;
use zeroize::Zeroizing;

use super::seal::{Payload, seal_payload};
use super::{EnrollmentError, EnrollmentRequest, OpenedRequest, code};
use crate::storage::vaultconfig::UnlockMethod;
use crate::storage::vaultconfig::VaultId;
use crate::trust::recipient::recipient_tests::Fixture;
use crate::vault::Vault;
use crate::{storage::config, storage::paths};

/// One vault, its owner's key, and nothing pending.
pub(crate) fn vault() -> Fixture {
    Fixture::new()
}

/// A fresh age public key: what a joining device puts in a request.
pub(crate) fn new_pubkey() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

/// A freshly generated code in its display form.
pub(crate) fn new_code() -> String {
    code::generate_code().unwrap().to_string()
}

/// One typed code, as the open path takes it.
pub(crate) fn codes(typed: &str) -> Vec<Zeroizing<String>> {
    vec![Zeroizing::new(typed.to_string())]
}

/// Now, plus or minus whole hours, at the second resolution gage writes.
pub(crate) fn hours(n: i64) -> Timestamp {
    let now = Timestamp::now().as_second();
    Timestamp::from_second(now + n * 3600).unwrap()
}

pub(crate) const DAY: Duration = Duration::from_secs(24 * 3600);

/// Drops raw bytes into `pending/` under `name`, creating the directory as
/// the first enroll does: the move any git writer can make.
pub(crate) fn write_pending(vault: &Vault, name: &str, content: &[u8]) -> PathBuf {
    let dir = vault.pending_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Seals a request for `vault` and files it under the name gage gives it.
pub(crate) fn seal_into(
    vault: &Vault,
    device: &str,
    pubkey: &str,
    ttl: Duration,
) -> EnrollmentRequest {
    let (req, sealed) = vault
        .seal_enrollment(device, pubkey, &UnlockMethod::Passphrase, ttl)
        .unwrap();
    write_pending(
        vault,
        &super::name::pending_file_name(req.id.as_deref().unwrap(), req.expires),
        &sealed,
    );
    req
}

/// Seals a payload chosen field by field, which a git writer and a
/// code-holder can both do. `code` may be any accepted spelling.
pub(crate) fn hand_seal(payload: &Payload, code: &str) -> Vec<u8> {
    let normalized = code::normalize_code(code)
        .unwrap_or_else(|| panic!("{code:?} is not a valid code to seal under"));
    seal_payload(payload, &normalized).unwrap()
}

/// A payload for `vault` that passes every check, so a test can spoil one
/// field and know the refusal came from that field.
pub(crate) fn good_payload(
    vault: &Vault,
    id: &str,
    device: &str,
    pubkey: &str,
    created: Timestamp,
    expires: Timestamp,
) -> Payload {
    Payload {
        request_id: id.to_string(),
        vault_id: vault.id.to_string(),
        device: device.to_string(),
        pubkey: pubkey.to_string(),
        method: UnlockMethod::Passphrase,
        created: created.to_string(),
        expires: expires.to_string(),
    }
}

/// Hand-seals `payload` under a fresh code and files it as `id`, live for
/// another hour by its filename. Returns the code.
pub(crate) fn file_hand_sealed(vault: &Vault, id: &str, payload: &Payload) -> String {
    let code = new_code();
    write_pending(
        vault,
        &super::name::pending_file_name(id, hours(1)),
        &hand_seal(payload, &code),
    );
    code
}

/// The broad run: every live request, one code.
pub(crate) fn open_all(vault: &Vault, code: &str) -> Result<Vec<OpenedRequest>, EnrollmentError> {
    let pending = vault.pending_enrollments().unwrap();
    vault.open_enrollment(&pending, &codes(code))
}

/// Adds `name` → `id` to the registry `vault`'s device reads, leaving the
/// rest alone.
pub(crate) fn register_also(vault: &Vault, name: &str, id: &VaultId) {
    let path = paths::global_config_file(vault.dirs());
    let mut global = config::load(&path).unwrap_or_default();
    global.vaults.insert(
        name.to_string(),
        config::VaultEntry {
            path: vault.path.with_file_name(name),
            id: Some(id.clone()),
            vault_type: crate::storage::vaultconfig::VaultType::Git,
            ..Default::default()
        },
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    config::save(&path, &global).unwrap();
}

/// Both halves of the contract at once: the variant a caller matches on
/// and the exit code the taxonomy gives it.
/// Evaluates to the error.
macro_rules! assert_enrollment_error {
    ($result:expr, $pat:pat, $code:expr) => {{
        let err = match $result {
            Err(e) => e,
            Ok(v) => panic!("succeeded with {v:?}; want {}", stringify!($pat)),
        };
        assert!(
            matches!(err, $pat),
            "error = {err:?} ({err}), want {}",
            stringify!($pat)
        );
        assert_eq!(
            $crate::error::HasKind::kind(&err),
            $code,
            "exit code for {err}"
        );
        err
    }};
}
pub(crate) use assert_enrollment_error;
