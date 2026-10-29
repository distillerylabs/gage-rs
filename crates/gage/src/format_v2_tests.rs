//! The golden format-v2 vault (`tests/fixtures/format-v2/`) read through
//! the library: every schema in `doc/format/v2.md` parses from the
//! committed bytes and round-trips, the commit layout is the specified
//! one, and the fixture itself has not changed.
//!
//! These tests are a format guard. A change that needs one of them edited
//! to pass is a format change, which takes a `format_version` bump, a new
//! fixture and an ADR (`doc/format/README.md`).

use std::collections::BTreeMap;
use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config;
use crate::crypt;
use crate::enrollment::{Payload, normalize_code};
use crate::entry::{self, Entry};
use crate::prompter::FakePrompter;
use crate::recovery::RECOVERY_DEVICE_LABEL;
use crate::testsupport::format_v2::{self as fx, Installed};
use crate::trustcache::{KNOWN_CONFIG_FILE, TRUST_FILE, TrustRecord};
use crate::vault::Vault;
use crate::vaultconfig::{self, Method, Recipient, VersionSkew};
use crate::{gitrepo, identityfile, paths, recipients};

/// The installed vault, resolved the way a command resolves it.
fn vault(i: &Installed) -> Vault {
    let global = config::read(&paths::global_config_file(&i.dirs)).unwrap();
    Vault::from_registration(&i.dirs, fx::VAULT_NAME, &global.vaults[fx::VAULT_NAME]).unwrap()
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn text(path: &Path) -> String {
    String::from_utf8(read(path)).unwrap()
}

fn expected_recipients() -> Vec<Recipient> {
    vec![
        Recipient {
            device: fx::DEVICE.into(),
            pubkey: fx::DEVICE_PUBKEY.into(),
        },
        Recipient {
            device: RECOVERY_DEVICE_LABEL.into(),
            pubkey: fx::RECOVERY_PUBKEY.into(),
        },
    ]
}

/// The pending request's file name, and the expiry its epoch names.
fn pending_file(i: &Installed) -> (String, Timestamp) {
    let names: Vec<String> = std::fs::read_dir(i.vault.join(".gage/pending"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let name = names[0].clone();
    let epoch = name
        .strip_prefix(&format!("{}-", fx::REQUEST_ID))
        .and_then(|rest| rest.strip_suffix(".age"))
        .unwrap_or_else(|| panic!("{name:?} is not <request-id>-<epoch>.age"));
    let expires = Timestamp::from_second(epoch.parse().unwrap()).unwrap();
    (name, expires)
}

/// An instant inside the committed request's life.
fn inside_request_life(i: &Installed) -> Timestamp {
    pending_file(i).1 - SignedDuration::from_hours(1)
}

/// Parses `text` as `T`, serializes it, and parses that again: the value
/// must survive, and is returned.
fn round_trip<T>(text: &str) -> T
where
    T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
{
    let parsed: T = toml::from_str(text).unwrap();
    let again: T = toml::from_str(&toml::to_string(&parsed).unwrap()).unwrap();
    assert_eq!(again, parsed);
    parsed
}

#[test]
fn format_v2_vault_config_parses() {
    let i = fx::install();
    let file = vaultconfig::read(&i.vault.join(".gage/config.toml")).unwrap();

    assert_eq!(file.vault.format_version, 2);
    assert_eq!(file.vault.id, fx::VAULT_ID);
    assert_eq!(file.vault.name, fx::VAULT_NAME);
    assert_eq!(file.vault.vault_type, "git");
    file.vault
        .created
        .parse::<jiff::civil::Date>()
        .unwrap_or_else(|e| panic!("created {:?}: {e}", file.vault.created));
    assert_eq!(file.vault.created.len(), "YYYY-MM-DD".len());
    assert_eq!(
        file.method,
        Method {
            default: "passphrase".into(),
            plugin: None,
        }
    );
    assert_eq!(file.recipients, expected_recipients());

    // .age-recipients: the same keys, in the same order, one per line.
    assert_eq!(
        text(&i.vault.join(".age-recipients")),
        format!("{}\n{}\n", fx::DEVICE_PUBKEY, fx::RECOVERY_PUBKEY)
    );
    assert_eq!(
        recipients::read(&i.vault.join(".age-recipients")).unwrap(),
        [fx::DEVICE_PUBKEY, fx::RECOVERY_PUBKEY]
    );
    assert_eq!(
        text(&i.vault.join(".gitattributes")),
        ".age-recipients -merge\n.gage/config.toml -merge\n"
    );
    assert_eq!(text(&i.vault.join(".gitignore")), ".DS_Store\nThumbs.db\n");

    // entries/: one `<uuid>.age` per entry, nothing else.
    let mut names: Vec<String> = std::fs::read_dir(i.vault.join("entries"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), fx::ENTRIES.len(), "{names:?}");
    for name in &names {
        let stem = name.strip_suffix(".age").unwrap();
        let id = Uuid::try_parse(stem).unwrap();
        assert_eq!(id.get_version_num(), 4, "{name}");
        assert_eq!(id.hyphenated().to_string(), stem, "{name}");
    }
}

/// What a reader MUST refuse, checked against the fixture's own config
/// with one thing changed.
#[test]
fn format_v2_reader_rejects_what_the_spec_rejects() {
    let i = fx::install();
    let path = i.vault.join(".gage/config.toml");
    let original = text(&path);
    let with = |from: &str, to: &str| {
        assert!(original.contains(from), "{from:?} not in the fixture");
        std::fs::write(&path, original.replace(from, to)).unwrap();
        vaultconfig::read(&path).unwrap_err()
    };

    for (version, skew) in [(3, VersionSkew::Newer), (1, VersionSkew::Older)] {
        let err = with("format_version = 2", &format!("format_version = {version}"));
        assert!(
            matches!(err, vaultconfig::Error::UnsupportedFormatVersion { found, skew: s, .. } if found == version && s == skew),
            "{err:?}"
        );
    }
    let err = with("format_version = 2\n", "");
    assert!(
        matches!(
            err,
            vaultconfig::Error::UnsupportedFormatVersion { found: 0, .. }
        ),
        "{err:?}"
    );
    let err = with(fx::VAULT_ID, &fx::VAULT_ID.to_uppercase());
    assert!(
        matches!(err, vaultconfig::Error::InvalidVaultId { .. }),
        "{err:?}"
    );
    let err = with(
        &format!("device = \"{}\"", fx::DEVICE),
        "device = \"../laptop-1\"",
    );
    assert!(
        matches!(err, vaultconfig::Error::InvalidDeviceName { .. }),
        "{err:?}"
    );

    // An identity file claiming more scrypt work than the cap is refused
    // before any KDF runs.
    std::fs::write(&path, &original).unwrap();
    let identity = paths::identity_file_path(&i.dirs, &fx::vault_id(), fx::DEVICE).unwrap();
    let wrapped = read(&identity);
    let header_end = wrapped.windows(4).position(|w| w == b"\n---").unwrap();
    let header = String::from_utf8(wrapped[..header_end].to_vec()).unwrap();
    let stanza = header
        .lines()
        .find(|l| l.starts_with("-> scrypt "))
        .unwrap();
    let over_cap = format!(
        "{} {}",
        stanza.rsplit_once(' ').unwrap().0,
        crypt::SCRYPT_MAX_WORK_FACTOR + 1
    );
    let mut tampered = header.replace(stanza, &over_cap).into_bytes();
    tampered.extend_from_slice(&wrapped[header_end..]);
    std::fs::write(&identity, tampered).unwrap();
    let err = vault(&i)
        .unlock(&FakePrompter::new().with_passphrase(fx::PASSPHRASE))
        .unwrap_err();
    assert!(
        matches!(err, identityfile::UnlockError::CorruptIdentityFile(_)),
        "{err:?}"
    );
}

#[test]
fn format_v2_identity_file_unwraps() {
    let i = fx::install();
    let path = paths::identity_file_path(&i.dirs, &fx::vault_id(), fx::DEVICE).unwrap();

    // An age file with one scrypt stanza, at the factor it was written at.
    let wrapped = read(&path);
    let header = String::from_utf8_lossy(&wrapped[..wrapped.len().min(200)]).to_string();
    let mut lines = header.lines();
    assert_eq!(lines.next(), Some("age-encryption.org/v1"));
    let stanza: Vec<&str> = lines.next().unwrap().split(' ').collect();
    assert_eq!(&stanza[..2], ["->", "scrypt"]);
    assert_eq!(stanza[3], "10");

    // Its plaintext: two comment lines, then the key.
    let plaintext = crypt::decrypt_with(&wrapped, &crypt::scrypt_identity(fx::PASSPHRASE)).unwrap();
    let plaintext = String::from_utf8(plaintext).unwrap();
    let lines: Vec<&str> = plaintext.lines().collect();
    assert_eq!(lines.len(), 3, "{}", lines.len());
    let created = lines[0].strip_prefix("# created: ").unwrap();
    created.parse::<Timestamp>().unwrap();
    assert_eq!(lines[1], format!("# public key: {}", fx::DEVICE_PUBKEY));
    assert!(lines[2].starts_with("AGE-SECRET-KEY-1"));
    assert!(plaintext.ends_with('\n'));

    // The marker beside it names the vault.
    assert_eq!(
        text(&path.with_file_name(identityfile::MARKER_FILE_NAME)),
        format!("{}\n", fx::VAULT_NAME)
    );

    let identity = vault(&i)
        .unlock(&FakePrompter::new().with_passphrase(fx::PASSPHRASE))
        .unwrap();
    assert_eq!(identity.recipient(), fx::DEVICE_PUBKEY);
    assert_eq!(identity.device(), fx::DEVICE);
}

#[test]
fn format_v2_schemas_round_trip() {
    let i = fx::install();
    let vault = vault(&i);

    // .gage/config.toml, and the trust cache's copy of it.
    let committed = text(&i.vault.join(".gage/config.toml"));
    let file: vaultconfig::File = round_trip(&committed);
    assert_eq!(file.vault.id, fx::VAULT_ID);
    let trust_dir = paths::trust_cache_dir(&i.dirs, &fx::vault_id());
    let known = text(&trust_dir.join(KNOWN_CONFIG_FILE));
    assert_eq!(known, committed, "the trust cache confirmed this config");
    round_trip::<vaultconfig::File>(&known);

    // trust.toml: the hash is of .age-recipients' bytes, exactly.
    let record: TrustRecord = round_trip(&text(&trust_dir.join(TRUST_FILE)));
    let keys = read(&i.vault.join(".age-recipients"));
    let hex: String = Sha256::digest(&keys)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(record.recipients_hash, hex);
    assert!(record.confirmed_at.is_some());
    assert_eq!(record.mismatch_seen, None);

    // The global config.
    let global: config::Global = round_trip(&text(&paths::global_config_file(&i.dirs)));
    assert_eq!(global.current, Some(fx::VAULT_NAME.into()));

    // The enrollment payload inside the pending request.
    let (name, expires) = pending_file(&i);
    let sealed = read(&i.vault.join(".gage/pending").join(&name));
    let code = normalize_code(fx::ENROLLMENT_CODE).unwrap();
    let plaintext =
        crypt::decrypt_with(&sealed, &crypt::enrollment_scrypt_identity(&code)).unwrap();
    let payload: Payload = round_trip(std::str::from_utf8(&plaintext).unwrap());
    assert_eq!(payload.request_id, fx::REQUEST_ID);
    assert_eq!(payload.vault_id, fx::VAULT_ID);
    assert_eq!(payload.device, fx::ENROLLING_DEVICE);
    assert_eq!(payload.method, "passphrase");
    assert_eq!(payload.expires.parse::<Timestamp>().unwrap(), expires);
    payload.created.parse::<Timestamp>().unwrap();

    // Every entry: the committed YAML parses to what was written, and
    // survives marshal and unmarshal, unknown keys included.
    let identity = vault
        .unlock(&FakePrompter::new().with_passphrase(fx::PASSPHRASE))
        .unwrap();
    let mut by_title: BTreeMap<String, Entry> = BTreeMap::new();
    for id in vault.entry_ids().unwrap() {
        let path = i.vault.join("entries").join(format!("{id}.age"));
        let yaml = crypt::decrypt(&read(&path), &identity).unwrap();
        let e = entry::unmarshal(&yaml).unwrap();
        let again = entry::unmarshal(&entry::marshal(&e).unwrap()).unwrap();
        assert_eq!(again, e, "{:?} changed on the round trip", e.title);
        by_title.insert(e.title.clone(), e);
    }
    assert_eq!(by_title.len(), fx::ENTRIES.len());
    for want in fx::ENTRIES {
        let got = &by_title[want.title];
        assert_eq!(got.description, want.description, "{}", want.title);
        assert_eq!(got.value, want.value, "{}", want.title);
        let fields: BTreeMap<String, String> = want
            .fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(got.fields, fields, "{}", want.title);
        assert_eq!(got.updated_by, fx::DEVICE, "{}", want.title);
        assert!(got.created.is_some(), "{}", want.title);
        assert_eq!(got.created, got.updated, "{} was never edited", want.title);
        let extra = if want.title == fx::RICH.title {
            fx::rich_extra()
        } else {
            BTreeMap::new()
        };
        assert_eq!(got.extra, extra, "{}", want.title);
    }
}

#[test]
fn format_v2_pending_request_opens_at_a_fixed_clock() {
    let i = fx::install();
    let vault = vault(&i);
    let now = inside_request_life(&i);

    let pending = vault.pending_enrollments_at(now).unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].id, fx::REQUEST_ID);
    assert_eq!(pending[0].expires, pending_file(&i).1);

    let opened = vault
        .open_enrollment_at(
            &pending,
            &[Zeroizing::new(fx::ENROLLMENT_CODE.to_string())],
            now,
        )
        .unwrap();
    assert_eq!(opened.len(), 1);
    let request = &opened[0];
    assert_eq!(request.id, fx::REQUEST_ID);
    assert_eq!(request.device, fx::ENROLLING_DEVICE);
    assert_eq!(request.method, "passphrase");
    crate::agekey::validate_recipient(&request.pubkey).unwrap();
    assert_eq!(request.expires, pending[0].expires);
    assert_eq!(
        request.expires.duration_since(request.created),
        SignedDuration::from_hours(24),
        "the default lifetime"
    );

    // Past its expiry it is filtered out of the listing.
    let later = pending[0].expires + SignedDuration::from_secs(1);
    assert!(vault.pending_enrollments_at(later).unwrap().is_empty());
}

#[test]
fn format_v2_commit_layout() {
    let i = fx::install();
    let repo = git2::Repository::open(&i.vault).unwrap();

    // What this build writes with is what the fixture was written with.
    assert_eq!(gitrepo::INITIAL_BRANCH, "main");
    assert_eq!(gitrepo::ORIGIN, "origin");
    assert_eq!(gitrepo::AUTHOR_NAME, "gage");
    assert_eq!(gitrepo::AUTHOR_EMAIL, "gage@localhost");
    assert_eq!(crate::vault::INIT_COMMIT_MESSAGE, "gage: initialize vault");
    assert_eq!(
        crate::remotesync::MERGE_COMMIT_MESSAGE,
        "gage: merge origin"
    );

    assert_eq!(repo.head().unwrap().name().ok(), Some("refs/heads/main"));
    let origin = repo.find_remote("origin").unwrap();
    assert_eq!(origin.url().ok(), i.remote.to_str());

    let mut walk = repo.revwalk().unwrap();
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
        .unwrap();
    walk.push_head().unwrap();

    let mut kinds: Vec<&str> = Vec::new();
    for id in walk {
        let commit = repo.find_commit(id.unwrap()).unwrap();
        for sig in [commit.author(), commit.committer()] {
            assert_eq!(sig.name().ok(), Some("gage"));
            assert_eq!(sig.email().ok(), Some("gage@localhost"));
        }
        let message = commit.message().unwrap();
        let tree = commit.tree().unwrap();
        let changed = |commit: &git2::Commit| -> Vec<String> {
            let parent = commit.parent(0).unwrap().tree().unwrap();
            let diff = repo
                .diff_tree_to_tree(Some(&parent), Some(&tree), None)
                .unwrap();
            diff.deltas()
                .map(|d| d.new_file().path().unwrap().to_str().unwrap().to_string())
                .collect()
        };
        match commit.parent_count() {
            0 => {
                assert_eq!(message, "gage: initialize vault");
                let mut paths = Vec::new();
                tree.walk(git2::TreeWalkMode::PreOrder, |dir, e| {
                    if e.kind() == Some(git2::ObjectType::Blob) {
                        paths.push(format!("{dir}{}", e.name().unwrap()));
                    }
                    git2::TreeWalkResult::Ok
                })
                .unwrap();
                paths.sort();
                assert_eq!(
                    paths,
                    [
                        ".age-recipients",
                        ".gage/config.toml",
                        ".gitattributes",
                        ".gitignore"
                    ]
                );
                kinds.push("init");
            }
            2 => {
                assert_eq!(message, "gage: merge origin");
                kinds.push("merge");
            }
            1 if message == "gage: enrollment request" => {
                let changed = changed(&commit);
                assert_eq!(changed.len(), 1, "{changed:?}");
                assert!(
                    changed[0].starts_with(&format!(".gage/pending/{}-", fx::REQUEST_ID)),
                    "{changed:?}"
                );
                kinds.push("enroll");
            }
            1 => {
                // An entry write's message is the entry's id, and it
                // touches that entry's file and nothing else.
                let id = Uuid::try_parse(message)
                    .unwrap_or_else(|_| panic!("unexpected commit message {message:?}"));
                assert_eq!(id.hyphenated().to_string(), message);
                assert_eq!(changed(&commit), [format!("entries/{message}.age")]);
                kinds.push("entry");
            }
            n => panic!("a commit with {n} parents"),
        }
    }
    kinds.sort();
    assert_eq!(
        kinds,
        ["enroll", "entry", "entry", "entry", "init", "merge"],
        "the fixture's history"
    );
}

/// Every key path in a TOML document, sorted: its shape, without values.
fn shape(value: &toml::Value) -> Vec<String> {
    fn walk(prefix: &str, value: &toml::Value, out: &mut Vec<String>) {
        match value {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    walk(&format!("{prefix}.{k}"), v, out);
                }
            }
            toml::Value::Array(items) => {
                for v in items {
                    walk(&format!("{prefix}[]"), v, out);
                }
            }
            _ => out.push(prefix.to_string()),
        }
    }
    let mut out = Vec::new();
    walk("", value, &mut out);
    out.sort();
    out.dedup();
    out
}

