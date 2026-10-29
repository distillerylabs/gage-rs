//! A vault's committed `.gage/config.toml`: plaintext control-plane data
//! naming the vault's id, type, method default and recipients. See
//! "On-disk layout" in the design doc.
//!
//! Unlike the global config, every field here can reach the local
//! filesystem (the id and each recipient's device name become path
//! components), and the file is committed, so any git-writer can edit it.
//! [`read`] therefore validates before returning, in a fixed order: the
//! format version first (a file this build doesn't understand is never
//! acted on), then the id, then the device names (ADR-0010, ADR-0006). The
//! id's check is [`VaultId`]'s own (ADR-0052).
//!
//! The schema (table and field names) is fixed by the format; the bytes are
//! the `toml` crate's own (ADR-0002).

mod id;
mod kind;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use id::{InvalidVaultId, VaultId, valid_id};
pub use kind::{UnknownVaultType, UnlockMethod, VaultType};

use crate::atomicfile;
use crate::devicename;
use crate::error::{ErrorKind, HasKind};
use crate::serde_opt::empty_as_none;

/// The only `format_version` this build understands. Bumped when the
/// file's shape changes in a way an older reader can't safely ignore.
///
/// 2 added `[vault].id` (ADR-0006). A v1 vault has no id at all, so there is
/// nothing to key its identities directory, trust cache or lock by, which
/// is why the refusal in [`read`] is not a formality it could parse past.
pub const CURRENT_FORMAT_VERSION: i64 = 2;

/// The `[vault]` table.
///
/// No `Default`: the id has no zero value. [`read`] has refused a file
/// without a valid id before this is ever deserialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultMeta {
    #[serde(default)]
    pub name: String,
    /// The vault's own id: minted once by `gage init`, never changed, and
    /// cloned with the vault, so every clone agrees on it however it was
    /// named locally (ADR-0006).
    pub id: VaultId,
    /// Refused at parse if this build doesn't know it (ADR-0053).
    #[serde(rename = "type")]
    pub vault_type: VaultType,
    /// Absent reads as 0, which is "older than this build".
    #[serde(default)]
    pub format_version: i64,
    /// The UTC creation date, `YYYY-MM-DD`.
    #[serde(default)]
    pub created: String,
}

/// The `[method]` table: a default for devices joining this vault, not a
/// vault-wide constraint. See "Decryption methods are per-device" and
/// ADR-0011.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Method {
    /// Kept as written even when this build doesn't perform it
    /// (ADR-0053).
    pub default: UnlockMethod,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub plugin: Option<String>,
}

/// One `[[recipients]]` entry. Deliberately only a device label and a
/// public key: a per-recipient method would tell anyone with read access
/// which recipient is the softest target (ADR-0011).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recipient {
    pub device: String,
    pub pubkey: String,
}

/// The whole of `.gage/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub vault: VaultMeta,
    #[serde(default)]
    pub method: Method,
    #[serde(default)]
    pub recipients: Vec<Recipient>,
}

/// The two keys [`read`] checks before it parses the rest, as plain values:
/// the version has to be judged before anything else in the file is
/// trusted, the id's refusal names the id rather than a parse position,
/// and both have to work on a file the full schema would refuse.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Preamble {
    vault: PreambleMeta,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PreambleMeta {
    format_version: i64,
    id: String,
}

/// Which way an unsupported `format_version` is off. The two need opposite
/// advice, so the error carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSkew {
    /// Written by an older gage (or missing). It can't be upgraded in
    /// place: version 2 added `[vault].id` and there is no id to invent for a
    /// vault that never had one, so the remedy is to re-create it.
    Older,
    /// Written by a newer gage: upgrade this one.
    Newer,
}

impl fmt::Display for VersionSkew {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            VersionSkew::Older => {
                "re-create this vault (`make reset-local-state` clears this machine's vaults and identities first)"
            }
            VersionSkew::Newer => "upgrade gage",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path} does not exist")]
    NotFound { path: PathBuf },
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    /// An unrecognized `format_version`: refused outright, never parsed
    /// best-effort ("format_version is enforced, not decorative").
    #[error(
        "{path} has format_version {found}, this gage understands {CURRENT_FORMAT_VERSION} — {skew}"
    )]
    UnsupportedFormatVersion {
        path: PathBuf,
        found: i64,
        skew: VersionSkew,
    },
    /// `[vault].id` is absent or not a canonical UUID (see [`VaultId`]).
    #[error("{path}: vault id {id:?} is not a valid id")]
    InvalidVaultId { path: PathBuf, id: String },
    /// A recipient's device name fails the filesystem-safe allowlist.
    #[error("{path}: recipient device name {device:?} is invalid")]
    InvalidDeviceName { path: PathBuf, device: String },
    #[error("encoding {path}: {source}")]
    Encode {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("writing {path}: {source}")]
    Write {
        path: PathBuf,
        source: atomicfile::Error,
    },
}

