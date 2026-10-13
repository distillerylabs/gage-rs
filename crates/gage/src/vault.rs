//! One vault's on-disk state: config, recipients, entries. It stays a
//! stateless, identity-agnostic operator over ciphertext in both
//! invocation modes; `Session`, not `Vault`, owns any "how long does this
//! stay unlocked" bookkeeping. See "Library architecture" in the design doc.
//!
//! M1 creates vaults and resolves registered ones; M2 implements
//! [`Vault::unlock`].

use std::path::{Path, PathBuf};

use crate::agekey;
use crate::atomicfile;
use crate::config::VaultEntry;
use crate::devicename;
use crate::exitcode::{Code, HasCode};
use crate::gitrepo;
use crate::identity::Identity;
use crate::prompter::Prompter;
use crate::recipients;
use crate::vaultconfig;

/// The git vault type: today's only one.
pub const TYPE_GIT: &str = "git";

/// The passphrase identity method: today's only one.
pub const METHOD_PASSPHRASE: &str = "passphrase";

/// Every vault type this build accepts. Validated against a list rather
/// than hardcoded, so a second type is additive to the CLI surface rather
/// than a breaking change ("Vault types", Q-METHOD-FLAG).
pub const ALLOWED_TYPES: &[&str] = &[TYPE_GIT];

/// Every identity method this build accepts, for the same reason.
pub const ALLOWED_METHODS: &[&str] = &[METHOD_PASSPHRASE];

/// The message of the one commit [`Vault::create`] makes.
pub const INIT_COMMIT_MESSAGE: &str = "gage: initialize vault";

/// `.gitignore`: the standard OS cruft, nothing gage-specific, since
/// nothing gage writes into a vault's working tree is meant to stay
/// uncommitted ("On-disk layout").
const GITIGNORE: &str = ".DS_Store\nThumbs.db\n";

/// `.gitattributes`: any divergence in either recipient-defining file is a
/// real conflict, rather than git silently line-merging a recipient list
/// neither device wrote ("On-disk layout", Q-SYNC-CONFLICT).
const GITATTRIBUTES: &str = ".age-recipients -merge\n.gage/config.toml -merge\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    /// The name this device registered the vault under: a label for
    /// humans, never a path component.
    pub name: String,
    pub path: PathBuf,
    /// The vault's committed `[vault].id`, which keys every piece of
    /// per-vault local state (see `paths`).
    pub id: String,
}

/// Everything [`Vault::create`] needs: no identity and no crypto, only a
/// name, an id, a place, and the recipients' public keys as strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSpec {
    /// Recorded as `[vault].name`; it need not match `path`'s last
    /// component.
    pub name: String,
    /// The vault's id, recorded as `[vault].id` and never changed.
    ///
    /// Handed in rather than minted here on purpose: `gage init` has to
    /// know the id before this call (M2 files this device's identity
    /// under it first), and a `create` that minted its own would leave the
    /// vault carrying one id and the identity filed under another, with
    /// nothing visibly wrong. See A20.
    pub id: String,
    /// The target directory: absent, or an empty directory. `create` never
    /// writes into anything else.
    pub path: PathBuf,
    /// One of [`ALLOWED_TYPES`].
    pub vault_type: String,
    /// One of [`ALLOWED_METHODS`]: a default for devices joining, not a
    /// constraint on any of them ("Decryption methods are per-device").
    pub method: String,
    /// Labels the first recipient as this device's own.
    pub device: String,
    /// Public keys in file order; at least one. The first is labelled
    /// `device`; each later one `recipient-<n>` by its position, since a
    /// bare key from the command line carries no real device name. Repeats
    /// are written as given, as Go does.
    pub recipients: Vec<String>,
    /// An optional `origin` URL; `None` starts the vault local-only.
    pub remote: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateError {
    #[error("vault name is required")]
    EmptyName,
    #[error("vault id {0:?} is invalid; it must be a UUID")]
    InvalidId(String),
    #[error("unknown vault type {0:?}; accepted: {accepted}", accepted = ALLOWED_TYPES.join(", "))]
    UnknownType(String),
    #[error("unknown method {0:?}; accepted: {accepted}", accepted = ALLOWED_METHODS.join(", "))]
    UnknownMethod(String),
    #[error("device name {0:?} is invalid")]
    InvalidDeviceName(String),
    #[error("a vault needs at least one recipient; the first must be this device's own public key")]
    NoRecipients,
    #[error(transparent)]
    InvalidRecipient(#[from] agekey::Error),
    #[error("target path is required")]
    EmptyPath,
    #[error("{} already exists and is not a directory", .0.display())]
    TargetNotADirectory(PathBuf),
    #[error("{} already exists and is not empty", .0.display())]
    TargetNotEmpty(PathBuf),
    #[error("inspecting {}: {source}", path.display())]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("creating {}: {source}", path.display())]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Recipients(#[from] recipients::Error),
    #[error("writing {}: {source}", path.display())]
    WriteFile {
        path: PathBuf,
        source: atomicfile::Error,
    },
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
}

impl HasCode for CreateError {
    fn code(&self) -> Code {
        use CreateError::*;
        match self {
            EmptyName | InvalidId(_) | UnknownType(_) | UnknownMethod(_) | InvalidDeviceName(_)
            | NoRecipients | InvalidRecipient(_) | EmptyPath => Code::Usage,
            TargetNotADirectory(_) | TargetNotEmpty(_) => Code::Conflict,
            Inspect { .. }
            | CreateDir { .. }
            | Config(_)
            | Recipients(_)
            | WriteFile { .. }
            | Git(_) => Code::Internal,
        }
    }
}

/// Why a registration couldn't be resolved to the vault it names.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// The registration and the vault at its path carry different ids.
    #[error(
        "the registration for {name:?} points at a different vault than the one at {} (registered id {}, vault id {vault_id}); re-register it with `gage vault remove {name}` and `gage clone`",
        path.display(),
        if registered_id.is_empty() { "none recorded" } else { registered_id.as_str() }
    )]
    Mismatch {
        name: String,
        path: PathBuf,
        registered_id: String,
        vault_id: String,
    },
    /// The vault's config is readable but unusable: a version this build
    /// doesn't speak, or an id it won't build a path from.
    #[error(transparent)]
    Config(vaultconfig::Error),
}

