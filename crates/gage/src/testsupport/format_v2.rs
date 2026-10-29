//! The golden format-v2 vault under `tests/fixtures/format-v2/`: what it
//! holds, and how a test puts it on disk. See `doc/format/v2.md` for the
//! format and the fixture's README for how it was made.
//!
//! The constants are the fixture's expected contents. They are written
//! down here, not read back from the fixture, so a test compares what the
//! fixture holds against what it was made to hold.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::entry::Extra;
use crate::gitrepo::{self, RemoteOptions};
use crate::paths;
use crate::vaultconfig::VaultId;
use crate::xdgpaths::Dirs;

/// The vault's local name on the device the fixture captures.
pub const VAULT_NAME: &str = "personal";

/// The committed `[vault].id`.
pub const VAULT_ID: &str = "ff336b5e-3a36-4164-a014-1b6890bb9785";

/// [`VAULT_ID`] as the type the path builders take.
pub fn vault_id() -> VaultId {
    VaultId::try_from(VAULT_ID).expect("format_v2: VAULT_ID is a vault id")
}

/// The device whose local state (identity, trust cache, global config)
/// the fixture captures.
pub const DEVICE: &str = "laptop-1";

/// `DEVICE`'s public key.
pub const DEVICE_PUBKEY: &str = "age1yrmhdexp2nu9m4ty3m7s8yyupyv5kdcgkwxce9nkr82jycnmde6qxph5l3";

/// The recovery recipient's public key. Its secret is `recovery-key.txt`.
pub const RECOVERY_PUBKEY: &str = "age1g6ltwxkauxtfhedctepamjta92w5cteyxe0zuxju4twpaleleamsjjgs30";

/// The device whose enrollment request sits in `.gage/pending/`.
pub const ENROLLING_DEVICE: &str = "phone-1";

/// The pending request's id.
pub const REQUEST_ID: &str = "f61694fd-e69d-4d65-ab67-371884a7fce5";

/// The code the pending request is sealed with. Public, like the
/// passphrase: nothing the fixture opens is a secret.
pub const ENROLLMENT_CODE: &str = "GAGE-YTYX-6Z0M-SDX2-JTV3";

/// What `DEVICE`'s identity file is wrapped with. Public.
pub const PASSPHRASE: &str = "format-v2 fixture passphrase";

/// `[shell]` in the global config.
pub const SHELL_PROMPT: &str = "{vault}{lock} fixture> ";
pub const SHELL_IDLE_TIMEOUT: &str = "15m";
pub const SHELL_HISTORY_FILE: &str = "$GAGE_STATE/history";
pub const SHELL_CLIPBOARD_TIMEOUT: &str = "1m30s";

/// What `config/config.toml.in` holds in place of the vault's absolute
/// path, and of its remote's.
pub const VAULT_PATH_PLACEHOLDER: &str = "@VAULT_PATH@";
pub const ORIGIN_PLACEHOLDER: &str = "@ORIGIN@";

/// The file listing every other file's SHA-256.
pub const MANIFEST_FILE: &str = "SHA256SUMS";

/// One of the fixture's entries, as it was written.
pub struct FixtureEntry {
    pub title: &'static str,
    pub description: &'static str,
    pub value: &'static str,
    pub fields: &'static [(&'static str, &'static str)],
}

/// Fields, unknown keys and a description: written with the library,
/// since only a newer gage writes keys this one doesn't know.
pub const RICH: FixtureEntry = FixtureEntry {
    title: "github",
    description: "work account",
    value: "fixture-not-a-secret",
    fields: &[
        ("url", "https://github.com/login"),
        ("username", "octo-fixture"),
    ],
};

/// A multi-line value, inserted with `--value-stdin`.
pub const MULTILINE: FixtureEntry = FixtureEntry {
    title: "recovery notes",
    description: "",
    value: "first line\nsecond line\n\n  indented fourth",
    fields: &[],
};

/// A title and a value, nothing else. Written while the remote was
/// unreachable, which is what made the merge.
pub const MINIMAL: FixtureEntry = FixtureEntry {
    title: "wifi",
    description: "",
    value: "fixture-psk",
    fields: &[],
};

pub const ENTRIES: [&FixtureEntry; 3] = [&RICH, &MULTILINE, &MINIMAL];

