//! Fixtures shared by this crate's tests and other crates' (gage-cli
//! enables `test-support` as a dev-dependency; the shipped binary never
//! does).

use std::sync::Once;

use jiff::Timestamp;

use crate::crypto::crypt;
use crate::vault::Vault;

pub mod format_v2;

/// What tests wrap and unlock identities at instead of the shipped,
/// deliberately expensive factor. Still a real scrypt pass, so every test
/// that unlocks a vault exercises the real wrap/unwrap path, at a cost too
/// small to notice.
pub const TEST_SCRYPT_WORK_FACTOR: u8 = 10;

/// Lowers both live scrypt work factors, the identity file's and the
/// enrollment seal's, to [`TEST_SCRYPT_WORK_FACTOR`] for the rest of the
/// process; idempotent. Rust has no `TestMain`, so every
/// fixture that creates or unlocks an identity calls this first. A test
/// that forgets is slow, not wrong. The shipped factor stays guarded by
/// `crypt`'s own tests, which check the constant and read it back off a
/// real file written by a process that never called this.
pub fn fast_scrypt() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        crypt::set_scrypt_work_factor_for_tests(TEST_SCRYPT_WORK_FACTOR);
        crypt::set_enrollment_work_factor_for_tests(TEST_SCRYPT_WORK_FACTOR);
    });
}

/// Seals a request for `vault` with exactly these fields, unvalidated (a
/// device name carrying escape sequences, an expiry already past under the
/// seal), and files it under a filename expiring at `filed_expires`,
/// uncommitted. Returns the request's id and its code. For frontends'
/// tests of what a hostile or broken request does to their screens; the
/// library's own tests seal through its fixtures.
pub fn file_unchecked_request(
    vault: &Vault,
    device: &str,
    pubkey: &str,
    created: Timestamp,
    expires: Timestamp,
    filed_expires: Timestamp,
) -> (String, String) {
    crate::enrollment::file_unchecked_request(
        vault,
        device,
        pubkey,
        created,
        expires,
        filed_expires,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The hook lowers the live factor,
    // and calling it again changes nothing.
    #[test]
    fn fast_scrypt_lowers_the_live_factor_and_is_idempotent() {
        fast_scrypt();
        assert_eq!(crypt::scrypt_work_factor(), TEST_SCRYPT_WORK_FACTOR);
        assert_eq!(
            crypt::enrollment_scrypt_work_factor(),
            TEST_SCRYPT_WORK_FACTOR
        );
        fast_scrypt();
        assert_eq!(crypt::scrypt_work_factor(), TEST_SCRYPT_WORK_FACTOR);
        assert_eq!(
            crypt::enrollment_scrypt_work_factor(),
            TEST_SCRYPT_WORK_FACTOR
        );
        const { assert!(TEST_SCRYPT_WORK_FACTOR < crypt::SHIPPED_SCRYPT_WORK_FACTOR) };
    }
}
