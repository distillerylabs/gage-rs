//! The age wrapper: bytes in, bytes out. It knows nothing about entries,
//! files, or what a plaintext means, which is what lets the entry layer,
//! `recipient add`'s re-encryption and cross-vault moves share it without
//! their semantics leaking down here. The rest of the library reaches age
//! only through this module (and `identityfile`, which builds the one kind
//! of age identity this module does not: a passphrase's).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use age::secrecy::SecretString;
use zeroize::Zeroizing;

use crate::crypto::identity::Identity;
use crate::error::{ErrorKind, HasKind};

/// The log2 scrypt cost identity files are wrapped at: a deliberate number,
/// not age's default by omission.
///
/// age's interactive default targets about a second; gage uses 19. This is
/// the only thing between a stolen identity file and an unlimited offline
/// brute force, so every step up doubles the attacker's cost for the same
/// passphrase. It stops at 19 because one-shot mode pays this cost on every
/// invocation, and a `gage show` that takes four seconds pushes people
/// toward weaker passphrases.
///
/// Raising it later needs no migration: each file records its own factor in
/// its scrypt stanza. A `const`, so nothing at runtime can move the shipped
/// number; `SCRYPT_WORK_FACTOR` is the copy the crypto path reads.
pub const SHIPPED_SCRYPT_WORK_FACTOR: u8 = 19;

/// The live work factor every passphrase recipient is built at, and the
/// only part of this a test can move (ADR-0024): the suites unlock
/// real vaults hundreds of times, which at the shipped cost is minutes
/// spent proving that scrypt still costs what scrypt costs.
///
/// The only writer is [`set_scrypt_work_factor_for_tests`], which exists
/// only in test builds. `only_the_test_hook_writes_the_work_factor` checks
/// that no other code stores to it and that it starts from the shipped
/// constant.
static SCRYPT_WORK_FACTOR: AtomicU8 = AtomicU8::new(SHIPPED_SCRYPT_WORK_FACTOR);

/// The most work gage will do on an identity file's *claimed* factor. Set
/// explicitly on every scrypt identity rather than inherited: age's own
/// ceiling is calibrated per machine, and a damaged or hostile file
/// claiming 2^30 must cost a bounded wait, not a hang. 22 leaves three
/// doublings of headroom above what gage writes.
pub const SCRYPT_MAX_WORK_FACTOR: u8 = 22;

/// The log2 scrypt cost an enrollment request is sealed at. Deliberately
/// **not** [`SHIPPED_SCRYPT_WORK_FACTOR`]: the two protect different things
/// against different attacks. Do not merge them.
///
/// A work factor buys cost per guess, which is what a passphrase a human
/// chose needs. An enrollment code is chosen by nobody: it is 16 Crockford
/// characters from the OS RNG, 80 bits (ADR-0039), and at 80
/// bits the guess count is already the defence at any factor. The extra
/// doublings would be paid in latency and buy nothing. Not zero, though:
/// the licence is a property of the generator, and the factor should still
/// hold up if a bug weakens it. 14 is about 32× cheaper than 19, and it is
/// what keeps the honest case (a mistyped code against a full `pending/`)
/// fast, which is the case ADR-0040 is calibrated for.
///
/// The coupling, recorded where someone would break it: the low factor is
/// licensed by the code being generated, uniform and 80 bits. If a chosen
/// code is ever accepted, or the code shortened, this goes back up in the
/// same change. Raising it needs no migration (each blob records its own
/// factor, and requests live a day), but it must stay at or below
/// [`ENROLLMENT_SCRYPT_MAX_WORK_FACTOR`].
pub const SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR: u8 = 14;

/// The live copy of [`SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR`]: what sealing
/// reads, and the only part a test can move (ADR-0025). Its own
/// atomic and its own setter are what keep the two factors independent:
/// sealing at [`SCRYPT_WORK_FACTOR`] would let a suite that lowered the
/// identity factor silently lower the seal's, hiding the calibration
/// ADR-0040 makes.
///
/// The only writer is [`set_enrollment_work_factor_for_tests`];
/// `only_the_test_hooks_write_the_work_factors` enforces it.
static ENROLLMENT_SCRYPT_WORK_FACTOR: AtomicU8 =
    AtomicU8::new(SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR);

/// The most work the enrollment open path will do on a request's *claimed*
/// factor (ADR-0026). Its own cap, not [`SCRYPT_MAX_WORK_FACTOR`]:
/// the identity file is local, while `pending/` is writable by any git
/// writer, and scrypt's memory is 1 KiB × N. At 22 one hostile file costs
/// 4 GiB and seconds; at 18, four doublings above what gage seals at
/// (age's own headroom), it costs 256 MiB and a third of a second.
pub const ENROLLMENT_SCRYPT_MAX_WORK_FACTOR: u8 = 18;