/// [`RICH`]'s unknown keys: a plain scalar that would resolve as an
/// integer, and a nested mapping holding a boolean and a sequence.
pub fn rich_extra() -> BTreeMap<String, Extra> {
    BTreeMap::from([
        ("x-future-plain".to_string(), Extra::Plain("0x1F".into())),
        (
            "x-future-map".to_string(),
            Extra::Map(vec![
                ("rotate".into(), Extra::Plain("true".into())),
                (
                    "tags".into(),
                    Extra::Seq(vec![Extra::Text("a".into()), Extra::Text("b".into())]),
                ),
            ]),
        ),
    ])
}

/// `tests/fixtures/format-v2/`.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("format-v2")
}

/// The fixture, put on disk as one device's machine.
pub struct Installed {
    /// Holds everything below; deleted on drop.
    pub root: tempfile::TempDir,
    pub dirs: Dirs,
    /// A copy of the committed `vault.git`, the clone's `origin`.
    pub remote: PathBuf,
    /// The clone: `$GAGE_DATA/vaults/<VAULT_NAME>`.
    pub vault: PathBuf,
}

/// Copies the fixture into a fresh temp root, clones its vault into place
/// and fills in the global config's paths. The committed tree is only
/// read: everything a test does lands in the copy.
pub fn install() -> Installed {
    let fixture = fixture_dir();
    let root = tempfile::tempdir().expect("format_v2: creating temp dir");
    let dirs = Dirs {
        config: root.path().join("config").join("gage"),
        data: root.path().join("data").join("gage"),
        state: root.path().join("state").join("gage"),
    };
    let remote = root.path().join("remote.git");
    copy_tree(&fixture.join("vault.git"), &remote);
    copy_tree(&fixture.join("data"), &dirs.data);
    copy_tree(&fixture.join("state"), &dirs.state);
    private_dirs(&dirs);

    let vault = paths::vaults_dir(&dirs).join(VAULT_NAME);
    let url = remote.to_str().expect("format_v2: temp path is UTF-8");
    gitrepo::clone(url, &vault, &RemoteOptions::new(&dirs))
        .unwrap_or_else(|e| panic!("format_v2: cloning the fixture: {e}"));

    let template = std::fs::read_to_string(fixture.join("config").join("config.toml.in"))
        .expect("format_v2: reading config.toml.in");
    let global = template
        .replace(VAULT_PATH_PLACEHOLDER, &toml_escape(&vault))
        .replace(ORIGIN_PLACEHOLDER, &toml_escape(&remote));
    std::fs::create_dir_all(&dirs.config).expect("format_v2: creating the config dir");
    std::fs::write(paths::global_config_file(&dirs), global)
        .expect("format_v2: writing the global config");

    Installed {
        root,
        dirs,
        remote,
        vault,
    }
}

/// `path` as the inside of a TOML basic string: Windows paths carry
/// backslashes.
fn toml_escape(path: &Path) -> String {
    path.to_str()
        .expect("format_v2: temp path is UTF-8")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Copies `src` into `dst` by reading and writing each file, so nothing
/// carries over a read-only bit (git's object files have one) that would
/// stop the temp dir being deleted on Windows.
pub fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {}: {e}", dst.display()));
    let entries =
        std::fs::read_dir(src).unwrap_or_else(|e| panic!("reading {}: {e}", src.display()));
    for entry in entries {
        let entry = entry.unwrap();
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&from, &to);
        } else {
            let data =
                std::fs::read(&from).unwrap_or_else(|e| panic!("reading {}: {e}", from.display()));
            std::fs::write(&to, data).unwrap_or_else(|e| panic!("writing {}: {e}", to.display()));
        }
    }
}

/// The modes gage itself gives its local state: the identities and trust
/// cache directories 0700, their files 0600.
fn private_dirs(dirs: &Dirs) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fn tighten(dir: &Path) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    tighten(&path);
                } else {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
            }
        }
        tighten(&dirs.data.join("identities"));
        tighten(&dirs.state);
    }
    #[cfg(not(unix))]
    let _ = dirs;
}

/// Every file under `dir` except the manifest itself, as `sha256sum`
/// writes it: `<hex>  <slash-separated path>`, sorted by path.
pub fn manifest(dir: &Path) -> String {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.retain(|rel| rel != MANIFEST_FILE);
    files.sort();
    files
        .iter()
        .map(|rel| {
            let data = std::fs::read(dir.join(rel)).unwrap();
            let hex: String = Sha256::digest(&data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            format!("{hex}  {rel}\n")
        })
        .collect()
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap();
            let parts: Vec<&str> = rel.iter().map(|p| p.to_str().unwrap()).collect();
            out.push(parts.join("/"));
        }
    }
}
