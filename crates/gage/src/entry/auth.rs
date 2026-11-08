//! An entry payload's `auth` line (format v3, ADR-0065): the YAML body,
//! then one last line `auth: hmac-sha256:<64 lowercase hex>`, the
//! [`VaultKey`] tag over the body bound to the entry's id.
//!
//! The tag covers the body's exact bytes, never a re-serialization, so it
//! doesn't depend on the emitter staying byte-identical, and nothing is
//! parsed before it is authenticated.

use std::fmt;

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::vaultkey::VaultKey;

/// The key the tag is written under, reserved: an entry can't carry an
/// unknown field by this name.
pub(crate) const AUTH_KEY: &str = "auth";

/// What the last line starts with, up to the hex.
const PREFIX: &[u8] = b"auth: hmac-sha256:";

/// A SHA-256 tag in hex.
const HEX_LEN: usize = 64;

/// Why a payload didn't authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    /// No `auth` line where it belongs: the payload's last line.
    Missing,
    /// The tag doesn't match: written without this vault's key, altered,
    /// or moved from another id.
    Mismatch,
}

impl fmt::Display for AuthFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AuthFailure::Missing => "it carries no auth tag",
            AuthFailure::Mismatch => {
                "its auth tag does not match (written without this vault's key, altered, or moved from another entry)"
            }
        })
    }
}

/// `body` with entry `id`'s `auth` line appended. `body` is the emitter's
/// output, which always ends in a newline.
pub(crate) fn stamp(key: &VaultKey, id: Uuid, body: &[u8]) -> Zeroizing<Vec<u8>> {
    let tag = key.entry_tag(id, body);
    let mut out = Zeroizing::new(Vec::with_capacity(body.len() + PREFIX.len() + HEX_LEN + 1));
    out.extend_from_slice(body);
    out.extend_from_slice(PREFIX);
    for b in tag {
        out.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    out.push(b'\n');
    out
}

/// The body of entry `id`'s `payload`, once its `auth` line has verified
/// under the current key.
pub(crate) fn verify<'p>(
    key: &VaultKey,
    id: Uuid,
    payload: &'p [u8],
) -> Result<&'p [u8], AuthFailure> {
    let (body, tag) = split(payload)?;
    if key.verify_entry(id, body, &tag) {
        Ok(body)
    } else {
        Err(AuthFailure::Mismatch)
    }
}

/// [`verify`] for a past revision: a tag under a retired key also counts,
/// because it was current when git recorded that revision (ADR-0066).
pub(crate) fn verify_revision<'p>(
    key: &VaultKey,
    id: Uuid,
    payload: &'p [u8],
) -> Result<&'p [u8], AuthFailure> {
    let (body, tag) = split(payload)?;
    if key.verify_revision(id, body, &tag) {
        Ok(body)
    } else {
        Err(AuthFailure::Mismatch)
    }
}

/// A payload's body and the tag on its last line.
fn split(payload: &[u8]) -> Result<(&[u8], [u8; 32]), AuthFailure> {
    let content = payload.strip_suffix(b"\n").ok_or(AuthFailure::Missing)?;
    let start = content
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let (body, line) = payload.split_at(start);
    let hex = line[..line.len() - 1]
        .strip_prefix(PREFIX)
        .filter(|h| h.len() == HEX_LEN)
        .ok_or(AuthFailure::Missing)?;
    let tag = decode_hex(hex).ok_or(AuthFailure::Missing)?;
    Ok((body, tag))
}

/// Lower-case hex only: the one spelling [`stamp`] writes.
fn decode_hex(hex: &[u8]) -> Option<[u8; 32]> {
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] = b"title: github\nvalue: hunter2\n";

    #[test]
    fn a_stamped_body_verifies_and_comes_back_exactly() {
        let key = VaultKey::generate();
        let id = Uuid::new_v4();
        let payload = stamp(&key, id, BODY);
        assert!(payload.starts_with(BODY));
        let line = std::str::from_utf8(&payload[BODY.len()..]).unwrap();
        assert!(line.starts_with("auth: hmac-sha256:"), "{line}");
        assert!(line.ends_with('\n') && line.len() == PREFIX.len() + HEX_LEN + 1);
        assert_eq!(verify(&key, id, &payload).unwrap(), BODY);
    }

    // The tag a payload carries is the key's tag over its body, in hex.
    #[test]
    fn the_auth_line_is_the_hex_tag_over_the_body() {
        let key = VaultKey::from_bytes_for_tests([1; 32]);
        let id = Uuid::new_v4();
        let want: String = key
            .entry_tag(id, BODY)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let payload = stamp(&key, id, BODY);
        assert_eq!(
            &payload[BODY.len()..],
            format!("auth: hmac-sha256:{want}\n").as_bytes()
        );
    }

    #[test]
    fn a_payload_without_an_auth_line_is_missing() {
        let key = VaultKey::generate();
        let id = Uuid::new_v4();
        for payload in [
            &b""[..],
            BODY,
            b"title: github\n",
            b"auth: hmac-sha256:\n",
            b"title: x\nauth: hmac-sha256:00\n",
            b"title: x\nauth: sha256:0000000000000000000000000000000000000000000000000000000000000000\n",
        ] {
            assert_eq!(
                verify(&key, id, payload),
                Err(AuthFailure::Missing),
                "{:?}",
                String::from_utf8_lossy(payload)
            );
        }
        // Without its final newline the line isn't the one stamp writes.
        let stamped = stamp(&key, id, BODY);
        assert_eq!(
            verify(&key, id, &stamped[..stamped.len() - 1]),
            Err(AuthFailure::Missing)
        );
        // Nor is upper-case hex.
        let mut upper = stamped.to_vec();
        let n = upper.len();
        upper[n - 1 - HEX_LEN..n - 1].copy_from_slice(&[b'A'; HEX_LEN]);
        assert_eq!(verify(&key, id, &upper), Err(AuthFailure::Missing));
    }

    #[test]
    fn a_wrong_key_id_or_body_is_a_mismatch() {
        let key = VaultKey::generate();
        let id = Uuid::new_v4();
        let payload = stamp(&key, id, BODY);
        assert_eq!(
            verify(&VaultKey::generate(), id, &payload),
            Err(AuthFailure::Mismatch)
        );
        assert_eq!(
            verify(&key, Uuid::new_v4(), &payload),
            Err(AuthFailure::Mismatch)
        );
        let mut altered = payload.to_vec();
        altered[BODY.len() - 2] ^= 1;
        assert_eq!(verify(&key, id, &altered), Err(AuthFailure::Mismatch));
        // A line slipped in ahead of the tag is part of the body it covers.
        let mut longer = BODY.to_vec();
        longer.extend_from_slice(b"extra: x\n");
        longer.extend_from_slice(&payload[BODY.len()..]);
        assert_eq!(verify(&key, id, &longer), Err(AuthFailure::Mismatch));
    }
}
