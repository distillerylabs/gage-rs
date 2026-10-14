//! The shipped `gage` binary is built without the library's `test-support`
//! feature. That feature is what exposes `set_scrypt_work_factor_for_tests`,
//! the one way to lower the identity file's KDF cost (Q-SCRYPT-HOOK), so a
//! build that enabled it would ship a binary able to weaken its own
//! passphrase wrapping. Nothing else checks this: every other test runs in
//! a build where dev-dependencies are unified in, and so has the feature by
//! design.
//!
//! The check asks cargo's own feature resolver rather than reading
//! `Cargo.toml`, so a feature enabled anywhere along the shipped dependency
//! graph (another crate, a default feature, a renamed dependency) counts.
//! `cargo tree -e no-dev` resolves features exactly as `cargo build` does.

use std::process::Command;

const FEATURE: &str = "test-support";

/// The features cargo resolves for the `gage` library in `gage-cli`'s
/// graph, over the given dependency edges, on every target platform.
fn gage_features(edges: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    // No --offline, for the reason `no_frontend_deps` gives: `make test`
    // fetches first and sets CARGO_NET_OFFLINE=true, which this inherits.
    let output = Command::new(cargo)
        .args([
            "tree", "--locked", "-p", "gage-cli", "-e", edges, "--target", "all", "--prefix",
            "none", "-f", "{p}|{f}",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let line = stdout
        .lines()
        .find(|l| {
            l.starts_with("gage v") && (l.contains("crates/gage)") || l.contains(r"crates\gage)"))
        })
        .unwrap_or_else(|| panic!("the gage library is not in gage-cli's tree:\n{stdout}"));
    let (_, features) = line.split_once('|').unwrap();
    features
        .split(',')
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_shipped_binary_is_built_without_test_support() {
    let features = gage_features("no-dev");
    assert!(
        !features.iter().any(|f| f == FEATURE),
        "a non-dev build of gage-cli enables gage/{FEATURE} ({features:?}): the shipped binary \
         could lower the scrypt work factor"
    );
}

// Guards the guard: the probe can see the feature when it is enabled, so
// the test above can't pass because the output format moved or the
// feature was renamed. Today the dev-dependency is what enables it.
#[test]
fn the_probe_sees_test_support_when_dev_dependencies_enable_it() {
    let features = gage_features("normal,build,dev");
    assert!(
        features.iter().any(|f| f == FEATURE),
        "with dev-dependencies in, gage/{FEATURE} should be enabled, but cargo tree reports \
         {features:?}; the shipped-build check above is no longer looking at the right thing"
    );
}