/// What a [`Recipient`] built for a passphrase renders as: it has no public
/// spelling, and must never be mistaken for a key.
pub const PASSPHRASE_SPELLING: &str = "<passphrase>";

/// Overrides the live work factor for the rest of the process and returns
/// the previous value. Test builds only; the shipped binary never enables
/// `test-support`, so it has no way to lower the factor. Tests call
/// [`crate::testsupport::fast_scrypt`] rather than this.
#[cfg(any(test, feature = "test-support"))]
pub fn set_scrypt_work_factor_for_tests(n: u8) -> u8 {
    SCRYPT_WORK_FACTOR.swap(n, Ordering::SeqCst)
}

/// Overrides the live enrollment work factor for the rest of the process
/// and returns the previous value: the twin of
/// [`set_scrypt_work_factor_for_tests`], with the same test-only rule.
#[cfg(any(test, feature = "test-support"))]
pub fn set_enrollment_work_factor_for_tests(n: u8) -> u8 {
    ENROLLMENT_SCRYPT_WORK_FACTOR.swap(n, Ordering::SeqCst)
}

/// The factor a new enrollment seal is written at right now.
pub fn enrollment_scrypt_work_factor() -> u8 {
    ENROLLMENT_SCRYPT_WORK_FACTOR.load(Ordering::SeqCst)
}

/// The factor a new passphrase recipient is built at right now.
pub fn scrypt_work_factor() -> u8 {
    SCRYPT_WORK_FACTOR.load(Ordering::SeqCst)
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file nobody can read is never produced. age refuses too; failing
    /// here keeps the reason gage's own.
    #[error("refusing to encrypt to zero recipients")]
    NoRecipients,
    /// The identity file's "always exactly one recipient" property
    /// rests on this, so gage states it rather than inheriting age's.
    #[error(
        "a passphrase recipient must be a file's only recipient (asked to combine it with {others} other recipient(s))"
    )]
    MixedScryptRecipient { others: usize },
    /// Not an age recipient this build can encrypt to: malformed, a secret
    /// key, or a plugin recipient (legal in `.age-recipients`, but a plugin
    /// method is a later milestone's, not something to fake).
    #[error("{recipient:?} is not a usable age recipient: {reason}")]
    InvalidRecipient {
        recipient: String,
        reason: &'static str,
    },
    /// The identity offered isn't among the file's recipients: distinct
    /// from a damaged file, because the two mean very different things to
    /// a user (clone leans on exactly this).
    #[error("this identity is not a recipient of that ciphertext")]
    NotARecipient,
    /// The bytes aren't an intact age file: a bad header, a failed MAC, a
    /// truncated or tampered payload.
    #[error("ciphertext is corrupt or truncated: {0}")]
    CorruptCiphertext(String),
    #[error("encrypting: {0}")]
    Encrypt(String),
}

impl HasKind for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::NoRecipients
            | Error::MixedScryptRecipient { .. }
            | Error::InvalidRecipient { .. } => ErrorKind::Usage,
            Error::NotARecipient => ErrorKind::LockedOrAuth,
            Error::CorruptCiphertext(_) => ErrorKind::Conflict,
            Error::Encrypt(_) => ErrorKind::Internal,
        }
    }
}

/// One recipient gage can encrypt to. Its fields are private and it has no
/// `Default`, so the only ways to get one are [`Recipient::parse`] and
/// [`Recipient::passphrase`], so there is no zero value to refuse:
///
/// ```compile_fail,E0599
/// let r = gage::crypto::crypt::Recipient::default();
/// ```
///
/// ```compile_fail,E0451
/// let r = gage::crypto::crypt::Recipient { kind: todo!(), spelling: String::new() };
/// ```
///
/// The literal names every field, and `todo!()` stands in for the one whose
/// type is private, so the fields' privacy is the only thing left for it to
/// fail on: a missing field or an unnameable type can't pass it for the
/// wrong reason. Meanwhile the type and its constructors are reachable from
/// the same place (stable rustdoc doesn't check the codes above; this is
/// what rules out a typo in the path):
///
/// ```
/// let r = gage::crypto::crypt::Recipient::parse(
///     "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m",
/// );
/// assert!(r.is_ok());
/// ```
pub struct Recipient {
    kind: Kind,
    spelling: String,
}

