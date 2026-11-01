//! The side of the Swift smoke test (`make ios-smoke`) that isn't the app: a
//! device already in the vault, working through the library as `gage-cli`
//! does.
//!
//! - `smoke_fixture setup <root> <remote>` creates a git vault named
//!   `personal` under `<root>/config`, `<root>/data` and `<root>/state`, made
//!   through the same `Vault::init` an app reaches, with two entries, and
//!   publishes it to the bare repository `<remote>`.
//! - `smoke_fixture approve <root> <code>` approves the request that opens
//!   with `<code>`, as `gage recipient approve --code` does.
//!
//! The passphrase is fixed and public, since the vault holds nothing real;
//! `tests/swift/main.swift` knows it.

use std::path::{Path, PathBuf};

use gage::enrollment::Approval;
use gage::entry::NewEntry;
use gage::prompter::FakePrompter;
use gage::storage::config;
use gage::storage::paths;
use gage::storage::xdgpaths::Dirs;
use gage::{InitSpec, UnlockMethod, Vault, VaultType};
use zeroize::Zeroizing;

const PASSPHRASE: &str = "correct horse battery staple";
const VAULT: &str = "personal";

fn dirs(root: &Path) -> Dirs {
    Dirs {
        config: root.join("config"),
        data: root.join("data"),
        state: root.join("state"),
    }
}

fn unlocking() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

fn setup(root: &Path, remote: &str) {
    let made = Vault::init(
        &dirs(root),
        &InitSpec {
            name: VAULT.into(),
            path: root.join(VAULT),
            vault_type: VaultType::Git,
            method: UnlockMethod::Passphrase,
            device: "smoke-device".into(),
            recipients: Vec::new(),
            recovery_key: None,
            remote: Some(remote.into()),
        },
        &unlocking(),
    )
    .expect("creating the vault");

    let identity = made
        .vault
        .unlock(&unlocking())
        .expect("unlocking the new vault");
    for (title, fields) in [("alpha", vec![]), ("bravo", vec![("user", "field-secret")])] {
        made.vault
            .insert(
                &identity,
                NewEntry {
                    title: title.into(),
                    description: format!("about {title}"),
                    value: format!("secret-{title}"),
                    fields: fields
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    ..NewEntry::default()
                },
                true,
                &FakePrompter::new(),
            )
            .expect("inserting an entry");
    }
    made.vault.push().expect("publishing the vault");
}

fn approve(root: &Path, code: &str) {
    let dirs = dirs(root);
    let global = config::load(&paths::global_config_file(&dirs)).expect("reading the registry");
    let entry = global.entry(VAULT).expect("finding the vault");
    let vault = Vault::from_registration(&dirs, VAULT, entry).expect("opening the vault");

    vault.pull().expect("fetching the request");
    let codes = [Zeroizing::new(code.to_string())];
    let pending = vault.pending_enrollments().expect("listing requests");
    let approvals: Vec<Approval> = vault
        .open_enrollment(&pending, &codes)
        .expect("opening the request with the code")
        .into_iter()
        .map(Approval::new)
        .collect();
    let identity = vault.unlock(&unlocking()).expect("unlocking to approve");
    vault
        .approve_enrollments(&approvals, &codes, &identity, &FakePrompter::new())
        .expect("approving the request");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["setup", root, remote] => setup(&PathBuf::from(root), remote),
        ["approve", root, code] => approve(&PathBuf::from(root), code),
        _ => panic!("usage: smoke_fixture setup <root> <remote> | approve <root> <code>"),
    }
}
