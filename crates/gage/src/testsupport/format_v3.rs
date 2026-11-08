//! The golden format-v3 vault under `tests/fixtures/format-v3/`: what it
//! holds, and how a test puts it on disk. See `doc/format/v3.md` for the
//! format and the fixture's README for how it was made.
//!
//! The constants are the fixture's expected contents, written down here
//! rather than read back from it. What v3 kept from v2 (the entries, the
//! device names, `[shell]`, the layout of the fixture itself) is
//! `format_v2`'s, re-exported, so the two fixtures can't drift apart on it.

use std::path::{Path, PathBuf};

use crate::storage::vaultconfig::VaultId;

pub use super::format_v2::{
    DEVICE, ENROLLING_DEVICE, ENTRIES, FixtureEntry, Installed, MANIFEST_FILE, MINIMAL, MULTILINE,
    ORIGIN_PLACEHOLDER, RICH, SHELL_CLIPBOARD_TIMEOUT, SHELL_HISTORY_FILE, SHELL_IDLE_TIMEOUT,
    SHELL_PROMPT, VAULT_NAME, VAULT_PATH_PLACEHOLDER, copy_tree, manifest, rich_extra,
};

/// The committed `[vault].id`.
pub const VAULT_ID: &str = "9acc298b-a964-494d-94ed-9a84413baada";

/// [`VAULT_ID`] as the type the path builders take.
pub fn vault_id() -> VaultId {
    VaultId::try_from(VAULT_ID).expect("format_v3: VAULT_ID is a vault id")
}

/// `DEVICE`'s public key.
pub const DEVICE_PUBKEY: &str = "age1ehftancy2fdl4vnanneal77z4s2qmfr5lcjwnr4jvv5qyzlduydqxgzaej";

/// The recovery recipient's public key. Its secret is `recovery-key.txt`.
pub const RECOVERY_PUBKEY: &str = "age1k6mh0rdalfdewvpqmpcux0tcmkl3vy4r0pf2kasaz3jvwzl5lynqm82rxu";

/// The pending request's id.
pub const REQUEST_ID: &str = "387ac573-b97b-47b5-8605-2b01f47792cc";

/// The code the pending request is sealed with. Public, like the
/// passphrase: nothing the fixture opens is a secret.
pub const ENROLLMENT_CODE: &str = "GAGE-68Q0-HZ8B-BSYH-TC3A";

/// What `DEVICE`'s identity file is wrapped with. Public.
pub const PASSPHRASE: &str = "format-v3 fixture passphrase";

/// `tests/fixtures/format-v3/`.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("format-v3")
}

/// Copies the fixture into a fresh temp root, clones its vault into place
/// and fills in the global config's paths. The committed tree is only
/// read: everything a test does lands in the copy.
pub fn install() -> Installed {
    super::format_v2::install_from(&fixture_dir())
}