enum Kind {
    X25519(age::x25519::Recipient),
    Passphrase(age::scrypt::Recipient),
}

impl Recipient {
    /// Parses an `age1…` X25519 public key.
    pub fn parse(s: &str) -> Result<Recipient, Error> {
        let key =
            s.parse::<age::x25519::Recipient>()
                .map_err(|reason| Error::InvalidRecipient {
                    recipient: s.to_string(),
                    reason,
                })?;
        Ok(Recipient {
            kind: Kind::X25519(key),
            spelling: s.to_string(),
        })
    }

    /// age's scrypt recipient for `passphrase`, at the live work factor.
    pub fn passphrase(passphrase: &str) -> Recipient {
        Recipient::passphrase_at(passphrase, scrypt_work_factor())
    }

    /// The one constructor of an age scrypt recipient, with the factor
    /// spelled out: what lets enrollment seal at its own factor without
    /// reading the identity file's.
    pub(crate) fn passphrase_at(passphrase: &str, work_factor: u8) -> Recipient {
        let mut r = age::scrypt::Recipient::new(SecretString::from(passphrase.to_string()));
        r.set_work_factor(work_factor);
        Recipient {
            kind: Kind::Passphrase(r),
            spelling: PASSPHRASE_SPELLING.to_string(),
        }
    }

    fn is_passphrase(&self) -> bool {
        matches!(self.kind, Kind::Passphrase(_))
    }

    fn as_age(&self) -> &dyn age::Recipient {
        match &self.kind {
            Kind::X25519(r) => r,
            Kind::Passphrase(r) => r,
        }
    }
}

/// The key as given, or [`PASSPHRASE_SPELLING`]: safe for error messages.
impl std::fmt::Display for Recipient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.spelling)
    }
}

impl std::fmt::Debug for Recipient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Recipient({})", self.spelling)
    }
}

/// Encrypts `plaintext` to every recipient in `to`, returning a complete
/// binary age file.
pub fn encrypt(plaintext: &[u8], to: &[Recipient]) -> Result<Vec<u8>, Error> {
    if to.is_empty() {
        return Err(Error::NoRecipients);
    }
    if to.len() > 1 && to.iter().any(Recipient::is_passphrase) {
        return Err(Error::MixedScryptRecipient {
            others: to.len() - 1,
        });
    }
    let fail = |e: &dyn std::fmt::Display| Error::Encrypt(e.to_string());
    let encryptor =
        age::Encryptor::with_recipients(to.iter().map(Recipient::as_age)).map_err(|e| fail(&e))?;
    let mut out = Vec::new();
    let mut writer = encryptor.wrap_output(&mut out).map_err(|e| fail(&e))?;
    writer.write_all(plaintext).map_err(|e| fail(&e))?;
    // finish encrypts and flushes the last chunk: its error is the
    // write's, and without it the file is truncated.
    writer.finish().map_err(|e| fail(&e))?;
    Ok(out)
}

/// Decrypts `ciphertext` with `id`'s private key. [`Error::NotARecipient`]
/// when `id` isn't one of the file's recipients, [`Error::CorruptCiphertext`]
/// when the bytes aren't an intact age file; never a panic and never
/// partial plaintext. A dropped identity can't be passed at all:
///
/// ```compile_fail,E0382
/// # fn f(vault: &gage::vault::Vault, p: &dyn gage::prompter::Prompter) {
/// let id = vault.unlock(p).unwrap();
/// drop(id);
/// let _ = gage::crypto::crypt::decrypt(b"", &id);
/// # }
/// ```
pub fn decrypt(ciphertext: &[u8], id: &Identity) -> Result<Vec<u8>, Error> {
    decrypt_with(ciphertext, &id.x25519()).map_err(|failure| match failure.cause {
        Cause::Header(age::DecryptError::NoMatchingKeys) => Error::NotARecipient,
        cause => Error::CorruptCiphertext(cause.to_string()),
    })
}