impl HasKind for Error {
    // The three refusals are anticipated states a human resolves
    // (upgrade gage, re-create or repair the vault), not gage bugs, so a
    // script can tell them from Internal.
    fn kind(&self) -> ErrorKind {
        match self {
            Error::UnsupportedFormatVersion { .. }
            | Error::InvalidVaultId { .. }
            | Error::InvalidDeviceName { .. } => ErrorKind::Conflict,
            _ => ErrorKind::Internal,
        }
    }
}

/// Parses and validates a vault's `.gage/config.toml`: the format version
/// first, then the id, then every recipient's device name. Nothing in a
/// file that fails one check is looked at by the next.
pub fn read(path: &Path) -> Result<File, Error> {
    let at = || path.to_path_buf();
    let text = std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound { path: at() }
        } else {
            Error::Read { path: at(), source }
        }
    })?;
    let preamble: Preamble =
        toml::from_str(&text).map_err(|source| Error::Parse { path: at(), source })?;

    let found = preamble.vault.format_version;
    if found != CURRENT_FORMAT_VERSION {
        let skew = if found > CURRENT_FORMAT_VERSION {
            VersionSkew::Newer
        } else {
            VersionSkew::Older
        };
        return Err(Error::UnsupportedFormatVersion {
            path: at(),
            found,
            skew,
        });
    }
    if let Err(InvalidVaultId(id)) = VaultId::try_from(preamble.vault.id) {
        return Err(Error::InvalidVaultId { path: at(), id });
    }
    let file: File = toml::from_str(&text).map_err(|source| Error::Parse { path: at(), source })?;
    if let Some(bad) = file
        .recipients
        .iter()
        .find(|r| !devicename::valid(&r.device))
    {
        return Err(Error::InvalidDeviceName {
            path: at(),
            device: bad.device.clone(),
        });
    }
    Ok(file)
}

