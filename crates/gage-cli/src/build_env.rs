// The environment variables that stamp a build's version and commit. One
// definition, shared by build.rs (through include!), the binary, and the
// test that checks the Makefile exports exactly these names: a stamp under
// any other name would build silently and print `dev`.

/// The release tag, or `dev`. Read at build time.
pub const VERSION_VAR: &str = "GAGE_VERSION";
/// The short commit hash. Read at build time.
pub const COMMIT_VAR: &str = "GAGE_COMMIT";

/// The short commit from a `.cargo_vcs_info.json` body, which `cargo
/// package` writes (`{"git":{"sha1":"<40 hex>"},"path_in_vcs":"..."}`) and
/// every crates.io build has. Pure so it is tested in-process; this is a
/// scan for the one key rather than a JSON parser, because build scripts
/// take no dependencies here. None for anything malformed.
pub fn commit_from_vcs_info(json: &str) -> Option<String> {
    let after_key = &json[json.find("\"sha1\"")? + "\"sha1\"".len()..];
    let value = after_key.trim_start().strip_prefix(':')?.trim_start();
    let (sha, _) = value.strip_prefix('"')?.split_once('"')?;
    if sha.len() < 7 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(sha[..7].to_string())
}