impl HasCode for ResolveError {
    fn code(&self) -> Code {
        match self {
            ResolveError::Mismatch { .. } => Code::Conflict,
            ResolveError::Config(e) => e.code(),
        }
    }
}

/// What `vault info` reports, read from the vault itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// The committed id, not the registration's copy.
    pub id: String,
    pub vault_type: String,
    pub method: String,
    pub recipients: usize,
    /// Present for a git vault only: remote and clean/dirty state mean
    /// nothing to another vault type ("Git-specific commands").
    pub git: Option<GitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    pub remote: Option<String>,
    pub clean: bool,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InfoError {
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
}

impl HasCode for InfoError {
    fn code(&self) -> Code {
        match self {
            InfoError::Config(e) => e.code(),
            InfoError::Git(e) => e.code(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnlockError {
    /// Unlocking arrives with identities in M2.
    #[error("unlocking a vault is not implemented yet")]
    NotImplemented,
}

impl HasCode for UnlockError {
    fn code(&self) -> Code {
        Code::LockedOrAuth
    }
}

impl Vault {
    /// Lays out a brand-new vault: the directory, `.gage/config.toml`,
    /// `.age-recipients`, `.gitignore`, `.gitattributes`, an empty
    /// `entries/`, a git repository and one commit of all of it, plus
    /// `origin` if a remote is given. No encryption and no identity: the
    /// vault it returns is created, not unlocked.
    ///
    /// Every field is validated here, before anything is touched, rather
    /// than trusted to a frontend: a GUI may call this directly.
    pub fn create(spec: &CreateSpec) -> Result<Vault, CreateError> {
        validate(spec)?;
        require_empty_or_absent(&spec.path)?;
        write_skeleton(spec)?;
        gitrepo::init_and_commit(&spec.path, INIT_COMMIT_MESSAGE)?;
        if let Some(remote) = &spec.remote {
            gitrepo::set_remote(&spec.path, remote)?;
        }
        Ok(Vault {
            name: spec.name.clone(),
            path: spec.path.clone(),
            id: spec.id.clone(),
        })
    }

    /// The vault a registration names, checked against the vault at its
    /// path. The one resolver every command that reads a vault goes
    /// through, so none can skip the check.
    ///
    /// The id comes from the registration, because that copy exists for
    /// when the vault's files are gone (`vault remove`). Wherever the
    /// vault's own config is readable the two are compared, and a
    /// mismatch is refused rather than either side silently winning: a
    /// vault re-created at the same path, a registration repointed by
    /// hand or a restored backup would otherwise look this device's
    /// identity up under one vault's id and read another's recipients.
    ///
    /// A vault whose config can't be read at all is not a mismatch (it is
    /// the case the registration's copy covers), but a config that reads
    /// and is unusable (wrong `format_version`, invalid id) is reported as
    /// itself: "re-create this vault" is better advice than a failure on
    /// an empty id several steps later.
    pub fn from_registration(name: &str, entry: &VaultEntry) -> Result<Vault, ResolveError> {
        match vaultconfig::read(&config_path(&entry.path)) {
            Ok(file) if file.vault.id != entry.id => {
                return Err(ResolveError::Mismatch {
                    name: name.to_string(),
                    path: entry.path.clone(),
                    registered_id: entry.id.clone(),
                    vault_id: file.vault.id,
                });
            }
            Err(
                e @ (vaultconfig::Error::UnsupportedFormatVersion { .. }
                | vaultconfig::Error::InvalidVaultId { .. }),
            ) => return Err(ResolveError::Config(e)),
            // Matching, or unreadable (or failing a check the caller's own
            // read of the file reports): the registration's id stands.
            _ => {}
        }
        Ok(Vault {
            name: name.to_string(),
            path: entry.path.clone(),
            id: entry.id.clone(),
        })
    }

    /// Reads the vault's committed config (validated) and, for a git
    /// vault, its remote and working-tree state.
    pub fn info(&self) -> Result<Info, InfoError> {
        let file = vaultconfig::read(&self.config_path())?;
        let git = if file.vault.vault_type == TYPE_GIT {
            Some(GitInfo {
                remote: gitrepo::remote_url(&self.path)?,
                clean: gitrepo::is_clean(&self.path)?,
            })
        } else {
            None
        };
        Ok(Info {
            id: file.vault.id,
            vault_type: file.vault.vault_type,
            method: file.method.default,
            recipients: file.recipients.len(),
            git,
        })
    }

    /// Proves this device can decrypt the vault, asking `prompter` for
    /// whatever its method needs, and returns the unlocked identity. The
    /// only producer of an [`Identity`].
    pub fn unlock(&self, _prompter: &dyn Prompter) -> Result<Identity, UnlockError> {
        Err(UnlockError::NotImplemented)
    }

    /// Where this vault's committed config lives.
    pub fn config_path(&self) -> PathBuf {
        config_path(&self.path)
    }
}

fn config_path(vault: &Path) -> PathBuf {
    vault.join(".gage").join("config.toml")
}

/// Go's order: name, id, type, method, device, recipients, path. Nothing
/// on disk is consulted.
fn validate(spec: &CreateSpec) -> Result<(), CreateError> {
    if spec.name.is_empty() {
        return Err(CreateError::EmptyName);
    }
    if !vaultconfig::valid_id(&spec.id) {
        return Err(CreateError::InvalidId(spec.id.clone()));
    }
    if !ALLOWED_TYPES.contains(&spec.vault_type.as_str()) {
        return Err(CreateError::UnknownType(spec.vault_type.clone()));
    }
    if !ALLOWED_METHODS.contains(&spec.method.as_str()) {
        return Err(CreateError::UnknownMethod(spec.method.clone()));
    }
    if !devicename::valid(&spec.device) {
        return Err(CreateError::InvalidDeviceName(spec.device.clone()));
    }
    if spec.recipients.is_empty() {
        return Err(CreateError::NoRecipients);
    }
    for key in &spec.recipients {
        agekey::validate_recipient(key)?;
    }
    if spec.path.as_os_str().is_empty() {
        return Err(CreateError::EmptyPath);
    }
    Ok(())
}

/// Refuses a target that exists and is a non-empty directory or not a
/// directory at all. Reads only; runs before any write.
fn require_empty_or_absent(path: &Path) -> Result<(), CreateError> {
    let inspect = |source| CreateError::Inspect {
        path: path.to_path_buf(),
        source,
    };
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(inspect(e)),
    };
    if !meta.is_dir() {
        return Err(CreateError::TargetNotADirectory(path.to_path_buf()));
    }
    if std::fs::read_dir(path).map_err(inspect)?.next().is_some() {
        return Err(CreateError::TargetNotEmpty(path.to_path_buf()));
    }
    Ok(())
}

/// Writes every file `create` promises; the git plumbing is separate, so
/// what's on disk and how it's versioned stay easy to reason about apart.
fn write_skeleton(spec: &CreateSpec) -> Result<(), CreateError> {
    // entries/ holds ciphertext, but its listing still leaks how many
    // secrets exist and when each changed, so 0700 like the identities
    // directory, where the rest of the committed config gets 0750. It is
    // created first, and the vault directory with it, so a vault created
    // from nothing is 0700 too, as Go's MkdirAll leaves it.
    make_dir(&spec.path.join("entries"), 0o700)?;
    make_dir(&spec.path.join(".gage"), 0o750)?;

    let recipients: Vec<vaultconfig::Recipient> = spec
        .recipients
        .iter()
        .enumerate()
        .map(|(i, key)| vaultconfig::Recipient {
            device: if i == 0 {
                spec.device.clone()
            } else {
                format!("recipient-{}", i + 1)
            },
            pubkey: key.clone(),
        })
        .collect();
    let file = vaultconfig::File {
        vault: vaultconfig::VaultMeta {
            name: spec.name.clone(),
            id: spec.id.clone(),
            vault_type: spec.vault_type.clone(),
            format_version: vaultconfig::CURRENT_FORMAT_VERSION,
            created: utc_date_today(),
        },
        method: vaultconfig::Method {
            default: spec.method.clone(),
            ..Default::default()
        },
        recipients,
    };
    vaultconfig::write(&config_path(&spec.path), &file)?;
    recipients::write(&spec.path.join(".age-recipients"), &spec.recipients)?;
    for (name, content) in [(".gitignore", GITIGNORE), (".gitattributes", GITATTRIBUTES)] {
        let path = spec.path.join(name);
        atomicfile::write(&path, content.as_bytes(), 0o644)
            .map_err(|source| CreateError::WriteFile { path, source })?;
    }
    Ok(())
}

/// Creates `path` and any missing parents with `mode` (Unix), then sets
/// `path`'s own mode explicitly so the umask can't loosen or tighten it.
fn make_dir(path: &Path, mode: u32) -> Result<(), CreateError> {
    let fail = |source| CreateError::CreateDir {
        path: path.to_path_buf(),
        source,
    };
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(mode);
        builder.create(path).map_err(fail)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(fail)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        builder.create(path).map_err(fail)
    }
}

/// Today's date in UTC, `YYYY-MM-DD`: the RFC 3339 timestamp's date part.
fn utc_date_today() -> String {
    let now = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    now[..10].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompter::FakePrompter;

    const RECIPIENT_1: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";
    const RECIPIENT_2: &str =
        "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0";

    /// A valid spec targeting a not-yet-existing directory inside `root`.
    fn valid_spec(root: &tempfile::TempDir, name: &str) -> CreateSpec {
        CreateSpec {
            name: name.into(),
            id: vaultconfig::new_id(),
            path: root.path().join(name),
            vault_type: TYPE_GIT.into(),
            method: METHOD_PASSPHRASE.into(),
            device: "laptop-1".into(),
            recipients: vec![RECIPIENT_1.into()],
            remote: None,
        }
    }

    fn created(spec: &CreateSpec) -> vaultconfig::File {
        Vault::create(spec).unwrap();
        vaultconfig::read(&config_path(&spec.path)).unwrap()
    }

    /// Every rejection fails "before creating any files": an exit code
    /// alone can't tell a clean rejection from one that bailed out halfway
    /// through writing a vault.
    fn assert_nothing_created(spec: &CreateSpec) {
        assert!(
            !spec.path.exists(),
            "create touched {} despite rejecting the spec",
            spec.path.display()
        );
    }

    fn assert_rejected(spec: &CreateSpec, code: Code) -> CreateError {
        let err = Vault::create(spec).unwrap_err();
        assert_eq!(err.code(), code, "{err}");
        assert_nothing_created(spec);
        err
    }

    /// The paths in HEAD's tree, so a test asserts what was committed
    /// rather than what merely exists on disk.
    fn committed_files(dir: &Path) -> Vec<String> {
        let repo = git2::Repository::open(dir).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let mut files = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                files.push(format!("{root}{}", entry.name().unwrap()));
            }
            git2::TreeWalkResult::Ok
        })
        .unwrap();
        files.sort();
        files
    }

    // Go: TestCreateWritesFullSkeleton
    #[test]
    fn create_writes_full_skeleton() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");

        let vault = Vault::create(&spec).unwrap();

        assert_eq!(
            vault,
            Vault {
                name: "myvault".into(),
                path: spec.path.clone(),
                id: spec.id.clone(),
            }
        );
        for rel in [
            ".gage/config.toml",
            ".age-recipients",
            "entries",
            ".gitignore",
            ".gitattributes",
            ".git",
        ] {
            assert!(spec.path.join(rel).exists(), "{rel} missing");
        }
        assert!(spec.path.join("entries").is_dir());
        assert_eq!(gitrepo::commit_count(&spec.path).unwrap(), 1);
        assert!(gitrepo::is_clean(&spec.path).unwrap());
    }

    // Go: TestCreateCommitsTheSkeletonFiles. entries/ is absent: git can't
    // track an empty directory, so a new vault commits four files.
    #[test]
    fn create_commits_the_skeleton_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        assert_eq!(
            committed_files(&spec.path),
            [
                ".age-recipients",
                ".gage/config.toml",
                ".gitattributes",
                ".gitignore"
            ]
        );
    }

    // The plan's git defaults, through create.
    #[test]
    fn create_commits_as_gage_on_main() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        let repo = git2::Repository::open(&spec.path).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(head.name().unwrap(), "refs/heads/main");
        let commit = head.peel_to_commit().unwrap();
        assert_eq!(commit.message().unwrap(), INIT_COMMIT_MESSAGE);
        assert_eq!(commit.author().name().unwrap(), "gage");
        assert_eq!(commit.author().email().unwrap(), "gage@localhost");
    }

    // Go: TestCreateGitattributesMarksRecipientFilesUnmergeable
    #[test]
    fn create_gitattributes_marks_recipient_files_unmergeable() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        let content = std::fs::read_to_string(spec.path.join(".gitattributes")).unwrap();
        for want in [".age-recipients -merge", ".gage/config.toml -merge"] {
            assert!(content.contains(want), "{content:?}");
        }
        // And git itself reads them that way.
        let repo = git2::Repository::open(&spec.path).unwrap();
        for file in [".age-recipients", ".gage/config.toml"] {
            let value = repo
                .get_attr(
                    Path::new(file),
                    "merge",
                    git2::AttrCheckFlags::FILE_THEN_INDEX,
                )
                .unwrap();
            assert_eq!(
                git2::AttrValue::from_string(value),
                git2::AttrValue::False,
                "{file}"
            );
        }
    }

    // Go: TestCreateGitignoreHasOnlyOSCruft
    #[test]
    fn create_gitignore_has_only_os_cruft() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        let content = std::fs::read_to_string(spec.path.join(".gitignore")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, [".DS_Store", "Thumbs.db"]);
    }

    // Go: TestCreateVaultConfigRoundTrip
    #[test]
    fn create_vault_config_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.recipients = vec![RECIPIENT_1.into(), RECIPIENT_2.into()];

        let file = created(&spec);

        assert_eq!(file.vault.name, "myvault");
        assert_eq!(file.vault.vault_type, TYPE_GIT);
        assert_eq!(
            file.vault.format_version,
            vaultconfig::CURRENT_FORMAT_VERSION
        );
        assert_eq!(file.method.default, METHOD_PASSPHRASE);
        assert_eq!(file.method.plugin, "");
        assert_eq!(
            file.recipients,
            [
                vaultconfig::Recipient {
                    device: "laptop-1".into(),
                    pubkey: RECIPIENT_1.into(),
                },
                vaultconfig::Recipient {
                    device: "recipient-2".into(),
                    pubkey: RECIPIENT_2.into(),
                },
            ]
        );
    }

    // The plan's decision: `created` is today's UTC date, YYYY-MM-DD.
    #[test]
    fn create_records_the_utc_creation_date() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        let before = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
        let created = created(&spec).vault.created;
        let after = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
        assert_eq!(created.len(), 10, "{created:?}");
        assert!(
            created == before[..10] || created == after[..10],
            "{created} not in [{before}, {after}]"
        );
    }

    // Go: TestCreateRecipientsMatchAgeRecipientsFile
    #[test]
    fn create_recipients_match_age_recipients_file() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.recipients = vec![RECIPIENT_1.into(), RECIPIENT_2.into()];

        let file = created(&spec);

        let keys = recipients::read(&spec.path.join(".age-recipients")).unwrap();
        let config_keys: Vec<String> = file.recipients.into_iter().map(|r| r.pubkey).collect();
        assert_eq!(keys, config_keys);
    }

    // Go: TestCreateRepeatedRecipientsAllWritten. Repeats are legal and
    // written as given; each later one is labelled by its position.
    #[test]
    fn create_repeated_recipients_all_written() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.recipients = vec![RECIPIENT_1.into(), RECIPIENT_2.into(), RECIPIENT_1.into()];

        let file = created(&spec);

        let keys = recipients::read(&spec.path.join(".age-recipients")).unwrap();
        assert_eq!(keys, [RECIPIENT_1, RECIPIENT_2, RECIPIENT_1]);
        let labels: Vec<&str> = file.recipients.iter().map(|r| r.device.as_str()).collect();
        assert_eq!(labels, ["laptop-1", "recipient-2", "recipient-3"]);
    }

    // Go: TestCreateRecordsTheIDItWasHandedRatherThanMintingOne
    #[test]
    fn create_records_the_id_it_was_handed_rather_than_minting_one() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");

        let vault = Vault::create(&spec).unwrap();

        let file = vaultconfig::read(&vault.config_path()).unwrap();
        assert_eq!(file.vault.id, spec.id);
        assert_eq!(vault.id, spec.id);
        assert_eq!(file.vault.format_version, 2);
    }

    // Go: TestCreateRefusesAnIDThatIsNotAUUID
    #[test]
    fn create_refuses_an_id_that_is_not_a_uuid() {
        let root = tempfile::tempdir().unwrap();
        for bad in ["", "personal", "../../etc/x", "not-a-uuid"] {
            let mut spec = valid_spec(&root, "myvault");
            spec.id = bad.into();
            let err = assert_rejected(&spec, Code::Usage);
            assert!(matches!(err, CreateError::InvalidId(_)), "{bad:?}: {err:?}");
        }
    }

    // Go: TestCreateRejectsEmptyName. The first check, ahead of every
    // other field.
    #[test]
    fn create_rejects_empty_name() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.name = String::new();
        spec.id = "bad".into();
        let err = assert_rejected(&spec, Code::Usage);
        assert!(matches!(err, CreateError::EmptyName), "{err:?}");
    }

    // Go: TestCreateRejectsEmptyPath
    #[test]
    fn create_rejects_empty_path() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.path = PathBuf::new();
        let err = Vault::create(&spec).unwrap_err();
        assert!(matches!(err, CreateError::EmptyPath), "{err:?}");
        assert_eq!(err.code(), Code::Usage);
    }

    // Go: TestCreateRejectsUnknownType
    #[test]
    fn create_rejects_unknown_type() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.vault_type = "s3".into();
        let err = assert_rejected(&spec, Code::Usage);
        assert!(err.to_string().contains("git"), "{err}");
    }

    // Go: TestCreateAcceptsExplicitGitType
    #[test]
    fn create_accepts_explicit_git_type() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.vault_type = "git".into();
        Vault::create(&spec).unwrap();
    }

    // Go: TestCreateRejectsUnknownMethod. The future-work methods the
    // design doc names fail like any other unknown value.
    #[test]
    fn create_rejects_unknown_method() {
        let root = tempfile::tempdir().unwrap();
        for method in [
            "ssh",
            "yubikey",
            "age-key",
            "secure-enclave",
            "plugin:foo",
            "bogus",
        ] {
            let mut spec = valid_spec(&root, "myvault");
            spec.method = method.into();
            let err = assert_rejected(&spec, Code::Usage);
            assert!(err.to_string().contains("passphrase"), "{method}: {err}");
        }
    }

    // Go: TestCreateAcceptsExplicitPassphraseMethod
    #[test]
    fn create_accepts_explicit_passphrase_method() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.method = "passphrase".into();
        Vault::create(&spec).unwrap();
    }

    // Go: TestCreateRequiresAtLeastOneRecipient
    #[test]
    fn create_requires_at_least_one_recipient() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.recipients.clear();
        let err = assert_rejected(&spec, Code::Usage);
        assert!(err.to_string().contains("recipient"), "{err}");
    }

    // Go: TestCreateRejectsMalformedRecipient. Every key is checked, not
    // only the first.
    #[test]
    fn create_rejects_malformed_recipient() {
        let root = tempfile::tempdir().unwrap();
        for keys in [
            vec!["not-a-valid-key"],
            vec![RECIPIENT_1, "not-a-valid-key"],
        ] {
            let mut spec = valid_spec(&root, "myvault");
            spec.recipients = keys.into_iter().map(String::from).collect();
            let err = assert_rejected(&spec, Code::Usage);
            assert!(matches!(err, CreateError::InvalidRecipient(_)), "{err:?}");
        }
    }

    // Go: TestCreateRejectsInvalidDeviceName
    #[test]
    fn create_rejects_invalid_device_name() {
        let root = tempfile::tempdir().unwrap();
        for bad in [
            "../../../etc/x",
            "/etc/passwd",
            r"..\..\windows\x",
            "UpperCase",
            "",
        ] {
            let mut spec = valid_spec(&root, "myvault");
            spec.device = bad.into();
            let err = assert_rejected(&spec, Code::Usage);
            assert!(matches!(err, CreateError::InvalidDeviceName(_)), "{bad:?}");
        }
    }

    // Go: TestCreateDeviceNameLandsInRecipientAndIdentityPath, the
    // recipient-label half. The identity-path half moves to P2 · M2.
    #[test]
    fn create_device_name_lands_in_recipient() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.device = "custom-device".into();
        assert_eq!(created(&spec).recipients[0].device, "custom-device");
    }

    // Go: TestCreateIntoNonEmptyDirectoryFailsWithoutTouchingFiles
    #[test]
    fn create_into_non_empty_directory_fails_without_touching_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        std::fs::create_dir(&spec.path).unwrap();
        std::fs::write(spec.path.join("keep.txt"), "do not touch").unwrap();

        let err = Vault::create(&spec).unwrap_err();

        assert!(matches!(err, CreateError::TargetNotEmpty(_)), "{err:?}");
        assert_eq!(err.code(), Code::Conflict);
        let names: Vec<_> = std::fs::read_dir(&spec.path)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["keep.txt"]);
        assert_eq!(
            std::fs::read_to_string(spec.path.join("keep.txt")).unwrap(),
            "do not touch"
        );
    }

    // An existing, empty directory is a valid target.
    #[test]
    fn create_into_an_empty_directory_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        std::fs::create_dir(&spec.path).unwrap();
        Vault::create(&spec).unwrap();
        assert_eq!(gitrepo::commit_count(&spec.path).unwrap(), 1);
    }

    // Go: TestCreateRejectsATargetThatIsARegularFile
    #[test]
    fn create_rejects_a_target_that_is_a_regular_file() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        spec.path = root.path().join("not-a-directory");
        std::fs::write(&spec.path, "i am a file").unwrap();

        let err = Vault::create(&spec).unwrap_err();

        assert!(
            matches!(err, CreateError::TargetNotADirectory(_)),
            "{err:?}"
        );
        assert_eq!(err.code(), Code::Conflict);
        assert_eq!(std::fs::read_to_string(&spec.path).unwrap(), "i am a file");
    }

    // Go: TestCreateWithRemoteSetsOrigin. The remote needn't exist:
    // setting origin contacts nothing.
    #[test]
    fn create_with_remote_sets_origin() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = valid_spec(&root, "myvault");
        let remote = "/does/not/need/to/exist/for/set-remote.git";
        spec.remote = Some(remote.into());
        Vault::create(&spec).unwrap();
        assert_eq!(
            gitrepo::remote_url(&spec.path).unwrap().as_deref(),
            Some(remote)
        );
    }

    // Go: TestCreateWithoutRemoteLeavesVaultRemoteless
    #[test]
    fn create_without_remote_leaves_vault_remoteless() {
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        assert_eq!(gitrepo::remote_url(&spec.path).unwrap(), None);
    }

    // The plan's file modes. entries/ is 0700 because its listing leaks
    // how many secrets exist and when each was touched; the committed
    // config is public to every recipient.
    #[cfg(unix)]
    #[test]
    fn create_sets_the_skeletons_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let spec = valid_spec(&root, "myvault");
        Vault::create(&spec).unwrap();
        let mode = |rel: &str| {
            std::fs::metadata(spec.path.join(rel))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("entries"), 0o700);
        assert_eq!(mode(".gage"), 0o750);
        for file in [
            ".gage/config.toml",
            ".age-recipients",
            ".gitignore",
            ".gitattributes",
        ] {
            assert_eq!(mode(file), 0o644, "{file}");
        }
    }

    /// A created vault and the registration `gage init` would write for
    /// it.
    fn registered(root: &tempfile::TempDir) -> (CreateSpec, VaultEntry) {
        let spec = valid_spec(root, "personal");
        Vault::create(&spec).unwrap();
        let entry = VaultEntry {
            path: spec.path.clone(),
            id: spec.id.clone(),
            vault_type: TYPE_GIT.into(),
            device: spec.device.clone(),
            method: METHOD_PASSPHRASE.into(),
            ..Default::default()
        };
        (spec, entry)
    }

    /// Replaces the first line of the vault's committed config that starts
    /// with `prefix`, as any git-writer can.
    fn rewrite_config_line(spec: &CreateSpec, prefix: &str, replacement: &str) {
        let path = config_path(&spec.path);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut done = false;
        let lines: Vec<String> = text
            .lines()
            .map(|l| {
                if !done && l.trim_start().starts_with(prefix) {
                    done = true;
                    replacement.to_string()
                } else {
                    l.to_string()
                }
            })
            .collect();
        assert!(done, "no line starting with {prefix:?} in:\n{text}");
        std::fs::write(&path, lines.join("\n")).unwrap();
    }

    #[test]
    fn from_registration_resolves_a_matching_vault() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        let vault = Vault::from_registration("personal", &entry).unwrap();
        assert_eq!(
            vault,
            Vault {
                name: "personal".into(),
                path: spec.path,
                id: spec.id,
            }
        );
    }

    // Go: TestAMismatchedRegistrationIsRefusedRatherThanSilentlyPreferred
    // (library half; the CLI half is in gage-cli).
    #[test]
    fn from_registration_refuses_a_mismatched_id() {
        let root = tempfile::tempdir().unwrap();
        let (spec, mut entry) = registered(&root);
        let stranger = vaultconfig::new_id();
        entry.id = stranger.clone();

        let err = Vault::from_registration("personal", &entry).unwrap_err();

        assert_eq!(err.code(), Code::Conflict);
        let msg = err.to_string();
        for want in ["different vault", "re-register", &stranger, &spec.id] {
            assert!(msg.contains(want), "{want:?} missing from {msg}");
        }
    }

    // A registration with no id at all is a mismatch too, and says so.
    #[test]
    fn from_registration_refuses_a_registration_without_an_id() {
        let root = tempfile::tempdir().unwrap();
        let (_, mut entry) = registered(&root);
        entry.id.clear();
        let err = Vault::from_registration("personal", &entry).unwrap_err();
        assert!(matches!(err, ResolveError::Mismatch { .. }), "{err:?}");
        assert!(err.to_string().contains("none recorded"), "{err}");
    }

    // Go: TestAnUnreadableVaultFallsBackToGlobalConfigsIDAlone (library
    // half): with the vault's files gone, the registration's id stands.
    #[test]
    fn from_registration_falls_back_to_the_registrations_id_when_unreadable() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        std::fs::remove_dir_all(&spec.path).unwrap();
        let vault = Vault::from_registration("personal", &entry).unwrap();
        assert_eq!(vault.id, entry.id);
    }

    #[test]
    fn from_registration_reports_an_unsupported_version_as_itself() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        rewrite_config_line(&spec, "format_version", "format_version = 1");
        let err = Vault::from_registration("personal", &entry).unwrap_err();
        assert!(
            matches!(
                err,
                ResolveError::Config(vaultconfig::Error::UnsupportedFormatVersion { .. })
            ),
            "{err:?}"
        );
        assert_eq!(err.code(), Code::Conflict);
        assert!(err.to_string().contains("re-create"), "{err}");
    }

    #[test]
    fn from_registration_reports_an_invalid_committed_id_as_itself() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        rewrite_config_line(&spec, "id =", "id = \"../../etc/x\"");
        let err = Vault::from_registration("personal", &entry).unwrap_err();
        assert!(
            matches!(
                err,
                ResolveError::Config(vaultconfig::Error::InvalidVaultId { .. })
            ),
            "{err:?}"
        );
        assert_eq!(err.code(), Code::Conflict);
    }

    #[test]
    fn info_reports_the_vaults_own_fields() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        let vault = Vault::from_registration("personal", &entry).unwrap();
        assert_eq!(
            vault.info().unwrap(),
            Info {
                id: spec.id,
                vault_type: TYPE_GIT.into(),
                method: METHOD_PASSPHRASE.into(),
                recipients: 1,
                git: Some(GitInfo {
                    remote: None,
                    clean: true,
                }),
            }
        );
    }

    #[test]
    fn info_reports_the_remote_and_a_dirty_tree() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        gitrepo::set_remote(&spec.path, "/somewhere.git").unwrap();
        std::fs::write(spec.path.join("entries/stray.age"), "x").unwrap();

        let info = Vault::from_registration("personal", &entry)
            .unwrap()
            .info()
            .unwrap();

        assert_eq!(
            info.git,
            Some(GitInfo {
                remote: Some("/somewhere.git".into()),
                clean: false,
            })
        );
    }

    #[test]
    fn info_refuses_a_tampered_device_name() {
        let root = tempfile::tempdir().unwrap();
        let (spec, entry) = registered(&root);
        rewrite_config_line(&spec, "device", "device = '../../../etc/cron.d/x'");
        let vault = Vault::from_registration("personal", &entry).unwrap();
        let err = vault.info().unwrap_err();
        assert!(
            matches!(
                err,
                InfoError::Config(vaultconfig::Error::InvalidDeviceName { .. })
            ),
            "{err:?}"
        );
        assert_eq!(err.code(), Code::Conflict);
    }

    // The contract M2 fills in: the signature takes any Prompter, and until
    // then the typed failure maps to LockedOrAuth without asking anything.
    #[test]
    fn unlock_is_typed_and_maps_to_locked_or_auth() {
        let vault = Vault {
            name: "personal".into(),
            path: "/data/vaults/personal".into(),
            id: "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497".into(),
        };
        let prompter = FakePrompter::new();

        let result: Result<Identity, UnlockError> = vault.unlock(&prompter);

        let err = result.unwrap_err();
        assert!(matches!(err, UnlockError::NotImplemented));
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert!(prompter.unlock_requests().is_empty());
    }
}
