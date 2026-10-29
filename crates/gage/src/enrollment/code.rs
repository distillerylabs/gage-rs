//! The enrollment code: generation, and the normalization every typed
//! spelling goes through before it is used as a passphrase. See
//! ADR-0039.

use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

/// Crockford's base32 alphabet: the digits and the uppercase letters minus
/// I, L, O and U. The first three are what gets confused with 1, 1 and 0
/// in handwriting and over a phone; U keeps the generator from spelling an
/// unfortunate word. Decoding maps the first three back rather than
/// rejecting them (see [`normalize_code`]); U has nothing to map to.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// How many Crockford characters a code carries. Sixteen is 80 bits, which
/// is what licenses the seal's low work factor
/// (`crypt::ENROLLMENT_SCRYPT_WORK_FACTOR`): the two move together or not
/// at all.
pub const CODE_LENGTH: usize = 16;

/// Characters between hyphens in the displayed form. Cosmetic:
/// normalization strips the hyphens before anything looks at the value.
const GROUP: usize = 4;

/// The cosmetic marker on the displayed form. It makes the string
/// recognizable where it shouldn't turn up (a chat log, a screenshot, a
/// terminal recording), which is a reminder that it doesn't belong there.
const PREFIX: &str = "GAGE";

/// One enrollment code in its display form, `GAGE-XXXX-XXXX-XXXX-XXXX`.
///
/// The entropy is the security parameter, not the KDF behind it ("The
/// reframe"): the sealed blob sits in a repository every reader of the
/// vault can fetch, so it is an offline, unlimited-guess target. So the
/// code is generated rather than chosen, from the OS RNG, and it is 80
/// bits: ten random bytes split into sixteen five-bit groups, exactly, so
/// the draw is uniform over the alphabet with no rejection sampling.
pub(crate) fn generate_code() -> Result<Zeroizing<String>, rand_core::Error> {
    let mut raw = Zeroizing::new([0u8; CODE_LENGTH * 5 / 8]);
    OsRng.try_fill_bytes(&mut *raw)?;
    let mut out = Zeroizing::new(String::with_capacity(PREFIX.len() + 4 + CODE_LENGTH));
    out.push_str(PREFIX);
    for i in 0..CODE_LENGTH {
        if i % GROUP == 0 {
            out.push('-');
        }
        out.push(ALPHABET[five_bits_at(&raw[..], i)] as char);
    }
    Ok(out)
}

/// The `i`th five-bit group of `raw`, most significant bit first.
fn five_bits_at(raw: &[u8], i: usize) -> usize {
    (0..5).fold(0, |v, bit| {
        let pos = i * 5 + bit;
        (v << 1) | usize::from(raw[pos / 8] & (0x80 >> (pos % 8)) != 0)
    })
}

/// A typed code as the canonical sixteen characters the seal was written
/// with, or `None` for anything that can't be one.
///
/// It absorbs how people retype a code (lowercase, spaces for the
/// hyphens, a dropped `GAGE-`) and maps the three ambiguous letters (I
/// and L to 1, O to 0), because a code containing one is a transcription
/// of a code gage generated, not a different code.
///
/// Its second job is to make a typo cost nothing: a code failing length or
/// alphabet is refused here, before any decryption, so a mistyped
/// character doesn't buy one scrypt run per pending request. The caller
/// still reports the same `CodeWrong` either way; this buys cost, not a
/// different answer.
pub(crate) fn normalize_code(typed: &str) -> Option<Zeroizing<String>> {
    let mut out = Zeroizing::new(String::with_capacity(typed.len()));
    for c in typed.trim().chars().flat_map(char::to_uppercase) {
        match c {
            // Dropped rather than positional: a code retyped with the
            // groups in the wrong places is still the same code.
            '-' | ' ' => {}
            'I' | 'L' => out.push('1'),
            'O' => out.push('0'),
            c if c.is_ascii() && ALPHABET.contains(&(c as u8)) => out.push(c),
            _ => return None,
        }
    }
    // The prefix goes after the separators, since it arrives attached by a
    // hyphen, a space or nothing. Length is what makes that unambiguous:
    // every letter of GAGE is also a legal code character, so a leading
    // GAGE is only a prefix when the string is four characters too long
    // without it.
    if out.len() == CODE_LENGTH + PREFIX.len() && out.starts_with(PREFIX) {
        out.drain(..PREFIX.len());
    }
    (out.len() == CODE_LENGTH).then_some(out)
}
