// Version stamping. The Makefile exports VERSION_VAR and COMMIT_VAR; this
// passes them to the crate as GAGE_STAMPED_*, always set (empty when the
// variable is absent), so `env!` never fails and src/version.rs decides the
// fallback. Declaring both as rerun triggers means a change rebuilds the
// binary instead of leaving a stale version string.
include!("src/build_env.rs");

fn main() {
    for (var, stamped) in [
        (VERSION_VAR, "GAGE_STAMPED_VERSION"),
        (COMMIT_VAR, "GAGE_STAMPED_COMMIT"),
    ] {
        println!("cargo::rerun-if-env-changed={var}");
        let value = std::env::var(var).unwrap_or_default();
        println!("cargo::rustc-env={stamped}={value}");
    }
}
