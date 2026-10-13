// The Makefile exports these for version stamping (read in M0b). Declared
// here so a change to either one rebuilds the binary instead of leaving a
// stale version string.
fn main() {
    println!("cargo::rerun-if-env-changed=GAGE_VERSION");
    println!("cargo::rerun-if-env-changed=GAGE_COMMIT");
}
