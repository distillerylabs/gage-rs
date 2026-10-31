// Version stamping. The Makefile exports VERSION_VAR and COMMIT_VAR; this
// passes them to the crate as GAGE_STAMPED_*, always set (empty when the
// variable is absent), so `env!` never fails and src/version.rs decides the
// fallback. Declaring both as rerun triggers means a change rebuilds the
// binary instead of leaving a stale version string.
//
// With no GAGE_COMMIT (a plain `cargo build`, cargo-dist, `cargo install`)
// the commit comes from the `.cargo_vcs_info.json` a published crate carries,
// then from `git`; a missing git or checkout is not an error.
include!("src/build_env.rs");

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-env-changed={VERSION_VAR}");
    let version = std::env::var(VERSION_VAR).unwrap_or_default();
    println!("cargo::rustc-env=GAGE_STAMPED_VERSION={version}");

    println!("cargo::rerun-if-env-changed={COMMIT_VAR}");
    let commit = std::env::var(COMMIT_VAR)
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(commit_from_package)
        .or_else(commit_from_git)
        .unwrap_or_default();
    println!("cargo::rustc-env=GAGE_STAMPED_COMMIT={commit}");
}

fn commit_from_package() -> Option<String> {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR")?;
    let path = Path::new(&dir).join(".cargo_vcs_info.json");
    println!("cargo::rerun-if-changed={}", path.display());
    commit_from_vcs_info(&std::fs::read_to_string(path).ok()?)
}

fn commit_from_git() -> Option<String> {
    // A new commit or a branch switch moves HEAD; without these a cached
    // build would keep printing the old sha.
    println!("cargo::rerun-if-changed=../../.git/HEAD");
    println!("cargo::rerun-if-changed=../../.git/refs");
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !sha.is_empty()).then_some(sha)
}
