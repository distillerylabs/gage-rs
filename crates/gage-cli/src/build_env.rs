// The environment variables that stamp a build's version and commit. One
// definition, shared by build.rs (through include!), the binary, and the
// test that checks the Makefile exports exactly these names: a stamp under
// any other name would build silently and print `dev`.

/// The release tag, or `dev`. Read at build time.
pub const VERSION_VAR: &str = "GAGE_VERSION";
/// The short commit hash. Read at build time.
pub const COMMIT_VAR: &str = "GAGE_COMMIT";
