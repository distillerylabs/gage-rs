//! Random secret values for `gage generate`. The
//! length floor, the default and the two alphabets are properties of what
//! a generated secret is, so they live here; a frontend only turns its
//! flags into [`GenerateOptions`].

use rand_core::{OsRng, RngCore};

use crate::error::{ErrorKind, HasKind};

/// The value's length when none is asked for. Public so a frontend's help
/// can state it without carrying a second copy of the number.
pub const DEFAULT_LENGTH: usize = 24;

/// The shortest value `generate` will produce. Below eight characters even
/// a full-alphabet draw (~49 bits at 8) is within reach of an offline
/// attack that 12 or 24 is not, and silently obliging `-l 4` would hand
/// back exactly what the command exists to prevent. A refusal, not a
/// clamp: rounding 4 up to 8 would tell the human they got what they asked
/// for.
pub const MIN_LENGTH: usize = 8;

const LETTERS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// Deliberately narrow: characters unlikely to need escaping wherever the
/// value ends up pasted (a shell, a URL, a CSV).
const SYMBOLS: &str = "!@#$%^&*-_=+";

/// What `-l` and `--no-symbols` become in the library: one value rather
/// than more parameters, so another knob is additive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerateOptions {
    /// Characters to draw. Zero means "not specified" and takes
    /// [`DEFAULT_LENGTH`]: a zero-length secret is never what anyone wants,
    /// so the unset flag and an explicit request stay distinguishable.
    pub length: usize,
    /// Letters and digits only, for systems that reject punctuation.
    pub no_symbols: bool,
}

impl GenerateOptions {
    fn alphabet(self) -> String {
        if self.no_symbols {
            format!("{LETTERS}{DIGITS}")
        } else {
            format!("{LETTERS}{DIGITS}{SYMBOLS}")
        }
    }