/// Why [`decrypt_with`] produced nothing, unclassified: entry decryption
/// and identity-file unlock draw different lines through the same causes.
#[derive(Debug)]
pub(crate) struct DecryptFailure {
    /// The header parsed and holds one scrypt stanza and nothing else.
    /// False when the header didn't parse at all.
    pub scrypt_only: bool,
    pub cause: Cause,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Cause {
    /// The header didn't parse, didn't authenticate, or no identity
    /// matched a stanza.
    #[error("{0}")]
    Header(age::DecryptError),
    /// The key matched but the payload didn't authenticate or ended early:
    /// age authenticates per chunk, so this is damage, never a wrong key.
    #[error("{0}")]
    Payload(std::io::Error),
}

/// The one place ciphertext becomes plaintext, shared by [`decrypt`] and
/// `Vault::unlock` (a passphrase identity, before any [`Identity`] exists).
/// Nothing partial is ever returned: the buffer a failed read was filling
/// is wiped and dropped.
pub(crate) fn decrypt_with(
    ciphertext: &[u8],
    identity: &dyn age::Identity,
) -> Result<Vec<u8>, DecryptFailure> {
    decrypt_with_limit(ciphertext, identity, None)
        .map(|plaintext| plaintext.expect("an unlimited read is never over its limit"))
}

/// [`decrypt_with`] holding at most `limit` bytes of plaintext. `Ok(None)`
/// is a payload over the limit, reported apart from damage so the caller
/// picks its own error; nothing partial is returned either way.
///
/// The limit exists for enrollment: age's plaintext isn't bounded by its
/// ciphertext, and the party who can produce a large one is a code-holder,
/// trusted enough to be granted access, not to be handed an unbounded
/// allocation. It reads one byte past the limit, so a payload exactly on
/// it is accepted and the first byte over is seen without reading the rest.
pub(crate) fn decrypt_with_limit(
    ciphertext: &[u8],
    identity: &dyn age::Identity,
    limit: Option<usize>,
) -> Result<Option<Vec<u8>>, DecryptFailure> {
    let header = |cause, scrypt_only| DecryptFailure {
        scrypt_only,
        cause: Cause::Header(cause),
    };
    let decryptor = age::Decryptor::new_buffered(ciphertext).map_err(|e| header(e, false))?;
    let scrypt_only = decryptor.is_scrypt();
    let mut reader = decryptor
        .decrypt(std::iter::once(identity))
        .map_err(|e| header(e, scrypt_only))?;
    let mut out = Zeroizing::new(Vec::new());
    let read = match limit {
        Some(limit) => (&mut reader).take(limit as u64 + 1).read_to_end(&mut out),
        None => reader.read_to_end(&mut out),
    };
    read.map_err(|e| DecryptFailure {
        scrypt_only,
        cause: Cause::Payload(e),
    })?;
    if limit.is_some_and(|limit| out.len() > limit) {
        return Ok(None);
    }
    Ok(Some(std::mem::take(&mut *out)))
}

/// An age scrypt identity for an identity file's passphrase, capped at
/// [`SCRYPT_MAX_WORK_FACTOR`].
pub(crate) fn scrypt_identity(passphrase: &str) -> age::scrypt::Identity {
    capped_scrypt_identity(passphrase, SCRYPT_MAX_WORK_FACTOR)
}

/// An age scrypt identity for an enrollment code, capped at
/// [`ENROLLMENT_SCRYPT_MAX_WORK_FACTOR`]: a hostile `pending/` file
/// claiming more is refused before any KDF runs.
pub(crate) fn enrollment_scrypt_identity(code: &str) -> age::scrypt::Identity {
    capped_scrypt_identity(code, ENROLLMENT_SCRYPT_MAX_WORK_FACTOR)
}

/// The one constructor of an age scrypt identity in the library, so a cap
/// is always set and set in one place: every passphrase decryption goes
/// through here, with one of the two named caps.
fn capped_scrypt_identity(passphrase: &str, max_work_factor: u8) -> age::scrypt::Identity {
    let mut id = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
    id.set_max_work_factor(max_work_factor);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair() -> (age::x25519::Identity, Recipient) {
        let key = age::x25519::Identity::generate();
        let recipient = Recipient::parse(&key.to_public().to_string()).unwrap();
        (key, recipient)
    }

    fn wrap(key: &age::x25519::Identity) -> Identity {
        Identity::for_tests(key)
    }

    /// The `-> scrypt <salt> <logN>` stanza's logN, read straight off the
    /// header, so the factor is checked on real ciphertext rather than by
    /// timing anything or reaching into age.
    pub(crate) fn scrypt_stanza_work_factor(file: &[u8]) -> Option<u8> {
        let header = String::from_utf8_lossy(file);
        header.lines().find_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields[..] {
                ["->", "scrypt", _salt, n] => n.parse().ok(),
                _ => None,
            }
        })
    }

    fn decrypt_scrypt(ct: &[u8], passphrase: &str) -> Vec<u8> {
        age::decrypt(&scrypt_identity(passphrase), ct).unwrap()
    }

    // A NUL and a
    // multi-byte character: the wrapper must not be quietly text-oriented.
    #[test]
    fn encrypt_decrypt_round_trip_is_byte_identical() {
        let (key, recipient) = keypair();
        let payload = "correcthorsebatterystaple\0\nsecond line\nπ".as_bytes();
        let ct = encrypt(payload, &[recipient]).unwrap();
        assert!(
            !ct.windows(payload.len()).any(|w| w == payload),
            "the plaintext appears verbatim in the ciphertext"
        );
        assert_eq!(decrypt(&ct, &wrap(&key)).unwrap(), payload);
    }

    #[test]
    fn encrypt_to_empty_payload_round_trips() {
        let (key, recipient) = keypair();
        let ct = encrypt(&[], &[recipient]).unwrap();
        assert!(decrypt(&ct, &wrap(&key)).unwrap().is_empty());
    }

    // N recipients are N
    // independently sufficient keys, not a quorum.
    #[test]
    fn each_recipient_can_decrypt_independently() {
        let pairs: Vec<_> = (0..3).map(|_| keypair()).collect();
        let (keys, recipients): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        let payload = b"shared across every recipient";
        let ct = encrypt(payload, &recipients).unwrap();
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(decrypt(&ct, &wrap(key)).unwrap(), payload, "recipient {i}");
        }
    }

    #[test]
    fn passphrase_recipient_alone_round_trips() {
        crate::testsupport::fast_scrypt();
        let payload = b"wrapped by a passphrase alone";
        let ct = encrypt(payload, &[Recipient::passphrase("hunter2")]).unwrap();
        assert_eq!(decrypt_scrypt(&ct, "hunter2"), payload);
    }

    #[test]
    fn encrypt_to_zero_recipients_is_refused() {
        let err = encrypt(b"nobody could read this", &[]).unwrap_err();
        assert!(matches!(err, Error::NoRecipients), "{err}");
        assert_eq!(err.kind(), ErrorKind::Usage);
    }

    // A passphrase recipient cannot be combined with another. Both orderings, and
    // two passphrases: gage refuses before age is reached either way.
    #[test]
    fn scrypt_recipient_cannot_be_combined() {
        crate::testsupport::fast_scrypt();
        let cases: [(&str, Vec<Recipient>); 3] = [
            (
                "passphrase first",
                vec![Recipient::passphrase("hunter2"), keypair().1],
            ),
            (
                "passphrase second",
                vec![keypair().1, Recipient::passphrase("hunter2")],
            ),
            (
                "two passphrases",
                vec![
                    Recipient::passphrase("hunter2"),
                    Recipient::passphrase("hunter2"),
                ],
            ),
        ];
        for (name, to) in cases {
            let err = encrypt(b"x", &to).unwrap_err();
            assert!(
                matches!(err, Error::MixedScryptRecipient { others: 1 }),
                "{name}: {err}"
            );
            assert_eq!(err.kind(), ErrorKind::Usage, "{name}");
        }
    }

    #[test]
    fn parse_recipient_rejects_non_encryptable_strings() {
        for s in [
            "",
            "not-an-age-key",
            // Legal in .age-recipients, not encryptable by this build.
            "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0",
            // A private key is not a recipient.
            "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ",
        ] {
            let err = Recipient::parse(s).unwrap_err();
            assert!(
                matches!(&err, Error::InvalidRecipient { recipient, .. } if recipient == s),
                "{s:?}: {err}"
            );
            assert_eq!(err.kind(), ErrorKind::Usage);
        }
    }

    // A recipient renders as its parsed spelling, and a passphrase
    // recipient: it renders as a label, never a key.
    #[test]
    fn recipient_string_renders_the_parsed_spelling() {
        crate::testsupport::fast_scrypt();
        let spelling = age::x25519::Identity::generate().to_public().to_string();
        assert_eq!(Recipient::parse(&spelling).unwrap().to_string(), spelling);
        let pass = Recipient::passphrase("hunter2");
        assert_eq!(pass.to_string(), PASSPHRASE_SPELLING);
        assert!(!format!("{pass:?}").contains("hunter2"));
    }

    // A wrong key never looks
    // like a damaged file.
    #[test]
    fn decrypt_with_a_non_recipient_fails_typed() {
        let (_, recipient) = keypair();
        let (stranger, _) = keypair();
        let ct = encrypt(b"not for you", &[recipient]).unwrap();
        let err = decrypt(&ct, &wrap(&stranger)).unwrap_err();
        assert!(matches!(err, Error::NotARecipient), "{err}");
        assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    }

    fn flip(ct: &[u8], i: usize) -> Vec<u8> {
        let mut out = ct.to_vec();
        out[i] ^= 0xff;
        out
    }

    // Damage across the whole
    // file, failing at different stages inside age; none may panic or
    // return plaintext (a Result carries none on Err by construction).
    // Some cases accept either error, since a damaged recipient stanza is
    // indistinguishable from someone else's; the others, which
    // damage nothing a recipient match depends on, must be corrupt.
    #[test]
    fn corrupted_ciphertext_fails_cleanly() {
        let (key, recipient) = keypair();
        let ct = encrypt(b"intact payload, for now", &[recipient]).unwrap();
        let header_end = ct.windows(4).position(|w| w == b"\n---").unwrap();
        let either: [(&str, Vec<u8>); 2] = [
            ("first byte flipped", flip(&ct, 0)),
            ("header byte flipped", flip(&ct, ct.len() / 4)),
        ];
        let corrupt: [(&str, Vec<u8>); 6] = [
            ("payload byte flipped", flip(&ct, ct.len() - 2)),
            ("truncated to header", ct[..ct.len() / 2].to_vec()),
            ("truncated mid-payload", ct[..ct.len() - 5].to_vec()),
            (
                "truncated before the MAC line ends",
                ct[..header_end + 6].to_vec(),
            ),
            ("truncated to nothing", Vec::new()),
            ("not an age file", b"this is just some text".to_vec()),
        ];
        for (name, damaged) in &either {
            let err = decrypt(damaged, &wrap(&key)).unwrap_err();
            assert!(
                matches!(err, Error::CorruptCiphertext(_) | Error::NotARecipient),
                "{name}: {err}"
            );
        }
        for (name, damaged) in &corrupt {
            let err = decrypt(damaged, &wrap(&key)).unwrap_err();
            assert!(matches!(err, Error::CorruptCiphertext(_)), "{name}: {err}");
            assert_eq!(err.kind(), ErrorKind::Conflict, "{name}");
        }
    }

    // Checks the constant, not the
    // live value the test hook lowers.
    #[test]
    fn scrypt_work_factor_is_deliberate() {
        const AGE_DEFAULT: u8 = 18;
        assert_eq!(SHIPPED_SCRYPT_WORK_FACTOR, 19);
        const { assert!(SHIPPED_SCRYPT_WORK_FACTOR > AGE_DEFAULT) };
        // A cap below the shipped factor would lock gage out of its own
        // files.
        const { assert!(SCRYPT_MAX_WORK_FACTOR >= SHIPPED_SCRYPT_WORK_FACTOR) };
        assert_eq!(SCRYPT_MAX_WORK_FACTOR, 22);
    }

    // The shipped
    // constant, not the live value. 14 is licensed by the code being 80
    // generated bits (ADR-0040); the rest stop it being
    // "simplified" into the identity file's, and keep gage able to open
    // what it seals under its own cap (ADR-0026).
    #[test]
    fn enrollment_scrypt_work_factor_is_deliberate() {
        assert_eq!(SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR, 14);
        const { assert!(SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR != SHIPPED_SCRYPT_WORK_FACTOR) };
        assert_eq!(ENROLLMENT_SCRYPT_MAX_WORK_FACTOR, 18);
        const { assert!(ENROLLMENT_SCRYPT_MAX_WORK_FACTOR >= SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR) };
        const { assert!(ENROLLMENT_SCRYPT_MAX_WORK_FACTOR <= SCRYPT_MAX_WORK_FACTOR) };
    }

    // The constant can be right
    // and never reach age: dropping the set_work_factor call would write
    // every file at age's calibrated default while everything else stayed
    // green.
    #[test]
    fn scrypt_work_factor_is_actually_applied() {
        crate::testsupport::fast_scrypt();
        let ct = encrypt(b"x", &[Recipient::passphrase("hunter2")]).unwrap();
        assert_eq!(scrypt_stanza_work_factor(&ct), Some(scrypt_work_factor()));
    }

    const SHIPPED_ENV: &str = "GAGE_CRYPT_TEST_SHIPPED_FACTOR";

    /// The child side of the shipped-factor test: a process that never
    /// called the test hook encrypts at whatever the live factor starts
    /// at and reports the stanza's factor and whether it reads back.
    #[test]
    #[allow(clippy::disallowed_macros)]
    fn shipped_factor_trampoline() {
        if std::env::var_os(SHIPPED_ENV).is_none() {
            return;
        }
        let payload = b"wrapped at the factor gage actually ships";
        let ct = encrypt(payload, &[Recipient::passphrase("hunter2")]).unwrap();
        let factor = scrypt_stanza_work_factor(&ct).unwrap();
        let reads_back = decrypt_scrypt(&ct, "hunter2") == payload;
        eprintln!("FACTOR {factor} {reads_back}");
    }

    // The one test that observes the shipped cost reaching a real header.
    // Lowering and restoring the live value around one encryption would
    // race every other scrypt test: tests share one process and run in
    // parallel. A child process that never calls the hook is the same proof
    // without the race: the live factor's *initial* value is the shipped
    // one, it reaches age, and the cap admits it on a real file.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn shipped_work_factor_reaches_a_real_age_file() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "crypto::crypt::tests::shipped_factor_trampoline",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(SHIPPED_ENV, "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        let line = stderr
            .lines()
            .find(|l| l.starts_with("FACTOR "))
            .unwrap_or_else(|| panic!("the child reported nothing:\n{stderr}"));
        assert_eq!(line, format!("FACTOR {SHIPPED_SCRYPT_WORK_FACTOR} true"));
    }

    /// This crate's non-test source: `#[cfg(test)] mod tests` blocks cut
    /// off (they sit at the end of each file by convention, and a test may
    /// mention what it checks for), and `*_tests.rs` files, which are
    /// compiled only under `cfg(test)`. Panics if the walk finds nothing, so
    /// the scans below can't pass by looking in the wrong place.
    /// Where `crypt.rs` sits under `src/`, `/`-separated on every platform.
    const CRYPT_SOURCE: &str = "crypto/crypt.rs";

    fn library_sources() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.to_string_lossy().ends_with("_tests.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = match text.find("#[cfg(test)]\nmod tests") {
                        Some(i) => text[..i].to_string(),
                        None => text,
                    };
                    let name = path
                        .strip_prefix(&root)
                        .unwrap()
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    files.push((name, code));
                }
            }
        }
        assert!(
            files.iter().any(|(n, _)| n == CRYPT_SOURCE),
            "the source walk found no {CRYPT_SOURCE}; it is not looking where it thinks it is"
        );
        files
    }

    /// The body of `fn name`, by brace matching from its signature.
    fn fn_body<'a>(code: &'a str, name: &str) -> Option<&'a str> {
        let start = code.find(&format!("fn {name}("))?;
        let open = start + code[start..].find('{')?;
        let mut depth = 0;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&code[open..=open + i]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Byte offsets of `ident` as a whole identifier (so `SCRYPT_WORK_FACTOR`
    /// doesn't match inside `SHIPPED_SCRYPT_WORK_FACTOR`).
    fn occurrences(code: &str, ident: &str) -> Vec<usize> {
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        code.match_indices(ident)
            .map(|(i, _)| i)
            .filter(|&i| {
                let before = code[..i].chars().next_back();
                let after = code[i + ident.len()..].chars().next();
                !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
            })
            .collect()
    }

    // Only the test hooks write the work factors. The
    // shipped constants can't move, but the live atomics can, and one
    // stray `SCRYPT_WORK_FACTOR.store(10, ..)` in library code would ship a
    // weakened KDF every other guard misses. So every mention of each
    // static outside comments must be its one declaration (initialized from
    // its own shipped constant, which also catches a rewritten initializer
    // or the two being cross-wired), a `.load(` read, or inside its own
    // test-only setter. A whitelist, not a pattern for writes: an alias
    // (`let f = &SCRYPT_WORK_FACTOR;`) is refused too.
    #[test]
    fn only_the_test_hooks_write_the_work_factors() {
        let hooks = [
            (
                "SCRYPT_WORK_FACTOR",
                "set_scrypt_work_factor_for_tests",
                "static SCRYPT_WORK_FACTOR: AtomicU8 = AtomicU8::new(SHIPPED_SCRYPT_WORK_FACTOR);",
            ),
            (
                "ENROLLMENT_SCRYPT_WORK_FACTOR",
                "set_enrollment_work_factor_for_tests",
                "static ENROLLMENT_SCRYPT_WORK_FACTOR: AtomicU8 = \
                 AtomicU8::new(SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR);",
            ),
        ];
        let sources = library_sources();
        let crypt = &sources.iter().find(|(n, _)| n == CRYPT_SOURCE).unwrap().1;
        for (stat, setter, declaration) in hooks {
            let mut declarations = 0;
            for (name, code) in &sources {
                let setter_body = fn_body(code, setter).map(|body| {
                    let start = body.as_ptr() as usize - code.as_ptr() as usize;
                    start..start + body.len()
                });
                for at in occurrences(code, stat) {
                    let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
                    let line = code[line_start..].lines().next().unwrap().trim();
                    if line.starts_with("//") {
                        continue;
                    }
                    // The whole statement, whitespace collapsed, since
                    // rustfmt wraps a long declaration.
                    let stmt = &code[line_start..];
                    let stmt = stmt[..stmt.find(';').map_or(stmt.len(), |i| i + 1)]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if stmt == declaration {
                        declarations += 1;
                        continue;
                    }
                    if code[at + stat.len()..].starts_with(".load(") {
                        continue;
                    }
                    if setter_body.as_ref().is_some_and(|r| r.contains(&at)) && name == CRYPT_SOURCE
                    {
                        continue;
                    }
                    panic!(
                        "{name}: `{line}` touches {stat} outside {setter}; only the test hook may \
                         write it, or a weakened KDF ships invisibly"
                    );
                }
            }
            assert_eq!(
                declarations, 1,
                "want exactly one declaration of {stat}: `{declaration}`"
            );
            // And the setter really is test-only.
            let at = crypt.find(&format!("pub fn {setter}(")).unwrap();
            assert!(
                crypt[..at]
                    .trim_end()
                    .ends_with(r#"#[cfg(any(test, feature = "test-support"))]"#),
                "{setter} must exist only under cfg(any(test, feature = \"test-support\"))"
            );
        }
    }

    // Every scrypt identity caps its work factor. A scrypt
    // identity built without a cap inherits age's per-machine one, and the
    // next decryption path would inherit a limit nobody chose. There is exactly one
    // construction site in the library, it sets the cap it is given, and
    // every call of it passes one of the two named caps
    // (ADR-0026): the identity file's, or the seal's.
    #[test]
    fn every_scrypt_identity_caps_its_work_factor() {
        const CONSTRUCTOR: &str = "capped_scrypt_identity";
        let mut sites = Vec::new();
        for (name, code) in library_sources() {
            for at in code.match_indices("scrypt::Identity::new").map(|(i, _)| i) {
                let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
                if code[line_start..at].trim_start().starts_with("//") {
                    continue;
                }
                sites.push((name.clone(), at, code.clone()));
            }
        }
        assert_eq!(
            sites.len(),
            1,
            "want exactly one construction of an age scrypt identity, found in {:?}",
            sites.iter().map(|(n, ..)| n).collect::<Vec<_>>()
        );
        let (name, _, code) = &sites[0];
        let body = fn_body(code, CONSTRUCTOR)
            .unwrap_or_else(|| panic!("{name}: the construction is not in fn {CONSTRUCTOR}"));
        assert!(body.contains("scrypt::Identity::new"), "{name}: {body}");
        assert!(
            body.contains("set_max_work_factor(max_work_factor)"),
            "{name}: {CONSTRUCTOR} builds an identity without capping its work factor"
        );

        let mut calls = Vec::new();
        for (name, code) in library_sources() {
            for at in occurrences(&code, CONSTRUCTOR) {
                let rest = &code[at + CONSTRUCTOR.len()..];
                if code[..at].ends_with("fn ") || !rest.starts_with('(') {
                    continue;
                }
                let args = &rest[..rest.find(')').unwrap()];
                let cap = args.rsplit(',').next().unwrap().trim();
                assert!(
                    [
                        "SCRYPT_MAX_WORK_FACTOR",
                        "ENROLLMENT_SCRYPT_MAX_WORK_FACTOR"
                    ]
                    .contains(&cap),
                    "{name}: {CONSTRUCTOR}({args}) caps at {cap:?}, not one of the named caps"
                );
                calls.push(cap.to_string());
            }
        }
        calls.sort();
        assert_eq!(
            calls,
            [
                "ENROLLMENT_SCRYPT_MAX_WORK_FACTOR",
                "SCRYPT_MAX_WORK_FACTOR"
            ],
            "want one capped constructor per decryption path: the identity file and the seal"
        );
    }

    // And the passphrase recipient side of the same rule: one construction
    // site, which sets the factor it was given.
    #[test]
    fn every_scrypt_recipient_sets_its_work_factor() {
        let crypt = library_sources()
            .into_iter()
            .find(|(n, _)| n == CRYPT_SOURCE)
            .unwrap()
            .1;
        let all: usize = library_sources()
            .iter()
            .map(|(_, c)| c.matches("scrypt::Recipient::new").count())
            .sum();
        assert_eq!(
            all, 1,
            "want exactly one construction of an age scrypt recipient"
        );
        let body = fn_body(&crypt, "passphrase_at").unwrap();
        assert!(body.contains("scrypt::Recipient::new"), "{body}");
        assert!(body.contains("set_work_factor(work_factor)"), "{body}");
    }
}
