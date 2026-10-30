//! What `gage --version` prints: `gage <version> (commit <sha7>)`.
//!
//! The version is the crate's own (`CARGO_PKG_VERSION`), so a Makefile build,
//! a plain `cargo build`, cargo-dist and `cargo install gage-cli` all print
//! the same string, with no leading `v`. Two stamps adjust it: the Makefile's
//! `GAGE_VERSION=dev` marks an untagged checkout (`<pkg>-dev`), and any other
//! non-empty `GAGE_VERSION` is an override that wins verbatim (the stamp
//! check uses it). The commit comes from build.rs (`GAGE_COMMIT`, then the
//! `.cargo_vcs_info.json` of a published crate, then `git`), else `unknown`.
//! See ADR-0058.

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
            env!("CARGO_PKG_VERSION"),
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

/// The stamped dev marker.
const DEV_MARKER: &str = "dev";

/// `pkg_version` is the crate version. An unset or empty `version` is the
/// crate version; the `dev` marker is `<pkg>-dev`; anything else is an
/// override. An unset or empty `commit` is `unknown`.
pub fn resolve_build_info(
    version: Option<&str>,
    commit: Option<&str>,
    pkg_version: &str,
) -> BuildInfo {
    let version = match version.filter(|s| !s.is_empty()) {
        None => pkg_version.to_string(),
        Some(DEV_MARKER) => format!("{pkg_version}-dev"),
        Some(other) => other.to_string(),
    };
    BuildInfo {
        version,
        commit: commit
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "0.1.0";

    #[test]
    fn version_string_release_build() {
        let b = resolve_build_info(None, Some("deadbee"), PKG);
        assert_eq!(b.to_string(), "gage 0.1.0 (commit deadbee)");
    }

    #[test]
    fn unset_or_empty_version_is_the_crate_version() {
        for v in [None, Some("")] {
            assert_eq!(resolve_build_info(v, None, PKG).version, "0.1.0");
        }
    }

    #[test]
    fn the_dev_marker_is_the_crate_version_with_a_dev_suffix() {
        let b = resolve_build_info(Some("dev"), Some("abcdef1"), PKG);
        assert_eq!(b.to_string(), "gage 0.1.0-dev (commit abcdef1)");
    }

    #[test]
    fn an_override_wins_verbatim() {
        let b = resolve_build_info(Some("v9.9.9-testtag"), Some("cafef00"), PKG);
        assert_eq!(
            (b.version.as_str(), b.commit.as_str()),
            ("v9.9.9-testtag", "cafef00")
        );
    }

    #[test]
    fn unset_or_empty_commit_is_unknown() {
        for c in [None, Some("")] {
            assert_eq!(resolve_build_info(None, c, PKG).commit, "unknown");
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

#[cfg(test)]
mod vcs_info_tests {
    use crate::build_env::commit_from_vcs_info;

    #[test]
    fn the_sha_in_a_vcs_info_body_is_shortened_to_seven() {
        let body = r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567"},"path_in_vcs":"crates/gage-cli"}"#;
        assert_eq!(commit_from_vcs_info(body).as_deref(), Some("0123456"));
    }

    #[test]
    fn whitespace_and_a_dirty_flag_do_not_matter() {
        let body = "{\n \"git\": {\n  \"dirty\": true,\n  \"sha1\" : \"ABCDEF1234567\"\n }\n}";
        assert_eq!(commit_from_vcs_info(body).as_deref(), Some("ABCDEF1"));
    }

    #[test]
    fn malformed_or_absent_json_falls_through() {
        for body in [
            "",
            "not json",
            "{}",
            r#"{"git":{"dirty":true}}"#,
            r#"{"git":{"sha1":}}"#,
            r#"{"git":{"sha1":"abc"}}"#,
            r#"{"git":{"sha1":"zzzzzzzzzz"}}"#,
            r#"{"git":{"sha1":"0123456"#,
        ] {
            assert_eq!(commit_from_vcs_info(body), None, "{body:?}");
        }
    }
}
