//! The enrollment code.

use std::collections::HashSet;

use super::code::{CODE_LENGTH, generate_code, normalize_code};

fn code() -> String {
    generate_code().unwrap().to_string()
}

// A cosmetic GAGE- prefix
// over four hyphenated groups of four: what a human copies off one screen
// and types into another.
#[test]
fn generated_code_has_the_documented_shape() {
    let code = code();
    let rest = code
        .strip_prefix("GAGE-")
        .unwrap_or_else(|| panic!("{code:?} lacks the GAGE- prefix"));
    let groups: Vec<&str> = rest.split('-').collect();
    assert_eq!(groups.len(), 4, "{code:?}");
    for g in &groups {
        assert_eq!(g.len(), 4, "{code:?} has a group of {}", g.len());
    }
    assert_eq!(rest.len() - 3, CODE_LENGTH, "{code:?}");
    assert_eq!(CODE_LENGTH * 5, 80, "the code is 80 bits");
}

// Over many generations,
// since any one code probably lacks them by chance.
#[test]
fn generated_codes_avoid_the_ambiguous_letters() {
    for _ in 0..500 {
        let code = code();
        assert!(
            !code.contains(['I', 'L', 'O', 'U']),
            "{code:?} contains one of I, L, O, U"
        );
        assert_eq!(code.to_uppercase(), code, "{code:?} is not uppercase");
    }
}

// What this build generates,
// this build accepts, prefix and hyphens included.
#[test]
fn generated_codes_normalize_to_themselves() {
    for _ in 0..50 {
        let code = code();
        let want = code.strip_prefix("GAGE-").unwrap().replace('-', "");
        let got =
            normalize_code(&code).unwrap_or_else(|| panic!("{code:?} fails its own validation"));
        assert_eq!(*got, want, "{code:?}");
    }
}

// The entropy smoke test: a fixed seed, a
// counter or the clock's seconds would pass every other row here. It
// samples, so a short period is caught as well as none.
#[test]
fn successive_codes_differ() {
    let mut seen = HashSet::new();
    for _ in 0..200 {
        let code = code();
        assert!(
            seen.insert(code.clone()),
            "generated {code:?} twice in 200 attempts: the source is not the OS RNG"
        );
    }
}

// The code's entropy, which no other test
// sees. Filling half the random bytes (40 bits) or reading four bits per
// character (64 bits, alphabet 0-F) passes the shape, the alphabet and
// successive_codes_differ. Every position must take every character: at
// 2000 draws the chance an honest generator misses one of the 512
// position-character pairs is below 2^-80.
#[test]
fn generated_codes_use_every_character_in_every_position() {
    let mut seen = vec![HashSet::new(); CODE_LENGTH];
    for _ in 0..2000 {
        let code = code().strip_prefix("GAGE-").unwrap().replace('-', "");
        for (i, c) in code.chars().enumerate() {
            seen[i].insert(c);
        }
    }
    for (i, chars) in seen.iter().enumerate() {
        assert_eq!(
            chars.len(),
            32,
            "position {i} took only {} of the 32 characters: the code has less than 80 bits",
            chars.len()
        );
    }
}

const CANONICAL: &str = "7K4M9QX2P3RH8WVN";

// Against the
// canonical form, so a failure points at the normalizer rather than the
// seal; seal_tests asserts the same spellings open a request.
#[test]
fn code_normalization_accepts_how_people_retype_codes() {
    for (name, typed) in [
        ("as displayed", "GAGE-7K4M-9QX2-P3RH-8WVN"),
        ("lowercase", "gage-7k4m-9qx2-p3rh-8wvn"),
        ("spaces for hyphens", "GAGE 7K4M 9QX2 P3RH 8WVN"),
        ("no prefix", "7K4M-9QX2-P3RH-8WVN"),
        ("no prefix, lowercase, spaces", "7k4m 9qx2 p3rh 8wvn"),
        ("no separators at all", "7K4M9QX2P3RH8WVN"),
        ("surrounding whitespace", "  GAGE-7K4M-9QX2-P3RH-8WVN\n"),
        ("lowercase prefix only", "gage-7K4M9QX2P3RH8WVN"),
    ] {
        let got = normalize_code(typed).unwrap_or_else(|| panic!("{name}: {typed:?} rejected"));
        assert_eq!(*got, CANONICAL, "{name}");
    }
}

// I and L read as 1,
// O as 0: a transcription of a code gage generated, not another code.
#[test]
fn code_normalization_maps_crockford_substitutions() {
    for (typed, want) in [
        ("1234567890ABCDEF", "1234567890ABCDEF"),
        ("I234567890ABCDEF", "1234567890ABCDEF"),
        ("L234567890ABCDEF", "1234567890ABCDEF"),
        ("i234567890abcdef", "1234567890ABCDEF"),
        ("l234567890abcdef", "1234567890ABCDEF"),
        ("123456789OABCDEF", "1234567890ABCDEF"),
        ("123456789oabcdef", "1234567890ABCDEF"),
        ("GAGE-IL34-5678-90AB-CDEF", "1134567890ABCDEF"),
    ] {
        let got = normalize_code(typed).unwrap_or_else(|| panic!("{typed:?} rejected"));
        assert_eq!(*got, want, "{typed:?}");
    }
}

// What makes a typo
// cost nothing. U is the excluded letter Crockford maps to nothing.
#[test]
fn code_validation_rejects_wrong_length_and_alphabet() {
    for (name, typed) in [
        ("empty", ""),
        ("prefix only", "GAGE-"),
        ("too short", "7K4M9QX2P3RH8WV"),
        ("too long", "7K4M9QX2P3RH8WVNX"),
        ("U is not mapped", "7K4M9QX2P3RH8WVU"),
        ("punctuation", "7K4M9QX2P3RH8WV!"),
        ("non-ascii", "7K4M9QX2P3RH8WVé"),
        (
            "a dropped character, spaces notwithstanding",
            "7K4M 9QX2 P3RH 8W N",
        ),
    ] {
        assert!(
            normalize_code(typed).is_none(),
            "{name}: {typed:?} accepted"
        );
    }
}