/// The writing side: a vault this build creates today has the fixture's
/// layout, so a renamed field or a changed constant can't slip past the
/// reading tests by changing both sides at once.
#[test]
fn format_v2_new_vault_writes_the_same_layout() {
    let i = fx::install();
    let fresh = i.root.path().join("fresh");
    Vault::create(
        &i.dirs,
        &crate::vault::CreateSpec {
            name: "fresh".into(),
            id: vaultconfig::VaultId::new(),
            path: fresh.clone(),
            vault_type: vaultconfig::VaultType::Git,
            method: "passphrase".into(),
            device: fx::DEVICE.into(),
            recipients: vec![fx::DEVICE_PUBKEY.into()],
            extra_recipients: vec![crate::vault::LabelledRecipient {
                device: RECOVERY_DEVICE_LABEL.into(),
                pubkey: fx::RECOVERY_PUBKEY.into(),
            }],
            remote: None,
        },
    )
    .unwrap();

    for name in [".age-recipients", ".gitattributes", ".gitignore"] {
        assert_eq!(read(&fresh.join(name)), read(&i.vault.join(name)), "{name}");
    }
    let toml_of = |path: &Path| -> toml::Value { toml::from_str(&text(path)).unwrap() };
    let config = ".gage/config.toml";
    assert_eq!(
        shape(&toml_of(&fresh.join(config))),
        shape(&toml_of(&i.vault.join(config))),
        "{config}"
    );
    assert_eq!(
        vaultconfig::read(&fresh.join(config))
            .unwrap()
            .vault
            .format_version,
        2
    );

    // The trust cache and global config this build would write.
    let trust_dir = paths::trust_cache_dir(&i.dirs, &fx::vault_id());
    let record: TrustRecord = toml::from_str(&text(&trust_dir.join(TRUST_FILE))).unwrap();
    let written: toml::Value = toml::from_str(&toml::to_string(&record).unwrap()).unwrap();
    assert_eq!(
        shape(&written),
        shape(&toml_of(&trust_dir.join(TRUST_FILE)))
    );
    let global_path = paths::global_config_file(&i.dirs);
    let global = config::read(&global_path).unwrap();
    let written: toml::Value = toml::from_str(&toml::to_string(&global).unwrap()).unwrap();
    assert_eq!(shape(&written), shape(&toml_of(&global_path)));

    // An entry this build writes uses the fixture's keys, in its order.
    let vault = vault(&i);
    let identity = vault
        .unlock(&FakePrompter::new().with_passphrase(fx::PASSPHRASE))
        .unwrap();
    let keys = |yaml: &[u8]| -> Vec<String> {
        std::str::from_utf8(yaml)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with(' ') && l.contains(':'))
            .map(|l| l.split(':').next().unwrap().to_string())
            .collect()
    };
    for id in vault.entry_ids().unwrap() {
        let path = i.vault.join("entries").join(format!("{id}.age"));
        let committed = crypt::decrypt(&read(&path), &identity).unwrap();
        let rewritten = entry::marshal(&entry::unmarshal(&committed).unwrap()).unwrap();
        assert_eq!(keys(&rewritten), keys(&committed), "entry {id}");
    }
}

#[test]
fn format_v2_fixture_is_byte_stable() {
    let dir = fx::fixture_dir();
    let want = text(&dir.join(fx::MANIFEST_FILE));
    assert_eq!(
        fx::manifest(&dir),
        want,
        "tests/fixtures/format-v2 changed. It is never edited: a format change adds format-v3/ beside it (doc/format/README.md)"
    );
}

// The installed copy is what the tests above read, never the fixture.
#[test]
fn format_v2_install_leaves_the_fixture_alone() {
    let before = fx::manifest(&fx::fixture_dir());
    let i = fx::install();
    assert!(!i.vault.starts_with(fx::fixture_dir()));
    assert!(!i.remote.starts_with(fx::fixture_dir()));
    vault(&i)
        .unlock(&FakePrompter::new().with_passphrase(fx::PASSPHRASE))
        .unwrap();
    assert_eq!(fx::manifest(&fx::fixture_dir()), before);
}
