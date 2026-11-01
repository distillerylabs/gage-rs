//! iOS and Android build the library without SSH (ADR-0062): no ssh-agent
//! exists there to authenticate through, and libssh2 is what keeps
//! OpenSSL linked. The desktop targets keep it (ADR-0015).
//!
//! `cargo tree --target` resolves the graph for one target without
//! needing its toolchain installed, so this runs in `make test` on every
//! host.

use std::collections::HashSet;
use std::process::Command;

const MOBILE: &[&str] = &["aarch64-apple-ios", "aarch64-linux-android"];
const DESKTOP: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// Every crate in the library's shipped graph on `target`, by name.
/// `cargo tree` rather than `cargo metadata`: metadata unifies features
/// across platforms, so it would report libssh2-sys on iOS even though no
/// iOS build compiles it.
#[allow(clippy::disallowed_methods)]
fn graph_on(target: &str) -> HashSet<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["tree", "--locked", "-p", "gage-core", "--target", target])
        .args(["-e", "normal,build", "--prefix", "none", "--format", "{p}"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo tree");
    assert!(
        output.status.success(),
        "cargo tree for {target} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("cargo tree prints UTF-8")
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_mobile_targets_resolve_the_library_without_libssh2() {
    for target in MOBILE {
        let graph = graph_on(target);
        assert!(
            graph.contains("git2"),
            "{target}: git2 is still the git crate"
        );
        assert!(
            !graph.contains("libssh2-sys"),
            "{target}: libssh2-sys is in the graph"
        );
    }
}

#[test]
fn the_desktop_targets_still_resolve_the_library_with_libssh2() {
    for target in DESKTOP {
        let graph = graph_on(target);
        assert!(
            graph.contains("libssh2-sys"),
            "{target}: libssh2-sys is missing"
        );
    }
}
