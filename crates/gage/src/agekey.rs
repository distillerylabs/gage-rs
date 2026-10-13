//! Validates the shape of an age public key, an `age1...` Bech32 string
//! (BIP-173), without any cryptographic operation, so a malformed
//! `--recipient` fails at the boundary rather than as a confusing encrypt
//! failure later.
//!
//! Shape-only is deliberate, and weaker than parsing a key gage is about
//! to encrypt to: it accepts every recipient `.age-recipients` may legally
//! hold, including plugin recipients (`age1yubikey1...`) this build can't
//! encrypt to itself, so a vault shared with a device that uses one still
//! round-trips its config. That is also why this is not the `age` crate's
//! X25519 parser (Q-AGE-CRATE).

use crate::exitcode::{Code, HasCode};

/// Bech32's 32-character data alphabet.
const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// Bech32's own string-length cap (BIP-173).
const MAX_LENGTH: usize = 90;

/// The polymod target for standard Bech32 (Bech32m's differs); age
/// recipients use standard Bech32.
const CHECKSUM_CONST: u32 = 1;

/// Why a string is not a well-formed age recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    WrongLength,
    MixedCase,
    /// No `1` separating the human-readable part from the data, or too
    /// little data after it to hold a checksum.
    NoSeparator,
    /// The human-readable part doesn't start with `age`.
    NotAge(String),
    NonPrintableHrp,
    NotInCharset(char),
    Checksum,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::WrongLength => f.write_str("wrong length"),
            Reason::MixedCase => f.write_str("mixed case"),
            Reason::NoSeparator => f.write_str("missing separator or data too short"),
            Reason::NotAge(hrp) => {
                write!(f, "human-readable part {hrp:?} does not start with \"age\"")
            }
            Reason::NonPrintableHrp => {
                f.write_str("human-readable part contains non-printable characters")
            }
            Reason::NotInCharset(c) => write!(f, "character {c:?} is not in the Bech32 charset"),
            Reason::Checksum => f.write_str("checksum mismatch"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("agekey: {recipient:?} is not a valid age recipient: {reason}")]
pub struct Error {
    pub recipient: String,
    pub reason: Reason,
}

impl HasCode for Error {
    fn code(&self) -> Code {
        Code::Usage
    }
}

/// Whether `s` is a well-formed age recipient: valid Bech32 (charset,
/// single case, checksum) whose human-readable part starts with `age`.
/// That covers a plain X25519 recipient (hrp `age`) and a plugin one (hrp
/// `age1yubikey`: by Bech32's own rule the split is at the *last* `1`).
pub fn validate_recipient(s: &str) -> Result<(), Error> {
    check(s).map_err(|reason| Error {
        recipient: s.to_string(),
        reason,
    })
}

fn check(s: &str) -> Result<(), Reason> {
    if s.len() < 8 || s.len() > MAX_LENGTH {
        return Err(Reason::WrongLength);
    }
    let lower = s.to_lowercase();
    if lower != s && s.to_uppercase() != s {
        return Err(Reason::MixedCase);
    }
    let sep = match lower.rfind('1') {
        Some(i) if i >= 1 && i + 7 <= lower.len() => i,
        _ => return Err(Reason::NoSeparator),
    };
    let (hrp, data) = (&lower[..sep], &lower[sep + 1..]);
    if !hrp.starts_with("age") {
        return Err(Reason::NotAge(hrp.to_string()));
    }
    if !is_printable_ascii(hrp) {
        return Err(Reason::NonPrintableHrp);
    }
    let mut values = hrp_expand(hrp);
    for c in data.chars() {
        let value = CHARSET.find(c).ok_or(Reason::NotInCharset(c))?;
        // CHARSET is 32 ASCII characters, so a byte index is a value.
        values.push(value as u32);
    }
    if polymod(&values) != CHECKSUM_CONST {
        return Err(Reason::Checksum);
    }
    Ok(())
}

fn is_printable_ascii(s: &str) -> bool {
    s.chars().all(|c| ('!'..='~').contains(&c))
}