    fn length(self) -> usize {
        if self.length == 0 {
            DEFAULT_LENGTH
        } else {
            self.length
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GenerateError {
    /// The human asked for something gage won't do: a usage error.
    #[error("a generated length of {length} is too short; the minimum is {MIN_LENGTH}")]
    TooShort { length: usize },
    #[error("a generated length must be positive")]
    ZeroLength,
    /// Empty, or longer than one byte can index without bias.
    #[error("a generator's alphabet must hold 1 to 256 characters, not {0}")]
    BadAlphabet(usize),
    /// The randomness source failed. Never papered over with a weaker one.
    #[error("generating random value: {0}")]
    Random(#[source] rand_core::Error),
}

impl HasKind for GenerateError {
    fn kind(&self) -> ErrorKind {
        match self {
            GenerateError::TooShort { .. } => ErrorKind::Usage,
            GenerateError::ZeroLength
            | GenerateError::BadAlphabet(_)
            | GenerateError::Random(_) => ErrorKind::Internal,
        }
    }
}

/// A fresh secret drawn from the OS's CSPRNG, shaped by `opts`. A length
/// below [`MIN_LENGTH`] is refused rather than obliged.
pub fn generate_value(opts: GenerateOptions) -> Result<String, GenerateError> {
    if opts.length != 0 && opts.length < MIN_LENGTH {
        return Err(GenerateError::TooShort {
            length: opts.length,
        });
    }
    random_string(&mut OsRng, opts.length(), opts.alphabet().as_bytes())
}

/// The pure core: the source and the shape are parameters, so a test can
/// hand it a scripted or failing source and see every output character
/// trace back to bytes the source produced.
///
/// Each character is one byte drawn by rejection sampling: a byte at or
/// above the largest multiple of the alphabet's length that fits in 256 is
/// discarded and redrawn, so every character is equally likely. A plain
/// `byte % len` would favor the front of the alphabet whenever its length
/// doesn't divide 256 (74 and 62 don't).
pub(crate) fn random_string(
    rng: &mut dyn RngCore,
    length: usize,
    alphabet: &[u8],
) -> Result<String, GenerateError> {
    if length == 0 {
        return Err(GenerateError::ZeroLength);
    }
    if alphabet.is_empty() || alphabet.len() > 256 {
        return Err(GenerateError::BadAlphabet(alphabet.len()));
    }
    let n = alphabet.len();
    let zone = 256 - (256 % n);
    let mut out = String::with_capacity(length);
    let mut byte = [0u8; 1];
    while out.len() < length {
        rng.try_fill_bytes(&mut byte)
            .map_err(GenerateError::Random)?;
        let b = usize::from(byte[0]);
        if b < zone {
            out.push(char::from(alphabet[b % n]));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hands out a fixed script of bytes, and fails the test if asked for
    /// more than it has.
    struct Scripted(std::vec::IntoIter<u8>);

    impl Scripted {
        fn new(bytes: impl IntoIterator<Item = u8>) -> Scripted {
            Scripted(bytes.into_iter().collect::<Vec<_>>().into_iter())
        }
    }

    impl RngCore for Scripted {
        fn next_u32(&mut self) -> u32 {
            unreachable!("random_string draws bytes")
        }
        fn next_u64(&mut self) -> u64 {
            unreachable!("random_string draws bytes")
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            self.try_fill_bytes(dest).unwrap()
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            for d in dest {
                *d = self.0.next().expect("the script ran out");
            }
            Ok(())
        }
    }

    /// A source that always fails.
    struct Failing;

    impl RngCore for Failing {
        fn next_u32(&mut self) -> u32 {
            unreachable!()
        }
        fn next_u64(&mut self) -> u64 {
            unreachable!()
        }
        fn fill_bytes(&mut self, _: &mut [u8]) {
            unreachable!()
        }
        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
            Err(rand_core::Error::new(std::io::Error::other("boom")))
        }
    }

    fn all_in(value: &str, alphabet: &str) -> bool {
        value.chars().all(|c| alphabet.contains(c))
    }

    // Two different scripted sources
    // give two different outputs, each character the one its byte names:
    // the draw really uses the source.
    #[test]
    fn random_string_draw_is_structural() {
        let alphabet = b"0123456789abcdef";
        let a = random_string(&mut Scripted::new([0x00; 16]), 16, alphabet).unwrap();
        let b = random_string(&mut Scripted::new([0xFF; 16]), 16, alphabet).unwrap();
        assert_eq!(a, "0".repeat(16));
        assert_eq!(b, "f".repeat(16));
        let c = random_string(&mut Scripted::new(0..16), 16, alphabet).unwrap();
        assert_eq!(c, "0123456789abcdef");
    }

    // A byte past
    // the largest multiple of the alphabet's length is redrawn, not folded
    // back with a modulo.
    #[test]
    fn random_string_rejects_bytes_that_would_bias_the_draw() {
        // 256 % 3 == 1, so byte 255 is the one out of range. A modulo
        // would map it to 'a'; it must be skipped and the next byte used.
        let got = random_string(&mut Scripted::new([255, 4]), 1, b"abc").unwrap();
        assert_eq!(got, "b");
        // 74 characters: 256 - 256 % 74 == 222.
        let full = GenerateOptions::default().alphabet();
        assert_eq!(full.len(), 74);
        let got = random_string(&mut Scripted::new([222, 255, 221]), 1, full.as_bytes()).unwrap();
        assert_eq!(got, (full.as_bytes()[221 % 74] as char).to_string());
    }

    #[test]
    fn random_string_propagates_reader_failure() {
        let err = random_string(&mut Failing, 8, b"ab").unwrap_err();
        assert!(matches!(err, GenerateError::Random(_)), "{err}");
        assert_eq!(err.kind(), ErrorKind::Internal);
        let source = std::error::Error::source(&err).unwrap().to_string();
        assert!(source.contains("boom"), "{source}");
    }

    // A
    // negative length has no `usize` spelling; the alphabet bound is also
    // checked from above, since one byte can't index past 256.
    #[test]
    fn random_string_rejects_zero_length_and_a_bad_alphabet() {
        let none = || Scripted::new([]);
        assert!(matches!(
            random_string(&mut none(), 0, b"abc"),
            Err(GenerateError::ZeroLength)
        ));
        assert!(matches!(
            random_string(&mut none(), 5, b""),
            Err(GenerateError::BadAlphabet(0))
        ));
        assert!(matches!(
            random_string(&mut none(), 5, &[b'a'; 257]),
            Err(GenerateError::BadAlphabet(257))
        ));
    }

    // Length and alphabet, on the real OS source.
    #[test]
    fn generate_value_length_and_alphabet() {
        let got = generate_value(GenerateOptions::default()).unwrap();
        assert_eq!(got.len(), DEFAULT_LENGTH);
        assert!(
            all_in(&got, &format!("{LETTERS}{DIGITS}{SYMBOLS}")),
            "{got}"
        );
    }

    #[test]
    fn generate_value_differs_across_calls() {
        let a = generate_value(GenerateOptions::default()).unwrap();
        let b = generate_value(GenerateOptions::default()).unwrap();
        assert_ne!(a, b);
    }

    // Every behavioral test
    // here passes just as well against a seeded PRNG, so this one reads the
    // source: the production half of this file must draw from `OsRng` and
    // name no seedable or thread-local generator.
    #[test]
    fn generate_draws_from_the_os_csprng_not_a_seeded_prng() {
        let src = include_str!("generate.rs");
        let production = src.split("#[cfg(test)]").next().unwrap();
        assert!(
            production.contains("random_string(&mut OsRng,"),
            "generate_value must draw from OsRng"
        );
        for banned in [
            "thread_rng",
            "ThreadRng",
            "StdRng",
            "SmallRng",
            "SeedableRng",
            "seed_from_u64",
            "fastrand",
        ] {
            assert!(!production.contains(banned), "generate.rs names {banned}");
        }
    }

    #[test]
    fn generate_value_honours_length() {
        for n in [MIN_LENGTH, 12, 24, 64, 200] {
            let got = generate_value(GenerateOptions {
                length: n,
                ..GenerateOptions::default()
            })
            .unwrap();
            assert_eq!(got.len(), n);
        }
    }

    #[test]
    fn generate_value_zero_length_uses_the_default() {
        let got = generate_value(GenerateOptions {
            length: 0,
            no_symbols: false,
        })
        .unwrap();
        assert_eq!(got.len(), DEFAULT_LENGTH);
    }

    // Fifty draws: one
    // 24-character draw from the full alphabet misses every symbol often
    // enough that a single check would pass against a flag that did
    // nothing.
    #[test]
    fn generate_value_no_symbols_excludes_symbols() {
        for _ in 0..50 {
            let got = generate_value(GenerateOptions {
                no_symbols: true,
                ..GenerateOptions::default()
            })
            .unwrap();
            assert!(got.chars().all(|c| c.is_ascii_alphanumeric()), "{got}");
        }
    }

    // 50 draws of 24 from
    // 74 with 12 symbols all miss with probability (62/74)^1200: never.
    #[test]
    fn generate_value_still_draws_symbols_by_default() {
        let drew_one = (0..50).any(|_| {
            generate_value(GenerateOptions::default())
                .unwrap()
                .chars()
                .any(|c| SYMBOLS.contains(c))
        });
        assert!(drew_one, "the default alphabet has lost its symbols");
    }

    // A negative length has no `usize`
    // spelling (the CLI rejects a negative `-l` at parse time, also as
    // Usage).
    #[test]
    fn generate_value_rejects_too_short_length() {
        for n in [1, MIN_LENGTH - 1] {
            let err = generate_value(GenerateOptions {
                length: n,
                ..GenerateOptions::default()
            })
            .unwrap_err();
            assert!(matches!(err, GenerateError::TooShort { .. }), "{err}");
            assert_eq!(err.kind(), ErrorKind::Usage);
            assert!(err.to_string().contains(&MIN_LENGTH.to_string()), "{err}");
        }
    }
}
