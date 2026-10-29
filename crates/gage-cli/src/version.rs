//! What `gage --version` prints: `gage <version> (commit <sha7>)`.
//!
//! The version is the Makefile's `GAGE_VERSION` at build time (a tag, or
//! `dev`), the commit its `GAGE_COMMIT`. Unset falls back to `dev` and
//! `unknown`; Rust embeds no module version or
//! VCS revision to fall back to first.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: String,
    pub commit: String,
}

impl BuildInfo {
    /// The values this binary was built with.
    pub fn stamped() -> BuildInfo {
        resolve_build_info(
            Some(env!("GAGE_STAMPED_VERSION")),
            Some(env!("GAGE_STAMPED_COMMIT")),
        )
    }

    /// `<version> (commit <sha7>)`, what follows `gage ` on the version line.
    pub fn version_text(&self) -> String {
        format!("{} (commit {})", self.version, self.commit)
    }
}

impl fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gage {}", self.version_text())
    }
}

/// Anything stamped wins, including an explicit `dev`; only an unset or
/// empty value falls back.
pub fn resolve_build_info(version: Option<&str>, commit: Option<&str>) -> BuildInfo {
    let or = |v: Option<&str>, fallback: &str| {
        v.filter(|s| !s.is_empty()).unwrap_or(fallback).to_string()
    };
    BuildInfo {
        version: or(version, "dev"),
        commit: or(commit, "unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_tagged_build() {
        let b = resolve_build_info(Some("v2.0.0"), Some("deadbee"));
        assert_eq!(b.to_string(), "gage v2.0.0 (commit deadbee)");
    }

    #[test]
    fn version_string_untagged_build() {
        let b = resolve_build_info(Some("dev"), Some("abcdef1"));
        assert_eq!(b.to_string(), "gage dev (commit abcdef1)");
    }

    #[test]
    fn resolve_build_info_prefers_stamped_values() {
        let b = resolve_build_info(Some("v9.9.9"), Some("cafef00"));
        assert_eq!(
            (b.version.as_str(), b.commit.as_str()),
            ("v9.9.9", "cafef00")
        );
    }

    #[test]
    fn resolve_build_info_keeps_explicit_dev() {
        assert_eq!(resolve_build_info(Some("dev"), None).version, "dev");
    }

    // Rust has no embedded module version or VCS revision to fall back to,
    // so unset goes straight to dev/unknown.
    #[test]
    fn resolve_build_info_without_stamps() {
        for (v, c) in [(None, None), (Some(""), Some(""))] {
            let b = resolve_build_info(v, c);
            assert_eq!(b.to_string(), "gage dev (commit unknown)");
        }
    }

    // Rust: the Makefile
    // exports exactly the variable names build.rs reads, from the one
    // shared definition.
    #[test]
    fn makefile_exports_the_variables_the_build_reads() {
        let makefile =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"))
                .unwrap();
        for var in [crate::build_env::VERSION_VAR, crate::build_env::COMMIT_VAR] {
            assert!(
                makefile
                    .lines()
                    .any(|l| l.starts_with(&format!("export {var} "))),
                "the Makefile does not export {var}"
            );
        }
    }
}
