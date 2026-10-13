//! Normalizes and validates the device names gage uses to label recipients
//! and identity files: lowercase, hostname-derived strings restricted to a
//! small filename-safe character set. See "Local identity storage" in the
//! design doc and Q-DEVICE-NAME.
//!
//! Device names become path components, and some are read back from a
//! vault's committed, git-writable config, so [`valid`] is a security
//! boundary, not cosmetics.

/// The longest device name, after normalization or on validation of an
/// explicitly given one.
pub const MAX_LENGTH: usize = 63;

/// The allowlist every device name must satisfy. It deliberately excludes
/// every path separator (`/` everywhere, `\` and `:` on Windows) so a name
/// read back from a committed config can never escape the directory it is
/// joined into.
fn allowed(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-')
}

/// Derives a device name from a hostname: lowercased, truncated at the
/// first `.`, every character outside the allowlist replaced with `-` (runs
/// collapsed to one), and length-capped. `None` if nothing usable
/// survives: callers should ask for an explicit name rather than invent
/// one.
pub fn normalize(hostname: &str) -> Option<String> {
    // One char to one char, as Go's strings.ToLower does: `İ` lowercases
    // to `i` there, where str::to_lowercase gives `i` plus a combining dot.
    let lower: String = hostname
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let host = lower.split('.').next().unwrap_or_default();
    let mut name = String::with_capacity(host.len());
    for c in host.chars() {
        if allowed(c) && c != '-' {
            name.push(c);
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let mut name = name.trim_matches('-').to_string();
    if name.len() > MAX_LENGTH {
        // Everything left is ASCII, so this is a char boundary.
        name.truncate(MAX_LENGTH);
        name.truncate(name.trim_end_matches('-').len());
    }
    (!name.is_empty()).then_some(name)
}

/// Whether `name` is safe to use as-is: non-empty, within [`MAX_LENGTH`],
/// not made only of dots, and built entirely from the allowlist. Every
/// device name gage turns into a path component, whether from `--device`
/// or read back from a committed config, must pass this first.
pub fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_LENGTH && !all_dots(name) && name.chars().all(allowed)
}

/// `.`, `..` and `...` satisfy the allowlist, because real names contain
/// dots. A name whose only meaning to a filesystem is "this directory" or
/// "the parent directory" is never a legitimate device name, so it is
/// refused here rather than left to each caller to neutralize (Windows
/// also strips trailing dots, so such a component is doubly ill-defined
/// there).
fn all_dots(name: &str) -> bool {
    name.chars().all(|c| c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go: TestNormalizeHostname
    #[test]
    fn normalize_hostname() {
        for (input, want) in [
            ("Andrews-MacBook-Pro.local", "andrews-macbook-pro"),
            ("laptop-1", "laptop-1"),
            ("My Computer!!!", "my-computer"),
            ("UPPER.CASE.HOST", "upper"),
        ] {
            assert_eq!(normalize(input).as_deref(), Some(want), "{input:?}");
        }
    }

    // Go: TestNormalizeCollapsesRuns
    #[test]
    fn normalize_collapses_runs() {
        assert_eq!(normalize("a!!!!!b").as_deref(), Some("a-b"));
        assert_eq!(normalize("a--!-b").as_deref(), Some("a-b"));
    }

    // Go: TestNormalizeNothingUsable
    #[test]
    fn normalize_nothing_usable() {
        for input in ["", "...", "!!!", "---"] {
            assert_eq!(normalize(input), None, "{input:?}");
        }
    }

    // Go: TestNormalizeCapsLength
    #[test]
    fn normalize_caps_length() {
        let got = normalize(&"a".repeat(200)).unwrap();
        assert_eq!(got.len(), MAX_LENGTH);
        // A cut that lands on a dash doesn't leave it dangling.
        let got = normalize(&format!("{}-b", "a".repeat(MAX_LENGTH - 1))).unwrap();
        assert_eq!(got, "a".repeat(MAX_LENGTH - 1));
    }

    #[test]
    fn normalize_replaces_non_ascii() {
        assert_eq!(normalize("Zoë's Mac").as_deref(), Some("zo-s-mac"));
    }

    // Lowercasing matches Go's strings.ToLower, which maps one rune to one
    // rune: `İ` is `i` (Rust's full mapping would add a combining dot,
    // which then becomes a `-`), and the Kelvin sign is `k`.
    #[test]
    fn normalize_lowercases_like_go() {
        assert_eq!(normalize("İstanbul").as_deref(), Some("istanbul"));
        assert_eq!(normalize("\u{212A}elvin").as_deref(), Some("kelvin"));
    }

    // Go: TestValidAccepts
    #[test]
    fn valid_accepts() {
        for name in ["laptop-1", "andrews-macbook-pro", "recovery.key_1", "a"] {
            assert!(valid(name), "{name:?}");
        }
        assert!(valid(&"a".repeat(MAX_LENGTH)));
    }

    // Go: TestValidRejectsPathTraversalAndSeparators
    #[test]
    fn valid_rejects_path_traversal_and_separators() {
        for name in [
            "../../../etc/x",
            "/etc/passwd",
            r"C:\Windows\x",
            "c:x",
            "a/b",
            r"a\b",
            "",
            "UpperCase",
            "has space",
            "nul\0byte",
        ] {
            assert!(!valid(name), "{name:?}");
        }
    }

    // Go: TestValidRejectsOverLength
    #[test]
    fn valid_rejects_over_length() {
        assert!(!valid(&"a".repeat(MAX_LENGTH + 1)));
    }

    // Go: TestValidRejectsAllDotsNames
    #[test]
    fn valid_rejects_all_dots_names() {
        for name in [".", "..", "...", "....."] {
            assert!(!valid(name), "{name:?}");
        }
    }

    // Go: TestValidStillAcceptsNamesContainingDots
    #[test]
    fn valid_still_accepts_names_containing_dots() {
        for name in ["laptop.1", "a.b.c", ".hidden", "trailing."] {
            assert!(valid(name), "{name:?}");
        }
    }

    // Go: TestNormalizeNeverProducesAnAllDotsName. Whatever a hostname
    // looks like, normalization never manufactures a name valid() rejects.
    #[test]
    fn normalize_never_produces_a_name_valid_rejects() {
        for hostname in [".", "..", "...", ".local", "..foo..", "a.b.c", "-.-", "!a!"] {
            if let Some(got) = normalize(hostname) {
                assert!(valid(&got), "normalize({hostname:?}) = {got:?}");
            }
        }
    }
}