fn polymod(values: &[u32]) -> u32 {
    const GENERATOR: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ v;
        for (i, g) in GENERATOR.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// The human-readable part as polymod input: each character's high bits,
/// a zero, then each character's low bits (BIP-173). Only called on a
/// part [`is_printable_ascii`] accepted.
fn hrp_expand(hrp: &str) -> Vec<u32> {
    let bytes = hrp.bytes().map(u32::from);
    bytes
        .clone()
        .map(|b| b >> 5)
        .chain(std::iter::once(0))
        .chain(bytes.map(|b| b & 31))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real, checksum-valid Bech32 strings from an independent encoder (Go's
    // fixtures): the plain X25519 hrp and a plugin-style nested hrp, the
    // two shapes validate_recipient has to accept.
    const VALID_X25519: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";
    const VALID_PLUGIN: &str =
        "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0";

    fn reason(s: &str) -> Reason {
        validate_recipient(s).unwrap_err().reason
    }

    // Go: TestValidateRecipientAccepts
    #[test]
    fn validate_recipient_accepts() {
        for s in [VALID_X25519, VALID_PLUGIN] {
            assert_eq!(validate_recipient(s), Ok(()), "{s}");
        }
        // Single case either way is legal Bech32.
        assert_eq!(validate_recipient(&VALID_X25519.to_uppercase()), Ok(()));
    }

    // Go: TestValidateRecipientRejectsBadChecksum
    #[test]
    fn validate_recipient_rejects_bad_checksum() {
        let mutated = format!("{}x", &VALID_X25519[..VALID_X25519.len() - 1]);
        assert_ne!(mutated, VALID_X25519);
        assert_eq!(reason(&mutated), Reason::Checksum);
    }

    // Go: TestValidateRecipientRejectsMalformed
    #[test]
    fn validate_recipient_rejects_malformed() {
        for (s, want) in [
            ("", Reason::WrongLength),
            ("not-an-age-key", Reason::NoSeparator),
            ("age1", Reason::WrongLength),
            ("age1bad checksum here!!", Reason::NotInCharset('b')),
            (
                "AGE1QQQSYQCYQ5RQWZQFPG9SCRGWPUGPZYSNZS23V9CCRYDPK8QARC0SAVHh7m",
                Reason::MixedCase,
            ),
            (
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBogus",
                Reason::MixedCase,
            ),
            (
                "bitcoin1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m",
                Reason::NotAge("bitcoin".into()),
            ),
        ] {
            assert_eq!(reason(s), want, "{s:?}");
        }
    }

    // The plan's cases: an identity is not a recipient, and too long is
    // refused before anything else is looked at.
    #[test]
    fn validate_recipient_rejects_an_identity_and_an_overlong_string() {
        let identity =
            "AGE-SECRET-KEY-1QQQSYQCYQ5RQWZQFPG9SCRGWPUGPZYSNZS23V9CCRYDPK8QARC0SQEXAMPLE";
        assert!(validate_recipient(identity).is_err());
        let long = format!("age1{}", "q".repeat(MAX_LENGTH));
        assert_eq!(reason(&long), Reason::WrongLength);
    }

    // Go: TestValidateRecipientRejectsNonPrintableHRP. A space has no
    // case, so it clears the mixed-case check and reaches this one.
    #[test]
    fn validate_recipient_rejects_non_printable_hrp() {
        let err = validate_recipient("age 1qqqqqqqqqq").unwrap_err();
        assert_eq!(err.reason, Reason::NonPrintableHrp);
        assert!(err.to_string().contains("non-printable"), "{err}");
    }

    // Go: TestIsPrintableASCIIBoundaries
    #[test]
    fn is_printable_ascii_boundaries() {
        for (c, want) in [(' ', false), ('!', true), ('~', true), ('\u{7f}', false)] {
            assert_eq!(is_printable_ascii(&c.to_string()), want, "{c:?}");
        }
    }

    #[test]
    fn errors_are_usage_and_name_the_input() {
        let err = validate_recipient("not-a-valid-key").unwrap_err();
        assert_eq!(err.code(), Code::Usage);
        assert!(err.to_string().contains("\"not-a-valid-key\""), "{err}");
    }
}