/// Atomically writes `file` to `path` (mode 0644: it is committed and
/// public), so an interrupted write never leaves a truncated config.
pub fn write(path: &Path, file: &File) -> Result<(), Error> {
    let text = toml::to_string(file).map_err(|source| Error::Encode {
        path: path.to_path_buf(),
        source,
    })?;
    atomicfile::write(path, text.as_bytes(), 0o644).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPIENT: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";

    fn valid_file() -> File {
        File {
            vault: VaultMeta {
                name: "myvault".into(),
                id: VaultId::try_from("9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497").unwrap(),
                vault_type: crate::vaultconfig::VaultType::Git,
                format_version: CURRENT_FORMAT_VERSION,
                created: "2026-08-29".into(),
            },
            method: Method {
                default: "passphrase".into(),
                ..Default::default()
            },
            recipients: vec![
                Recipient {
                    device: "laptop-1".into(),
                    pubkey: RECIPIENT.into(),
                },
                Recipient {
                    device: "recovery-key".into(),
                    pubkey:
                        "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0"
                            .into(),
                },
            ],
        }
    }

    /// Writes `file` and reads it back.
    fn write_then_read(file: &File) -> Result<File, Error> {
        write_then_read_with_id(file, file.vault.id.as_str())
    }

    /// Writes `file` with its id replaced by `id`, which no `File` can
    /// hold if it is invalid, and reads it back.
    fn write_then_read_with_id(file: &File, id: &str) -> Result<File, Error> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = toml::to_string(file).unwrap();
        let from = format!("id = \"{}\"", file.vault.id);
        assert!(text.contains(&from));
        let to = format!("id = {}", toml::Value::from(id));
        std::fs::write(&path, text.replace(&from, &to)).unwrap();
        read(&path)
    }

    #[test]
    fn round_trip() {
        let want = valid_file();
        assert_eq!(write_then_read(&want).unwrap(), want);
    }

    // The table and field names are the format's (ADR-0002).
    #[test]
    fn written_with_the_formats_table_and_field_names() {
        let mut file = valid_file();
        file.method.plugin = Some("yubikey".into());
        let text = toml::to_string(&file).unwrap();
        for needle in [
            "[vault]",
            "name = \"myvault\"",
            "id = \"9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497\"",
            "type = \"git\"",
            "format_version = 2",
            "created = \"2026-08-29\"",
            "[method]",
            "default = \"passphrase\"",
            "plugin = \"yubikey\"",
            "[[recipients]]",
            "device = \"laptop-1\"",
            "pubkey = ",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        assert!(!toml::to_string(&valid_file()).unwrap().contains("plugin"));
    }

    // The exhaustive pattern stops compiling if
    // a field is added, and the serialized keys are checked too.
    #[test]
    fn recipients_carry_only_device_and_pubkey() {
        let Recipient { device, pubkey } = Recipient::default();
        let _ = (device, pubkey);

        let table: toml::Table = toml::from_str(&toml::to_string(&valid_file()).unwrap()).unwrap();
        for recipient in table["recipients"].as_array().unwrap() {
            let keys: Vec<&str> = recipient
                .as_table()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, ["device", "pubkey"]);
        }
    }

    #[test]
    fn read_rejects_unrecognized_format_version() {
        let mut file = valid_file();
        file.vault.format_version = 99;

        let err = write_then_read(&file).unwrap_err();

        assert!(
            matches!(
                err,
                Error::UnsupportedFormatVersion {
                    found: 99,
                    skew: VersionSkew::Newer,
                    ..
                }
            ),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("upgrade gage"), "{msg}");
        assert!(msg.contains("99"), "{msg}");
        assert_eq!(err.kind(), ErrorKind::Conflict);
    }

    // Treated as older than this
    // build, never best-effort parsed.
    #[test]
    fn read_rejects_missing_format_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = toml::to_string(&valid_file()).unwrap();
        let without: String = text
            .lines()
            .filter(|l| !l.starts_with("format_version"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_ne!(without, text);
        std::fs::write(&path, without).unwrap();

        let err = read(&path).unwrap_err();

        assert!(
            matches!(
                err,
                Error::UnsupportedFormatVersion {
                    found: 0,
                    skew: VersionSkew::Older,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    // Telling the owner of a v1 vault to upgrade gage is advice that
    // can't work: the refusal is directional.
    #[test]
    fn read_refuses_a_vault_older_than_this_build_with_a_recreate_message() {
        let mut file = valid_file();
        file.vault.format_version = 1;

        let err = write_then_read(&file).unwrap_err();

        assert!(
            matches!(
                err,
                Error::UnsupportedFormatVersion {
                    found: 1,
                    skew: VersionSkew::Older,
                    ..
                }
            ),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(!msg.contains("upgrade gage"), "{msg}");
        assert!(msg.contains("re-create"), "{msg}");
        assert!(msg.contains("make reset-local-state"), "{msg}");
        assert!(msg.contains("format_version 1"), "{msg}");
    }

    // The "re-create the vault" story
    // only works if the marker that triggers it actually moved.
    #[test]
    fn current_format_version_is_two() {
        assert_eq!(CURRENT_FORMAT_VERSION, 2);
    }

    // Rust adds a distinguishable variant.
    #[test]
    fn read_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = read(&dir.path().join("nope.toml")).unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }), "{err:?}");
        assert_eq!(err.kind(), ErrorKind::Internal);
    }

    #[test]
    fn read_rejects_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[vault\nname = ").unwrap();
        let err = read(&path).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        assert_eq!(err.kind(), ErrorKind::Internal);
    }

    // The id becomes a path
    // component and arrives from a file any git-writer can edit, so it is
    // refused on read, before any path is built from it.
    #[test]
    fn read_rejects_an_id_that_is_not_a_uuid() {
        for (case, id) in [
            ("absent", ""),
            ("malformed", "not-a-uuid"),
            ("traversal", "../../../../etc/cron.d/x"),
            (
                "traversal in path",
                "../9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497",
            ),
            ("absolute path", "/etc/passwd"),
            ("windows traversal", r"..\..\Windows\x"),
            ("a vault name", "personal"),
            ("nil uuid", "00000000-0000-0000-0000-000000000000"),
        ] {
            let err = write_then_read_with_id(&valid_file(), id).unwrap_err();

            match &err {
                Error::InvalidVaultId { id: got, .. } => assert_eq!(got, id, "{case}"),
                other => panic!("{case}: {other:?}"),
            }
            assert_eq!(err.kind(), ErrorKind::Conflict, "{case}");
        }
    }

    // A missing `id` key, not only an empty one, is refused.
    #[test]
    fn read_rejects_an_absent_id_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text: String = toml::to_string(&valid_file())
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("id ="))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&path, text).unwrap();
        let err = read(&path).unwrap_err();
        assert!(matches!(err, Error::InvalidVaultId { .. }), "{err:?}");
    }

    /// Writes `file` with `from` replaced by `to`, and reads it back.
    fn write_then_read_replacing(
        file: &File,
        from: &str,
        to: &str,
    ) -> (String, Result<File, Error>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = toml::to_string(file).unwrap();
        assert!(text.contains(from), "{from:?} not in:\n{text}");
        let text = text.replace(from, to);
        std::fs::write(&path, &text).unwrap();
        (text, read(&path))
    }

    // ADR-0053: an unknown type is refused as the file is read, and the
    // refusal names it; until D3a it parsed and silently lost git behavior.
    #[test]
    fn read_refuses_an_unknown_vault_type_naming_it() {
        let (_, got) = write_then_read_replacing(&valid_file(), "type = \"git\"", "type = \"s3\"");
        let err = got.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        assert!(err.to_string().contains("s3"), "{err}");
    }

    // ADR-0053: an unknown method default is kept, so a vault naming a
    // method a later build adds still opens here, and is written back
    // byte for byte.
    #[test]
    fn read_keeps_an_unknown_method_default_and_rewrites_it_byte_identically() {
        let (text, got) = write_then_read_replacing(
            &valid_file(),
            "default = \"passphrase\"",
            "default = \"yubikey\"",
        );
        let file = got.unwrap();
        assert_eq!(file.method.default, UnlockMethod::Other("yubikey".into()));
        assert_eq!(toml::to_string(&file).unwrap(), text);
    }

    // Rejected, never
    // normalized: a normalized name is a different device.
    #[test]
    fn read_rejects_path_traversal_device_name_before_use() {
        for bad in [
            "../../../../etc/cron.d/x",
            "../../../etc/x",
            "/etc/passwd",
            r"..\..\Windows\x",
            "a/b",
            r"a\b",
            "C:x",
            "..",
            "",
            "Laptop-1",
        ] {
            let mut file = valid_file();
            // Not the first recipient: every one is checked.
            file.recipients[1].device = bad.into();

            let err = write_then_read(&file).unwrap_err();

            match &err {
                Error::InvalidDeviceName { device, .. } => assert_eq!(device, bad),
                other => panic!("{bad:?}: {other:?}"),
            }
            assert_eq!(err.kind(), ErrorKind::Conflict, "{bad:?}");
        }
    }

    // A file whose version
    // is unrecognized is never looked into, not even its recipients.
    #[test]
    fn format_version_checked_before_device_names() {
        let mut file = valid_file();
        file.vault.format_version = 99;
        file.recipients[0].device = "../../etc/x".into();

        let err = write_then_read(&file).unwrap_err();

        assert!(
            matches!(err, Error::UnsupportedFormatVersion { .. }),
            "{err:?}"
        );
    }

    // A v1 vault's missing id is
    // a version mismatch, not a malformed id: only one of those messages
    // says what actually happened.
    #[test]
    fn format_version_checked_before_the_id() {
        let mut file = valid_file();
        file.vault.format_version = 1;

        let err = write_then_read_with_id(&file, "").unwrap_err();

        assert!(
            matches!(err, Error::UnsupportedFormatVersion { .. }),
            "{err:?}"
        );
    }

    // The id is checked before the device names.
    #[test]
    fn the_id_is_checked_before_device_names() {
        let mut file = valid_file();
        file.recipients[0].device = "../y".into();

        let err = write_then_read_with_id(&file, "../x").unwrap_err();

        assert!(matches!(err, Error::InvalidVaultId { .. }), "{err:?}");
    }

    // The failure comes back named
    // as a vaultconfig write, not a bare atomicfile error.
    #[test]
    fn write_wraps_an_atomicfile_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does/not/exist/config.toml");

        let err = write(&path, &valid_file()).unwrap_err();

        assert!(matches!(err, Error::Write { .. }), "{err:?}");
        let msg = err.to_string();
        assert!(msg.starts_with("writing"), "{msg}");
    }

    #[cfg(unix)]
    #[test]
    fn written_with_mode_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, &valid_file()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}
